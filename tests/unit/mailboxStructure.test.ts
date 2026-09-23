import { beforeEach, describe, expect, it, vi } from 'vitest'

import {
  allNodes,
  buildSidebar,
  canOpenMailboxMenu,
  selectionForNode,
  visibleRows,
  type SidebarNode,
} from '@/features/sidebar/model'
import { isGmail, mailboxLocations } from '@/features/sidebar/useMailboxMenu'
import { beforeFor, reordered } from '@/features/sidebar/favouriteDrag'
import type { AccountDetail } from '@/lib/generated/AccountDetail'
import type { AccountRow } from '@/lib/generated/AccountRow'
import type { FavouriteRow } from '@/lib/generated/FavouriteRow'
import type { MailboxRow } from '@/lib/generated/MailboxRow'
import { MAX_MAILBOX_NAME, mailboxNameProblem } from '@/lib/mailboxName'
import type * as BrowserStore from '@/mock/browserStore'
import { useMailStore } from '@/store/mail'

/**
 * The window's half of the mailbox menu: the name rules the sheets check as you type, Favourites
 * in the sidebar, and the browser store's folder commands the Playwright suite drives.
 */

describe('a mailbox name', () => {
  it('says the same things the core says, for the same reasons', () => {
    expect(mailboxNameProblem('Receipts 2026', '/')).toBeNull()
    expect(mailboxNameProblem('  Été  ', '/')).toBeNull()

    expect(mailboxNameProblem('', '/')).toBe('Enter a name for the mailbox.')
    expect(mailboxNameProblem('   ', '/')).toBe('Enter a name for the mailbox.')
    expect(mailboxNameProblem('Two\nlines', '/')).toBe(
      'A mailbox name can’t contain tabs or line breaks.',
    )
    expect(mailboxNameProblem('Q3/Q4', '/')).toBe('A mailbox name can’t contain “/”.')
    expect(mailboxNameProblem('100%', '/')).toBe('A mailbox name can’t contain “%” or “*”.')
    expect(mailboxNameProblem('*', '/')).toBe('A mailbox name can’t contain “%” or “*”.')
  })

  it('checks the server’s own separator, and "/" until it is known', () => {
    expect(mailboxNameProblem('Q3/Q4', '.')).toBeNull()
    expect(mailboxNameProblem('v1.2', '.')).toBe('A mailbox name can’t contain “.”.')
    expect(mailboxNameProblem('Q3/Q4', null)).toBe('A mailbox name can’t contain “/”.')
  })

  it('counts characters the way Rust does', () => {
    // An emoji is two UTF-16 units and one `char`. Counting units would refuse this name at 100.
    const emoji = '😀'.repeat(MAX_MAILBOX_NAME)
    expect(emoji.length).toBe(MAX_MAILBOX_NAME * 2)
    expect(mailboxNameProblem(emoji, '/')).toBeNull()
    expect(mailboxNameProblem(`${emoji}x`, '/')).toMatch(/at most 200 characters/)
  })
})

const ACCOUNTS: AccountRow[] = [
  { id: 1, displayName: 'Google', email: 'a@gmail.test', provider: 'google', color: 'green' },
  { id: 2, displayName: 'Work', email: 'b@work.test', provider: 'other', color: null },
]

function row(overrides: Partial<MailboxRow> & Pick<MailboxRow, 'id' | 'accountId'>): MailboxRow {
  return {
    displayName: 'Folder',
    parentId: null,
    role: null,
    unreadCount: 0,
    totalCount: 0,
    favouriteOrder: null,
    roleChosen: false,
    delimiter: '/',
    editable: true,
    descendants: 0,
    canContain: true,
    ...overrides,
  }
}

const MAILBOXES: MailboxRow[] = [
  row({ id: 1, accountId: 1, displayName: 'Inbox', role: 'inbox', editable: false }),
  row({ id: 2, accountId: 2, displayName: 'Inbox', role: 'inbox', editable: false }),
  row({ id: 3, accountId: 1, displayName: 'Receipts', unreadCount: 4, favouriteOrder: 7 }),
  row({ id: 4, accountId: 2, displayName: 'Projects', favouriteOrder: 3 }),
  row({ id: 5, accountId: 2, displayName: 'Receipts' }),
]

