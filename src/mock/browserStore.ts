import type { AccountRow } from '@/lib/generated/AccountRow'
import type { Cursor } from '@/lib/generated/Cursor'
import type { FlagPatch } from '@/lib/generated/FlagPatch'
import type { ListQuery } from '@/lib/generated/ListQuery'
import type { MailboxRow } from '@/lib/generated/MailboxRow'
import type { MessageFull } from '@/lib/generated/MessageFull'
import type { MessageRow } from '@/lib/generated/MessageRow'
import type { Page } from '@/lib/generated/Page'
import type { SearchQuery } from '@/lib/generated/SearchQuery'

import {
  ATTACHMENT_NAMES,
  BODY_PARAGRAPHS,
  CONVERSATION_SUBJECTS,
  PEOPLE,
  PREVIEW_SENTENCES,
  SERVICES,
  TRANSACTIONAL_SUBJECTS,
} from './corpus'
import { createRng } from './random'

/**
 * The mail store the *browser* sees.
 *
 * This is not a mock of the app. It is what the app genuinely is when served by Vite rather
 * than hosted in a WebView: there is no Rust core, so there is no SQLite, so the commands
 * have to be answered by something. `src/lib/ipc.ts` has taken this shape since Phase 0 for
 * appearance, and the mail commands follow it.
 *
 * It matters that the semantics match the Rust implementation exactly — keyset pagination
 * with a `(dateReceived, id)` cursor, incremental counts, the same sort order — because the
 * UI has one code path and the e2e suite drives it here. A browser store that paged
 * differently would let the tests pass over a bug that only appears in the real app.
 *
 * Deliberately smaller than the real seed: a few thousand messages, not a hundred thousand.
 * The scale claims are measured against the real store by the `seed` binary.
 */

const MESSAGE_COUNT = 4_000
const SEED = 20260825

/** Frozen, for the same reason the Phase 2 fixtures were: "Today" must not move overnight. */
export const BROWSER_NOW = new Date('2026-08-26T19:30:00')

interface StoredMessage extends MessageFull {
  /** Denormalised for search, mirroring the FTS columns on the Rust side. */
  searchText: string
  hasAttachment: boolean
}

interface Store {
  accounts: AccountRow[]
  mailboxes: MailboxRow[]
  messages: Map<number, StoredMessage>
  /** Message ids per mailbox, newest first — the same order `ix_msg_list` provides. */
  byMailbox: Map<number, number[]>
}

const ACCOUNTS: { name: string; email: string; provider: string; folders: string[] }[] = [
  {
    name: 'Northgate',
    email: 'vishal@northgate.example',
    provider: 'imap',
    folders: ['Clients', 'Contracts', 'Receipts', 'Travel'],
  },
  {
    name: 'iCloud',
    email: 'vishal@icloud.example',
    provider: 'icloud',
    folders: ['Family', 'Bills', 'Shopping'],
  },
  {
    name: 'Gmail',
    email: 'vishal.singh@gmail.example',
    provider: 'gmail',
    folders: ['Newsletters'],
  },
]

const ROLES: { role: string; name: string }[] = [
  { role: 'inbox', name: 'Inbox' },
  { role: 'drafts', name: 'Drafts' },
  { role: 'sent', name: 'Sent' },
  { role: 'junk', name: 'Junk' },
  { role: 'trash', name: 'Bin' },
  { role: 'archive', name: 'Archive' },
]

const DAY_SECONDS = 24 * 60 * 60

