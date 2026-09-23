import { expect, test, type Page } from '@playwright/test'

/**
 * The per-account colour, from the swatch that sets it to the row that draws it.
 *
 * It had never worked, in two separate ways, and the second outlived the fix for the first.
 * Setting a colour was rejected at the IPC seam for a fortnight (see CHANGELOG, 2026-09-10);
 * that was repaired, and the colour then persisted correctly into a database column that
 * nothing outside the settings pane ever read. `AccountRow` — the struct the sidebar is
 * served — had no `color` field at all, so the mailbox window could not have drawn it even
 * if it had wanted to. The pane that set the colour was also the only pane that read it
 * back, which is precisely why it kept looking fine.
 *
 * Hence two tests rather than one. The first drives the picker; the second proves the paint.
 * Either alone would have passed throughout the whole period the feature was broken.
 *
 * **What these cannot see.** Playwright serves the app with `npm run dev`
 * (`playwright.config.ts:59-65`), so there is no Tauri here and `src/lib/ipc.ts:393` short
 * -circuits every call into `src/mock/browserStore.ts`. So the picker test never reaches
 * `invoke('account_update')` and never exercises the `{ value: … }` wire encoding that was
 * the *first* bug; and the paint test starts from a mock-only seed, so it never touches
 * `AccountRow` or `query::accounts_list`, which were the *second*. Both would still pass with
 * the Rust half of the fix reverted.
 *
 * That half is covered where it can be:
 * `accounts::store::tests::a_colour_reaches_the_sidebar_and_not_only_the_settings_pane` reads
 * the colour back through `query::accounts_list` against a real SQLite database. The two
 * halves are tested in two languages because no single harness in this repo spans the seam —
 * which is worth saying out loud, because believing otherwise is how the first bug survived a
 * fortnight of green runs.
 */

/** Opens Settings on the Accounts pane, as `accounts.spec.ts` does. */
async function openSettings(page: Page) {
  await page.goto('/?settings=1&pane=accounts')
  await expect(page.getByRole('heading', { level: 1, name: 'Accounts' })).toBeVisible()
}

/** A design token resolved to the `rgb()` string `toHaveCSS` compares against. */
async function resolved(page: Page, token: string): Promise<string> {
  return page.evaluate((name) => {
    const probe = document.createElement('span')
    probe.style.color = `var(${name})`
    document.body.append(probe)
    const value = getComputedStyle(probe).color
    probe.remove()
    return value
  }, token)
}

test.describe('per-account colour', () => {
  test('a swatch sets the colour, and clicking it again clears it', async ({ page }) => {
    await openSettings(page)

    // The first account's group, by its accessible name rather than by position: the pane
    // renders one radiogroup per account and an unscoped query would find all three.
    const picker = page.getByRole('radiogroup', { name: /^Colour for / }).first()
    const purple = picker.getByRole('radio', { name: 'Purple' })
    const green = picker.getByRole('radio', { name: 'Green' })

    await expect(purple).toHaveAttribute('aria-checked', 'false')

    await purple.click()
    // Retrying matcher, not a one-shot read: the write round-trips through the store and a
    // query invalidation, so a plain assertion races the refetch.
    await expect(purple).toHaveAttribute('aria-checked', 'true')

    // One at a time — it is a radiogroup, and the old colour has to let go.
    await green.click()
    await expect(green).toHaveAttribute('aria-checked', 'true')
    await expect(purple).toHaveAttribute('aria-checked', 'false')

    // Clicking the current colour clears it. The core's `ColorChange` wrapper exists for this
    // distinction — "leave it alone" and "remove it" are different requests, and a bare string
    // could not tell them apart — though note the wrapper itself is only exercised in Tauri;
    // see the header. What this pins is the *intent*: the third click must not be a no-op.
    await green.click()
    await expect(green).toHaveAttribute('aria-checked', 'false')
  })

  test('the sidebar draws each account in its own colour', async ({ page }) => {
    // `?account-colours=1` seeds one colour per account. A browser page holds the mock's
    // overlay in module memory, so the colour set by the test above is gone by the time the
    // mailbox loads — the two are separate page loads sharing nothing.
    await page.goto('/?account-colours=1')
    await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()

    const drawn: string[] = []

    // Mirrors `SEEDED_COLOURS` in the browser store.
    for (const colour of ['purple', 'green', 'orange']) {
      const icon = page.locator(`[data-account='${colour}']`).first()
      await expect(icon, `an account should be drawn in --flag-${colour}`).toHaveCSS(
        'color',
        await resolved(page, `--flag-${colour}`),
      )

      drawn.push(await icon.evaluate((el) => getComputedStyle(el).color))
    }

    // The regression stated directly: every account row was --accent, so they were all one
    // colour. Asserting each against its own token is not enough by itself — a machine whose
    // accent happened to be one of the seven would pass that for the wrong reason.
    expect(new Set(drawn).size, 'three accounts should be three different colours').toBe(3)

    // And the per-account children of All Inboxes, which are labelled by account name and are
    // where identical grey inboxes are hardest to tell apart.
    const favourites = page.getByRole('group', { name: 'Favourites' })
    await expect(favourites.locator('[data-account]').first()).toBeVisible()
  })

  test('an account with no colour keeps the accent, rather than an empty attribute', async ({
    page,
  }) => {
    // The default state, and the one the committed visual baselines are of. `data-account` is
    // selected on by existence, so a row that carried it empty would match nothing in the
    // palette and lose the accent to an unset custom property.
    await page.goto('/')
    await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()

    await expect(page.locator('[data-account]')).toHaveCount(0)

    // By the icon's own Lucide class, not `svg` — the first `svg` in a row is the disclosure
    // chevron, which is deliberately a quiet grey and would pass or fail for its own reasons.
    const inbox = page
      .getByRole('group', { name: 'Favourites' })
      .getByRole('treeitem')
      .first()
      .locator('svg.lucide-inbox')

    await expect(inbox).toHaveCSS('color', await resolved(page, '--accent'))
  })
})
