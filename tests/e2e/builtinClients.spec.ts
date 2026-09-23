import { expect, test } from '@playwright/test'

/**
 * A build that carries its own sign-in applications.
 *
 * Reported from the freshly installed app: choosing Google said *"Google requires every app to
 * register its own sign-in application, and Halcyon ships without one"*, and choosing Microsoft
 * said the same. That was true — no build had ever carried a client — and it is the one thing a
 * mail client cannot say about Gmail and still be taken seriously.
 *
 * Builds now can (`src-tauri/oauth/`), and these tests are what a user of such a build sees. They
 * run against the browser store with `?builtin-clients=…`, which stands in for a build whose
 * `clients.env` supplied those providers. Without that switch the store is a build from public
 * source, which carries none — and every other spec in this suite runs that way, so the
 * bring-your-own path stays covered too.
 *
 * What these cannot see is the core: whether `build.rs` really compiled a client in, and whether
 * `resolve_client` really prefers the user's own. Those are `accounts::tests`, where a built-in
 * client can be passed in rather than depending on the machine the suite runs on.
 */

const FIRST_RUN_WITH_GOOGLE = '/?first-run=1&builtin-clients=google'
const SETTINGS_WITH_GOOGLE = '/?settings=1&pane=accounts&builtin-clients=google'

test.describe('a build that carries a Google client', () => {
  test('lets a new user choose Google and carry on, with nothing to set up', async ({ page }) => {
    await page.goto(FIRST_RUN_WITH_GOOGLE)
    await page.getByRole('button', { name: 'Add your account' }).click()

    const google = page.getByRole('radio', { name: /Google/ })
    await expect(google).not.toContainText('Needs setting up')

    await google.click()

    // The complaint, asserted as absent: no registration note and no detour through Settings.
    await expect(page.getByText(/registered application/)).toHaveCount(0)
    await expect(page.getByRole('button', { name: 'Open Settings' })).toHaveCount(0)
    await expect(page.getByRole('button', { name: 'Continue' })).toBeEnabled()
  })

  test('still asks about Microsoft, which this build does not carry', async ({ page }) => {
    // Built-in is per provider. A build given only a Google client must not start pretending
    // Microsoft works too — that would open a browser onto an error page.
    await page.goto(FIRST_RUN_WITH_GOOGLE)
    await page.getByRole('button', { name: 'Add your account' }).click()

    const microsoft = page.getByRole('radio', { name: /Microsoft/ })
    await expect(microsoft).toContainText('Needs setting up in Settings first')

    await microsoft.click()

    await expect(page.getByText(/this copy of Halcyon was built without one/)).toBeVisible()
    await expect(page.getByRole('button', { name: 'Open Settings' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Continue' })).toBeDisabled()
  })

  test('shows no provider needing setup when both are carried', async ({ page }) => {
    await page.goto('/?first-run=1&builtin-clients=google,microsoft')
    await page.getByRole('button', { name: 'Add your account' }).click()

    await expect(page.getByRole('radio', { name: /Google/ })).not.toContainText('Needs setting up')
    await expect(page.getByRole('radio', { name: /Microsoft/ })).not.toContainText(
      'Needs setting up',
    )
  })
})

test.describe('Settings in a build that carries a Google client', () => {
  test('says the built-in application is in use rather than asking for one', async ({ page }) => {
    await page.goto(SETTINGS_WITH_GOOGLE)
    await expect(page.getByRole('heading', { level: 1, name: 'Accounts' })).toBeVisible()

    // An empty box is the working state here, and it has to read as one.
    const googleId = page.getByLabel('Google client ID')
    await expect(googleId).toHaveValue('')
    await expect(googleId).toHaveAccessibleDescription(/Halcyon's own application is in use/)

    // The built-in client has its secret, so the empty secret box is not an error.
    const googleSecret = page.getByLabel('Google client secret')
    await expect(googleSecret).toHaveAttribute('aria-invalid', 'false')
    await expect(googleSecret).toHaveAccessibleDescription(
      'Only needed with an application of your own.',
    )

    // Microsoft is not carried, and says what that means.
    await expect(page.getByLabel('Microsoft client ID')).toHaveAccessibleDescription(
      'Needed before a Microsoft account can be added.',
    )
  })

  test('offers no secret box for Microsoft, which would only break sign-in', async ({ page }) => {
    // A Microsoft desktop app is a public client, and a public client that sends a secret is
    // refused (AADSTS700025). The box used to be there, labelled "(optional)".
    await page.goto(SETTINGS_WITH_GOOGLE)
    await expect(page.getByLabel('Microsoft client ID')).toBeVisible()

    await expect(page.getByLabel('Microsoft client secret')).toHaveCount(0)
    await expect(page.getByLabel('Google client secret')).toBeVisible()
  })

  test('lets your own application take over, and clearing it goes back', async ({ page }) => {
    await page.goto(SETTINGS_WITH_GOOGLE)

    const googleId = page.getByLabel('Google client ID')
    const googleSecret = page.getByLabel('Google client secret')
    // Google's row is the first provider, so its Save is the first one.
    const save = page.getByRole('button', { name: 'Save' }).first()

    await googleId.fill('mine.apps.googleusercontent.com')
    await save.click()

    await expect(googleId).toHaveAccessibleDescription(
      "Your own application is in use. Clear this and save to go back to Halcyon's.",
    )
    // Your own Google client needs your own secret — the built-in one belongs to a different id.
    await expect(googleSecret).toHaveAttribute('aria-invalid', 'true')
    await expect(googleSecret).toHaveAccessibleDescription(
      'Required. Google will not refresh an account without it.',
    )

    await googleId.fill('')
    await save.click()

    await expect(googleId).toHaveAccessibleDescription(/Halcyon's own application is in use/)
    await expect(googleSecret).toHaveAttribute('aria-invalid', 'false')
  })
})

test('a build that carries nothing still asks for a client, as a public build must', async ({
  page,
}) => {
  // docs/05 §9: a build from public source carries no client. Nothing about the built-in path
  // may leak into that one.
  await page.goto('/?settings=1&pane=accounts')

  await expect(page.getByLabel('Google client ID')).toHaveAccessibleDescription(
    'Needed before a Google account can be added.',
  )
  await expect(page.getByLabel('Google client secret')).toHaveAccessibleDescription(
    'Required. Google will not refresh an account without it.',
  )
})
