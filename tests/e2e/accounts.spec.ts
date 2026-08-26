import { expect, test, type Page } from '@playwright/test'

/**
 * The account assistant and the accounts pane, driven end to end.
 *
 * These exist partly because the visual baselines cannot be trusted to catch this:
 * `maxDiffPixelRatio: 0.002` allows about 2,500 differing pixels on a 1400×900 frame, and a
 * whole 28px control fits inside that. Adding the Settings button to the sidebar header did
 * not fail a single baseline. Chrome that matters gets an assertion, not a screenshot.
 *
 * Everything here runs against `src/mock/browserStore.ts`, which answers the provider table
 * and the known-domain lookups for real and refuses the two commands a browser genuinely
 * cannot serve — writing to Credential Manager and opening a TLS socket to port 993.
 */

async function openSettings(page: Page) {
  await page.goto('/')
  await page.waitForSelector('[role="tree"]')
  await page.getByRole('button', { name: 'Settings' }).click()
  await expect(page.getByRole('heading', { name: 'Accounts' })).toBeVisible()
}

test.describe('accounts settings', () => {
  test('opens from the sidebar and lists every account', async ({ page }) => {
    await openSettings(page)

    // The three seeded accounts, by address rather than by row count — a count passes
    // just as well when the list renders the same account three times.
    await expect(page.getByText('vishal@northgate.example')).toBeVisible()
    await expect(page.getByText('vishal@icloud.example')).toBeVisible()
    await expect(page.getByText('vishal.singh@gmail.example')).toBeVisible()
  })

  test('reorders accounts, and the first cannot move up', async ({ page }) => {
    await openSettings(page)

    const first = page.getByRole('button', { name: /^Move .* up$/ }).first()
    await expect(first).toBeDisabled()

    await page
      .getByRole('button', { name: /^Move .* down$/ })
      .first()
      .click()

    // The whole order, asserted with a retrying matcher rather than a one-shot
    // `allTextContents()`. The reorder round-trips through the store and an invalidation,
    // so a plain read races the refetch — which it lost about two runs in three.
    await expect(page.locator('li').getByText(/@.*\.example$/)).toHaveText([
      'vishal@icloud.example',
      'vishal@northgate.example',
      'vishal.singh@gmail.example',
    ])
  })

  test('says what removing an account actually does before doing it', async ({ page }) => {
    // "Remove account" on its own does not tell anyone their downloaded mail is about to
    // be deleted, and it is not recoverable from here.
    await openSettings(page)

    await page
      .getByRole('button', { name: /^Remove / })
      .first()
      .click()

    const dialog = page.getByRole('dialog').last()
    await expect(dialog).toContainText('deleted from this computer')
    await expect(dialog).toContainText('Credential Manager')
    await expect(dialog).toContainText('Nothing is deleted from the mail server')
  })
})