describe('Favourites', () => {
  it('come after the rows every sidebar starts with, in the order they were added', () => {
    const [favourites] = buildSidebar(ACCOUNTS, MAILBOXES)
    const labels = favourites?.nodes.map((node) => node.label)

    expect(labels).toEqual([
      'All Inboxes',
      'Flagged',
      'All Drafts',
      'All Sent',
      'Projects',
      'Receipts',
    ])
  })

  it('are real mailbox rows: selectable, droppable, with a menu, and coloured by account', () => {
    const [favourites] = buildSidebar(ACCOUNTS, MAILBOXES)
    const receipts = favourites?.nodes.find((node) => node.id === 'favourite-3')

    expect(receipts).toMatchObject({
      mailboxIds: [3],
      accountId: 1,
      accountColor: 'green',
      unreadCount: 4,
    })
    expect(receipts && canOpenMailboxMenu(receipts)).toBe(true)
    expect(receipts && selectionForNode(receipts)).toEqual({
      nodeId: 'favourite-3',
      label: 'Receipts',
      mailboxIds: [3],
    })
  })

  it('say whose inbox they are, and tell two folders of one name apart', () => {
    const mailboxes = MAILBOXES.map((mailbox) =>
      mailbox.id === 2 || mailbox.id === 5
        ? { ...mailbox, favouriteOrder: 9 + mailbox.id }
        : mailbox,
    )
    const [favourites] = buildSidebar(ACCOUNTS, mailboxes)
    const labels = favourites?.nodes.slice(4).map((node) => node.label)

    // "Inbox" alone would not say whose; two favourites called Receipts would not either.
    expect(labels).toEqual(['Projects', 'Receipts – Google', 'Inbox – Work', 'Receipts – Work'])
  })

  it('leave the folder in its account’s section too', () => {
    const sections = buildSidebar(ACCOUNTS, MAILBOXES)
    const work = sections.find((section) => section.title === 'Work')

    expect(work?.nodes.map((node) => node.label)).toContain('Projects')
  })

  // The store's order: Projects dragged to the top, All Inboxes to the bottom, VIPs in between
  // with no VIPs to show.
  const STORED: FavouriteRow[] = [
    { id: 7, builtin: null, mailboxId: 4 },
    { id: 3, builtin: 'flagged', mailboxId: null },
    { id: 2, builtin: 'vips', mailboxId: null },
    { id: 4, builtin: 'allDrafts', mailboxId: null },
    { id: 5, builtin: 'allSent', mailboxId: null },
    { id: 6, builtin: null, mailboxId: 3 },
    // A favourite whose mailbox the tree has not caught up with.
    { id: 9, builtin: null, mailboxId: 99 },
    { id: 1, builtin: 'allInboxes', mailboxId: null },
  ]

  it('follow the order the user left them in, built-in rows and all', () => {
    const [favourites] = buildSidebar(ACCOUNTS, MAILBOXES, [], [], [], STORED)

    expect(favourites?.nodes.map((node) => [node.label, node.favouriteId])).toEqual([
      ['Projects', 7],
      ['Flagged', 3],
      ['All Drafts', 4],
      ['All Sent', 5],
      ['Receipts', 6],
      ['All Inboxes', 1],
    ])
  })

  it('show VIPs where the store has them, once there are VIPs', () => {
    const vips = [{ address: 'ada@example.test', addedAt: 0 }]
    const [favourites] = buildSidebar(ACCOUNTS, MAILBOXES, [], [], vips, STORED)

    expect(favourites?.nodes.map((node) => node.id).slice(0, 3)).toEqual([
      'favourite-4',
      'flagged',
      'vips',
    ])
  })

  it('cannot be moved before the store’s order has been read', () => {
    const [favourites] = buildSidebar(ACCOUNTS, MAILBOXES)
    expect(favourites?.nodes.every((node) => node.favouriteId === undefined)).toBe(true)
  })

  it('are moved in the store’s terms, whatever the sidebar is not drawing', () => {
    const order = STORED.map((entry) => entry.id)

    // Dropped after Flagged is dropped before VIPs, which is not on screen — and so still between
    // Flagged and All Drafts, where the user saw it land.
    expect(beforeFor(order, 3, 'after')).toBe(2)
    expect(beforeFor(order, 3, 'before')).toBe(3)
    expect(beforeFor(order, 1, 'after')).toBeNull()

    expect(reordered(order, 1, 7)).toEqual([1, 7, 3, 2, 4, 5, 6, 9])
    expect(reordered(order, 7, null)).toEqual([3, 2, 4, 5, 6, 9, 1, 7])
    expect(reordered(order, 7, 7)).toEqual(order)
    expect(reordered(order, 7, 42)).toEqual(order)
  })
})

