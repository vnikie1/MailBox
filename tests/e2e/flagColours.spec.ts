import { expect, test, type Page } from '@playwright/test'

/**
 * Setting a flag colour, end to end. docs/01 §8.
 *
 * This exists because the feature was complete and unreachable. `FlagMenu` was written,
 * exported, and rendered nowhere; `flagSet` had exactly one caller, which was that component.
 * So the core could store a colour, the sidebar offered seven colours to filter by, and the
 * message row knew how to draw one — and there was no way to put a colour on a message. Every
 * piece passed its own test and the feature did not exist.
 *
 * The test therefore goes through the toolbar rather than calling anything: what broke was the
 * wiring between the parts, which is the one thing a unit test of any single part cannot see.
 */

async function ready(page: Page) {
  await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()
}

/** The flag on a row, if it has one. The colour lives in `data-flag`. */
function flagOf(page: Page, row: number) {
  return page.getByRole('option').nth(row).locator('[data-flag]')
}

test.describe('flag colours', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await ready(page)
  })

  test('the toolbar flag button offers all seven colours', async ({ page }) => {
    await page.getByRole('option').first().click()
    await page.getByRole('button', { name: 'Flag', exact: true }).click()

    for (const colour of ['Red', 'Orange', 'Yellow', 'Green', 'Blue', 'Purple', 'Gray']) {
      await expect(page.getByRole('menuitemcheckbox', { name: colour })).toBeVisible()
    }
  })

  test('choosing a colour puts it on the message', async ({ page }) => {
    const row = page.getByRole('option').nth(1)
    await row.click()

    await page.getByRole('button', { name: 'Flag', exact: true }).click()
    await page.getByRole('menuitemcheckbox', { name: 'Purple' }).click()

    // The row now carries a purple flag, drawn in the purple token rather than the accent.
    await expect(flagOf(page, 1)).toHaveAttribute('data-flag', 'purple')

    const purple = await page.evaluate(() => {
      const value = getComputedStyle(document.documentElement)
        .getPropertyValue('--flag-purple')
        .trim()
      const probe = document.createElement('span')
      probe.style.color = value
      document.body.append(probe)
      const resolved = getComputedStyle(probe).color
      probe.remove()
      return resolved
    })

    await expect(flagOf(page, 1)).toHaveCSS('color', purple)
  })

  test('the menu ticks the colour the message already carries', async ({ page }) => {
    const row = page.getByRole('option').nth(2)
    await row.click()

    await page.getByRole('button', { name: 'Flag', exact: true }).click()
    await page.getByRole('menuitemcheckbox', { name: 'Green' }).click()
    await expect(flagOf(page, 2)).toHaveAttribute('data-flag', 'green')

    await page.getByRole('button', { name: 'Flag', exact: true }).click()
    await expect(page.getByRole('menuitemcheckbox', { name: 'Green' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    await expect(page.getByRole('menuitemcheckbox', { name: 'Red' })).toHaveAttribute(
      'aria-checked',
      'false',
    )
  })

  test('clearing takes the flag off again', async ({ page }) => {
    const row = page.getByRole('option').nth(3)
    await row.click()

    await page.getByRole('button', { name: 'Flag', exact: true }).click()
    await page.getByRole('menuitemcheckbox', { name: 'Blue' }).click()
    await expect(flagOf(page, 3)).toHaveAttribute('data-flag', 'blue')

    await page.getByRole('button', { name: 'Flag', exact: true }).click()
    await page.getByRole('menuitem', { name: 'Clear Flag' }).click()

    // Colour gone, and so is the flag: the core's own note is that colour rides on a plain
    // `\Flagged`, so a message with no colour is not flagged either.
    await expect(flagOf(page, 3)).toHaveCount(0)
  })
})
