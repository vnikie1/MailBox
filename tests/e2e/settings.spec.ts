import { expect, test } from '@playwright/test'

/**
 * The Settings window. docs/06 Phase 11.
 *
 * Driven in the browser, where a second OS window is a second tab. That covers everything the
 * WebView renders — the pane list, which pane is showing, the controls inside it — and none of
 * the Win32 half: whether Tauri actually creates the window, and whether `settings` is in
 * `capabilities/default.json` so its IPC calls are allowed. Both of those fail *silently* when
 * they are wrong, and neither is visible from here; they are checked against the running window
 * and recorded in the phase verification.
 *
 * The appearance controls get the most attention because they are the ones with no other test:
 * theme, density and translucency have existed since Phase 1 with no UI at all, so the risk is
 * not that a control looks wrong but that turning it does nothing.
 */

test.describe('the settings window', () => {
  test('opens in its own window from the sidebar', async ({ page, context }) => {
    await page.goto('/')
    await page.waitForSelector('[role="tree"]')

    // A new page, not a panel over the mailbox. This is the whole shape of the change: before
    // Phase 11 the same click opened a modal sheet in this tab.
    const opened = context.waitForEvent('page')
    await page.getByRole('button', { name: 'Settings' }).click()
    const settings = await opened

    await expect(settings.getByRole('navigation', { name: 'Settings' })).toBeVisible()
    // The mailbox is still there, and still usable, which is the point of a window.
    await expect(page.locator('[role="tree"]')).toBeVisible()
  })

  test('offers the seven panes docs/06 asks for', async ({ page }) => {
    await page.goto('/?settings=1')

    const nav = page.getByRole('navigation', { name: 'Settings' })
    await expect(nav.getByRole('button')).toHaveText([
      'General',
      'Accounts',
      'Composing',
      'Signatures',
      'Rules',
      'Privacy',
      'Advanced',
    ])
  })

  test('opens on the pane it was asked for', async ({ page }) => {
    await page.goto('/?settings=1&pane=privacy')
    await expect(page.getByRole('heading', { name: 'What Halcyon sends' })).toBeVisible()
  })

  test('falls back to General rather than refusing to open', async ({ page }) => {
    // A wrong pane name is one click from the right one; a window that will not open is not.
    await page.goto('/?settings=1&pane=nonsense')
    await expect(page.getByRole('heading', { name: 'Appearance' })).toBeVisible()
  })

  test('shows one pane at a time', async ({ page }) => {
    await page.goto('/?settings=1')
    await expect(page.getByRole('heading', { name: 'Appearance' })).toBeVisible()

    await page.getByRole('button', { name: 'Composing' }).click()
    await expect(page.getByRole('heading', { name: 'Composing' })).toBeVisible()

    // The pane that was showing is gone, not merely scrolled away. Six sections stacked in one
    // sheet is what this replaced, and every one of them ran its queries on open.
    await expect(page.getByRole('heading', { name: 'Appearance' })).toBeHidden()
  })

  test('changes the density, and the app follows', async ({ page }) => {
    await page.goto('/?settings=1')

    // Not a screenshot: `maxDiffPixelRatio` allows about 2,500 differing pixels, and a density
    // change moves row heights by a few pixels each. The attribute is what the token layer
    // keys off, so it is the thing worth asserting.
    await expect(page.locator('html')).toHaveAttribute('data-density', 'default')

    await page.getByRole('radio', { name: 'Compact' }).check()
    await expect(page.locator('html')).toHaveAttribute('data-density', 'compact')
  })

  test('pins the theme against what the OS asked for', async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'light' })
    await page.goto('/?settings=1')
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')

    await page.getByRole('radio', { name: 'Dark' }).check()
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')

    // And back to following Windows, which is the default and the one people return to.
    await page.getByRole('radio', { name: 'Follow Windows' }).first().check()
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')
  })

  test('lets a signature be typed, which nothing could do before', async ({ page }) => {
    await page.goto('/?settings=1&pane=signatures')

    const editor = page.getByRole('textbox', { name: 'Signature' })
    await expect(editor).toBeVisible()

    await editor.click()
    await editor.pressSequentially('Vishal Singh')
    await expect(editor).toContainText('Vishal Singh')
  })

  test('says plainly that a browser writes no crash reports', async ({ page }) => {
    await page.goto('/?settings=1&pane=advanced')
    await expect(page.getByRole('heading', { name: 'Diagnostics' })).toBeVisible()
    await expect(page.getByText('there are none in a browser')).toBeVisible()
  })
})