test.describe('the account assistant', () => {
  async function openAssistant(page: Page) {
    await openSettings(page)
    await page.getByRole('button', { name: 'Add Account' }).click()
    await expect(
      page.getByRole('heading', { name: /Choose a Mail Account Provider/ }),
    ).toBeVisible()
  }

  test('offers every provider, Yahoo included', async ({ page }) => {
    await openAssistant(page)

    for (const name of ['Google', 'Microsoft', 'iCloud', 'Yahoo', 'Other Mail Account']) {
      await expect(page.getByRole('radio', { name: new RegExp(name) })).toBeVisible()
    }
  })

  test('cannot continue until a provider is chosen', async ({ page }) => {
    await openAssistant(page)

    const dialog = page.getByRole('dialog').last()
    await expect(dialog.getByRole('button', { name: 'Continue' })).toBeDisabled()

    await page.getByRole('radio', { name: /iCloud/ }).click()
    await expect(dialog.getByRole('button', { name: 'Continue' })).toBeEnabled()
  })

  test('tells an iCloud user they need an app-specific password, with a way to get one', async ({
    page,
  }) => {
    // docs/05 §4 calls this the single most common support issue for third-party clients.
    await openAssistant(page)
    await page.getByRole('radio', { name: /iCloud/ }).click()

    const dialog = page.getByRole('dialog').last()
    await expect(dialog).toContainText('app-specific password')
    await expect(dialog).toContainText('two-factor authentication')
    await expect(dialog.getByRole('button', { name: /Open in browser/ })).toBeVisible()
  })

  test('tells a Yahoo user about app passwords, not about iCloud', async ({ page }) => {
    await openAssistant(page)
    await page.getByRole('radio', { name: /Yahoo/ }).click()

    const dialog = page.getByRole('dialog').last()
    await expect(dialog).toContainText('app password')
    await expect(dialog).toContainText('Account Security')
    await expect(dialog).not.toContainText('appleid.apple.com')
  })

  test('blocks Google until a sign-in application is registered, and says so on the tile', async ({
    page,
  }) => {
    // Nothing is compiled in (docs/05 §2, bring your own OAuth client), so on a fresh
    // install Google cannot sign anyone in. Letting the user continue would open a browser
    // onto a Google error page, which reads as the app being broken rather than
    // unconfigured.
    await openAssistant(page)
    await page.getByRole('radio', { name: /Google/ }).click()

    const dialog = page.getByRole('dialog').last()
    await expect(dialog).toContainText('Needs setting up in Settings first')
    await expect(dialog.getByRole('button', { name: 'Continue' })).toBeDisabled()
  })

  test('offers no password box once a sign-in application is registered', async ({ page }) => {
    // A password field here would invite the user to type their Google password into an
    // app that must never see it. docs/05 §2.
    await openSettings(page)

    await page.getByLabel('Google client ID').fill('1234.apps.googleusercontent.com')
    await page.getByRole('button', { name: 'Save' }).first().click()

    await page.getByRole('button', { name: 'Add Account' }).click()
    await page.getByRole('radio', { name: /Google/ }).click()

    const dialog = page.getByRole('dialog').last()
    // Scoped to the Google tile — Microsoft still has no client, and correctly still warns.
    await expect(page.getByRole('radio', { name: /Google/ })).not.toContainText('Needs setting up')
    await dialog.getByRole('button', { name: 'Continue' }).click()

    await expect(dialog.getByLabel('Email Address')).toBeVisible()
    await expect(dialog.getByLabel(/password/i)).toHaveCount(0)
    await expect(dialog).toContainText('never sees your Google password')
  })

  test('asks an Other account for its servers, and a known provider for none', async ({ page }) => {
    await openAssistant(page)
    await page.getByRole('radio', { name: /Other Mail Account/ }).click()

    const dialog = page.getByRole('dialog').last()
    await dialog.getByRole('button', { name: 'Continue' }).click()
    await dialog.getByLabel('Email Address').fill('ada@example.test')
    await dialog.getByLabel('Password').fill('correct-horse')
    await dialog.getByRole('button', { name: 'Continue' }).click()

    await expect(dialog.getByText('Incoming (IMAP)')).toBeVisible()
    await expect(dialog.getByText('Outgoing (SMTP)')).toBeVisible()
    // Prefilled with the conventional ports rather than left blank.
    await expect(dialog.getByLabel('Port').first()).toHaveValue('993')
  })

  test('rejects an address that is not one', async ({ page }) => {
    await openAssistant(page)
    await page.getByRole('radio', { name: /iCloud/ }).click()

    const dialog = page.getByRole('dialog').last()
    await dialog.getByRole('button', { name: 'Continue' }).click()
    await dialog.getByLabel('Email Address').fill('not-an-address')

    await expect(dialog).toContainText('does not look like an email address')
    await expect(dialog.getByRole('button', { name: 'Continue' })).toBeDisabled()
  })

  test('says plainly that a browser cannot test a mail connection', async ({ page }) => {
    // Standing rule 18 — the browser path refuses rather than returning a shape that looks
    // like success. This asserts the refusal is legible, not that it is absent.
    await openAssistant(page)
    await page.getByRole('radio', { name: /iCloud/ }).click()

    const dialog = page.getByRole('dialog').last()
    await dialog.getByRole('button', { name: 'Continue' }).click()
    await dialog.getByLabel('Email Address').fill('ada@icloud.com')
    await dialog.getByLabel('App Password').fill('abcd-efgh-ijkl-mnop')
    await dialog.getByRole('button', { name: 'Continue' }).click()

    await expect(dialog.getByRole('heading', { name: 'Could Not Connect' })).toBeVisible()
    await expect(dialog).toContainText('cannot open a mail connection')

    // Every step is listed rather than the report stopping at the failure.
    await expect(dialog.getByText('Open Inbox')).toBeVisible()
    await expect(dialog.getByRole('heading', { name: 'Outgoing mail' })).toBeVisible()
  })
})