describe('an account’s mailboxes', () => {
  const TREE: MailboxRow[] = [
    row({ id: 10, accountId: 2, displayName: 'Inbox', role: 'inbox', editable: false }),
    row({ id: 11, accountId: 2, displayName: 'Archive', role: 'archive', editable: false }),
    row({ id: 12, accountId: 2, displayName: '2024', parentId: 11 }),
    row({ id: 13, accountId: 2, displayName: 'Clients', unreadCount: 2 }),
    row({ id: 14, accountId: 2, displayName: 'Acme', parentId: 13 }),
    row({ id: 15, accountId: 2, displayName: 'Beta', parentId: 13 }),
    row({ id: 16, accountId: 2, displayName: 'Invoices', parentId: 14 }),
    row({ id: 17, accountId: 2, displayName: 'All Mail', role: 'all', editable: false }),
    row({ id: 18, accountId: 2, displayName: 'Starred', parentId: 17 }),
    row({ id: 19, accountId: 2, displayName: 'Bills' }),
  ]

  function shape(nodes: SidebarNode[]): unknown[] {
    return nodes.map((node) =>
      node.children.length === 0
        ? [node.label, node.depth]
        : [node.label, node.depth, shape(node.children)],
    )
  }

  it('nest as their paths do, with the account’s own folders first', () => {
    const work = buildSidebar(ACCOUNTS, TREE).find((section) => section.title === 'Work')

    expect(shape(work?.nodes ?? [])).toEqual([
      ['Inbox', 0],
      ['Archive', 0, [['2024', 1]]],
      // A folder whose parent is not drawn (All Mail) sits at the top.
      ['Bills', 0],
      [
        'Clients',
        0,
        [
          ['Acme', 1, [['Invoices', 2]]],
          ['Beta', 1],
        ],
      ],
      ['Starred', 0],
    ])
  })

  it('open and close as a tree, and keep their own counts and menus', () => {
    const work = buildSidebar(ACCOUNTS, TREE).find((section) => section.title === 'Work')
    const nodes = work?.nodes ?? []

    expect(visibleRows(nodes, new Set()).map((node) => node.label)).toEqual([
      'Inbox',
      'Archive',
      '2024',
      'Bills',
      'Clients',
      'Acme',
      'Invoices',
      'Beta',
      'Starred',
    ])
    expect(visibleRows(nodes, new Set(['mailbox-13'])).map((node) => node.label)).toEqual([
      'Inbox',
      'Archive',
      '2024',
      'Bills',
      'Clients',
      'Starred',
    ])

    const acme = allNodes(buildSidebar(ACCOUNTS, TREE)).find((node) => node.id === 'mailbox-14')
    expect(acme).toMatchObject({ mailboxIds: [14], accountId: 2, depth: 1 })
    expect(acme && canOpenMailboxMenu(acme)).toBe(true)
  })

  it('are offered to New Mailbox as places, indented, where they can hold another', () => {
    const detail = (id: number, name: string): AccountDetail => ({
      id,
      displayName: name,
      email: `${name}@example.test`,
      provider: 'other',
      authKind: 'password',
      imap: { host: 'imap.example.test', port: 993, security: 'tls' },
      smtp: null,
      color: null,
      sortOrder: id,
      syncEnabled: true,
      hasCredential: true,
    })
    const tree = TREE.map((mailbox) =>
      mailbox.role === 'inbox' || mailbox.role === 'all'
        ? { ...mailbox, canContain: false }
        : mailbox,
    )

    const places = mailboxLocations([detail(2, 'Work')], tree)
    const indent = (depth: number) => '\u2007\u2007'.repeat(depth)

    expect(places.map((place) => [place.key, place.label, place.parentId])).toEqual([
      ['account-2', 'Work', null],
      ['mailbox-11', `${indent(1)}Archive`, 11],
      ['mailbox-12', `${indent(2)}2024`, 12],
      ['mailbox-19', `${indent(1)}Bills`, 19],
      ['mailbox-13', `${indent(1)}Clients`, 13],
      ['mailbox-14', `${indent(2)}Acme`, 14],
      ['mailbox-16', `${indent(3)}Invoices`, 16],
      ['mailbox-15', `${indent(2)}Beta`, 15],
      // Inside All Mail, which the sidebar does not draw, so at the top as the sidebar has it.
      ['mailbox-18', `${indent(1)}Starred`, 18],
    ])
    expect(places.every((place) => place.delimiter === '/')).toBe(true)
  })
})

