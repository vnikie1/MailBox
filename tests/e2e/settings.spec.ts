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

  test('names the pane it is showing', async ({ page }) => {
    // The pane's own title, matching the entry that opened it. It used to be in the window's
    // title bar and nowhere else, so General opened on a heading that read "Appearance" and
    // there was nothing on screen saying which of the seven you were in.
    await page.goto('/?settings=1&pane=rules')
    await expect(page.getByRole('heading', { level: 1, name: 'Rules' })).toBeVisible()
  })

  test('shows one pane at a time', async ({ page }) => {
    await page.goto('/?settings=1')
    await expect(page.getByRole('heading', { name: 'Appearance' })).toBeVisible()

    await page.getByRole('button', { name: 'Composing' }).click()
    await expect(page.getByRole('heading', { level: 1, name: 'Composing' })).toBeVisible()

    // The pane that was showing is gone, not merely scrolled away. Six sections stacked in one
    // sheet is what this replaced, and every one of them ran its queries on open.
    await expect(page.getByRole('heading', { name: 'Appearance' })).toBeHidden()
  })

  test('walks the pane list with the arrow keys', async ({ page }) => {
    await page.goto('/?settings=1')

    await page.getByRole('button', { name: 'General' }).focus()
    await page.keyboard.press('ArrowDown')
    await expect(page.getByRole('heading', { level: 1, name: 'Accounts' })).toBeVisible()

    // Focus follows the selection, so the next arrow keeps walking rather than starting over.
    await page.keyboard.press('End')
    await expect(page.getByRole('heading', { level: 1, name: 'Advanced' })).toBeVisible()
  })

  test('changes the density, and the app follows', async ({ page }) => {
    await page.goto('/?settings=1')

    // Not a screenshot: `maxDiffPixelRatio` allows about 2,500 differing pixels, and a density
    // change moves row heights by a few pixels each. The attribute is what the token layer
    // keys off, so it is the thing worth asserting.
    await expect(page.locator('html')).toHaveAttribute('data-density', 'default')

    // Clicked rather than checked: these are segments, not inputs. `check()` needs a real
    // checkbox or radio element, and a segmented control is buttons carrying `role="radio"`.
    await page.getByRole('radio', { name: 'Compact' }).click()
    await expect(page.locator('html')).toHaveAttribute('data-density', 'compact')
  })

  test('pins the theme against what the OS asked for', async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'light' })
    await page.goto('/?settings=1')
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')

    const theme = page.getByRole('radiogroup', { name: 'Theme' })

    await theme.getByRole('radio', { name: 'Dark' }).click()
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')

    // And back to following Windows, which is the default and the one people return to.
    await theme.getByRole('radio', { name: 'Follow Windows' }).click()
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')
  })

  test('moves the theme with the arrow keys, as a radio group does', async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'light' })
    await page.goto('/?settings=1')

    // Scoped: "Follow Windows" is also the name of the first accent swatch, which is a radio
    // too and deliberately says the same thing.
    const theme = page.getByRole('radiogroup', { name: 'Theme' })

    await theme.getByRole('radio', { name: 'Follow Windows' }).focus()
    await page.keyboard.press('ArrowRight')
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')

    await page.keyboard.press('ArrowRight')
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
  })

  test('lines every control in a pane up on one column', async ({ page }) => {
    // The whole point of the form. Two controls in the same pane whose left edges differ means
    // one of them is not in the grid, which is exactly the drift the stacked layout had and
    // the reason nothing in this window looked aligned with anything else.
    await page.goto('/?settings=1')

    const cells = page.locator('main [class*="control"]')
    // `evaluateAll` does not auto-wait, so without this it measures an empty page and passes
    // an emptiness check it was never meant to make.
    await expect(cells.first()).toBeVisible()

    const lefts = await cells.evaluateAll((found) =>
      found.map((cell) => Math.round(cell.getBoundingClientRect().left)),
    )

    expect(lefts.length).toBeGreaterThan(3)
    expect(new Set(lefts).size).toBe(1)
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
