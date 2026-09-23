import type { AccountRow } from '@/lib/generated/AccountRow'
import type { BuiltinFavourite } from '@/lib/generated/BuiltinFavourite'
import type { Cursor } from '@/lib/generated/Cursor'
import type { FavouriteRow } from '@/lib/generated/FavouriteRow'
import type { FlagPatch } from '@/lib/generated/FlagPatch'
import type { ListQuery } from '@/lib/generated/ListQuery'
import type { MailboxRow } from '@/lib/generated/MailboxRow'
import type { MailboxUse } from '@/lib/generated/MailboxUse'
import type { MessageFull } from '@/lib/generated/MessageFull'
import type { MessageRow } from '@/lib/generated/MessageRow'
import type { Page } from '@/lib/generated/Page'
import type { SearchQuery } from '@/lib/generated/SearchQuery'

import { mailboxNameProblem } from '@/lib/mailboxName'

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
  /**
   * The plain-text body.
   *
   * On the store rather than on `MessageFull`, which is where it used to live: the real
   * `message` table still has the column — the search index, the rules engine and the junk
   * classifier all read it in SQL — but `message_full` no longer returns it, because it was
   * being selected, serialised, sent over IPC and cached on every message selected, and read
   * by nothing at all.
   *
   * Kept generated rather than dropped because this fixture is seeded: removing the draw would
   * shift every later one and change every message in it, moving four visual baselines to tidy
   * away a field nobody sees.
   */
  bodyText: string
}

interface Store {
  accounts: AccountRow[]
  mailboxes: MailboxRow[]
  messages: Map<number, StoredMessage>
  /** Message ids per mailbox, newest first — the same order `ix_msg_list` provides. */
  byMailbox: Map<number, number[]>
  /**
   * Each mailbox's path, "/"-separated, as the core keeps `remote_path`. Beside the rows rather
   * than on them because `MailboxRow` does not carry it: the window never needs a server's
   * spelling of a name.
   */
  paths: Map<number, string>
  /** Favourites in order, as the `favourite` table holds them. */
  favourites: FavouriteRow[]
  /** The mailboxes whose role the user chose, as `mailbox_role` holds them. */
  chosen: Set<number>
}

/** The rows every sidebar's Favourites starts with, in the migration's order. */
const BUILTIN_FAVOURITES: BuiltinFavourite[] = [
  'allInboxes',
  'vips',
  'flagged',
  'allDrafts',
  'allSent',
]

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

/** The seven flag colours, in the core's order — `engine::FLAG_COLOURS`. */
const FLAG_COLOURS = ['red', 'orange', 'yellow', 'green', 'blue', 'purple', 'gray'] as const

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
      // Null exactly as a freshly added account is in the real database. The colour a user
      // picks lives in the overlay, and `accountsList` reads it back from there.
      color: null,
    })

    for (const { role, name } of ROLES) {
      const id = nextMailboxId++
      mailboxes.push(folderRow(id, accountId, name, role))
      if (role === 'inbox') inboxIds.push(id)
    }

    for (const folder of spec.folders) {
      mailboxes.push(folderRow(nextMailboxId++, accountId, folder, null))
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
      // Spread across the seven rather than all orange. A mock where every flag is the same
      // colour cannot show whether the row draws the colour or merely draws a flag, and that
      // is exactly the difference the sidebar was getting wrong.
      flagColor: flagged ? (FLAG_COLOURS[i % FLAG_COLOURS.length] ?? 'orange') : null,
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

  const paths = new Map(
    mailboxes.map((mailbox) => [
      mailbox.id,
      mailbox.role === 'inbox' ? 'INBOX' : mailbox.displayName,
    ]),
  )
  const favourites = BUILTIN_FAVOURITES.map((builtin, index) => ({
    id: index + 1,
    builtin,
    mailboxId: null,
  }))

  const store: Store = {
    accounts,
    mailboxes,
    messages,
    byMailbox,
    paths,
    favourites,
    chosen: new Set(),
  }
  recount(
    store,
    mailboxes.map((mailbox) => mailbox.id),
  )
  return store
}

/**
 * A mailbox row as `mailboxes_tree` returns one, before `mailboxesTree` works out where it sits.
 * The browser's folders separate with "/".
 */
function folderRow(
  id: number,
  accountId: number,
  displayName: string,
  role: string | null,
): MailboxRow {
  return {
    id,
    accountId,
    displayName,
    parentId: null,
    role,
    unreadCount: 0,
    totalCount: 0,
    favouriteOrder: null,
    roleChosen: false,
    delimiter: '/',
    // `sync::folders::editable`: the user's own folders, not the ones the account files into.
    editable: role === null,
    descendants: 0,
    canContain: role !== 'inbox',
  }
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
    // The browser build has no threads table, so nothing here is ever muted. Stated rather
    // than omitted: the field is part of a row, and a mock that lacks it drifts from the type.
    muted: false,
  }
}

