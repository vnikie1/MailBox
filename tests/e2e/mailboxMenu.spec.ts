import { expect, test, type Locator, type Page } from '@playwright/test'

/**
 * The mailbox context menu, row by row, against Mail's own.
 *
 * Built from a capture of Mail's menu on an account's inbox (under All Inboxes):
 *
 *   New Mailbox… | Add to Favourites, Export Mailbox… | Erase Deleted Items…, Erase Junk Mail…,
 *   Mark All Messages as Read | Synchronise “Google”, Edit “Google”… | Get Account Info
 *
 * with the two rows docs/01 §3 adds, Use This Mailbox As ▸ and Rebuild, and Favourites that move
 * by drag.
 *
 * These run against the browser store, whose folder commands follow the same rules and say the
 * same sentences as `sync::folders`. What they cannot reach is the server half — the queued
 * CREATE, RENAME, DELETE and erase, and a rebuild's reading of the mailbox — which
 * `src-tauri/tests/folders_gate.rs` drives against a real IMAP server.
 */

function mailboxMenu(page: Page): Locator {
  return page.getByRole('menu', { name: 'Mailbox actions' })
}

function entry(page: Page, name: string): Locator {
  return mailboxMenu(page).getByRole('menuitem', { name, exact: true })
}

function section(page: Page, name: string): Locator {
  return page.getByRole('group', { name, exact: true })
}

/** A sidebar row by its label, ignoring the unread count drawn beside it. */
function row(page: Page, sectionName: string, label: string): Locator {
  return section(page, sectionName)
    .getByRole('treeitem')
    .filter({ has: page.getByText(label, { exact: true }) })
    .first()
}

async function openMenu(page: Page, sectionName: string, label: string): Promise<void> {
  await row(page, sectionName, label).click({ button: 'right' })
  await expect(mailboxMenu(page)).toBeVisible()
}

/** The menu top to bottom, a separator written as "—". */
async function entries(page: Page): Promise<string[]> {
  return mailboxMenu(page)
    .locator('[role="menuitem"], [role="separator"]')
    .evaluateAll((nodes) =>
      nodes.map((node) =>
        node.getAttribute('role') === 'separator' ? '—' : node.textContent.trim(),
      ),
    )
}

/** The labels of a section's rows, without their unread counts. */
async function labels(page: Page, sectionName: string): Promise<string[]> {
  const texts = await section(page, sectionName).getByRole('treeitem').allTextContents()
  return texts.map((text) => text.replace(/\d+$/, '').trim())
}

/**
 * The row's unread count, from the badge's own label ("184 unread").
 *
 * Not from the row's text: the label and the count run together there, and a folder whose name
 * ends in a number reads as a much larger count — which is exactly how the first run against the
 * real app misread "App E2E Renamed 185822" with three unread.
 */
async function unreadOf(target: Locator): Promise<number> {
  // Read in one step. Counting the badge and then asking for its label raced the badge going
  // away — the very thing the caller is waiting for — and the second call then waited for good.
  const labels = await target
    .locator('[aria-label$=" unread"]')
    .evaluateAll((nodes) => nodes.map((node) => node.getAttribute('aria-label') ?? ''))
  const [label] = labels
  return label === undefined ? 0 : Number(label.split(' ')[0])
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('tree', { name: 'Mailboxes' })).toBeVisible()
})

