import { expect, test, type Locator, type Page } from '@playwright/test'

/**
 * Dragging mail onto a mailbox, driven as a real drag. docs/01 §3.
 *
 * The unit tests beside this (`tests/unit/messageDrag.test.ts`) pin the rules that decide
 * where a drag may land. They cannot pin that a drag *happens*: HTML5 drag and drop is a
 * browser gesture, not a function call, and the parts most likely to break — the row being
 * `draggable` at all, the payload surviving the round trip, the sidebar's `dragover` refusing
 * by *not* calling `preventDefault` — only exist while a real pointer is being dragged. So
 * this drags with the mouse and checks the mail actually moved.
 *
 * Counts come from the list header rather than from counting rows, because the list is
 * virtualised: counting `option` elements counts what is on screen, which is a different
 * number and changes with the window size.
 */

/** The seeded accounts and their folders. `src/mock/browserStore.ts`. */
const NORTHGATE = 'Northgate'
const ICLOUD = 'iCloud'

/** The list header's "N messages, M unread". A total, not what is rendered. */
function subtitle(page: Page): Locator {
  return page.locator('h1 + p').first()
}

async function messageCount(page: Page): Promise<number> {
  const text = (await subtitle(page).textContent()) ?? ''
  return Number(/^(\d+)/.exec(text)?.[1] ?? Number.NaN)
}

/**
 * A mailbox row, found by its label.
 *
 * Matched on the label element rather than on the row's text, because a row's text runs the
 * label straight into its unread badge: the Northgate inbox reads "Inbox184", where "Inbox"
 * has no word boundary after it and a `^Inbox\b` match finds nothing at all.
 *
 * Scoped to the account's section because the names repeat — every account has an Inbox, and
 * Favourites has an "All Inboxes" above them all.
 */
function mailboxRow(page: Page, account: string, mailbox: string): Locator {
  return page
    .getByRole('group', { name: account })
    .getByRole('treeitem')
    .filter({ has: page.getByText(mailbox, { exact: true }) })
    .first()
}

async function openMailbox(page: Page, account: string, mailbox: string): Promise<void> {
  await mailboxRow(page, account, mailbox).click()
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(mailbox)
}