function build(): Store {
  const rng = createRng(SEED)
  const nowSeconds = Math.floor(BROWSER_NOW.getTime() / 1000)

  const accounts: AccountRow[] = []
  const mailboxes: MailboxRow[] = []
  const inboxIds: number[] = []

  let nextMailboxId = 1

  ACCOUNTS.forEach((spec, index) => {
    const accountId = index + 1
    accounts.push({
      id: accountId,
      displayName: spec.name,
      email: spec.email,
      provider: spec.provider,
    })

    for (const { role, name } of ROLES) {
      const id = nextMailboxId++
      mailboxes.push({
        id,
        accountId,
        displayName: name,
        parentId: null,
        role,
        unreadCount: 0,
        totalCount: 0,
      })
      if (role === 'inbox') inboxIds.push(id)
    }

    for (const folder of spec.folders) {
      mailboxes.push({
        id: nextMailboxId++,
        accountId,
        displayName: folder,
        parentId: null,
        role: null,
        unreadCount: 0,
        totalCount: 0,
      })
    }
  })

  const messages = new Map<number, StoredMessage>()
  const owner = { name: 'Vishal Singh', address: 'vishal@northgate.example' }

  for (let i = 1; i <= MESSAGE_COUNT; i += 1) {
    // Most mail lands in an inbox; the rest spreads over every folder.
    const mailboxId = rng.chance(0.7) ? rng.pick(inboxIds) : rng.pick(mailboxes).id
    const mailbox = mailboxes.find((entry) => entry.id === mailboxId)
    if (!mailbox) continue

    const conversation = rng.chance(0.34)
    const person = conversation ? rng.pick(PEOPLE) : rng.pick(SERVICES)
    const subject = conversation
      ? rng.pick(CONVERSATION_SUBJECTS)
      : rng.pick(TRANSACTIONAL_SUBJECTS).replace(/\{\{\w+\}\}/g, String(rng.int(1000, 9999)))

    // Squared roll bunches dates toward the present, so the top of the list has Today and
    // Yesterday to group rather than a year of month headers.
    const roll = rng.next()
    const age = 730 * roll * roll
    const date = nowSeconds - Math.floor(age * DAY_SECONDS)

    const seen = rng.chance(0.82)
    const flagged = rng.chance(0.05)
    const attachmentCount = rng.chance(0.14) ? rng.int(1, 2) : 0
    const body = rng.shuffle(BODY_PARAGRAPHS).slice(0, rng.int(2, 4)).join('\n\n')
    const preview = rng.shuffle(PREVIEW_SENTENCES).slice(0, 2).join(' ')

    messages.set(i, {
      id: i,
      // Threading proper arrives with the sync engine; here a thread is the message itself,
      // which is what the Rust store also returns until Phase 5 populates `thread`.
      threadId: i,
      mailboxId,
      accountId: mailbox.accountId,
      subject,
      fromName: person.name,
      fromAddr: person.address,
      toJson: JSON.stringify([owner]),
      ccJson: null,
      dateSent: date,
      dateReceived: date,
      size: rng.int(2400, 60000),
      preview,
      bodyText: `Hi Vishal,\n\n${body}\n\nBest,`,
      seen,
      answered: rng.chance(0.18),
      flagged,
      flagColor: flagged ? 'orange' : null,
      // The browser gallery has no classifier behind it, so nothing is junk and nothing has a
      // score. Inventing one would make the banner look implemented when it is not wired here.
      isJunk: false,
      junkByUser: false,
      junkScore: null,
      attachments: Array.from({ length: attachmentCount }, (_, index) => {
        const file = rng.pick(ATTACHMENT_NAMES)
        return {
          id: i * 10 + index,
          filename: file.filename,
          mime: file.mime,
          size: rng.int(12_000, 8_400_000),
          isInline: false,
        }
      }),
      hasAttachment: attachmentCount > 0,
      searchText: `${subject} ${preview} ${person.name} ${person.address}`.toLowerCase(),
    })
  }

  const byMailbox = new Map<number, number[]>()
  for (const message of messages.values()) {
    const list = byMailbox.get(message.mailboxId) ?? []
    list.push(message.id)
    byMailbox.set(message.mailboxId, list)
  }

  for (const [mailboxId, ids] of byMailbox) {
    ids.sort((a, b) => {
      const left = messages.get(a)
      const right = messages.get(b)
      if (!left || !right) return 0
      // Newest first, ties broken by id descending — exactly ix_msg_list.
      return right.dateReceived - left.dateReceived || right.id - left.id
    })
    byMailbox.set(mailboxId, ids)
  }

  const store: Store = { accounts, mailboxes, messages, byMailbox }
  recount(
    store,
    mailboxes.map((mailbox) => mailbox.id),
  )
  return store
}