test.describe('the rows', () => {
  test('on an account’s inbox under All Inboxes, are Mail’s rows in Mail’s order', async ({
    page,
  }) => {
    // The exact case in the capture: the account's row under All Inboxes.
    await openMenu(page, 'Favourites', 'Northgate')

    expect(await entries(page)).toEqual([
      'New Mailbox…',
      '—',
      'Add to Favourites',
      'Export Mailbox…',
      'Rebuild',
      '—',
      'Erase Deleted Items…',
      'Erase Junk Mail…',
      'Mark All Messages as Read',
      '—',
      'Synchronise “Northgate”',
      'Edit “Northgate”…',
      '—',
      'Get Account Info',
    ])
  })

  test('on a folder the user made, Rename, Delete and Use This Mailbox As follow New Mailbox', async ({
    page,
  }) => {
    await openMenu(page, 'Northgate', 'Clients')

    expect((await entries(page)).slice(0, 5)).toEqual([
      'New Mailbox…',
      'Rename Mailbox…',
      'Delete Mailbox…',
      'Use This Mailbox As',
      '—',
    ])
  })

  test('are absent for the folders the account files into', async ({ page }) => {
    for (const mailbox of ['Inbox', 'Drafts', 'Sent', 'Junk', 'Bin', 'Archive']) {
      await openMenu(page, 'Northgate', mailbox)
      await expect(entry(page, 'Rename Mailbox…')).toHaveCount(0)
      await expect(entry(page, 'Delete Mailbox…')).toHaveCount(0)
      await page.keyboard.press('Escape')
      await expect(mailboxMenu(page)).toBeHidden()
    }
  })

  test('can be walked and chosen from the keyboard', async ({ page }) => {
    await openMenu(page, 'Northgate', 'Clients')

    await page.keyboard.press('ArrowDown')
    await expect(entry(page, 'New Mailbox…')).toBeFocused()
    await page.keyboard.press('ArrowDown')
    await expect(entry(page, 'Rename Mailbox…')).toBeFocused()
    await page.keyboard.press('Enter')

    await expect(page.getByRole('dialog', { name: 'Rename Mailbox' })).toBeVisible()
  })
})

test.describe('New Mailbox', () => {
  test('makes a folder at the top of the account, from a mailbox that holds none', async ({
    page,
  }) => {
    // The Inbox cannot hold another mailbox, so the sheet opens on its account.
    await openMenu(page, 'Northgate', 'Inbox')
    await entry(page, 'New Mailbox…').click()

    const sheet = page.getByRole('dialog', { name: 'New Mailbox' })
    await expect(sheet).toBeVisible()
    await expect(sheet.getByRole('combobox', { name: 'Location' })).toHaveValue('account-1')

    const name = sheet.getByRole('textbox', { name: 'Name' })
    await expect(name).toBeFocused()
    await name.fill('Invoices')
    await name.press('Enter')

    await expect(sheet).toBeHidden()
    await expect(row(page, 'Northgate', 'Invoices')).toBeVisible()

    // Among the user's own folders, alphabetically — after the ones the account files into.
    const order = await labels(page, 'Northgate')
    expect(order.slice(order.indexOf('Archive') + 1)).toEqual([
      'Clients',
      'Contracts',
      'Invoices',
      'Receipts',
      'Travel',
    ])
  })

  test('says what is wrong with a name before it is sent', async ({ page }) => {
    await openMenu(page, 'Northgate', 'Clients')
    await entry(page, 'New Mailbox…').click()

    const sheet = page.getByRole('dialog', { name: 'New Mailbox' })
    const name = sheet.getByRole('textbox', { name: 'Name' })
    const create = sheet.getByRole('button', { name: 'Create' })

    await expect(create).toBeDisabled()

    await name.fill('Q3/Q4')
    await expect(sheet.getByText('A mailbox name can’t contain “/”.')).toBeVisible()
    await expect(create).toBeDisabled()

    await name.fill('50%')
    await expect(sheet.getByText('A mailbox name can’t contain “%” or “*”.')).toBeVisible()

    await name.fill('   ')
    await expect(create).toBeDisabled()
  })

  test('keeps the sheet open when the name is taken, and a new place clears it', async ({
    page,
  }) => {
    await openMenu(page, 'Northgate', 'Clients')
    await entry(page, 'New Mailbox…').click()

    const sheet = page.getByRole('dialog', { name: 'New Mailbox' })
    // At the top of the account, where Clients already is.
    await sheet.getByRole('combobox', { name: 'Location' }).selectOption({ label: 'Northgate' })
    await sheet.getByRole('textbox', { name: 'Name' }).fill('clients')
    await sheet.getByRole('button', { name: 'Create' }).click()

    await expect(sheet.getByText('There’s already a mailbox called “clients”.')).toBeVisible()
    await expect(sheet).toBeVisible()

    // The same name is free in another account.
    await sheet.getByRole('combobox', { name: 'Location' }).selectOption({ label: 'iCloud' })
    await expect(sheet.getByText('There’s already a mailbox called “clients”.')).toBeHidden()
    await sheet.getByRole('button', { name: 'Create' }).click()

    await expect(sheet).toBeHidden()
    await expect(row(page, 'iCloud', 'clients')).toBeVisible()
  })

  test('makes nothing when cancelled', async ({ page }) => {
    const before = await labels(page, 'Northgate')

    await openMenu(page, 'Northgate', 'Clients')
    await entry(page, 'New Mailbox…').click()
    const sheet = page.getByRole('dialog', { name: 'New Mailbox' })
    await sheet.getByRole('textbox', { name: 'Name' }).fill('Never')
    await page.keyboard.press('Escape')

    await expect(sheet).toBeHidden()
    expect(await labels(page, 'Northgate')).toEqual(before)
  })
})

