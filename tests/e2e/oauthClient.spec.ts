import { expect, test, type Page } from '@playwright/test'

/**
 * Saving an OAuth client must make the provider usable *without a restart*.
 *
 * This exists because it did not. `oauth_client_set` was the one mutation in the account
 * command surface that did not emit `accounts:changed`, and `useProviders` is cached with
 * `staleTime: Infinity` — so the client id was written to the database correctly and the
 * provider tile stayed greyed out until the app was restarted. Everything worked except
 * being told about it, which is the hardest kind of failure to read from the outside.
 *
 * **This test would have passed before that fix, and it is important to say so.** The
 * notification helper only no-ops *inside Tauri*; served by Vite it dispatched on the browser
 * bus as it always had, so the browser path was never broken and this suite could not see the
 * bug. The whole class of failure — a core command that forgets to announce itself — is
 * invisible from here.
 *
 * What it does pin is the fix that makes the class survivable: `useAccountsChanged`
 * invalidates the queries directly, in shared code, so correctness no longer depends on every
 * command remembering to emit. Deleting that invalidation now fails here. The emit itself is
 * covered by `accounts::tests::a_client_written_in_a_transaction_is_visible_immediately` and,
 * for the event, only by running the real window.
 */

async function openSettings(page: Page) {
  await page.goto('/')
  await page.waitForSelector('[role="tree"]')
  await page.getByRole('button', { name: 'Settings' }).click()
  await expect(page.getByRole('heading', { name: 'Accounts' })).toBeVisible()
}

test('saving a client ID ungreys the provider in the same session', async ({ page }) => {
  await openSettings(page)

  // Greyed out to begin with.
  await page.getByRole('button', { name: 'Add Account' }).click()
  await expect(page.getByRole('radio', { name: /Google/ })).toContainText(
    'Needs setting up in Settings first',
  )
  await page.getByRole('button', { name: 'Cancel' }).click()

  await page.getByLabel('Google client ID').fill('1234.apps.googleusercontent.com')
  await page.getByRole('button', { name: 'Save' }).first().click()

  // No reload, no restart. The provider list has to have refetched on its own.
  await page.getByRole('button', { name: 'Add Account' }).click()

  const google = page.getByRole('radio', { name: /Google/ })
  await expect(google).not.toContainText('Needs setting up')
  await expect(
    page.getByRole('dialog').last().getByRole('button', { name: 'Continue' }),
  ).toBeDisabled()

  await google.click()
  await expect(
    page.getByRole('dialog').last().getByRole('button', { name: 'Continue' }),
  ).toBeEnabled()
})

test('Microsoft stays greyed out when only Google was configured', async ({ page }) => {
  // The invalidation must not be mistaken for "everything is now configured".
  await openSettings(page)

  await page.getByLabel('Google client ID').fill('1234.apps.googleusercontent.com')
  await page.getByRole('button', { name: 'Save' }).first().click()

  await page.getByRole('button', { name: 'Add Account' }).click()

  await expect(page.getByRole('radio', { name: /Microsoft/ })).toContainText(
    'Needs setting up in Settings first',
  )
})