function recount(store: Store, mailboxIds: number[]): void {
  for (const mailboxId of mailboxIds) {
    const ids = store.byMailbox.get(mailboxId) ?? []
    const mailbox = store.mailboxes.find((entry) => entry.id === mailboxId)
    if (!mailbox) continue

    mailbox.totalCount = ids.length
    mailbox.unreadCount = ids.filter((id) => store.messages.get(id)?.seen === false).length
  }
}

function toRow(message: StoredMessage): MessageRow {
  return {
    id: message.id,
    threadId: message.threadId,
    mailboxId: message.mailboxId,
    accountId: message.accountId,
    subject: message.subject,
    fromName: message.fromName,
    fromAddr: message.fromAddr,
    dateReceived: message.dateReceived,
    preview: message.preview,
    size: message.size,
    seen: message.seen,
    answered: message.answered,
    flagged: message.flagged,
    flagColor: message.flagColor,
    hasAttachment: message.hasAttachment,
  }
}

let store: Store | null = null

function current(): Store {
  store ??= build()
  return store
}

export function accountsList(): AccountRow[] {
  return current().accounts
}

export function mailboxesTree(accountId?: number): MailboxRow[] {
  const all = current().mailboxes
  return accountId === undefined ? all : all.filter((mailbox) => mailbox.accountId === accountId)
}

/**
 * Keyset pagination, matching `db::query::messages_page`.
 *
 * The comparison is on the pair `(dateReceived, id)`, not on the date alone. Timestamps
 * collide constantly, and a cursor that ignores the id repeats or skips the whole colliding
 * run — the same bug in either language.
 */
export function messagesPage(query: ListQuery): Page<MessageRow> {
  const data = current()

  const merged = query.mailboxIds
    .flatMap((mailboxId) => data.byMailbox.get(mailboxId) ?? [])
    .map((id) => data.messages.get(id))
    .filter((message): message is StoredMessage => message !== undefined)
    .filter((message) => !query.unreadOnly || !message.seen)
    .sort((a, b) => b.dateReceived - a.dateReceived || b.id - a.id)

  const cursor: Cursor | null = query.cursor
  const after = cursor
    ? merged.filter((message) => {
        return (
          message.dateReceived < cursor.dateReceived ||
          (message.dateReceived === cursor.dateReceived && message.id < cursor.id)
        )
      })
    : merged

  // One more than asked for, so the caller learns there is a next page without a count.
  const window = after.slice(0, query.limit + 1)
  const hasMore = window.length > query.limit
  const items = window.slice(0, query.limit).map(toRow)
  const last = items[items.length - 1]

  return {
    items,
    nextCursor: hasMore && last ? { dateReceived: last.dateReceived, id: last.id } : null,
  }
}

export function messageGet(id: number): MessageFull | null {
  return current().messages.get(id) ?? null
}

export function threadGet(threadId: number): MessageFull[] {
  return [...current().messages.values()]
    .filter((message) => message.threadId === threadId)
    .sort((a, b) => a.dateSent - b.dateSent || a.id - b.id)
}

export function search(query: SearchQuery): MessageRow[] {
  const text = query.text.trim().toLowerCase()
  if (text === '') return []

  const terms = text.split(/\s+/)
  const data = current()

  return [...data.messages.values()]
    .filter(
      (message) => query.mailboxIds.length === 0 || query.mailboxIds.includes(message.mailboxId),
    )
    .filter((message) => terms.every((term) => message.searchText.includes(term)))
    .sort((a, b) => b.dateReceived - a.dateReceived)
    .slice(0, query.limit)
    .map(toRow)
}