test.describe('dragging messages onto a mailbox', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.getByRole('tree', { name: 'Mailboxes' })).toBeVisible()
    await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()
  })

  test('moves a message into another folder of the same account', async ({ page }) => {
    await openMailbox(page, NORTHGATE, 'Clients')
    const clientsBefore = await messageCount(page)

    await openMailbox(page, NORTHGATE, 'Inbox')
    const inboxBefore = await messageCount(page)

    await page
      .getByRole('option')
      .first()
      .dragTo(mailboxRow(page, NORTHGATE, 'Clients'))

    // The source loses it...
    await expect.poll(async () => messageCount(page)).toBe(inboxBefore - 1)

    // ...and the destination gains it. Both ends, because a move that corrected only one
    // would leave a count nobody can explain — the same reason `move_to` recounts both.
    await openMailbox(page, NORTHGATE, 'Clients')
    await expect.poll(async () => messageCount(page)).toBe(clientsBefore + 1)
  })

  /**
   * The highlight the user aims at, held steady across a row.
   *
   * Reported from using the app: "i am not able to see the label clearly underneath while
   * dragging and dropping the mail". Two separate causes, both reproduced here rather than
   * argued about:
   *
   *  - a `dragleave` fires on the row every time the pointer crosses onto one of its own
   *    children — chevron, icon, label, badge — and clearing on all of them made the fill
   *    strobe, measured at runs of three and four dark frames while gliding along one row;
   *  - `dragover` does not fire when the pointer enters a row and stops, so approaching from
   *    the right and resting left the row dark indefinitely.
   *
   * Driven with raw drag events rather than `dragTo`, because `dragTo` moves to a row's
   * centre in one step and would sail straight over the child boundaries that break it.
   */
  test('keeps the drop target lit while the pointer crosses the row it is over', async ({
    page,
  }) => {
    const source = page.getByRole('option').first()
    const target = mailboxRow(page, NORTHGATE, 'Clients')
    const label = target.getByText('Clients', { exact: true })

    await source.hover()
    await page.mouse.down()

    // Onto the row, then onto the label inside it. It is that second move that used to put
    // the highlight out, because the leave from the row arrives with nothing to restore it.
    await target.hover()
    await expect(target).toHaveClass(/dropTarget/)

    await label.hover()
    await expect(target).toHaveClass(/dropTarget/)

    // And still lit after resting there, which is the case with no further `dragover`.
    await page.waitForTimeout(700)
    await expect(target).toHaveClass(/dropTarget/)

    await page.mouse.up()
  })

  test('leaves no drag deck behind in the document', async ({ page }) => {
    // The deck is mounted so Blink can photograph it and removed a task later. If either the
    // timer or the `dragend` listener regressed, a card would be left sitting at the window's
    // top-left corner — and every committed screenshot baseline would start failing.
    await page
      .getByRole('option')
      .first()
      .dragTo(mailboxRow(page, NORTHGATE, 'Travel'))

    // `body >` matters: the reader draws a deck of its own for a multi-selection, and it
    // is a child of the pane. This one is appended straight to the body, which is the only
    // thing that distinguishes a leaked drag image from the one that is meant to be there.
    await expect(page.locator('body > [class*="deck"]')).toHaveCount(0)
  })

  test('refuses a folder belonging to a different account', async ({ page }) => {
    // The core refuses this outright — `msg_move` answers `crossAccount` — so the sidebar
    // must not offer it. A row that accepts a drop and then reports a failure is worse than
    // one that never lights up.
    await openMailbox(page, ICLOUD, 'Family')
    const familyBefore = await messageCount(page)

    await openMailbox(page, NORTHGATE, 'Inbox')
    const inboxBefore = await messageCount(page)

    await page
      .getByRole('option')
      .first()
      .dragTo(mailboxRow(page, ICLOUD, 'Family'))

    // Nothing moved, and nothing was said, because nothing was attempted.
    expect(await messageCount(page)).toBe(inboxBefore)

    await openMailbox(page, ICLOUD, 'Family')
    expect(await messageCount(page)).toBe(familyBefore)
  })

  test('refuses a unified row, which has no single destination', async ({ page }) => {
    await openMailbox(page, NORTHGATE, 'Inbox')
    const inboxBefore = await messageCount(page)

    const allInboxes = page
      .getByRole('group', { name: 'Favourites' })
      .getByRole('treeitem')
      .filter({ has: page.getByText('All Inboxes', { exact: true }) })
      .first()

    await page.getByRole('option').first().dragTo(allInboxes)

    expect(await messageCount(page)).toBe(inboxBefore)
  })

  test('moves a whole selection at once', async ({ page }) => {
    await openMailbox(page, NORTHGATE, 'Contracts')
    const contractsBefore = await messageCount(page)

    await openMailbox(page, NORTHGATE, 'Inbox')
    const inboxBefore = await messageCount(page)

    // Ctrl-click adds to the selection, which is what `onDragStart` reads to decide it is
    // dragging the selection rather than the one row under the pointer.
    await page.getByRole('option').nth(0).click()
    await page
      .getByRole('option')
      .nth(1)
      .click({ modifiers: ['Control'] })
    await page
      .getByRole('option')
      .nth(2)
      .click({ modifiers: ['Control'] })
    await expect(page.locator('[role="option"][aria-selected="true"]')).toHaveCount(3)

    await page
      .getByRole('option')
      .nth(1)
      .dragTo(mailboxRow(page, NORTHGATE, 'Contracts'))

    await expect.poll(async () => messageCount(page)).toBe(inboxBefore - 3)

    await openMailbox(page, NORTHGATE, 'Contracts')
    await expect.poll(async () => messageCount(page)).toBe(contractsBefore + 3)
  })

  test('dragging an unselected row moves that row, not the selection', async ({ page }) => {
    // A stray drag must not move nine messages somebody selected earlier — what every list on
    // both platforms does, and what the list's `onDragStart` is written for.
    await openMailbox(page, NORTHGATE, 'Inbox')
    const inboxBefore = await messageCount(page)

    await page.getByRole('option').nth(0).click()
    await page
      .getByRole('option')
      .nth(1)
      .click({ modifiers: ['Control'] })
    await expect(page.locator('[role="option"][aria-selected="true"]')).toHaveCount(2)

    await page
      .getByRole('option')
      .nth(4)
      .dragTo(mailboxRow(page, NORTHGATE, 'Travel'))

    await expect.poll(async () => messageCount(page)).toBe(inboxBefore - 1)
  })
})
