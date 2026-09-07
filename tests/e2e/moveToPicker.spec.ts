import { expect, test, type Page } from '@playwright/test'

/**
 * The Move to… picker, which is the keyboard route to the operation drag and drop performs.
 *
 * It had the same fault the sidebar's drop targets had: it listed every folder of every
 * account, and the core refused the impossible ones afterwards with `crossAccount`. Fixing one
 * and not the other would leave the app disagreeing with itself about where mail can go
 * depending on which hand you used.
 */

const NORTHGATE = 'Northgate'

async function ready(page: Page) {
  await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()
}

/** Scoped to the account's section, because every account has an Inbox. */
async function openMailbox(page: Page, account: string, mailbox: string) {
  await page
    .getByRole('group', { name: account })
    .getByRole('treeitem')
    .filter({ has: page.getByText(mailbox, { exact: true }) })
    .first()
    .click()

  await expect(page.getByRole('heading', { level: 1 })).toHaveText(mailbox)
}

async function openPicker(page: Page) {
  await page.getByRole('button', { name: 'Move to', exact: true }).click()
  await expect(page.getByRole('textbox')).toBeVisible()
}

test.describe('the Move to picker', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await ready(page)
  })

  test('offers folders from the selected message’s own account', async ({ page }) => {
    await openMailbox(page, NORTHGATE, 'Inbox')
    await page.getByRole('option').first().click()
    await openPicker(page)

    // Northgate's own folders are there...
    await expect(page.getByRole('button', { name: /^Clients/ })).toBeVisible()
    await expect(page.getByRole('button', { name: /^Contracts/ })).toBeVisible()
  })

  test('does not offer another account’s folders', async ({ page }) => {
    await openMailbox(page, NORTHGATE, 'Inbox')
    await page.getByRole('option').first().click()
    await openPicker(page)

    // ...and iCloud's are not, however hard they are searched for. `Family` and `Shopping`
    // belong to iCloud; a Northgate message cannot go to either.
    await expect(page.getByRole('button', { name: /^Family/ })).toHaveCount(0)
    await expect(page.getByRole('button', { name: /^Shopping/ })).toHaveCount(0)

    await page.getByRole('textbox').fill('Family')
    await expect(page.getByText('No mailbox matches that.')).toBeVisible()
  })

  test('says why when the selection spans two accounts', async ({ page }) => {
    // Reachable by accident from a unified mailbox, which lists every account at once.
    await page
      .getByRole('group', { name: 'Favourites' })
      .getByRole('treeitem')
      .filter({ has: page.getByText('All Inboxes', { exact: true }) })
      .first()
      .click()
    await ready(page)

    // Ctrl-click down the list until the selection covers two accounts. The unified list is
    // interleaved by date, so a handful of rows is enough.
    await page.getByRole('option').nth(0).click()
    for (let row = 1; row < 8; row++) {
      await page
        .getByRole('option')
        .nth(row)
        .click({ modifiers: ['Control'] })
    }

    await openPicker(page)

    await expect(
      page.getByText('These messages are in different accounts, and mail cannot move between'),
    ).toBeVisible()
  })

  test('moves the message when a folder is chosen', async ({ page }) => {
    await openMailbox(page, NORTHGATE, 'Receipts')
    const receiptsBefore = Number(
      /^(\d+)/.exec((await page.locator('h1 + p').first().textContent()) ?? '')?.[1] ?? Number.NaN,
    )

    await openMailbox(page, NORTHGATE, 'Inbox')
    await page.getByRole('option').first().click()
    await openPicker(page)
    await page.getByRole('button', { name: /^Receipts/ }).click()

    await openMailbox(page, NORTHGATE, 'Receipts')
    await expect
      .poll(async () =>
        Number(
          /^(\d+)/.exec((await page.locator('h1 + p').first().textContent()) ?? '')?.[1] ??
            Number.NaN,
        ),
      )
      .toBe(receiptsBefore + 1)
  })
})
