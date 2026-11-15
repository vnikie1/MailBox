import { expect, test, type Page } from '@playwright/test'

/**
 * The one-pane layout, below 700px. docs/01 §1.
 *
 * M38 in the 2026-11-14 audit: picking a mailbox jumped straight into its first message and marked
 * it read, because the reader opened for any new selection, including the one the list makes for
 * itself; and Delete left the deleted message on screen, because the list, which moves the
 * selection on, is not mounted while the reader is up.
 */

const NARROW = { width: 640, height: 900 }

function reader(page: Page) {
  return page.getByRole('button', { name: 'Messages' })
}

test.describe('one pane', () => {
  test.use({ viewport: NARROW })

  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()
  })

  test('picking a mailbox shows its list, not its first message', async ({ page }) => {
    await page.getByRole('button', { name: 'Mailboxes' }).click()
    await page
      .getByRole('treeitem')
      .filter({ hasText: /^Archive/ })
      .first()
      .click()

    // The list is up, with its first row picked for it, and the reader has not opened over it.
    await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()
    await expect(page.locator('[role="option"][aria-selected="true"]')).toHaveCount(1)
    await expect(reader(page)).toHaveCount(0)
  })

  test('the row the list picked opens when it is clicked', async ({ page }) => {
    // The selection does not move, so the reader has to open on the click itself.
    await page.locator('[role="option"][aria-selected="true"]').click()
    await expect(reader(page)).toBeVisible()
  })

  test('deleting from the reader goes back to the list', async ({ page }) => {
    const first = page.getByRole('option').first()
    const id = await first.getAttribute('data-message-id')
    await first.click()
    await expect(reader(page)).toBeVisible()

    await page.getByRole('button', { name: 'Delete' }).first().click()

    await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()
    await expect(reader(page)).toHaveCount(0)
    await expect(page.locator(`[data-message-id="${id ?? ''}"]`)).toHaveCount(0)
  })
})
