import { expect, test, type Page } from '@playwright/test'

/**
 * The right-click menus. docs/01 §5.
 *
 * Until now nothing in the app answered a right-click, so Edge's own menu — Reload, Save as,
 * Inspect — came up over the mail. That menu is drawn by the browser and is invisible to
 * Playwright, so what these tests can check is the half that is ours: that the app's menu
 * appears on a row, that it names the right message, that it acts on the selection rather than
 * on whatever the pointer happened to be over, and that the empty space below the rows opens
 * nothing.
 *
 * What they cannot check is that Edge's menu no longer appears. No automated test on any
 * platform can see it. That is verified by hand in the running app.
 */

function menu(page: Page) {
  return page.getByRole('menu', { name: 'Message actions' })
}

async function rightClickRow(page: Page, index: number) {
  await page.getByRole('option').nth(index).click({ button: 'right' })
  await expect(menu(page)).toBeVisible()
}

test.describe('the message context menu', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()
  })

  test('opens on a message row', async ({ page }) => {
    await rightClickRow(page, 0)

    // The rows Mail puts here, in Mail's order.
    await expect(menu(page).getByRole('menuitem', { name: 'Reply', exact: true })).toBeVisible()
    await expect(menu(page).getByRole('menuitem', { name: 'Archive' })).toBeVisible()
    await expect(menu(page).getByRole('menuitem', { name: 'Apply Rules' })).toBeVisible()
  })

  test('carries the flag colours as a row of swatches, not a submenu', async ({ page }) => {
    await rightClickRow(page, 0)

    // docs/01 §8 — a colour is chosen by eye. Seven colours and a clear cell, on one row.
    const flags = menu(page).getByRole('group', { name: 'Flag:' })
    await expect(flags).toBeVisible()
    await expect(flags.getByRole('menuitemradio')).toHaveCount(8)
  })

  test('right-clicking a row outside the selection selects it first', async ({ page }) => {
    // Otherwise the menu acts on something the pointer is nowhere near, which is how a user
    // deletes the wrong message.
    await page.getByRole('option').nth(0).click()
    await rightClickRow(page, 2)

    await expect(page.getByRole('option').nth(2)).toHaveAttribute('aria-selected', 'true')
    await expect(page.getByRole('option').nth(0)).toHaveAttribute('aria-selected', 'false')
  })

  test('right-clicking inside a multi-selection keeps it', async ({ page }) => {
    // The whole point of a menu on a selection: it has to act on all nine, not collapse to one.
    await page.getByRole('option').nth(0).click()
    await page
      .getByRole('option')
      .nth(2)
      .click({ modifiers: ['Shift'] })
    await rightClickRow(page, 1)

    for (const index of [0, 1, 2]) {
      await expect(page.getByRole('option').nth(index)).toHaveAttribute('aria-selected', 'true')
    }

    // Reply is for one message. With three selected it must not offer to reply to them.
    await expect(menu(page).getByRole('menuitem', { name: 'Reply', exact: true })).toBeDisabled()
  })

  test('does nothing on a part of the list that is not a message', async ({ page }) => {
    // A date header is inside the listbox and is not a row. A menu opening here would act on
    // whatever was selected, which may be nowhere near the pointer.
    //
    // The header rather than the space below the last row: a full mailbox has no such space,
    // so that version of this test would pass by never finding anywhere to click.
    // The in-flow header, by class. `getByText` finds the sticky overlay above it first, which
    // is aria-hidden and never settles as a click target.
    await page
      .getByRole('listbox', { name: 'Messages' })
      .locator('[class*="sectionHeader"]')
      .first()
      .click({ button: 'right' })

    await expect(menu(page)).toBeHidden()
  })

  test('marks a message read from the menu', async ({ page }) => {
    // End to end rather than a click on a mock: the label says what it will do, and doing it
    // has to change the row.
    const unread = page.getByRole('option').filter({ hasText: /.*/ }).first()
    await unread.click({ button: 'right' })
    await expect(menu(page)).toBeVisible()

    const item = menu(page).getByRole('menuitem', { name: /^Mark as (Read|Unread)$/ })
    const wasRead = (await item.textContent()) === 'Mark as Unread'
    await item.click()

    await expect(menu(page)).toBeHidden()
    await unread.click({ button: 'right' })
    await expect(
      menu(page).getByRole('menuitem', { name: wasRead ? 'Mark as Read' : 'Mark as Unread' }),
    ).toBeVisible()
  })
})

test.describe('the mailbox context menu', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.getByRole('tree', { name: 'Mailboxes' })).toBeVisible()
  })

  function mailboxMenu(page: Page) {
    return page.getByRole('menu', { name: 'Mailbox actions' })
  }

  function folderRow(page: Page, account: string, mailbox: string) {
    return page
      .getByRole('group', { name: account })
      .getByRole('treeitem')
      .filter({ has: page.getByText(mailbox, { exact: true }) })
      .first()
  }

  test('opens on a real mailbox', async ({ page }) => {
    await folderRow(page, 'Northgate', 'Clients').click({ button: 'right' })

    await expect(mailboxMenu(page)).toBeVisible()
    await expect(mailboxMenu(page).getByRole('menuitem', { name: 'Export Mailbox…' })).toBeVisible()
    await expect(
      mailboxMenu(page).getByRole('menuitem', { name: 'Get Account Info' }),
    ).toBeVisible()
  })

  test('names the account in the rows that act on it', async ({ page }) => {
    // Quoted the way Mail writes it, so a menu opened on the wrong account is obvious before
    // anything happens rather than after.
    await folderRow(page, 'Northgate', 'Clients').click({ button: 'right' })

    await expect(mailboxMenu(page).getByRole('menuitem', { name: /^Synchronise/ })).toBeVisible()
    await expect(mailboxMenu(page).getByRole('menuitem', { name: /^Edit/ })).toBeVisible()
  })

  test('refuses a row that is not one real mailbox', async ({ page }) => {
    // "All Inboxes" spans accounts, so every row of this menu would have to pick one. Better
    // to open nothing than to open a menu that acts on an account the user did not name.
    await page
      .getByRole('treeitem')
      .filter({ has: page.getByText('All Inboxes', { exact: true }) })
      .first()
      .click({ button: 'right' })

    await expect(mailboxMenu(page)).toBeHidden()
  })

  test('does not change which mailbox is open', async ({ page }) => {
    // Unlike the message list, a right-click here must not select: loading a mailbox is real
    // work and a visible change, far too much to do on the way to a menu that may be closed.
    const heading = page.getByRole('heading', { level: 1 })
    const before = await heading.textContent()

    await folderRow(page, 'Northgate', 'Travel').click({ button: 'right' })
    await expect(mailboxMenu(page)).toBeVisible()

    await expect(heading).toHaveText(before ?? '')
  })

  test('shows the account information sheet', async ({ page }) => {
    await folderRow(page, 'Northgate', 'Clients').click({ button: 'right' })
    await mailboxMenu(page).getByRole('menuitem', { name: 'Get Account Info' }).click()

    const sheet = page.getByRole('dialog', { name: 'Account Information' })
    await expect(sheet).toBeVisible()

    // Only what the app actually knows. No quota row: there is no IMAP QUOTA support and
    // nothing sums message sizes, so drawing one would be a claim it cannot back.
    await expect(sheet.getByText('Incoming (IMAP)')).toBeVisible()
    await expect(sheet.getByText('Quota')).toHaveCount(0)
  })
})