/** Which mailboxes the given messages are in — the browser twin of `write::mailboxes_of`. */
function mailboxesOf(ids: number[]): number[] {
  const data = current()
  const seen = new Set<number>()
  for (const id of ids) {
    const message = data.messages.get(id)
    if (message) seen.add(message.mailboxId)
  }
  return [...seen]
}

export function setFlags(
  ids: number[],
  patch: FlagPatch,
): { changed: number; mailboxIds: number[] } {
  const data = current()
  const mailboxIds = mailboxesOf(ids)
  let changed = 0

  for (const id of ids) {
    const message = data.messages.get(id)
    if (!message) continue

    if (patch.seen !== null) message.seen = patch.seen
    if (patch.flagged !== null) {
      message.flagged = patch.flagged
      message.flagColor = patch.flagged ? (message.flagColor ?? 'orange') : null
    }
    changed += 1
  }

  recount(data, mailboxIds)
  return { changed, mailboxIds }
}

export function moveTo(
  ids: number[],
  mailboxId: number,
): { changed: number; mailboxIds: number[] } {
  const data = current()
  const affected = new Set(mailboxesOf(ids))
  affected.add(mailboxId)
  let changed = 0

  for (const id of ids) {
    const message = data.messages.get(id)
    if (!message || message.mailboxId === mailboxId) continue

    const from = data.byMailbox.get(message.mailboxId) ?? []
    data.byMailbox.set(
      message.mailboxId,
      from.filter((entry) => entry !== id),
    )

    message.mailboxId = mailboxId
    const into = [...(data.byMailbox.get(mailboxId) ?? []), id].sort((a, b) => {
      const left = data.messages.get(a)
      const right = data.messages.get(b)
      if (!left || !right) return 0
      return right.dateReceived - left.dateReceived || right.id - left.id
    })
    data.byMailbox.set(mailboxId, into)
    changed += 1
  }

  recount(data, [...affected])
  return { changed, mailboxIds: [...affected] }
}

export function remove(
  ids: number[],
  permanent: boolean,
): { changed: number; mailboxIds: number[] } {
  const data = current()

  if (!permanent) {
    // Resolve Trash the way the Rust command does: per account, and refuse rather than
    // improvise when a selection spans more than one.
    const accounts = new Set(ids.map((id) => data.messages.get(id)?.accountId))
    if (accounts.size !== 1) return { changed: 0, mailboxIds: [] }

    const [accountId] = [...accounts]
    const trash = data.mailboxes.find(
      (mailbox) => mailbox.accountId === accountId && mailbox.role === 'trash',
    )
    return trash ? moveTo(ids, trash.id) : { changed: 0, mailboxIds: [] }
  }

  const mailboxIds = mailboxesOf(ids)
  let changed = 0

  for (const id of ids) {
    const message = data.messages.get(id)
    if (!message) continue

    const from = data.byMailbox.get(message.mailboxId) ?? []
    data.byMailbox.set(
      message.mailboxId,
      from.filter((entry) => entry !== id),
    )
    data.messages.delete(id)
    changed += 1
  }

  recount(data, mailboxIds)
  return { changed, mailboxIds }
}

/** Counts as the sidebar reads them, after a mutation. */
export function mailboxCounts(
  mailboxIds: number[],
): { mailboxId: number; unread: number; total: number }[] {
  const data = current()
  return mailboxIds
    .map((mailboxId) => data.mailboxes.find((mailbox) => mailbox.id === mailboxId))
    .filter((mailbox): mailbox is MailboxRow => mailbox !== undefined)
    .map((mailbox) => ({
      mailboxId: mailbox.id,
      unread: mailbox.unreadCount,
      total: mailbox.totalCount,
    }))
}

/* --------------------------------------------------------------------- accounts */