describe('finding a row', () => {
  it('looks inside the unified rows, where the account inboxes are', () => {
    // Right-clicking an account under All Inboxes opened nothing, because the lookup read only
    // the top level of each section.
    const nodes = allNodes(buildSidebar(ACCOUNTS, MAILBOXES))
    const child = nodes.find((node) => node.id === 'all-inboxes-2')

    expect(child).toMatchObject({ label: 'Work', mailboxIds: [2], accountId: 2 })
    expect(child && canOpenMailboxMenu(child)).toBe(true)
  })
})

describe('Gmail', () => {
  function account(provider: string, host: string | null): AccountDetail {
    return {
      id: 1,
      displayName: 'A',
      email: 'a@example.test',
      provider,
      authKind: 'password',
      imap: host === null ? null : { host, port: 993, security: 'tls' },
      smtp: null,
      color: null,
      sortOrder: 0,
      syncEnabled: true,
      hasCredential: true,
    }
  }

  it('is recognised by provider, or by server for an address added as Other', () => {
    expect(isGmail(account('google', 'imap.gmail.com'))).toBe(true)
    expect(isGmail(account('gmail', null))).toBe(true)
    expect(isGmail(account('other', 'IMAP.GMAIL.COM'))).toBe(true)
    expect(isGmail(account('other', 'imap.googlemail.com'))).toBe(true)
    expect(isGmail(account('other', 'imap.fastmail.com'))).toBe(false)
    expect(isGmail(undefined)).toBe(false)
  })
})

describe('the selection', () => {
  it('can move to another row without losing the open message', () => {
    const store = useMailStore.getState()
    store.selectMailbox({ nodeId: 'favourite-3', label: 'Receipts', mailboxIds: [3] })
    useMailStore.getState().selectMessage(42)

    useMailStore.getState().retargetSelection('mailbox-3', 'Invoices')

    const after = useMailStore.getState()
    expect(after.selection).toEqual({ nodeId: 'mailbox-3', label: 'Invoices', mailboxIds: [3] })
    expect(after.selectedMessageIds).toEqual([42])
  })
})