let store: Store | null = null

function current(): Store {
  store ??= build()
  return store
}

/**
 * The sidebar's view of the accounts, with the user's edits applied.
 *
 * This returned the seed rows untouched, which made the browser mock disagree with the core
 * on three fields at once: `accounts_list` in Rust reads `display_name`, `color` and
 * `ORDER BY sort_order` straight from the table the settings pane writes to. So renaming,
 * recolouring or reordering an account changed the settings pane and left the sidebar on the
 * original seed — a divergence that would have made a real bug look like a mock artefact,
 * and a mock artefact look like a real bug.
 */
export function accountsList(): AccountRow[] {
  const store = current()

  return store.accounts
    .map((account, index) => {
      const overlay = overlayFor(account.id, account.displayName, index)
      return {
        row: { ...account, displayName: overlay.displayName, color: overlay.color },
        sortOrder: overlay.sortOrder,
      }
    })
    .sort((left, right) => left.sortOrder - right.sortOrder)
    .map((entry) => entry.row)
}

/**
 * Copies, not the stored rows. The store edits its rows and its array in place, and TanStack
 * Query decides whether anything changed by comparing what it is given with what it had: handed
 * the very same array back after a folder was added to it, structural sharing keeps the old
 * reference and a component watching `data` has no reason to render again. The core's answers
 * are fresh objects every time; these now are too.
 */