import type { AccountDetail } from '@/lib/generated/AccountDetail'
import type { AddedAccount } from '@/lib/generated/AddedAccount'
import type { DiagnosticReport } from '@/lib/generated/DiagnosticReport'
import type { DiscoveryResult } from '@/lib/generated/DiscoveryResult'
import type { OAuthClientStatus } from '@/lib/generated/OAuthClientStatus'
import type { ProviderInfo } from '@/lib/generated/ProviderInfo'
import type { Security } from '@/lib/generated/Security'

/**
 * The account side of the browser store.
 *
 * Two of these commands cannot be answered honestly by a browser, and they say so rather
 * than returning a shape that looks like success: a page served by Vite has no Windows
 * Credential Manager to put a password in, and no way to open a TLS socket to port 993.
 * Standing rule 18 — a command that returns a plausible shape and does nothing is worse
 * than one that refuses.
 *
 * Everything else here is real. The provider table is data, the known-domain lookups are
 * data, and editing, reordering and removing operate on the same in-memory store the rest
 * of this file serves. That is what the Playwright suite drives.
 */

interface AccountOverlay {
  displayName: string
  color: string | null
  sortOrder: number
  syncEnabled: boolean
}

/**
 * Held beside `Store` rather than inside it, because these are settings the user changes
 * rather than mail the generator produced — and `build()` should stay a pure function of
 * the seed.
 */
const overlays = new Map<number, AccountOverlay>()
const oauthClients = new Map<string, string>()

const KNOWN_SERVERS: Record<
  string,
  { imapHost: string; imapPort: number; smtpHost: string; smtpPort: number; smtpTls: Security }
> = {
  google: {
    imapHost: 'imap.gmail.com',
    imapPort: 993,
    smtpHost: 'smtp.gmail.com',
    smtpPort: 587,
    smtpTls: 'startTls',
  },
  gmail: {
    imapHost: 'imap.gmail.com',
    imapPort: 993,
    smtpHost: 'smtp.gmail.com',
    smtpPort: 587,
    smtpTls: 'startTls',
  },
  microsoft: {
    imapHost: 'outlook.office365.com',
    imapPort: 993,
    smtpHost: 'smtp.office365.com',
    smtpPort: 587,
    smtpTls: 'startTls',
  },
  icloud: {
    imapHost: 'imap.mail.me.com',
    imapPort: 993,
    smtpHost: 'smtp.mail.me.com',
    smtpPort: 587,
    smtpTls: 'startTls',
  },
  yahoo: {
    imapHost: 'imap.mail.yahoo.com',
    imapPort: 993,
    smtpHost: 'smtp.mail.yahoo.com',
    smtpPort: 465,
    smtpTls: 'tls',
  },
  imap: {
    imapHost: 'imap.northgate.example',
    imapPort: 993,
    smtpHost: 'smtp.northgate.example',
    smtpPort: 587,
    smtpTls: 'startTls',
  },
}

/// Used for a provider the generator invented that this table does not list.
const FALLBACK_SERVERS = {
  imapHost: 'imap.example',
  imapPort: 993,
  smtpHost: 'smtp.example',
  smtpPort: 587,
  smtpTls: 'startTls' as Security,
}

const PROVIDER_SETUP: Record<string, string> = {
  icloud: 'https://appleid.apple.com/account/manage',
  yahoo: 'https://login.yahoo.com/account/security',
}

