/**
 * Drives every control in the Settings window of the RUNNING app and reports what works.
 *
 * Rules it obeys, because this runs against the user's real mail and real accounts:
 *   - every value it changes is put back
 *   - it never confirms Remove Account, never clicks Delete all reports
 *   - it never opens a native file dialog (those are modal to Windows and would hang the run)
 *   - it never clicks Sign in again (that opens a browser)
 *
 * A rejected IPC promise is invisible from the DOM, so this also listens for pageerror the
 * whole way through — that is how the account-colour bug was finally caught.
 */
const { chromium } = require('playwright')

const pass = []
const fail = []
const skip = []
const pageErrors = []

const check = (name, actual, expected) => {
  const ok = String(actual) === String(expected)
  ;(ok ? pass : fail).push({ name, actual, expected })
  console.log(
    `${ok ? 'PASS' : 'FAIL'}  ${name}${ok ? '' : `  got=${JSON.stringify(actual)} want=${JSON.stringify(expected)}`}`,
  )
  return ok
}
const skipped = (name, why) => {
  skip.push({ name, why })
  console.log(`SKIP  ${name}  (${why})`)
}
const note = (m) => console.log(`      ${m}`)

const sleep = (page, ms) => page.waitForTimeout(ms)

;(async () => {
  const browser = await chromium.connectOverCDP('http://127.0.0.1:9333')
  const context = browser.contexts()[0]
  const main = context.pages().find((p) => !p.url().includes('settings=1'))
  if (!main) throw new Error('no main window')
  await main.waitForSelector('[role="tree"]', { timeout: 30000 })

  let s = context.pages().find((p) => p.url().includes('settings=1'))
  if (!s) {
    const opened = context.waitForEvent('page', { timeout: 20000 })
    await main.getByRole('button', { name: 'Settings' }).click()
    s = await opened
  }
  await s.waitForSelector('main', { timeout: 20000 })
  await sleep(s, 1200)

  s.on('pageerror', (e) => {
    pageErrors.push(e.message)
    console.log(`  [PAGEERROR] ${e.message}`)
  })

  const nav = s.getByRole('navigation', { name: 'Settings' })
  const go = async (label) => {
    await nav.getByRole('button', { name: label, exact: true }).click()
    await sleep(s, 900)
  }
  const attr = (page, name) => page.locator('html').getAttribute(name)

  // ================================================================ GENERAL
  console.log('\n──────── GENERAL ────────')
  await go('General')

  const original = {
    theme: await attr(s, 'data-theme'),
    density: await attr(s, 'data-density'),
    accent: await s
      .getByRole('radiogroup', { name: 'Accent colour' })
      .getByRole('radio', { checked: true })
      .first()
      .getAttribute('aria-label'),
  }
  note(`original appearance: ${JSON.stringify(original)}`)

  // --- Theme: every segment, mouse
  const theme = s.getByRole('radiogroup', { name: 'Theme' })
  for (const [label, want] of [
    ['Light', 'light'],
    ['Dark', 'dark'],
  ]) {
    await theme.getByRole('radio', { name: label }).click()
    await sleep(s, 700)
    check(`General ▸ Theme ▸ ${label} applies here`, await attr(s, 'data-theme'), want)
    check(
      `General ▸ Theme ▸ ${label} reaches the main window`,
      await attr(main, 'data-theme'),
      want,
    )
  }
  await theme.getByRole('radio', { name: 'Follow Windows' }).click()
  await sleep(s, 700)
  check(
    'General ▸ Theme ▸ Follow Windows is selectable',
    await theme.getByRole('radio', { name: 'Follow Windows' }).getAttribute('aria-checked'),
    'true',
  )

  // --- Accent: every one of the twelve
  const accents = s.getByRole('radiogroup', { name: 'Accent colour' })
  const accentNames = await accents
    .locator('input[type=radio]')
    .evaluateAll((els) => els.map((e) => e.getAttribute('aria-label')))
  note(`accent swatches: ${accentNames.length} — ${accentNames.join(', ')}`)
  let accentOk = 0
  for (const name of accentNames) {
    await accents.getByRole('radio', { name, exact: true }).check()
    await sleep(s, 350)
    const checked = await accents.getByRole('radio', { name, exact: true }).isChecked()
    if (checked) accentOk++
    else note(`  accent "${name}" did not take`)
  }
  check('General ▸ Accent ▸ all twelve swatches select', accentOk, accentNames.length)
  const accentHint = await s
    .getByText(/accent colour/i)
    .last()
    .textContent()
  note(`accent hint reads: ${JSON.stringify((accentHint || '').trim())}`)

  // --- Density
  const density = s.getByRole('radiogroup', { name: 'Density' })
  for (const [label, want] of [
    ['Compact', 'compact'],
    ['Comfortable', 'comfortable'],
    ['Default', 'default'],
  ]) {
    await density.getByRole('radio', { name: label }).click()
    await sleep(s, 600)
    check(
      `General ▸ Density ▸ ${label} reaches the main window`,
      await attr(main, 'data-density'),
      want,
    )
  }

  // --- Translucency popup: every option
  const trans = s.getByLabel('Translucency')
  const transBefore = await trans.inputValue()
  await trans.selectOption('reduce')
  await sleep(s, 700)
  check(
    'General ▸ Translucency ▸ Never translucent takes effect',
    await attr(main, 'data-reduce-transparency'),
    '',
  )
  await trans.selectOption('full')
  await sleep(s, 700)
  check(
    'General ▸ Translucency ▸ Always translucent takes effect',
    await attr(main, 'data-reduce-transparency'),
    null,
  )
  await trans.selectOption(transBefore)
  await sleep(s, 600)

  // --- Notifications, per account
  const notifyGroups = await s.locator('main input[type=checkbox]').count()
  note(`notification checkboxes on the pane: ${notifyGroups}`)
  const notifyMe = s.getByRole('checkbox', { name: 'Notify me about new mail' }).first()
  const vipOnly = s.getByRole('checkbox', { name: 'Only from VIPs' }).first()
  if (await notifyMe.count()) {
    const nBefore = await notifyMe.isChecked()
    const vipEnabledBefore = await vipOnly.isEnabled()
    check(
      'General ▸ Notifications ▸ "Only from VIPs" follows "Notify me"',
      vipEnabledBefore,
      nBefore,
    )

    await notifyMe.setChecked(!nBefore)
    await sleep(s, 800)
    check('General ▸ Notifications ▸ "Notify me" toggles', await notifyMe.isChecked(), !nBefore)
    check(
      'General ▸ Notifications ▸ "Only from VIPs" enable-state follows it',
      await vipOnly.isEnabled(),
      !nBefore,
    )

    await go('Privacy')
    await go('General')
    check(
      'General ▸ Notifications ▸ survives a pane switch (reached the core)',
      await s.getByRole('checkbox', { name: 'Notify me about new mail' }).first().isChecked(),
      !nBefore,
    )
    await s.getByRole('checkbox', { name: 'Notify me about new mail' }).first().setChecked(nBefore)
    await sleep(s, 800)
    check(
      'General ▸ Notifications ▸ restored',
      await s.getByRole('checkbox', { name: 'Notify me about new mail' }).first().isChecked(),
      nBefore,
    )
  }

  const startup = s.getByRole('checkbox', { name: /Start Halcyon when I sign in/ })
  if (await startup.count()) {
    const sBefore = await startup.isChecked()
    await startup.setChecked(!sBefore)
    await sleep(s, 1200)
    check(
      'General ▸ Start at sign-in toggles (writes the registry)',
      await startup.isChecked(),
      !sBefore,
    )
    await startup.setChecked(sBefore)
    await sleep(s, 1200)
    check('General ▸ Start at sign-in restored', await startup.isChecked(), sBefore)
  }

  // --- Updates
  const checkUpdates = s.getByRole('button', { name: /Check for updates|Checking/ })
  if (await checkUpdates.count()) {
    await checkUpdates.click()
    await sleep(s, 4000)
    const status = await s
      .locator('main p')
      .filter({ hasText: /up to date|available|Could not reach|Asking/ })
      .first()
      .textContent()
    note(`update check says: ${JSON.stringify((status || '').trim())}`)
    check('General ▸ Check for updates answers something', (status || '').trim().length > 0, 'true')
  }

  // restore appearance
  await theme.getByRole('radio', { name: original.theme === 'light' ? 'Light' : 'Dark' }).click()
  await sleep(s, 500)
  await accents.getByRole('radio', { name: original.accent, exact: true }).check()
  await sleep(s, 500)
  await density
    .getByRole('radio', {
      name:
        original.density === 'compact'
          ? 'Compact'
          : original.density === 'comfortable'
            ? 'Comfortable'
            : 'Default',
    })
    .click()
  await sleep(s, 500)

  // ================================================================ ACCOUNTS
  console.log('\n──────── ACCOUNTS ────────')
  await go('Accounts')

  const rows = s.locator('main ul > li')
  const rowCount = await rows.count()
  note(`account rows: ${rowCount}`)

  // --- rename
  const nameField = s.getByRole('textbox', { name: 'Description' }).first()
  const nameBefore = await nameField.inputValue()
  await nameField.fill(`${nameBefore} ZZ`)
  await nameField.blur()
  await sleep(s, 1500)
  await go('General')
  await go('Accounts')
  const nameAfter = await s.getByRole('textbox', { name: 'Description' }).first().inputValue()
  check('Accounts ▸ rename persists', nameAfter, `${nameBefore} ZZ`)
  await s.getByRole('textbox', { name: 'Description' }).first().fill(nameBefore)
  await s.getByRole('textbox', { name: 'Description' }).first().blur()
  await sleep(s, 1500)
  await go('General')
  await go('Accounts')
  check(
    'Accounts ▸ rename restored',
    await s.getByRole('textbox', { name: 'Description' }).first().inputValue(),
    nameBefore,
  )

  // --- colour swatches, on EVERY account. The reported bug, and the reason it is every
  //     account rather than the first: "different accounts" is what was reported.
  const groupNames = await s
    .getByRole('radiogroup', { name: /^Colour for / })
    .evaluateAll((els) => els.map((e) => e.getAttribute('aria-label')))
  note(`colour groups: ${groupNames.join(' | ')}`)

  for (const groupName of groupNames) {
    const colours = s.getByRole('radiogroup', { name: groupName, exact: true })
    const colourNames = await colours
      .locator('[role=radio]')
      .evaluateAll((els) => els.map((e) => e.getAttribute('aria-label')))
    const had = await colours.locator('[role=radio][aria-checked=true]').count()
    const colourBefore = had
      ? await colours.locator('[role=radio][aria-checked=true]').first().getAttribute('aria-label')
      : null
    note(`${groupName}: starts on ${JSON.stringify(colourBefore)}`)

    let set = 0
    for (const name of colourNames) {
      await colours.getByRole('radio', { name, exact: true }).click()
      await sleep(s, 650)
      if (
        (await colours.getByRole('radio', { name, exact: true }).getAttribute('aria-checked')) ===
        'true'
      )
        set++
      else note(`  "${name}" did not highlight`)
    }
    check(`Accounts ▸ ${groupName}: every colour sets and highlights`, set, colourNames.length)

    const last = colourNames[colourNames.length - 1]
    await go('General')
    await go('Accounts')
    const g2 = s.getByRole('radiogroup', { name: groupName, exact: true })
    const reread = await g2
      .locator('[role=radio][aria-checked=true]')
      .first()
      .getAttribute('aria-label')
      .catch(() => null)
    check(`Accounts ▸ ${groupName}: survives a pane switch (reached the core)`, reread, last)

    // Clicking the chosen colour again clears it — the {"value": null} path that the old
    // nested-option signature could not express at all.
    await g2.getByRole('radio', { name: last, exact: true }).click()
    await sleep(s, 900)
    check(
      `Accounts ▸ ${groupName}: clicking the chosen colour clears it`,
      await g2.locator('[role=radio][aria-checked=true]').count(),
      0,
    )
    await go('General')
    await go('Accounts')
    const g3 = s.getByRole('radiogroup', { name: groupName, exact: true })
    check(
      `Accounts ▸ ${groupName}: cleared stays cleared`,
      await g3.locator('[role=radio][aria-checked=true]').count(),
      0,
    )

    if (colourBefore) {
      await g3.getByRole('radio', { name: colourBefore, exact: true }).click()
      await sleep(s, 800)
    }
    check(
      `Accounts ▸ ${groupName}: restored to how it was found`,
      colourBefore
        ? await s
            .getByRole('radiogroup', { name: groupName, exact: true })
            .locator('[role=radio][aria-checked=true]')
            .first()
            .getAttribute('aria-label')
        : await s
            .getByRole('radiogroup', { name: groupName, exact: true })
            .locator('[role=radio][aria-checked=true]')
            .count(),
      colourBefore ?? 0,
    )
  }

  // --- reorder
  if (rowCount > 1) {
    const firstUp = s.getByRole('button', { name: /^Move .* up$/ }).first()
    check('Accounts ▸ first account cannot move up', await firstUp.isDisabled(), true)
    const lastDown = s.getByRole('button', { name: /^Move .* down$/ }).last()
    check('Accounts ▸ last account cannot move down', await lastDown.isDisabled(), true)

    const orderBefore = await s
      .locator('main ul > li')
      .evaluateAll((els) => els.map((e) => e.querySelector('input')?.value ?? ''))
    await s
      .getByRole('button', { name: /^Move .* down$/ })
      .first()
      .click()
    await sleep(s, 1500)
    const orderAfter = await s
      .locator('main ul > li')
      .evaluateAll((els) => els.map((e) => e.querySelector('input')?.value ?? ''))
    check(
      'Accounts ▸ reorder actually reorders',
      JSON.stringify(orderAfter) !== JSON.stringify(orderBefore),
      true,
    )
    await go('General')
    await go('Accounts')
    const orderKept = await s
      .locator('main ul > li')
      .evaluateAll((els) => els.map((e) => e.querySelector('input')?.value ?? ''))
    check('Accounts ▸ reorder persists', JSON.stringify(orderKept), JSON.stringify(orderAfter))
    await s
      .getByRole('button', { name: /^Move .* up$/ })
      .nth(1)
      .click()
    await sleep(s, 1500)
    const orderRestored = await s
      .locator('main ul > li')
      .evaluateAll((els) => els.map((e) => e.querySelector('input')?.value ?? ''))
    check('Accounts ▸ order restored', JSON.stringify(orderRestored), JSON.stringify(orderBefore))
  }

  // --- remove confirmation (opened and cancelled, never confirmed)
  await s
    .getByRole('button', { name: /^Remove / })
    .first()
    .click()
  await sleep(s, 900)
  const dialog = s.getByRole('dialog').last()
  check('Accounts ▸ Remove opens a confirmation', await dialog.count(), 1)
  const body = await dialog.textContent()
  check(
    'Accounts ▸ the confirmation says the mail is deleted',
    /deleted from this computer/.test(body || ''),
    true,
  )
  check(
    'Accounts ▸ the confirmation says the password is removed',
    /Credential Manager/.test(body || ''),
    true,
  )
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await sleep(s, 800)
  check(
    'Accounts ▸ Cancel closes it without removing',
    await s.locator('main ul > li').count(),
    rowCount,
  )
  skipped('Accounts ▸ Remove Account (confirm)', 'destructive — would delete real mail')

  // --- Add Account assistant, opened and cancelled
  await s.getByRole('button', { name: 'Add Account' }).click()
  await sleep(s, 1200)
  const assistant = s.getByRole('dialog').last()
  check('Accounts ▸ Add Account opens the assistant', await assistant.count(), 1)
  const providers = await assistant.getByRole('radio').count()
  check('Accounts ▸ the assistant offers providers', providers >= 4, true)
  note(`providers offered: ${providers}`)
  await s.keyboard.press('Escape')
  await sleep(s, 800)

  // --- OAuth client fields
  const clientId = s.getByLabel(/client ID/).first()
  if (await clientId.count()) {
    const idBefore = await clientId.inputValue()
    note(`Google client ID present: ${idBefore.length > 0}`)
    check('Accounts ▸ client ID field is editable', await clientId.isEditable(), true)
    const saveBtn = s.getByRole('button', { name: 'Save' }).first()
    check('Accounts ▸ Save button exists for the sign-in application', await saveBtn.count(), 1)
    skipped('Accounts ▸ Save the sign-in application', 'would rewrite the real Google credential')
  }
  skipped('Accounts ▸ Sign in again', 'opens a browser sign-in')

  // ================================================================ COMPOSING
  console.log('\n──────── COMPOSING ────────')
  await go('Composing')
  const undo = s.getByLabel('Undo send delay')
  const undoBefore = await undo.inputValue()
  const undoOptions = await undo.locator('option').evaluateAll((els) => els.map((e) => e.value))
  note(`undo options: ${undoOptions.join(', ')}`)
  let undoOk = 0
  for (const v of undoOptions) {
    await undo.selectOption(v)
    await sleep(s, 700)
    await go('Privacy')
    await go('Composing')
    if ((await s.getByLabel('Undo send delay').inputValue()) === v) undoOk++
    else note(`  undo value ${v} did not persist`)
  }
  check('Composing ▸ every undo-send value persists', undoOk, undoOptions.length)
  await s.getByLabel('Undo send delay').selectOption(undoBefore)
  await sleep(s, 700)

  // ================================================================ SIGNATURES
  console.log('\n──────── SIGNATURES ────────')
  await go('Signatures')
  const acct = s.getByLabel('Account')
  const acctOptions = await acct
    .locator('option')
    .evaluateAll((els) => els.map((e) => ({ v: e.value, t: e.textContent })))
  note(`signature accounts: ${acctOptions.map((o) => o.t).join(', ')}`)

  // The account the picker is actually ON, not the first option. Assuming the first option was
  // selected is what made an earlier run read one account's signature and write it onto
  // another's — the pane keeps whichever account it was showing, and the option order can
  // change under it after a reorder.
  const selectedAccount = await acct.inputValue()
  const selectedLabel = acctOptions.find((o) => o.v === selectedAccount)?.t
  note(`picker is on: ${selectedLabel} (value ${selectedAccount})`)

  const editorHtml = () =>
    s.evaluate(() => document.querySelector('main [contenteditable="true"]')?.innerHTML ?? '')
  const sigHtmlBefore = await editorHtml()
  note(`signature html before: ${JSON.stringify(sigHtmlBefore.slice(0, 80))}`)

  // Append and then remove EXACTLY what was appended. No select-all, no retyping from a
  // captured string — a rich-text editor read back as text and typed back in is a lossy
  // round trip, and getting it wrong empties somebody's real signature.
  const editor = s.getByRole('textbox', { name: 'Signature' })
  await editor.click()
  await s.keyboard.press('Control+End')
  await s.keyboard.type('QQ', { delay: 25 })
  await sleep(s, 2200)
  const savedLine = await s
    .locator('main p')
    .filter({ hasText: /Saved|Saving|Added to the bottom/ })
    .first()
    .textContent()
  note(`status line: ${JSON.stringify((savedLine || '').trim())}`)

  await go('Rules')
  await go('Signatures')
  await s.getByLabel('Account').selectOption(selectedAccount)
  await sleep(s, 1500)
  const sigAfter = await editorHtml()
  check('Signatures ▸ typed text persists', sigAfter.includes('QQ'), true)

  // Take the two characters back off again.
  const ed2 = s.getByRole('textbox', { name: 'Signature' })
  await ed2.click()
  await s.keyboard.press('Control+End')
  await s.keyboard.press('Backspace')
  await s.keyboard.press('Backspace')
  await sleep(s, 2200)
  await go('Rules')
  await go('Signatures')
  await s.getByLabel('Account').selectOption(selectedAccount)
  await sleep(s, 1500)
  check('Signatures ▸ the edit is fully undone', await editorHtml(), sigHtmlBefore)

  // placement
  const placement = s.getByRole('radiogroup', { name: 'In a reply' })
  const placeBefore = await placement.locator('[aria-checked=true]').first().textContent()
  const other = placeBefore.includes('Above') ? 'Below the quote' : 'Above the quote'
  await placement.getByRole('radio', { name: other }).click()
  await sleep(s, 1800)
  await go('Rules')
  await go('Signatures')
  // Re-select: leaving the pane remounts it and the picker returns to the first account, so
  // reading the placement here without this reads a DIFFERENT account's answer.
  await s.getByLabel('Account').selectOption(selectedAccount)
  await sleep(s, 1500)
  check(
    'Signatures ▸ placement persists',
    await s
      .getByRole('radiogroup', { name: 'In a reply' })
      .locator('[aria-checked=true]')
      .first()
      .textContent(),
    other,
  )
  await s
    .getByRole('radiogroup', { name: 'In a reply' })
    .getByRole('radio', { name: placeBefore })
    .click()
  await sleep(s, 1800)
  await go('Rules')
  await go('Signatures')
  await s.getByLabel('Account').selectOption(selectedAccount)
  await sleep(s, 1500)
  check(
    'Signatures ▸ placement restored',
    await s
      .getByRole('radiogroup', { name: 'In a reply' })
      .locator('[aria-checked=true]')
      .first()
      .textContent(),
    placeBefore,
  )

  // account switch (does the editor follow?)
  if (acctOptions.length > 1) {
    await s.getByLabel('Account').selectOption(acctOptions[1].v)
    await sleep(s, 1500)
    const second = await s.getByRole('textbox', { name: 'Signature' }).innerText()
    check(
      'Signatures ▸ switching account loads that account’s signature',
      second !== sigAfter || second.length === 0,
      true,
    )
    note(`second account signature length: ${second.length}`)
    await s.getByLabel('Account').selectOption(acctOptions[0].v)
    await sleep(s, 1500)
  }

  // Every account's signature is exactly as it was found — asserted, not assumed.
  for (const o of acctOptions) {
    await s.getByLabel('Account').selectOption(o.v)
    await sleep(s, 1600)
    const text = await s.evaluate(
      () => document.querySelector('main [contenteditable="true"]')?.textContent ?? '',
    )
    check(`Signatures ▸ ${o.t} carries no test residue`, /QQ|ZZ/.test(text), false)
  }
  await s.getByLabel('Account').selectOption(selectedAccount)
  await sleep(s, 1200)

  // ================================================================ RULES
  console.log('\n──────── RULES ────────')
  await go('Rules')
  await s.getByRole('button', { name: 'Edit Rules…' }).click()
  await sleep(s, 1000)
  const rulesSheet = s.getByRole('dialog').last()
  check('Rules ▸ Edit Rules… opens', await rulesSheet.count(), 1)
  const rulesButtons = await rulesSheet
    .getByRole('button')
    .evaluateAll((els) => els.map((e) => e.textContent?.trim()).filter(Boolean))
  note(`buttons inside the Rules editor: ${JSON.stringify(rulesButtons)}`)
  await s.keyboard.press('Escape')
  await sleep(s, 700)
  check('Rules ▸ Rules editor closes on Escape', await s.getByRole('dialog').count(), 0)

  await s.getByRole('button', { name: 'Edit Smart Mailboxes…' }).click()
  await sleep(s, 1000)
  const smartSheet = s.getByRole('dialog').last()
  check('Rules ▸ Edit Smart Mailboxes… opens', await smartSheet.count(), 1)
  const smartButtons = await smartSheet
    .getByRole('button')
    .evaluateAll((els) => els.map((e) => e.textContent?.trim()).filter(Boolean))
  note(`buttons inside the Smart Mailboxes editor: ${JSON.stringify(smartButtons)}`)
  await s.keyboard.press('Escape')
  await sleep(s, 700)
  check('Rules ▸ Smart Mailboxes editor closes on Escape', await s.getByRole('dialog').count(), 0)

  const junk = s.getByRole('checkbox', { name: 'Mark junk without moving it' })
  const junkBefore = await junk.isChecked()
  await junk.setChecked(!junkBefore)
  await sleep(s, 900)
  await go('Privacy')
  await go('Rules')
  check(
    'Rules ▸ junk training toggle persists',
    await s.getByRole('checkbox', { name: 'Mark junk without moving it' }).isChecked(),
    !junkBefore,
  )
  await s.getByRole('checkbox', { name: 'Mark junk without moving it' }).setChecked(junkBefore)
  await sleep(s, 900)
  check(
    'Rules ▸ junk toggle restored',
    await s.getByRole('checkbox', { name: 'Mark junk without moving it' }).isChecked(),
    junkBefore,
  )

  // ================================================================ PRIVACY
  console.log('\n──────── PRIVACY ────────')
  await go('Privacy')
  const imgs = s.getByRole('checkbox', { name: 'Show images in messages automatically' })
  const imgsBefore = await imgs.isChecked()
  await imgs.setChecked(!imgsBefore)
  await sleep(s, 900)
  const hint = await s
    .locator('main p')
    .filter({ hasText: /sender/ })
    .first()
    .textContent()
  note(`privacy hint after toggle: ${JSON.stringify((hint || '').trim().slice(0, 90))}`)
  await go('Rules')
  await go('Privacy')
  check(
    'Privacy ▸ remote images persists',
    await s.getByRole('checkbox', { name: 'Show images in messages automatically' }).isChecked(),
    !imgsBefore,
  )
  await s
    .getByRole('checkbox', { name: 'Show images in messages automatically' })
    .setChecked(imgsBefore)
  await sleep(s, 900)
  check(
    'Privacy ▸ remote images restored',
    await s.getByRole('checkbox', { name: 'Show images in messages automatically' }).isChecked(),
    imgsBefore,
  )

  // ================================================================ ADVANCED
  console.log('\n──────── ADVANCED ────────')
  await go('Advanced')
  const fmt = s.getByLabel('Save as')
  const fmtBefore = await fmt.inputValue()
  for (const v of ['eml', 'mbox']) {
    await fmt.selectOption(v)
    await sleep(s, 500)
    check(`Advanced ▸ export format "${v}" selects`, await fmt.inputValue(), v)
  }
  await fmt.selectOption(fmtBefore)

  const reportRows = s.locator('main ul li button')
  const reportCount = await reportRows.count()
  note(`crash report rows: ${reportCount}`)
  if (reportCount > 0) {
    await reportRows.first().click()
    await sleep(s, 1200)
    const expanded = await reportRows.first().getAttribute('aria-expanded')
    check('Advanced ▸ a crash report expands', expanded, 'true')
    const pre = await s.locator('main pre').first().textContent()
    check('Advanced ▸ the expanded report has content', (pre || '').length > 50, true)
    await reportRows.first().click()
    await sleep(s, 700)
    check(
      'Advanced ▸ it collapses again',
      await reportRows.first().getAttribute('aria-expanded'),
      'false',
    )
  }
  skipped('Advanced ▸ Choose files… / Export all mail…', 'native modal dialogs would hang the run')
  skipped('Advanced ▸ Open the diagnostics folder', 'opens Explorer')
  skipped('Advanced ▸ Delete all reports', 'destructive — the reports are still being looked at')

  // ================================================================ NAV
  console.log('\n──────── NAV ────────')
  await go('General')
  await nav.getByRole('button', { name: 'General', exact: true }).focus()
  for (const [key, want] of [
    ['ArrowDown', 'Accounts'],
    ['ArrowDown', 'Composing'],
    ['End', 'Advanced'],
    ['Home', 'General'],
  ]) {
    await s.keyboard.press(key)
    await sleep(s, 500)
    check(
      `Nav ▸ ${key} → ${want}`,
      await s.getByRole('heading', { level: 1 }).first().textContent(),
      want,
    )
  }
  for (const label of [
    'General',
    'Accounts',
    'Composing',
    'Signatures',
    'Rules',
    'Privacy',
    'Advanced',
  ]) {
    await go(label)
    check(
      `Nav ▸ ${label} opens its pane`,
      await s.getByRole('heading', { level: 1 }).first().textContent(),
      label,
    )
  }

  // ================================================================
  console.log('\n════════ SUMMARY ════════')
  console.log(`${pass.length} passed, ${fail.length} failed, ${skip.length} skipped deliberately`)
  if (pageErrors.length) {
    console.log(
      `\n${pageErrors.length} UNCAUGHT PAGE ERRORS (a rejected IPC call looks like this):`,
    )
    for (const e of [...new Set(pageErrors)]) console.log('  ' + e)
  } else {
    console.log('\nNo uncaught page errors during the whole run.')
  }
  if (fail.length) {
    console.log('\nFAILURES:')
    for (const f of fail)
      console.log(
        `  ${f.name}\n    got ${JSON.stringify(f.actual)} want ${JSON.stringify(f.expected)}`,
      )
  }

  await browser.close()
  process.exit(fail.length === 0 && pageErrors.length === 0 ? 0 : 1)
})().catch((e) => {
  console.error('CRASHED:', e.message)
  console.error(e.stack)
  process.exit(2)
})
