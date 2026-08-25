import { expect, test } from '@playwright/test'

/**
 * The shell, served by Vite and driven in Chromium.
 *
 * This covers what the WebView renders: token application, layout metrics and theme
 * swapping at runtime. It deliberately does NOT cover the Win32 half — the DWM material,
 * the system caption and its Snap Layouts flyout — because a browser cannot produce any
 * of them. Those are verified against the running Tauri window and recorded in
 * docs/PHASE-0-VERIFICATION.md.
 */

test.describe('window shell', () => {
  test('renders the shell', async ({ page }) => {
    await page.goto('/')
    await expect(page.getByRole('heading', { name: 'Halyard' })).toBeVisible()
  })

  test('lays the toolbar out to the spec height', async ({ page }) => {
    await page.goto('/')

    // docs/02 §6.1 / §7 — 52px at default density.
    const bar = await page.locator('header').boundingBox()
    expect(bar).not.toBeNull()
    expect(bar?.height).toBe(52)
    expect(bar?.y).toBe(0)
  })

  test('lays the sidebar out to the spec width', async ({ page }) => {
    await page.goto('/')

    // docs/01 §2 — sidebar default width 232.
    const sidebar = await page.locator('header + div > div').first().boundingBox()
    expect(sidebar).not.toBeNull()
    expect(sidebar?.width).toBe(232)
  })

  test('follows the OS theme at runtime with no reload', async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'light' })
    await page.goto('/')
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')

    await page.emulateMedia({ colorScheme: 'dark' })
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')

    await page.emulateMedia({ colorScheme: 'light' })
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')
  })

  test('falls back to opaque surfaces when there is no DWM material', async ({ page }) => {
    await page.goto('/')

    await expect(page.locator('html')).toHaveAttribute('data-backdrop', 'none')

    // The fallback has to be a real opaque colour, not a transparent sidebar over nothing.
    const sidebarBg = await page
      .locator('header + div > div')
      .first()
      .evaluate((el) => getComputedStyle(el).backgroundColor)
    expect(sidebarBg).not.toContain('rgba')
  })

  test('reports its host honestly rather than pretending to be the app', async ({ page }) => {
    await page.goto('/')
    await expect(page.getByText('browser preview (no Win32 layer)')).toBeVisible()
  })
})