/** Mirrors `accounts::provider::describe`, which is a table rather than behaviour. */
export function providersList(): ProviderInfo[] {
  return [
    {
      id: 'google',
      displayName: 'Google',
      authKind: 'oAuth2',
      needsManualSetup: false,
      setupNote: 'Sign in happens in your browser. Halcyon never sees your Google password.',
      setupUrl: null,
      needsOauthClient: !oauthClients.has('google'),
      requiresClientSecret: true,
    },
    {
      id: 'microsoft',
      displayName: 'Microsoft',
      authKind: 'oAuth2',
      needsManualSetup: false,
      setupNote:
        'Sign in happens in your browser. If your work or school account fails, your administrator may have blocked IMAP for third-party apps.',
      setupUrl: null,
      needsOauthClient: !oauthClients.has('microsoft'),
      requiresClientSecret: false,
    },
    {
      id: 'icloud',
      displayName: 'iCloud',
      authKind: 'password',
      needsManualSetup: false,
      setupNote:
        'iCloud needs an app-specific password, not your Apple ID password. Sign in at appleid.apple.com, go to Sign-In and Security, choose App-Specific Passwords, and create one for Halcyon. Your Apple ID must have two-factor authentication turned on.',
      setupUrl: PROVIDER_SETUP.icloud ?? null,
      needsOauthClient: false,
      requiresClientSecret: false,
    },
    {
      id: 'yahoo',
      displayName: 'Yahoo',
      authKind: 'password',
      needsManualSetup: false,
      setupNote:
        'Yahoo needs an app password. Generate one under Account Security in your Yahoo account settings.',
      setupUrl: PROVIDER_SETUP.yahoo ?? null,
      needsOauthClient: false,
      requiresClientSecret: false,
    },
    {
      id: 'other',
      displayName: 'Other Mail Account',
      authKind: 'password',
      needsManualSetup: true,
      setupNote: null,
      setupUrl: null,
      needsOauthClient: false,
      requiresClientSecret: false,
    },
  ]
}

const KNOWN_DOMAINS: Record<string, string> = {
  'gmail.com': 'google',
  'googlemail.com': 'google',
  'outlook.com': 'microsoft',
  'hotmail.com': 'microsoft',
  'live.com': 'microsoft',
  'msn.com': 'microsoft',
  'icloud.com': 'icloud',
  'me.com': 'icloud',
  'mac.com': 'icloud',
  'yahoo.com': 'yahoo',
  'ymail.com': 'yahoo',
  'rocketmail.com': 'yahoo',
}

/**
 * Only the recognised domains. The other three sources the core tries — Mozilla's ISPDB,
 * a domain's own autoconfig, SRV records — are cross-origin requests a browser is not
 * allowed to make, and a TCP probe is not something a page can do at all.
 */
export function accountDiscover(email: string): DiscoveryResult | null {
  const domain = email.trim().toLowerCase().split('@').pop() ?? ''
  const provider = KNOWN_DOMAINS[domain]
  if (provider === undefined) return null

  const servers = KNOWN_SERVERS[provider]
  if (servers === undefined) return null

  return {
    imap: { host: servers.imapHost, port: servers.imapPort, security: 'tls' },
    smtp: { host: servers.smtpHost, port: servers.smtpPort, security: servers.smtpTls },
    source: 'known',
    explanation: "Halcyon knows this provider's servers.",
    needsConfirmation: false,
    suggestedProvider: provider,
  }
}

const NO_NETWORK =
  'Halcyon is running in a browser, which cannot open a mail connection. Run the desktop app to test and add accounts.'

export function accountTest(): DiagnosticReport {
  const skipped = (name: string) => ({
    name,
    status: 'skipped' as const,
    detail: NO_NETWORK,
    remedy: null,
    serverSaid: null,
    elapsedMs: 0,
  })

  return {
    ok: false,
    imap: ['Connect', 'Secure the connection', 'Sign in', 'Open Inbox'].map(skipped),
    smtp: ['Connect', 'Secure the connection', 'Sign in'].map(skipped),
    summary: NO_NETWORK,
  }
}

export function accountAdd(): AddedAccount {
  // Refused rather than faked: adding an account means writing a secret to the Windows
  // Credential Manager, which does not exist here.
  throw new Error(NO_NETWORK)
}

function overlayFor(id: number, fallbackName: string, index: number): AccountOverlay {
  const existing = overlays.get(id)
  if (existing) return existing

  const created: AccountOverlay = {
    displayName: fallbackName,
    color: null,
    sortOrder: index,
    syncEnabled: true,
  }
  overlays.set(id, created)
  return created
}

