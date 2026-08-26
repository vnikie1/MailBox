import { expect, test, type Page } from '@playwright/test'

/**
 * The component gallery. Phase 1 exit gate.
 *
 * The screenshot at the end is the visual baseline: it captures every primitive in every
 * state, in both themes at once, because the gallery renders its specimens twice under
 * forced `[data-theme]` subtrees. A change that alters any primitive's appearance has to
 * be acknowledged by updating this snapshot, which is the point.
 *
 * Everything before it is behaviour a snapshot cannot check — that the toggles actually
 * remap the token layer rather than merely re-rendering.
 */

const SETTINGS_KEY = 'halcyon.settings.display'

/**
 * Pin the stored preferences before the app boots. main.tsx reads them synchronously to
 * set the pre-paint theme, so leaving them to whatever a previous run persisted would
 * make the baseline depend on test order.
 */
async function pinPreferences(page: Page, theme: 'system' | 'light' | 'dark' = 'light') {
  await page.addInitScript(
    ([key, value]) => {
      window.localStorage.setItem(
        key,
        JSON.stringify({
          state: { theme: value, density: 'default', transparency: 'system' },
          version: 0,
        }),
      )
    },
    [SETTINGS_KEY, theme] as const,
  )
}

test.describe('component gallery', () => {
  test.beforeEach(async ({ page }) => {
    await pinPreferences(page)
  })

  test('renders every primitive in both themes', async ({ page }) => {
    await page.goto('/dev/gallery')

    // By landmark, not by [data-theme] — <html> carries that attribute too, so a raw
    // attribute selector matches the whole document as well as the column.
    const light = page.getByRole('region', { name: 'Light' })
    const dark = page.getByRole('region', { name: 'Dark' })
    await expect(light).toBeVisible()
    await expect(dark).toBeVisible()

    // One section per primitive. If a primitive is added without a specimen, this fails.
    const expected = [
      'Button',
      'IconButton',
      'TextField',
      'TokenField',
      'Chip',
      'Avatar',
      'Badge',
      'Divider',
      'Skeleton',
      'Menu',
      'Popover',
      'Sheet',
      'Toast',
      'ScrollArea',
    ]

    for (const name of expected) {
      await expect(
        light.getByRole('heading', { name, exact: true }),
        `${name} has no specimen`,
      ).toBeVisible()
      await expect(dark.getByRole('heading', { name, exact: true })).toBeVisible()
    }
  })

  test('the theme toggle remaps the tokens without a reload', async ({ page }) => {
    await page.goto('/dev/gallery')
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')

    await page.getByRole('group', { name: 'Theme' }).getByRole('button', { name: 'Dark' }).click()
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')

    // The header is a semantic-token surface, so its colour is proof the remap reached
    // the cascade rather than only the attribute.
    const headerColour = await page
      .locator('header')
      .evaluate((el) => getComputedStyle(el).backgroundColor)
    expect(headerColour).not.toBe('rgb(255, 255, 255)')

    await page.getByRole('group', { name: 'Theme' }).getByRole('button', { name: 'Light' }).click()
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')
  })

  test('the density toggle resizes only what docs/02 §7 says it may', async ({ page }) => {
    await page.goto('/dev/gallery')

    const fontSize = () =>
      page
        .locator('html')
        .evaluate((el) => getComputedStyle(el).getPropertyValue('--font-size-base').trim())
    const sidebarRow = () =>
      page
        .locator('html')
        .evaluate((el) => getComputedStyle(el).getPropertyValue('--sidebar-row-height').trim())

    await expect(page.locator('html')).toHaveAttribute('data-density', 'default')
    expect(await fontSize()).toBe('13px')
    expect(await sidebarRow()).toBe('32px')

    await page
      .getByRole('group', { name: 'Density' })
      .getByRole('button', { name: 'Compact' })
      .click()
    await expect(page.locator('html')).toHaveAttribute('data-density', 'compact')
    expect(await fontSize()).toBe('12.5px')
    expect(await sidebarRow()).toBe('28px')

    await page
      .getByRole('group', { name: 'Density' })
      .getByRole('button', { name: 'Comfortable' })
      .click()
    expect(await fontSize()).toBe('14px')
    expect(await sidebarRow()).toBe('36px')
  })

  test('a menu opens, positions and closes', async ({ page }) => {
    await page.goto('/dev/gallery')

    const trigger = page.getByRole('button', { name: 'Actions' }).first()
    await trigger.click()

    const menu = page.getByRole('menu', { name: 'Message actions' })
    await expect(menu).toBeVisible()

    // Positioned by Floating UI, so it must have left the origin.
    const box = await menu.boundingBox()
    expect(box?.x ?? 0).toBeGreaterThan(0)
    expect(box?.y ?? 0).toBeGreaterThan(0)

    await page.keyboard.press('Escape')
    await expect(menu).toBeHidden()
  })

  test('matches the committed visual baseline', async ({ page }) => {
    await page.goto('/dev/gallery')

    // Wait for the gallery to actually mount before anything else. `page.goto` resolves
    // on load, but main.tsx reaches this route through a dynamic import, so at that
    // moment the document is still empty — and the first version of this test happily
    // committed a screenshot of a blank white page as the baseline.
    await expect(page.getByRole('region', { name: 'Light' })).toBeVisible()
    await expect(page.getByRole('region', { name: 'Dark' })).toBeVisible()

    // Inter is bundled rather than fetched from a CDN, but it still loads asynchronously.
    // Screenshotting before it lands captures the Segoe fallback and every text metric
    // shifts by a pixel or two.
    await page.evaluate(() => document.fonts.ready)

    // fullPage captures the *document*, and this document never scrolls — the gallery
    // scrolls inside a ScrollArea, so a plain fullPage shot is one viewport of specimens
    // and no evidence at all about the twelve primitives below the fold. Growing the
    // viewport to the scroller's content is what makes the baseline cover all of them.
    const contentHeight = await page
      .locator('[data-gallery-scroll]')
      .evaluate((el) => el.scrollHeight)
    const header = await page.locator('header').evaluate((el) => el.getBoundingClientRect().height)

    await page.setViewportSize({ width: 1400, height: Math.ceil(contentHeight + header) })
    await expect(page.getByRole('region', { name: 'Light' })).toBeVisible()

    await expect(page).toHaveScreenshot('gallery.png', { fullPage: true })
  })
})