test.describe('Folders inside folders', () => {
  async function makeInside(page: Page, parent: string, name: string): Promise<void> {
    await openMenu(page, 'Northgate', parent)
    await entry(page, 'New Mailbox…').click()
    const sheet = page.getByRole('dialog', { name: 'New Mailbox' })
    await sheet.getByRole('textbox', { name: 'Name' }).fill(name)
    await sheet.getByRole('button', { name: 'Create' }).click()
    await expect(sheet).toBeHidden()
  }

  test('a folder made from another’s menu goes inside it, and the parent opens and closes', async ({
    page,
  }) => {
    await openMenu(page, 'Northgate', 'Clients')
    await entry(page, 'New Mailbox…').click()

    const sheet = page.getByRole('dialog', { name: 'New Mailbox' })
    const location = sheet.getByRole('combobox', { name: 'Location' })
    await expect(location).toHaveValue(/^mailbox-\d+$/)
    await expect(location.locator('option:checked')).toHaveText(/Clients$/)

    await sheet.getByRole('textbox', { name: 'Name' }).fill('Acme')
    await sheet.getByRole('button', { name: 'Create' }).click()
    await expect(sheet).toBeHidden()

    await expect(row(page, 'Northgate', 'Acme')).toHaveAttribute('aria-level', '2')
    const order = await labels(page, 'Northgate')
    const at = order.indexOf('Clients')
    expect(order.slice(at, at + 3)).toEqual(['Clients', 'Acme', 'Contracts'])

    const clients = row(page, 'Northgate', 'Clients')
    await expect(clients).toHaveAttribute('aria-expanded', 'true')
    await clients.getByRole('button', { name: 'Collapse Clients' }).click()
    await expect(row(page, 'Northgate', 'Acme')).toHaveCount(0)
    await clients.getByRole('button', { name: 'Expand Clients' }).click()
    await expect(row(page, 'Northgate', 'Acme')).toBeVisible()
  })

  test('the location lists every folder that can hold another, as the sidebar nests them', async ({
    page,
  }) => {
    await makeInside(page, 'Clients', 'Acme')

    await openMenu(page, 'Northgate', 'Inbox')
    await entry(page, 'New Mailbox…').click()
    const sheet = page.getByRole('dialog', { name: 'New Mailbox' })
    const options = await sheet
      .getByRole('combobox', { name: 'Location' })
      .locator('option')
      .allTextContents()

    // Each account, then its folders indented beneath it. Never an Inbox, which holds none.
    expect(options.slice(0, 11)).toEqual([
      'Northgate',
      '  Drafts',
      '  Sent',
      '  Junk',
      '  Bin',
      '  Archive',
      '  Clients',
      '    Acme',
      '  Contracts',
      '  Receipts',
      '  Travel',
    ])
    expect(options[11]).toBe('iCloud')
    expect(options.some((option) => option.trim() === 'Inbox')).toBe(false)
  })

  test('a name is taken only inside the same folder', async ({ page }) => {
    // Contracts exists at the top of the account; inside Clients the name is free.
    await makeInside(page, 'Clients', 'Contracts')
    await expect(
      section(page, 'Northgate').locator('[role="treeitem"][aria-level="2"]', {
        hasText: 'Contracts',
      }),
    ).toHaveCount(1)

    await openMenu(page, 'Northgate', 'Clients')
    await entry(page, 'New Mailbox…').click()
    const sheet = page.getByRole('dialog', { name: 'New Mailbox' })
    await sheet.getByRole('textbox', { name: 'Name' }).fill('contracts')
    await sheet.getByRole('button', { name: 'Create' }).click()

    await expect(
      sheet.getByText('There’s already a mailbox called “contracts” in “Clients”.'),
    ).toBeVisible()
  })

  test('a renamed folder takes the folders inside it along', async ({ page }) => {
    await makeInside(page, 'Clients', 'Acme')

    await openMenu(page, 'Northgate', 'Clients')
    await entry(page, 'Rename Mailbox…').click()
    const sheet = page.getByRole('dialog', { name: 'Rename Mailbox' })
    await sheet.getByRole('textbox', { name: 'Name' }).fill('Customers')
    await sheet.getByRole('button', { name: 'Rename' }).click()
    await expect(sheet).toBeHidden()

    const order = await labels(page, 'Northgate')
    const at = order.indexOf('Customers')
    expect(order.slice(at, at + 2)).toEqual(['Customers', 'Acme'])
    await expect(row(page, 'Northgate', 'Acme')).toHaveAttribute('aria-level', '2')
  })

  test('deleting a folder says it takes the one inside, and does', async ({ page }) => {
    await makeInside(page, 'Clients', 'Acme')

    await openMenu(page, 'Northgate', 'Clients')
    await entry(page, 'Delete Mailbox…').click()
    const sheet = page.getByRole('dialog', { name: 'Delete “Clients”?' })
    await expect(sheet).toContainText(
      '“Clients” and the mailbox inside it and every message in them will be deleted',
    )
    await sheet.getByRole('button', { name: 'Delete' }).click()
    await expect(sheet).toBeHidden()

    await expect(row(page, 'Northgate', 'Clients')).toHaveCount(0)
    await expect(row(page, 'Northgate', 'Acme')).toHaveCount(0)
  })
})