export function mailboxesTree(accountId?: number): MailboxRow[] {
  const data = current()

  const all = data.mailboxes.map((mailbox): MailboxRow => {
    const path = pathOf(mailbox)
    const others = data.mailboxes.filter(
      (other) => other.accountId === mailbox.accountId && other.id !== mailbox.id,
    )

    // `db::query::mailboxes_tree`: the longest path that contains this one, never the Inbox.
    const parent = others
      .filter((other) => !isInboxPath(pathOf(other)) && path.startsWith(`${pathOf(other)}/`))
      .sort((a, b) => pathOf(b).length - pathOf(a).length)[0]
    const position = data.favourites.findIndex((entry) => entry.mailboxId === mailbox.id)

    return {
      ...mailbox,
      parentId: parent?.id ?? null,
      descendants: others.filter((other) => pathOf(other).startsWith(`${path}/`)).length,
      canContain: !isInboxPath(path),
      favouriteOrder: position < 0 ? null : position + 1,
      roleChosen: data.chosen.has(mailbox.id),
    }
  })

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

/**
 * A thread's messages.
 *
 * Returned as **copies**, which is not tidiness. Everything else in this file hands back a
 * freshly built row, and this used to hand back the stored objects themselves — so a mutation
 * that edited a message in place (setting a flag colour does) left the query cache already
 * holding the new value in the same object it had before. React had no new reference to
 * notice, so the refetch changed nothing on screen and the flag menu appeared to tick nothing
 * after setting a colour. Over a real IPC boundary the answer is serialised and is always a
 * fresh object; the mock has to be too, or it is a mock of something the app never does.
 */
export function threadGet(messageId: number): MessageFull[] {
  // Resolve the message to its thread first, exactly as the core does. Filtering on the
  // message id directly matched only the conversation whose thread id happened to equal it.
  const data = current()
  const threadId = data.messages.get(messageId)?.threadId ?? null
  if (threadId === null) {
    const single = data.messages.get(messageId)
    return single === undefined ? [] : [{ ...single }]
  }

  return [...data.messages.values()]
    .filter((message) => message.threadId === threadId)
    .sort((a, b) => a.dateSent - b.dateSent || a.id - b.id)
    .map((message) => ({ ...message }))
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

/**
 * Sets or clears the colour flag on a selection, the way `flag_set` does.
 *
 * A colour implies the flag: the core's own note is that colour is a local concept riding on
 * a plain `Flagged`, so a message with a colour is flagged and clearing the colour unflags
 * it. Without this the browser build had no path at all and the menu threw.
 */
export function setFlagColor(
  ids: number[],
  color: string | null,
): { changed: number; mailboxIds: number[] } {
  const data = current()
  const mailboxIds = mailboxesOf(ids)
  let changed = 0

  for (const id of ids) {
    const message = data.messages.get(id)
    if (!message) continue
    message.flagColor = color
    message.flagged = color !== null
    changed += 1
  }

  recount(data, mailboxIds)
  return { changed, mailboxIds }
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

  // Refused here for the same reason `msg_move` refuses it in the core: mail cannot be moved
  // between accounts by moving a row, because the message lives on a different server. The
  // sidebar will not offer such a drop, and this is what stops the browser build quietly
  // accepting one anyway and reporting that a broken UI works.
  const destination = data.mailboxes.find((mailbox) => mailbox.id === mailboxId)
  if (destination === undefined) throw new Error('No such mailbox')

  const crossing = ids.some((id) => {
    const message = data.messages.get(id)
    return message !== undefined && message.accountId !== destination.accountId
  })
  if (crossing) throw new Error('A message can only be moved to a folder in its own account.')

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

/**
 * Mark All Messages as Read, the way `mailbox_mark_read` does it: every unread message in the
 * mailbox. The browser path used to return 0 and change nothing, which the menu then reported
 * as "Nothing was unread" over a badge that said otherwise.
 */
export function mailboxMarkRead(mailboxId: number): number {
  const data = current()
  const unread = (data.byMailbox.get(mailboxId) ?? []).filter(
    (id) => data.messages.get(id)?.seen === false,
  )

  return setFlags(unread, { seen: true, flagged: null }).changed
}

/* ------------------------------------------------------------ mailbox structure */

/**
 * The mailbox context menu's commands, against the in-memory store. `sync::folders` in the core.
 *
 * The same rules and the same sentences, so the sheets the Playwright suite drives behave as they
 * do in the app. The one simplification is the one the browser's folders already have: a name is
 * its own path, so "taken" means another folder in the account with that name.
 */

function folderOrThrow(mailboxId: number): MailboxRow {
  const mailbox = current().mailboxes.find((entry) => entry.id === mailboxId)
  if (mailbox === undefined) throw new Error('That mailbox no longer exists.')
  return mailbox
}

/** A mailbox's path, as the core keeps `remote_path`. */
function pathOf(mailbox: MailboxRow): string {
  return current().paths.get(mailbox.id) ?? mailbox.displayName
}

function isInboxPath(path: string): boolean {
  return path.toLowerCase() === 'inbox'
}

/** Whether a mailbox is at `path` or inside it. */
function within(candidate: string, path: string): boolean {
  return candidate === path || candidate.startsWith(`${path}/`)
}

/**
 * A name the user typed, checked as `sync::folders` checks it: the rules, then whether the
 * account already has a mailbox at that path — ignoring case, as the core does.
 */
function checkName(
  accountId: number,
  parentPath: string | null,
  parentName: string | null,
  name: string,
  except: number | null,
): { name: string; path: string } {
  const problem = mailboxNameProblem(name, '/')
  if (problem !== null) throw new Error(problem)

  const trimmed = name.trim()
  const path = parentPath === null ? trimmed : `${parentPath}/${trimmed}`

  if (isInboxPath(path)) {
    throw new Error('“Inbox” is the name of the account’s inbox.')
  }

  const taken = current().mailboxes.some(
    (entry) =>
      entry.accountId === accountId &&
      entry.id !== except &&
      pathOf(entry).toLowerCase() === path.toLowerCase(),
  )
  if (taken) {
    throw new Error(
      parentName === null
        ? `There’s already a mailbox called “${trimmed}”.`
        : `There’s already a mailbox called “${trimmed}” in “${parentName}”.`,
    )
  }

  return { name: trimmed, path }
}

export function mailboxCreate(accountId: number, parentId: number | null, name: string): number {
  const data = current()
  if (!data.accounts.some((account) => account.id === accountId)) {
    throw new Error('That mailbox no longer exists.')
  }

  let parentPath: string | null = null
  let parentName: string | null = null
  if (parentId !== null) {
    const parent = folderOrThrow(parentId)
    if (parent.accountId !== accountId) throw new Error('That mailbox no longer exists.')
    if (isInboxPath(pathOf(parent))) {
      throw new Error(`A mailbox can’t be made inside “${parent.displayName}”.`)
    }
    parentPath = pathOf(parent)
    parentName = parent.displayName
  }

  const checked = checkName(accountId, parentPath, parentName, name, null)
  const id = Math.max(0, ...data.mailboxes.map((mailbox) => mailbox.id)) + 1
  data.mailboxes.push(folderRow(id, accountId, checked.name, null))
  data.paths.set(id, checked.path)
  return id
}

/** Returns the account, for the event. */
export function mailboxRename(mailboxId: number, name: string): number {
  const data = current()
  const mailbox = folderOrThrow(mailboxId)
  if (!mailbox.editable) {
    throw new Error('This mailbox belongs to the account and can’t be renamed.')
  }

  const from = pathOf(mailbox)
  const cut = from.lastIndexOf('/')
  const parentPath = cut < 0 ? null : from.slice(0, cut)
  const parent =
    parentPath === null
      ? undefined
      : data.mailboxes.find(
          (entry) => entry.accountId === mailbox.accountId && pathOf(entry) === parentPath,
        )

  const checked = checkName(
    mailbox.accountId,
    parentPath,
    parent?.displayName ?? null,
    name,
    mailbox.id,
  )

  // Everything inside goes with it, as a server's RENAME takes the children.
  for (const entry of data.mailboxes) {
    const entryPath = pathOf(entry)
    if (entry.accountId === mailbox.accountId && within(entryPath, from)) {
      data.paths.set(entry.id, checked.path + entryPath.slice(from.length))
    }
  }
  mailbox.displayName = checked.name
  return mailbox.accountId
}

export function mailboxDelete(mailboxId: number): {
  accountId: number
  mailboxIds: number[]
  messages: number
} {
  const data = current()
  const mailbox = folderOrThrow(mailboxId)
  if (!mailbox.editable) {
    throw new Error('This mailbox belongs to the account and can’t be deleted.')
  }

  const root = pathOf(mailbox)
  const doomed = data.mailboxes.filter(
    (entry) => entry.accountId === mailbox.accountId && within(pathOf(entry), root),
  )
  const ids = doomed.map((entry) => entry.id)

  let messages = 0
  for (const id of ids) {
    const held = data.byMailbox.get(id) ?? []
    for (const messageId of held) data.messages.delete(messageId)
    messages += held.length
    data.byMailbox.delete(id)
    data.paths.delete(id)
    data.chosen.delete(id)
  }
  data.mailboxes = data.mailboxes.filter((entry) => !ids.includes(entry.id))
  // `favourite.mailbox_id ... ON DELETE CASCADE`.
  data.favourites = data.favourites.filter(
    (entry) => entry.mailboxId === null || !ids.includes(entry.mailboxId),
  )

  return { accountId: mailbox.accountId, mailboxIds: ids, messages }
}

export function mailboxErase(
  accountId: number,
  target: 'trash' | 'junk',
): { mailboxId: number; messages: number } {
  const data = current()
  const mailbox = data.mailboxes.find(
    (entry) => entry.accountId === accountId && entry.role === target,
  )
  if (mailbox === undefined) {
    throw new Error(
      target === 'junk' ? 'This account has no Junk mailbox.' : 'This account has no Bin.',
    )
  }

  const ids = data.byMailbox.get(mailbox.id) ?? []
  for (const id of ids) data.messages.delete(id)
  data.byMailbox.set(mailbox.id, [])
  recount(data, [mailbox.id])

  return { mailboxId: mailbox.id, messages: ids.length }
}

/** Favourites in order. Copies, for the reason `mailboxesTree` gives. */
export function favouritesList(): FavouriteRow[] {
  return current().favourites.map((entry) => ({ ...entry }))
}

/** `folders::move_favourite`: in front of `before`, or at the end. */
export function favouriteMove(favouriteId: number, before: number | null): void {
  const data = current()
  const moving = data.favourites.find((entry) => entry.id === favouriteId)
  if (moving === undefined) throw new Error('That mailbox no longer exists.')
  if (before === favouriteId) return

  const rest = data.favourites.filter((entry) => entry.id !== favouriteId)
  const at = before === null ? rest.length : rest.findIndex((entry) => entry.id === before)
  if (at < 0) throw new Error('That mailbox no longer exists.')

  rest.splice(at, 0, moving)
  data.favourites = rest
}

/** Returns the account, for the event. */
export function mailboxSetFavourite(
  mailboxId: number,
  favourite: boolean,
  before: number | null = null,
): number {
  const data = current()
  const mailbox = folderOrThrow(mailboxId)

  if (!favourite) {
    data.favourites = data.favourites.filter((entry) => entry.mailboxId !== mailboxId)
    return mailbox.accountId
  }

  let entry = data.favourites.find((each) => each.mailboxId === mailboxId)
  if (entry === undefined) {
    entry = {
      id: Math.max(0, ...data.favourites.map((each) => each.id)) + 1,
      builtin: null,
      mailboxId,
    }
    data.favourites.push(entry)
  }
  if (before !== null) favouriteMove(entry.id, before)

  return mailbox.accountId
}

/** `folders::CHOOSABLE_ROLES`. */
const CHOOSABLE: ReadonlySet<string> = new Set(['drafts', 'sent', 'junk', 'trash', 'archive'])

/** `folders::use_as`. Returns the account, for the event. */
export function mailboxUseAs(mailboxId: number, usage: MailboxUse): number {
  const data = current()
  const mailbox = folderOrThrow(mailboxId)
  const account = data.accounts.find((entry) => entry.id === mailbox.accountId)

  if (isInboxPath(pathOf(mailbox))) {
    throw new Error('The Inbox can’t be used as another mailbox.')
  }
  if (account?.provider === 'gmail' || account?.provider === 'google') {
    throw new Error('Gmail decides which of its mailboxes are Drafts, Sent, Junk and Bin.')
  }
  if (!CHOOSABLE.has(usage)) {
    throw new Error('A mailbox can only be used as Drafts, Sent, Junk, Bin or Archive.')
  }

  for (const entry of data.mailboxes) {
    if (entry.accountId !== mailbox.accountId || entry.id === mailbox.id) continue
    if (entry.role === usage) {
      entry.role = null
      entry.editable = true
      data.chosen.delete(entry.id)
    }
  }

  mailbox.role = usage
  mailbox.editable = false
  data.chosen.add(mailbox.id)
  return mailbox.accountId
}

/** Rebuild. The browser's mail is generated, so there is nothing to read again. */
export function mailboxRebuild(mailboxId: number): {
  accountId: number
  mailboxId: number
  messages: number
} {
  const data = current()
  const mailbox = folderOrThrow(mailboxId)
  return {
    accountId: mailbox.accountId,
    mailboxId,
    messages: (data.byMailbox.get(mailboxId) ?? []).length,
  }
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
      needsOauthClient: !oauthClients.has('google') && !builtinClient('google'),
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
      needsOauthClient: !oauthClients.has('microsoft') && !builtinClient('microsoft'),
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

/**
 * `?account-colours=1` gives the seeded accounts a colour each.
 *
 * The same device as `?first-run=1` below, and for the same reason: a browser page holds the
 * overlay in module memory, so Settings at `/?settings=1` and the mailbox at `/` are two page
 * loads that share nothing. A colour picked in one is gone by the time the other renders, and
 * without this there is no way for a test — or for anyone looking at the app in a browser — to
 * see a coloured sidebar at all.
 *
 * The seeds stay `null` rather than being coloured outright, because a freshly added account
 * has no colour in the real database and the committed visual baselines are of that state.
 */
const SEEDED_COLOURS = ['purple', 'green', 'orange']

function seededColour(index: number): string | null {
  if (!new URLSearchParams(window.location.search).has('account-colours')) return null
  return SEEDED_COLOURS[index % SEEDED_COLOURS.length] ?? null
}

function overlayFor(id: number, fallbackName: string, index: number): AccountOverlay {
  const existing = overlays.get(id)
  if (existing) return existing

  const created: AccountOverlay = {
    displayName: fallbackName,
    color: seededColour(index),
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

/**
 * Whether this page is standing in for a build that carries its own client for `provider`.
 *
 * By default the browser store is a build with **nothing compiled in** — a build from public
 * source, which is what docs/05 §9 requires such a build to be — so the Playwright suite
 * exercises the bring-your-own path that such a build takes.
 *
 * `?builtin-clients=google,microsoft` makes it the other kind: the developer's own build, where
 * `src-tauri/oauth/clients.env` supplied a client. The same device as `?first-run=1` and
 * `?account-colours=1`. Without it, no browser test could ever render the state that a user of
 * a real build is actually in — which is how a note reading "Halcyon ships without one" could
 * have gone on appearing in a build that shipped with one.
 */
function builtinClient(provider: string): boolean {
  const listed = new URLSearchParams(window.location.search).get('builtin-clients')
  if (listed === null) return false
  return listed.split(',').some((entry) => entry.trim() === provider)
}

export function oauthClientGet(provider: string): OAuthClientStatus {
  const clientId = oauthClients.get(provider)
  const builtin = builtinClient(provider)
  const source = clientId !== undefined ? 'custom' : builtin ? 'builtin' : null

  return {
    provider,
    configured: source !== null,
    source,
    builtin,
    clientId: clientId ?? null,
    hasSecret: false,
    // The browser store never keeps a secret — `oauthClientSet` here takes only the id — so a
    // Google client entered in a browser is one the real core would refuse to use. The built-in
    // one always has its secret: the build refuses to carry half a Google client.
    missingSecret: source === 'custom' && provider === 'google',
  }
}

export function oauthClientSet(provider: string, clientId: string): void {
  if (clientId.trim() === '') oauthClients.delete(provider)
  else oauthClients.set(provider, clientId.trim())
}

export function providerSetupUrl(provider: string): string | null {
  return PROVIDER_SETUP[provider] ?? null
}