export function accountsDetail(): AccountDetail[] {
  // `?first-run=1` empties the account list, which is the one state the seeded browser store
  // cannot otherwise reach: it starts with three accounts, and first run is defined by having
  // none. Only the browser path has this — the packaged app has a real database, where the
  // state arrives by being a fresh install.
  if (new URLSearchParams(window.location.search).has('first-run')) return []

  const store = current()

  return store.accounts
    .map((account, index): AccountDetail => {
      const servers = KNOWN_SERVERS[account.provider] ?? FALLBACK_SERVERS
      const overlay = overlayFor(account.id, account.displayName, index)

      return {
        id: account.id,
        displayName: overlay.displayName,
        email: account.email,
        provider: account.provider,
        authKind:
          account.provider === 'gmail' || account.provider === 'google' ? 'oAuth2' : 'password',
        imap: { host: servers.imapHost, port: servers.imapPort, security: 'tls' },
        smtp: {
          host: servers.smtpHost,
          port: servers.smtpPort,
          security: servers.smtpTls,
        },
        color: overlay.color,
        sortOrder: overlay.sortOrder,
        syncEnabled: overlay.syncEnabled,
        hasCredential: true,
      }
    })
    .sort((left, right) => left.sortOrder - right.sortOrder)
}

export function accountUpdate(
  id: number,
  patch: {
    displayName?: string | undefined
    color?: string | null | undefined
    syncEnabled?: boolean | undefined
  },
): void {
  const store = current()
  const index = store.accounts.findIndex((account) => account.id === id)
  if (index < 0) return

  const account = store.accounts[index]
  if (account === undefined) return

  const overlay = overlayFor(id, account.displayName, index)

  if (patch.displayName !== undefined) overlay.displayName = patch.displayName
  // `undefined` leaves the colour alone; `null` clears it. Collapsing the two would make
  // a colour impossible to remove once set.
  if (patch.color !== undefined) overlay.color = patch.color
  if (patch.syncEnabled !== undefined) overlay.syncEnabled = patch.syncEnabled
}

export function accountsReorder(ids: number[]): void {
  const store = current()

  ids.forEach((id, position) => {
    const index = store.accounts.findIndex((account) => account.id === id)
    const account = store.accounts[index]
    if (account === undefined) return
    overlayFor(id, account.displayName, index).sortOrder = position
  })

  // Ids the caller did not mention keep their relative order after the ones it did,
  // matching `store::reorder` — a stale settings pane must not drop an account.
  store.accounts.forEach((account, index) => {
    if (ids.includes(account.id)) return
    overlayFor(account.id, account.displayName, index).sortOrder = ids.length + account.id
  })
}

export function accountRemove(id: number): void {
  const store = current()

  // The same order the core uses: mail first, then mailboxes, then the account. The other
  // way round would leave orphaned rows the list would still try to render.
  const mailboxIds = store.mailboxes.filter((mailbox) => mailbox.accountId === id).map((m) => m.id)

  for (const mailboxId of mailboxIds) {
    for (const messageId of store.byMailbox.get(mailboxId) ?? []) {
      store.messages.delete(messageId)
    }
    store.byMailbox.delete(mailboxId)
  }

  store.mailboxes = store.mailboxes.filter((mailbox) => mailbox.accountId !== id)
  store.accounts = store.accounts.filter((account) => account.id !== id)
  overlays.delete(id)
}

export function oauthClientGet(provider: string): OAuthClientStatus {
  const clientId = oauthClients.get(provider)

  return {
    provider,
    configured: clientId !== undefined,
    clientId: clientId ?? null,
    hasSecret: false,
  }
}

export function oauthClientSet(provider: string, clientId: string): void {
  if (clientId.trim() === '') oauthClients.delete(provider)
  else oauthClients.set(provider, clientId.trim())
}

export function providerSetupUrl(provider: string): string | null {
  return PROVIDER_SETUP[provider] ?? null
}