describe('the browser store’s folder commands', () => {
  // A fresh store for each: the module keeps its state for the life of the page.
  let store: typeof BrowserStore

  beforeEach(async () => {
    vi.resetModules()
    store = await import('@/mock/browserStore')
  })

  function mailbox(accountId: number, name: string) {
    return store
      .mailboxesTree()
      .find((entry) => entry.accountId === accountId && entry.displayName === name)
  }

  it('makes, renames and deletes a folder, refusing what the core refuses', () => {
    const id = store.mailboxCreate(1, null, '  Invoices ')
    expect(mailbox(1, 'Invoices')).toMatchObject({ id, editable: true, role: null })

    expect(() => store.mailboxCreate(1, null, 'invoices')).toThrow(
      'There’s already a mailbox called “invoices”.',
    )
    expect(() => store.mailboxCreate(1, null, 'Inbox')).toThrow('“Inbox” is the name')
    expect(() => store.mailboxCreate(1, null, 'a/b')).toThrow('can’t contain “/”')

    store.mailboxRename(id, 'Bills 2026')
    expect(mailbox(1, 'Bills 2026')?.id).toBe(id)

    const inbox = mailbox(1, 'Inbox')
    expect(inbox).toBeDefined()
    expect(() => store.mailboxRename(inbox?.id ?? 0, 'Mail')).toThrow('can’t be renamed')
    expect(() => store.mailboxDelete(inbox?.id ?? 0)).toThrow('can’t be deleted')

    const deleted = store.mailboxDelete(id)
    expect(deleted).toEqual({ accountId: 1, mailboxIds: [id], messages: 0 })
    expect(mailbox(1, 'Bills 2026')).toBeUndefined()
  })

  it('deletes a folder’s mail with it', () => {
    const clients = mailbox(1, 'Clients')
    expect(clients?.totalCount).toBeGreaterThan(0)

    const deleted = store.mailboxDelete(clients?.id ?? 0)

    expect(deleted.messages).toBe(clients?.totalCount)
    const page = store.messagesPage({
      mailboxIds: [clients?.id ?? 0],
      cursor: null,
      limit: 10,
      unreadOnly: false,
    })
    expect(page.items).toHaveLength(0)
  })

  it('erases the Bin and Junk and nothing else', () => {
    const bin = mailbox(1, 'Bin')
    const inboxBefore = mailbox(1, 'Inbox')?.totalCount

    const erased = store.mailboxErase(1, 'trash')

    expect(erased).toEqual({ mailboxId: bin?.id, messages: bin?.totalCount })
    expect(mailbox(1, 'Bin')).toMatchObject({ totalCount: 0, unreadCount: 0 })
    expect(mailbox(1, 'Inbox')?.totalCount).toBe(inboxBefore)

    expect(store.mailboxErase(1, 'junk').mailboxId).toBe(mailbox(1, 'Junk')?.id)
    expect(mailbox(1, 'Junk')?.totalCount).toBe(0)
  })

  it('adds favourites at the end, and takes them away', () => {
    const clients = mailbox(1, 'Clients')?.id ?? 0
    const travel = mailbox(1, 'Travel')?.id ?? 0

    store.mailboxSetFavourite(clients, true)
    store.mailboxSetFavourite(travel, true)
    store.mailboxSetFavourite(clients, true)

    // After the five rows every sidebar starts with.
    expect(mailbox(1, 'Clients')?.favouriteOrder).toBe(6)
    expect(mailbox(1, 'Travel')?.favouriteOrder).toBe(7)

    store.mailboxSetFavourite(clients, false)
    expect(mailbox(1, 'Clients')?.favouriteOrder).toBeNull()
  })

  function order(): string[] {
    return store
      .favouritesList()
      .map((entry) => entry.builtin ?? `mailbox ${String(entry.mailboxId)}`)
  }

  it('moves favourites among the built-in rows, and drops one in where it is dragged', () => {
    const clients = mailbox(1, 'Clients')?.id ?? 0
    const travel = mailbox(1, 'Travel')?.id ?? 0
    store.mailboxSetFavourite(clients, true)

    const byKey = (key: string) =>
      store
        .favouritesList()
        .find((entry) => (entry.builtin ?? `mailbox ${String(entry.mailboxId)}`) === key)?.id ?? 0

    store.favouriteMove(byKey(`mailbox ${String(clients)}`), byKey('allInboxes'))
    store.favouriteMove(byKey('allSent'), null)
    store.mailboxSetFavourite(travel, true, byKey('flagged'))

    expect(order()).toEqual([
      `mailbox ${String(clients)}`,
      'allInboxes',
      'vips',
      `mailbox ${String(travel)}`,
      'flagged',
      'allDrafts',
      'allSent',
    ])
    expect(() => {
      store.favouriteMove(999, null)
    }).toThrow('no longer exists')
  })

  it('makes a folder inside another, moves it with its parent, and deletes it with it', () => {
    const clients = mailbox(1, 'Clients')?.id ?? 0
    const acme = store.mailboxCreate(1, clients, 'Acme')
    const invoices = store.mailboxCreate(1, acme, 'Invoices')

    expect(mailbox(1, 'Acme')).toMatchObject({ parentId: clients, canContain: true })
    expect(mailbox(1, 'Invoices')?.parentId).toBe(acme)
    expect(mailbox(1, 'Clients')?.descendants).toBe(2)

    // Taken inside the same parent, free elsewhere; never inside the Inbox.
    expect(() => store.mailboxCreate(1, clients, 'acme')).toThrow(
      'There’s already a mailbox called “acme” in “Clients”.',
    )
    store.mailboxCreate(1, null, 'Acme')
    const inbox = mailbox(1, 'Inbox')?.id ?? 0
    expect(mailbox(1, 'Inbox')?.canContain).toBe(false)
    expect(() => store.mailboxCreate(1, inbox, 'Sub')).toThrow('can’t be made inside “Inbox”')
    expect(() => store.mailboxCreate(2, clients, 'Sub')).toThrow('no longer exists')

    store.mailboxRename(clients, 'Customers')
    expect(mailbox(1, 'Invoices')?.parentId).toBe(acme)
    expect(store.mailboxesTree().find((entry) => entry.id === acme)?.parentId).toBe(clients)

    store.mailboxSetFavourite(invoices, true)
    const deleted = store.mailboxDelete(clients)
    expect(deleted.mailboxIds.sort((a, b) => a - b)).toEqual(
      [clients, acme, invoices].sort((a, b) => a - b),
    )
    expect(order()).not.toContain(`mailbox ${String(invoices)}`)
  })

  it('gives a mailbox a role, and takes it from the one that had it', () => {
    const contracts = mailbox(1, 'Contracts')?.id ?? 0
    const bin = mailbox(1, 'Bin')?.id ?? 0

    expect(store.mailboxUseAs(contracts, 'trash')).toBe(1)
    expect(store.mailboxesTree().find((entry) => entry.id === contracts)).toMatchObject({
      role: 'trash',
      roleChosen: true,
      editable: false,
    })
    expect(store.mailboxesTree().find((entry) => entry.id === bin)).toMatchObject({
      role: null,
      roleChosen: false,
      editable: true,
    })

    expect(() => store.mailboxUseAs(mailbox(1, 'Inbox')?.id ?? 0, 'archive')).toThrow('Inbox')
    // The browser's Gmail account, as the core refuses Gmail.
    expect(() => store.mailboxUseAs(mailbox(3, 'Newsletters')?.id ?? 0, 'archive')).toThrow(
      'Gmail decides',
    )
  })

  it('answers a rebuild with what the mailbox holds', () => {
    const inbox = mailbox(1, 'Inbox')
    expect(store.mailboxRebuild(inbox?.id ?? 0)).toEqual({
      accountId: 1,
      mailboxId: inbox?.id,
      messages: inbox?.totalCount,
    })
    expect(() => store.mailboxRebuild(9999)).toThrow('no longer exists')
  })

  it('marks a whole mailbox read', () => {
    const inbox = mailbox(1, 'Inbox')
    expect(inbox?.unreadCount).toBeGreaterThan(0)

    expect(store.mailboxMarkRead(inbox?.id ?? 0)).toBe(inbox?.unreadCount)
    expect(mailbox(1, 'Inbox')?.unreadCount).toBe(0)
    expect(store.mailboxMarkRead(inbox?.id ?? 0)).toBe(0)
  })

  it('hands out copies, so a change is a new answer', () => {
    const before = store.mailboxesTree()
    store.mailboxCreate(1, null, 'Fresh')
    const after = store.mailboxesTree()

    expect(after).not.toBe(before)
    expect(after.length).toBe(before.length + 1)
  })
})
