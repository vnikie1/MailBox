import { expect, test, type Page } from '@playwright/test'

/**
 * What is drawn on top of what.
 *
 * ## Why this exists
 *
 * Reported from using the app: two full-height vertical lines ran down the attachment preview
 * modal. They were the **pane dividers**, drawn on top of it. `PaneDivider` set `z-index: 1`
 * and the sheet's overlay set nothing at all, and a positive z-index beats `auto` however late
 * in the DOM the portal is mounted — so every modal in the app was under the window chrome.
 *
 * The bug was invisible to every other kind of test: the markup was right, the styles were
 * each individually right, and it only showed as two pale lines on a screenshot. What makes it
 * catchable is that all these layers share the root stacking context, so their computed
 * z-indexes compare directly.
 */

/** The computed z-index of the first element matching, as a number. `auto` reads as 0. */
async function layer(page: Page, selector: string): Promise<number> {
  return page
    .locator(selector)
    .first()
    .evaluate((element) => {
      const value = getComputedStyle(element).zIndex
      return value === 'auto' ? 0 : Number(value)
    })
}

test.describe('stacking order', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.getByRole('tree', { name: 'Mailboxes' })).toBeVisible()
  })

  test('a modal is drawn above the window chrome, not under it', async ({ page }) => {
    // Open a real sheet rather than asserting the stylesheet: what matters is the value the
    // browser computes for the element that is actually on screen.
    await page
      .getByRole('group', { name: 'Northgate' })
      .getByRole('treeitem')
      .filter({ has: page.getByText('Clients', { exact: true }) })
      .first()
      .click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Get Account Info' }).click()
    await expect(page.getByRole('dialog', { name: 'Account Information' })).toBeVisible()

    const divider = await layer(page, '[class*="divider"]')
    const overlay = await layer(page, '[class*="overlay"]')

    expect(
      overlay,
      `the modal overlay is at z-index ${String(overlay)} and the pane divider at ${String(
        divider,
      )} — the divider will be drawn down the middle of the modal`,
    ).toBeGreaterThan(divider)
  })

  test('a menu is drawn above the window chrome too', async ({ page }) => {
    // Menus only ever cleared the dividers by being later in the DOM, which is not a rule
    // anyone stated and would break the moment a divider moved.
    await page.getByRole('listbox', { name: 'Messages' }).getByRole('option').first().click({
      button: 'right',
    })
    await expect(page.getByRole('menu', { name: 'Message actions' })).toBeVisible()

    const divider = await layer(page, '[class*="divider"]')
    const menu = await layer(page, '[class*="menu"]')

    expect(menu).toBeGreaterThan(divider)
  })
})
