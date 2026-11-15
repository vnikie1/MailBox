import { expect, test, type Page } from '@playwright/test'

/**
 * The message list after a density change.
 *
 * L19 in the 2026-11-14 audit: the list measured its row height on the render the setting
 * changed, before the new density's tokens were on the page, so every row went on being laid out
 * in a slot sized for the previous density until a reload.
 *
 * So the test compares against a reload: the slot a row gets straight after choosing a density
 * must be the slot it gets when the page starts at that density.
 */

/** The height of the first row's slot in the virtual list, in pixels. */
function slotHeight(page: Page) {
  return page.evaluate(() => {
    const row = document.querySelector<HTMLElement>('[role="option"]')
    const slot = row?.closest<HTMLElement>('[style*="translateY"]')
    return slot ? Math.round(slot.getBoundingClientRect().height) : null
  })
}

async function ready(page: Page) {
  await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()
  await expect(page.getByRole('option').first()).toBeVisible()
}

async function chooseDensity(page: Page, density: string) {
  await page.getByRole('button', { name: 'List options' }).click()
  await page.getByRole('menuitemcheckbox', { name: density }).click()
  await expect(page.locator('html')).toHaveAttribute('data-density', density.toLowerCase())
}

test('rows are laid out at the new density as soon as it is chosen', async ({ page }) => {
  await page.goto('/')
  await ready(page)

  const seen = new Set<number | null>([await slotHeight(page)])

  for (const density of ['Comfortable', 'Compact']) {
    await chooseDensity(page, density)
    // Given a moment, so a list that catches up late is not mistaken for one that never does.
    await page.waitForTimeout(300)
    const chosen = await slotHeight(page)

    await page.reload()
    await ready(page)
    const reloaded = await slotHeight(page)

    expect(chosen, `${density}: after choosing it, against after a reload`).toBe(reloaded)
    seen.add(reloaded)
  }

  // Three densities, three heights: otherwise the comparison above proves nothing.
  expect(seen.size).toBe(3)
})