test.describe('Use This Mailbox As', () => {
  async function openRoles(page: Page, sectionName: string, label: string): Promise<Locator> {
    await openMenu(page, sectionName, label)
    await entry(page, 'Use This Mailbox As').hover()
    const roles = page.getByRole('menu', { name: 'Use This Mailbox As' })
    await expect(roles).toBeVisible()
    return roles
  }

  test('ticks the role the mailbox has', async ({ page }) => {
    const roles = await openRoles(page, 'Northgate', 'Bin')

    await expect(roles.getByRole('menuitemcheckbox')).toHaveText([
      'Drafts',
      'Sent',
      'Junk',
      'Bin',
      'Archive',
    ])
    await expect(roles.getByRole('menuitemcheckbox', { name: 'Bin' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    await expect(roles.getByRole('menuitemcheckbox', { name: 'Archive' })).toHaveAttribute(
      'aria-checked',
      'false',
    )
  })

  test('gives the role to the mailbox chosen, and takes it from the one that had it', async ({
    page,
  }) => {
    const roles = await openRoles(page, 'Northgate', 'Contracts')
    await roles.getByRole('menuitemcheckbox', { name: 'Archive' }).click()
    await expect(roles).toBeHidden()

    // Contracts joins the folders the account files into, in Archive's place; the old Archive is
    // one of the user's own now, alphabetically.
    await expect
      .poll(() => labels(page, 'Northgate'))
      .toEqual([
        'Inbox',
        'Drafts',
        'Sent',
        'Junk',
        'Bin',
        'Contracts',
        'Archive',
        'Clients',
        'Receipts',
        'Travel',
      ])

    // And the menu agrees: Contracts is the account's now, Archive the user's.
    await openMenu(page, 'Northgate', 'Contracts')
    await expect(entry(page, 'Rename Mailbox…')).toHaveCount(0)
    await page.keyboard.press('Escape')
    await openMenu(page, 'Northgate', 'Archive')
    await expect(entry(page, 'Rename Mailbox…')).toBeVisible()
  })

  test('is not offered on an Inbox, nor on Gmail, which decides its own', async ({ page }) => {
    await openMenu(page, 'Northgate', 'Inbox')
    await expect(entry(page, 'Use This Mailbox As')).toHaveCount(0)
    await page.keyboard.press('Escape')

    await openMenu(page, 'Gmail', 'Newsletters')
    await expect(entry(page, 'Use This Mailbox As')).toHaveCount(0)
  })
})

test.describe('Rebuild', () => {
  test('says when it starts and when it has finished', async ({ page }) => {
    await openMenu(page, 'Northgate', 'Clients')
    await entry(page, 'Rebuild').click()

    await expect(page.getByText('Rebuilding “Clients”')).toBeVisible()
    await expect(page.getByText('Rebuilt “Clients”')).toBeVisible()
  })
})

test.describe('Favourites, reordered', () => {
  /** The top-level rows of Favourites — what Ctrl+1–9 walk — without their counts. */
  async function topLevel(page: Page): Promise<string[]> {
    const texts = await section(page, 'Favourites')
      .locator('[role="treeitem"][aria-level="1"]')
      .allTextContents()
    return texts.map((text) => text.replace(/\d+$/, '').trim())
  }

  async function box(target: Locator) {
    const found = await target.boundingBox()
    if (found === null) throw new Error('the row is not on screen')
    return found
  }

  test('show where a favourite will land, without moving any row', async ({ page }) => {
    const flagged = row(page, 'Favourites', 'Flagged')
    const drafts = row(page, 'Favourites', 'All Drafts')
    const from = await box(flagged)
    const onto = await box(drafts)

    await page.mouse.move(from.x + 24, from.y + from.height / 2)
    await page.mouse.down()
    await page.mouse.move(onto.x + 24, onto.y + onto.height - 4, { steps: 8 })

    await expect(drafts).toHaveAttribute('data-insert', 'after')
    await expect(flagged).toHaveAttribute('data-dragging', '')
    expect(await box(drafts)).toEqual(onto)

    await page.mouse.up()
    await expect
      .poll(() => topLevel(page))
      .toEqual(['All Inboxes', 'All Drafts', 'Flagged', 'All Sent'])
    await expect(section(page, 'Favourites').locator('[data-insert]')).toHaveCount(0)
  })

  test('a favourite dragged above another lands there, and Ctrl+1 follows', async ({ page }) => {
    await row(page, 'Favourites', 'Flagged').dragTo(row(page, 'Favourites', 'All Inboxes'), {
      targetPosition: { x: 24, y: 4 },
    })

    await expect
      .poll(() => topLevel(page))
      .toEqual(['Flagged', 'All Inboxes', 'All Drafts', 'All Sent'])

    await page.getByRole('tree', { name: 'Mailboxes' }).click({ position: { x: 5, y: 5 } })
    await page.keyboard.press('Control+1')
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Flagged')
  })

  test('a folder dragged in from its account lands where it is dropped', async ({ page }) => {
    await row(page, 'Northgate', 'Receipts').dragTo(row(page, 'Favourites', 'All Drafts'), {
      targetPosition: { x: 24, y: 4 },
    })

    await expect
      .poll(() => topLevel(page))
      .toEqual(['All Inboxes', 'Flagged', 'Receipts', 'All Drafts', 'All Sent'])

    // Still in its account, where it is now a favourite too.
    await expect(row(page, 'Northgate', 'Receipts')).toBeVisible()
    await openMenu(page, 'Northgate', 'Receipts')
    await expect(entry(page, 'Remove from Favourites')).toBeVisible()
  })

  test('Alt+Up and Alt+Down move the focused favourite, and say where to', async ({ page }) => {
    const drafts = row(page, 'Favourites', 'All Drafts')
    await drafts.focus()

    await page.keyboard.press('Alt+ArrowUp')
    await expect
      .poll(() => topLevel(page))
      .toEqual(['All Inboxes', 'All Drafts', 'Flagged', 'All Sent'])
    await expect(row(page, 'Favourites', 'All Drafts')).toBeFocused()
    await expect(
      page.getByRole('status').filter({ hasText: 'All Drafts moved to position 2 of 4' }),
    ).toHaveCount(1)

    await page.keyboard.press('Alt+ArrowDown')
    await expect
      .poll(() => topLevel(page))
      .toEqual(['All Inboxes', 'Flagged', 'All Drafts', 'All Sent'])
    await page.keyboard.press('Alt+ArrowDown')
    await expect
      .poll(() => topLevel(page))
      .toEqual(['All Inboxes', 'Flagged', 'All Sent', 'All Drafts'])

    // At the bottom, Down has nowhere to go.
    await page.keyboard.press('Alt+ArrowDown')
    await expect(row(page, 'Favourites', 'All Drafts')).toBeFocused()
    expect(await topLevel(page)).toEqual(['All Inboxes', 'Flagged', 'All Sent', 'All Drafts'])
  })

  test('a row outside Favourites does not move with Alt+Up', async ({ page }) => {
    const before = await labels(page, 'Northgate')
    await row(page, 'Northgate', 'Receipts').focus()
    await page.keyboard.press('Alt+ArrowUp')
    expect(await labels(page, 'Northgate')).toEqual(before)
  })
})

test.describe('Rename Mailbox', () => {
  test('renames the folder where it is, starting from its name selected', async ({ page }) => {
    await openMenu(page, 'Northgate', 'Travel')
    await entry(page, 'Rename Mailbox…').click()

    const sheet = page.getByRole('dialog', { name: 'Rename Mailbox' })
    const name = sheet.getByRole('textbox', { name: 'Name' })
    await expect(name).toHaveValue('Travel')
    await expect(name).toBeFocused()
    await expect
      .poll(() =>
        name.evaluate((input: HTMLInputElement) => [input.selectionStart, input.selectionEnd]),
      )
      .toEqual([0, 6])

    // Nothing to do until the name changes.
    await expect(sheet.getByRole('button', { name: 'Rename' })).toBeDisabled()

    // Typing replaces the selected name.
    await page.keyboard.type('Trips')
    await page.keyboard.press('Enter')

    await expect(sheet).toBeHidden()
    await expect(row(page, 'Northgate', 'Trips')).toBeVisible()
    await expect(row(page, 'Northgate', 'Travel')).toHaveCount(0)
  })

  test('refuses a name the account already has', async ({ page }) => {
    await openMenu(page, 'Northgate', 'Travel')
    await entry(page, 'Rename Mailbox…').click()

    const sheet = page.getByRole('dialog', { name: 'Rename Mailbox' })
    await sheet.getByRole('textbox', { name: 'Name' }).fill('Receipts')
    await sheet.getByRole('button', { name: 'Rename' }).click()

    await expect(sheet.getByText('There’s already a mailbox called “Receipts”.')).toBeVisible()
    await expect(sheet).toBeVisible()

    // Checked once the sheet is gone: while it is open the page behind it is hidden from the
    // accessibility tree, sidebar included.
    await page.keyboard.press('Escape')
    await expect(sheet).toBeHidden()
    await expect(row(page, 'Northgate', 'Travel')).toBeVisible()
    await expect(row(page, 'Northgate', 'Receipts')).toHaveCount(1)
  })

  test('keeps the open folder open under its new name', async ({ page }) => {
    await row(page, 'Northgate', 'Travel').click()
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Travel')

    await openMenu(page, 'Northgate', 'Travel')
    await entry(page, 'Rename Mailbox…').click()
    const sheet = page.getByRole('dialog', { name: 'Rename Mailbox' })
    await sheet.getByRole('textbox', { name: 'Name' }).fill('Trips')
    await sheet.getByRole('button', { name: 'Rename' }).click()

    await expect(row(page, 'Northgate', 'Trips')).toHaveAttribute('aria-selected', 'true')
  })
})

test.describe('Delete Mailbox', () => {
  test('asks first, and says what will go', async ({ page }) => {
    await openMenu(page, 'Northgate', 'Contracts')
    await entry(page, 'Delete Mailbox…').click()

    const sheet = page.getByRole('dialog', { name: 'Delete “Contracts”?' })
    await expect(sheet).toBeVisible()
    await expect(sheet).toContainText(
      '“Contracts” and every message in it will be deleted, on this computer and on the server.',
    )
    await expect(sheet).toContainText('This can’t be undone.')

    await sheet.getByRole('button', { name: 'Cancel' }).click()
    await expect(sheet).toBeHidden()
    await expect(row(page, 'Northgate', 'Contracts')).toBeVisible()
  })

  test('removes the folder once confirmed', async ({ page }) => {
    await openMenu(page, 'Northgate', 'Contracts')
    await entry(page, 'Delete Mailbox…').click()

    const sheet = page.getByRole('dialog', { name: 'Delete “Contracts”?' })
    await sheet.getByRole('button', { name: 'Delete' }).click()

    await expect(sheet).toBeHidden()
    await expect(row(page, 'Northgate', 'Contracts')).toHaveCount(0)
    await expect(page.getByText('Deleted “Contracts”')).toBeVisible()
  })

  test('moves off the folder if it was the one open', async ({ page }) => {
    await row(page, 'Northgate', 'Contracts').click()
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Contracts')

    await openMenu(page, 'Northgate', 'Contracts')
    await entry(page, 'Delete Mailbox…').click()
    await page
      .getByRole('dialog', { name: 'Delete “Contracts”?' })
      .getByRole('button', { name: 'Delete' })
      .click()

    // Not an empty pane under a name that no longer exists.
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Inbox')
    await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()
  })

  test('on Gmail, says a label goes and the mail stays', async ({ page }) => {
    await openMenu(page, 'Gmail', 'Newsletters')
    await entry(page, 'Delete Mailbox…').click()

    const sheet = page.getByRole('dialog', { name: 'Delete “Newsletters”?' })
    await expect(sheet).toContainText('The label “Newsletters” will be removed from Gmail.')
    await expect(sheet).toContainText('they stay in All Mail')
    await sheet.getByRole('button', { name: 'Delete' }).click()

    await expect(page.getByText('Removed the label “Newsletters”')).toBeVisible()
  })
})

test.describe('Favourites', () => {
  test('a folder added goes to the end of Favourites, and opens from there', async ({ page }) => {
    await openMenu(page, 'Northgate', 'Clients')
    await entry(page, 'Add to Favourites').click()

    // Polled, not read once. `labels` is a snapshot, and the click returns before the section
    // re-renders — see the removal test below, which failed a full `npm run verify` that way.
    await expect.poll(async () => (await labels(page, 'Favourites')).at(-1)).toBe('Clients')

    await row(page, 'Favourites', 'Clients').click()
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Clients')
    // Keyed by row: the favourite is lit, not the folder's own row as well.
    await expect(row(page, 'Favourites', 'Clients')).toHaveAttribute('aria-selected', 'true')
    await expect(row(page, 'Northgate', 'Clients')).toHaveAttribute('aria-selected', 'false')
  })

  test('the row says Remove once the folder is a favourite, from either place', async ({
    page,
  }) => {
    await openMenu(page, 'Northgate', 'Clients')
    await entry(page, 'Add to Favourites').click()

    await openMenu(page, 'Northgate', 'Clients')
    await expect(entry(page, 'Remove from Favourites')).toBeVisible()
    await expect(entry(page, 'Add to Favourites')).toHaveCount(0)
    await page.keyboard.press('Escape')

    await openMenu(page, 'Favourites', 'Clients')
    await entry(page, 'Remove from Favourites').click()

    // Polled. A one-off read raced the re-render: on 2026-10-02 a full gate run caught the
    // section with "Clients" still in it, a moment after the removal it was about to show.
    await expect.poll(() => labels(page, 'Favourites')).not.toContain('Clients')
  })

  test('removing the open favourite keeps the folder open, on its own row', async ({ page }) => {
    await openMenu(page, 'Northgate', 'Clients')
    await entry(page, 'Add to Favourites').click()
    await row(page, 'Favourites', 'Clients').click()

    await openMenu(page, 'Favourites', 'Clients')
    await entry(page, 'Remove from Favourites').click()

    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Clients')
    await expect(row(page, 'Northgate', 'Clients')).toHaveAttribute('aria-selected', 'true')
  })

  test('an account’s inbox is named with its account', async ({ page }) => {
    // Every account has an Inbox; "Inbox" alone would not say whose.
    await openMenu(page, 'iCloud', 'Inbox')
    await entry(page, 'Add to Favourites').click()

    await expect(row(page, 'Favourites', 'Inbox – iCloud')).toBeVisible()
  })

  test('a favourite takes the next Ctrl-number, and the ones before it keep theirs', async ({
    page,
  }) => {
    await openMenu(page, 'Northgate', 'Receipts')
    await entry(page, 'Add to Favourites').click()

    // Ctrl+1–9 walks the top-level rows of Favourites.
    const topLevel = await section(page, 'Favourites')
      .locator('[role="treeitem"][aria-level="1"]')
      .allTextContents()
    const position = topLevel.findIndex((text) => text.startsWith('Receipts')) + 1
    expect(position).toBeGreaterThan(1)

    await page.getByRole('tree', { name: 'Mailboxes' }).click({ position: { x: 5, y: 5 } })
    await page.keyboard.press(`Control+${String(position)}`)
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Receipts')

    await page.keyboard.press('Control+1')
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('All Inboxes')
  })
})

test.describe('Erase', () => {
  test('Erase Deleted Items empties the account’s Bin after asking', async ({ page }) => {
    await openMenu(page, 'Northgate', 'Clients')
    await expect(entry(page, 'Erase Deleted Items…')).toBeEnabled()
    await entry(page, 'Erase Deleted Items…').click()

    const sheet = page.getByRole('dialog', { name: 'Erase Deleted Items?' })
    await expect(sheet).toContainText(
      'Every message in “Bin” in Northgate will be permanently erased, on this computer and on the server.',
    )
    await sheet.getByRole('button', { name: 'Erase' }).click()

    await expect(sheet).toBeHidden()
    await expect(page.getByText('Erased “Bin”')).toBeVisible()

    await row(page, 'Northgate', 'Bin').click()
    await expect(page.getByText('This mailbox is empty')).toBeVisible()

    // Nothing left to erase, and the row says so.
    await openMenu(page, 'Northgate', 'Clients')
    await expect(entry(page, 'Erase Deleted Items…')).toBeDisabled()
  })

  test('Erase Junk Mail empties Junk and leaves the Bin alone', async ({ page }) => {
    await openMenu(page, 'iCloud', 'Family')
    await entry(page, 'Erase Junk Mail…').click()

    const sheet = page.getByRole('dialog', { name: 'Erase Junk Mail?' })
    await expect(sheet).toContainText('Every message in “Junk” in iCloud')
    await sheet.getByRole('button', { name: 'Erase' }).click()
    await expect(sheet).toBeHidden()

    await row(page, 'iCloud', 'Junk').click()
    await expect(page.getByText('This mailbox is empty')).toBeVisible()

    await openMenu(page, 'iCloud', 'Family')
    await expect(entry(page, 'Erase Junk Mail…')).toBeDisabled()
    await expect(entry(page, 'Erase Deleted Items…')).toBeEnabled()
  })

  test('Cancel erases nothing', async ({ page }) => {
    await openMenu(page, 'Gmail', 'Newsletters')
    await entry(page, 'Erase Deleted Items…').click()
    await page
      .getByRole('dialog', { name: 'Erase Deleted Items?' })
      .getByRole('button', { name: 'Cancel' })
      .click()

    await openMenu(page, 'Gmail', 'Newsletters')
    await expect(entry(page, 'Erase Deleted Items…')).toBeEnabled()
  })
})

test.describe('Mark All Messages as Read', () => {
  test('clears the unread count, and then has nothing to do', async ({ page }) => {
    const inbox = row(page, 'Northgate', 'Inbox')
    const before = await unreadOf(inbox)
    expect(before).toBeGreaterThan(0)

    await openMenu(page, 'Northgate', 'Inbox')
    await entry(page, 'Mark All Messages as Read').click()

    await expect(page.getByText(`Marked ${String(before)} messages as read`)).toBeVisible()
    await expect.poll(() => unreadOf(inbox)).toBe(0)

    await openMenu(page, 'Northgate', 'Inbox')
    await expect(entry(page, 'Mark All Messages as Read')).toBeDisabled()
  })
})

test.describe('the account rows', () => {
  test('Synchronise is offered for an account that syncs', async ({ page }) => {
    await openMenu(page, 'iCloud', 'Bills')
    await expect(entry(page, 'Synchronise “iCloud”')).toBeEnabled()
    await entry(page, 'Synchronise “iCloud”').click()
    await expect(mailboxMenu(page)).toBeHidden()
  })

  test('Edit opens Settings on that account', async ({ page }) => {
    await openMenu(page, 'iCloud', 'Bills')

    const opened = page.waitForEvent('popup')
    await entry(page, 'Edit “iCloud”…').click()
    const settings = await opened
    await settings.waitForLoadState()

    expect(settings.url()).toContain('pane=accounts')
    expect(settings.url()).toContain('account=2')
    await expect(settings.getByRole('heading', { level: 1 })).toHaveText('Accounts')
    // Its name field, ready to edit — not the first account's.
    await expect(settings.locator('[data-account-id="2"] input').first()).toBeFocused()
  })

  test('Get Account Info names the account the menu was opened on', async ({ page }) => {
    await openMenu(page, 'Gmail', 'Newsletters')
    await entry(page, 'Get Account Info').click()

    const sheet = page.getByRole('dialog', { name: 'Account Information' })
    await expect(sheet).toContainText('vishal.singh@gmail.example')
  })
})
