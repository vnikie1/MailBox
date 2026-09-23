import { expect, test } from '@playwright/test'

/**
 * The first-run experience. docs/06 Phase 11.
 *
 * The gate is *first run to reading mail in under three minutes*. What is measured here is the
 * UI half — welcome, add an account, done — driven at machine speed against the browser store.
 * It is a floor, not the real figure: a person reads the screens and types a password, and the
 * mail then has to arrive over a network this cannot simulate.
 *
 * A floor is still worth having. If the screens themselves cost thirty seconds of waiting, no
 * amount of typing speed saves the gate, and that is the failure this catches.
 */

/** The browser store starts with accounts, so first run has to be asked for. */
const FIRST_RUN = '/?first-run=1'

test.describe('the first run', () => {
  test('opens on a welcome screen rather than a password box', async ({ page }) => {
    await page.goto(FIRST_RUN)

    // The risk to the gate is not the number of screens. It is opening an unexplained
    // credential form as the very first thing somebody sees.
    await expect(page.getByRole('heading', { name: 'Welcome to Halcyon' })).toBeVisible()
    await expect(page.getByRole('textbox', { name: /password/i })).toHaveCount(0)
  })

  test('says where the password goes before asking for one', async ({ page }) => {
    await page.goto(FIRST_RUN)
    await expect(page.getByText(/Windows Credential Manager/)).toBeVisible()
    await expect(page.getByText(/no analytics/)).toBeVisible()
  })

  test('cannot be dismissed, because there is nothing behind it', async ({ page }) => {
    await page.goto(FIRST_RUN)
    await page.keyboard.press('Escape')
    await expect(page.getByRole('heading', { name: 'Welcome to Halcyon' })).toBeVisible()
  })

  test('reaches the account assistant in one click', async ({ page }) => {
    await page.goto(FIRST_RUN)
    await page.getByRole('button', { name: 'Add your account' }).click()

    // Provider first, which is the assistant's own order — the answer changes every question
    // after it.
    await expect(page.getByRole('radio', { name: /Google/ })).toBeVisible()
  })

  test('picking a provider that needs a sign-in application offers a way to reach Settings', async ({
    page,
    context,
  }) => {
    // The trap this closes. On a first run, choosing Google or Microsoft before registering an
    // OAuth client disabled Continue and left no way forward: `firstRun` suppresses Cancel,
    // the sheet's `onOpenChange` is a deliberate no-op so Escape does nothing, the modal
    // overlay covers the sidebar's Settings button, and `useShortcuts` suppresses Ctrl+, while
    // a dialog is mounted. The tile said "Needs setting up in Settings first" and named a
    // place that could not be reached from where the user was standing. The only escape was
    // to pick a different provider, and nothing on screen said so.
    await page.goto(FIRST_RUN)
    await page.getByRole('button', { name: 'Add your account' }).click()
    await page.getByRole('radio', { name: /Google/ }).click()

    // The four walls, asserted rather than assumed — if any of them is ever lifted, this test
    // should be reconsidered rather than silently protecting a door nobody needs.
    await expect(page.getByRole('button', { name: 'Continue' })).toBeDisabled()
    await expect(page.getByRole('button', { name: 'Cancel' })).toHaveCount(0)
    await page.keyboard.press('Escape')
    await expect(page.getByRole('radio', { name: /Google/ })).toBeVisible()

    // The door.
    const openSettings = page.getByRole('button', { name: 'Open Settings' })
    await expect(openSettings).toBeVisible()

    // And it goes somewhere. In the browser there are no OS windows, so Settings opens as a
    // second tab — which is `settingsOpen`'s documented browser behaviour, not a test artefact.
    const [settings] = await Promise.all([context.waitForEvent('page'), openSettings.click()])
    await expect(settings.getByRole('heading', { level: 1, name: 'Accounts' })).toBeVisible()

    // The assistant is still standing behind it. Opening Settings must not have dismissed the
    // sheet — that would drop somebody into an empty app, which is what the no-cancel rule
    // exists to prevent.
    await expect(page.getByRole('radio', { name: /Google/ })).toBeVisible()
    await settings.close()
  })

  test('a provider that needs nothing shows no sign-in-application note', async ({ page }) => {
    // The note is conditional, and a note that showed for every provider would be noise on the
    // three tiles it does not apply to.
    await page.goto(FIRST_RUN)
    await page.getByRole('button', { name: 'Add your account' }).click()
    await page.getByRole('radio', { name: /iCloud/ }).click()

    await expect(page.getByRole('button', { name: 'Open Settings' })).toHaveCount(0)
    await expect(page.getByRole('button', { name: 'Continue' })).toBeEnabled()
  })

  test('spends well under the three-minute budget on its own screens', async ({ page }) => {
    // Warm first, then measure. The first navigation of a run pays for Vite compiling the
    // first-run chunk on demand, which is the dev server's cost and not the app's — measuring
    // it made this test fail about one run in five on a machine that was merely busy. A gate
    // that fails on load is one people learn to ignore; the same lesson as the scrolling budget
    // in shell.spec.ts.
    await page.goto(FIRST_RUN)
    await expect(page.getByRole('heading', { name: 'Welcome to Halcyon' })).toBeVisible()

    const started = Date.now()

    await page.reload()
    await expect(page.getByRole('heading', { name: 'Welcome to Halcyon' })).toBeVisible()
    await page.getByRole('button', { name: 'Add your account' }).click()
    await expect(page.getByRole('radio', { name: /Google/ })).toBeVisible()

    const elapsed = Date.now() - started

    // Five seconds, against a budget of a hundred and eighty. The margin is the point: this
    // fails when a screen starts *waiting* on something, not when a machine is slow.
    console.log(`first-run screens: ${String(elapsed)}ms`)
    expect(elapsed).toBeLessThan(5000)
  })
})
