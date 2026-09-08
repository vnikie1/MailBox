import { expect, test, type Page } from '@playwright/test'

/**
 * How a message with attachments reads. docs/01 §5.
 *
 * The card replaced the 44px row docs/02 §6.8 specifies — a deviation recorded in CHANGELOG.md
 * — because the row made every attachment look identical and saving one meant opening the
 * preview to find the button.
 *
 * The saving itself cannot be driven here: it opens the system file dialog, which lives outside
 * the page and outside Playwright. What these cover is that the controls exist, are reachable,
 * and name the right things.
 */

/** The first message in the list carrying attachments. */
async function openOneWithAttachments(page: Page) {
  const withClip = page.getByRole('option').filter({ has: page.locator('[class*="icon"]') })
  const count = await withClip.count()

  for (let index = 0; index < count; index++) {
    await withClip.nth(index).click()
    if ((await page.locator('[class*="tile"]').count()) > 0) return
  }

  throw new Error('no message in the seeded mailbox has an attachment')
}

test.describe('attachments', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()
    await openOneWithAttachments(page)
  })

  test('each one is a card naming its kind, its size and a way to save it', async ({ page }) => {
    const tile = page.locator('[class*="tile"]').first()
    await expect(tile).toBeVisible()

    // The card is the preview trigger and says what it is, so a screen reader gets the kind
    // and the size rather than only a hex filename.
    await expect(tile.getByRole('button', { name: /^Preview / })).toBeVisible()
    await expect(tile.getByRole('button', { name: /^Save / })).toBeVisible()
  })

  test('the paperclip menu offers Save All and every file by name', async ({ page }) => {
    await page
      .getByRole('button', { name: /attachments?$/ })
      .first()
      .click()

    const menu = page.getByRole('menu', { name: 'Attachments' })
    await expect(menu).toBeVisible()

    // The total, not a count: "Save All (2.4 MB)" tells the user what they are about to write.
    await expect(menu.getByRole('menuitem', { name: /^Save All (.+)…$/ })).toBeVisible()

    // And one row per file, so a single attachment can be opened without going via the card.
    const files = await page.locator('[class*="tile"]').count()
    await expect(menu.getByRole('menuitem')).toHaveCount(files + 1)
  })

  test('a message with no attachments has no paperclip', async ({ page }) => {
    // Standing rule 18: the control exists only where it can do something.
    //
    // Found by opening rows until one shows no card, rather than by guessing from the row's
    // markup — the paperclip in a list row shares a class with every other icon in it, so a
    // selector was quietly matching everything and skipping the test.
    const rows = page.getByRole('option')
    const total = await rows.count()
    let found = false

    for (let index = 0; index < total; index++) {
      await rows.nth(index).click()
      if ((await page.locator('[class*="tile"]').count()) === 0) {
        found = true
        break
      }
    }

    expect(found, 'every seeded message has an attachment, so this cannot be checked').toBe(true)
    await expect(page.getByRole('button', { name: /attachments?$/ })).toHaveCount(0)
  })
})
