import { describe, expect, it } from 'vitest'

import { buildSidebar, visibleRows } from '@/features/sidebar/model'
import { sortRows } from '@/features/messageList/sort'
import type { MessageRow } from '@/lib/generated/MessageRow'
import * as store from '@/mock/browserStore'

/**
 * The browser store and the sidebar it feeds.
 *
 * The browser store is not a toy: it is what the app is when served by Vite, and it is what
 * the whole Playwright suite drives. Its paging semantics have to match the Rust
 * implementation exactly, or the e2e tests pass over bugs that only appear in the real app.
 * The pagination cases below deliberately mirror `db::tests_queries`.
 */

const accounts = store.accountsList()
const mailboxes = store.mailboxesTree()
const inbox = mailboxes.find((mailbox) => mailbox.role === 'inbox')

describe('browser store pagination', () => {
  it('walks every row exactly once, in order, across page boundaries', () => {
    expect(inbox).toBeDefined()
    if (!inbox) return

    const seen: number[] = []
    let cursor = null as Parameters<typeof store.messagesPage>[0]['cursor']

    // Small pages on purpose: the interesting failures are at the seams.
    for (let guard = 0; guard < 200; guard += 1) {
      const page = store.messagesPage({
        mailboxIds: [inbox.id],
        cursor,
        limit: 7,
        unreadOnly: false,
      })

      seen.push(...page.items.map((item) => item.id))
      if (page.nextCursor === null) break
      cursor = page.nextCursor
    }

    expect(new Set(seen).size, 'no row appears twice').toBe(seen.length)
    expect(seen.length, 'no row is skipped').toBe(inbox.totalCount)

    const dates = seen.map((id) => store.messageGet(id)?.dateReceived ?? 0)
    expect(dates, 'newest first throughout').toEqual([...dates].sort((a, b) => b - a))
  })

  it('breaks timestamp ties by id, which is why the cursor carries one', () => {
    // Two messages sharing a received time is common — a sync commits a batch with one
    // clock reading. A cursor on the date alone repeats or skips that whole run.
    expect(inbox).toBeDefined()
    if (!inbox) return

    const all = store.messagesPage({
      mailboxIds: [inbox.id],
      cursor: null,
      limit: 500,
      unreadOnly: false,
    }).items

    for (let i = 1; i < all.length; i += 1) {
      const previous = all[i - 1]
      const current = all[i]
      if (!previous || !current) continue

      if (previous.dateReceived === current.dateReceived) {
        expect(previous.id).toBeGreaterThan(current.id)
      }
    }
  })

  it('stops promising a next page once the end is reached', () => {
    expect(inbox).toBeDefined()
    if (!inbox) return

    const page = store.messagesPage({
      mailboxIds: [inbox.id],
      cursor: null,
      limit: inbox.totalCount,
      unreadOnly: false,
    })

    expect(page.items).toHaveLength(inbox.totalCount)
    expect(page.nextCursor).toBeNull()
  })

  it('returns nothing for an empty mailbox set rather than everything', () => {
    // The dangerous failure: no filter at all, and the caller pages the entire store.
    const page = store.messagesPage({ mailboxIds: [], cursor: null, limit: 10, unreadOnly: false })
    expect(page.items).toHaveLength(0)
    expect(page.nextCursor).toBeNull()
  })

  it('merges a unified selection, still ordered', () => {
    const inboxes = mailboxes.filter((mailbox) => mailbox.role === 'inbox')
    expect(inboxes.length).toBeGreaterThan(1)

    const merged = store.messagesPage({
      mailboxIds: inboxes.map((mailbox) => mailbox.id),
      cursor: null,
      limit: 300,
      unreadOnly: false,
    }).items

    expect(new Set(merged.map((row) => row.accountId)).size).toBeGreaterThan(1)

    const dates = merged.map((row) => row.dateReceived)
    expect(dates).toEqual([...dates].sort((a, b) => b - a))
  })

  it('filters to unread without disturbing the order', () => {
    expect(inbox).toBeDefined()
    if (!inbox) return

    const page = store.messagesPage({
      mailboxIds: [inbox.id],
      cursor: null,
      limit: 200,
      unreadOnly: true,
    })

    expect(page.items.every((row) => !row.seen)).toBe(true)
    const dates = page.items.map((row) => row.dateReceived)
    expect(dates).toEqual([...dates].sort((a, b) => b - a))
  })
})

describe('sidebar tree', () => {
  const sections = buildSidebar(accounts, mailboxes)
  const nodes = sections.flatMap((section) => visibleRows(section.nodes, new Set()))

  it('gives every row a unique id even where two rows share a mailbox', () => {
    // The same mailbox appears under All Inboxes and in its account's section. Keying
    // selection on the mailbox instead of the row lit up three rows at once.
    const ids = nodes.map((node) => node.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('makes All Inboxes a real union rather than an alias for the first account', () => {
    const unified = nodes.find((node) => node.id === 'all-inboxes')
    const inboxes = mailboxes.filter((mailbox) => mailbox.role === 'inbox')

    expect(inboxes.length).toBeGreaterThan(1)
    expect(unified?.mailboxIds).toEqual(inboxes.map((mailbox) => mailbox.id))
    expect(unified?.unreadCount).toBe(
      inboxes.reduce((sum, mailbox) => sum + mailbox.unreadCount, 0),
    )
  })

  it('leaves container rows unselectable rather than pointing them at a mailbox', () => {
    expect(nodes.find((node) => node.id === 'flagged')?.mailboxIds).toEqual([])
  })
})

describe('sortRows', () => {
  const row = (over: Partial<MessageRow>): MessageRow => ({
    id: 1,
    threadId: 1,
    mailboxId: 1,
    accountId: 1,
    subject: 'Subject',
    fromName: 'Ada',
    fromAddr: 'ada@example.test',
    dateReceived: 100,
    preview: '',
    size: 1000,
    seen: true,
    answered: false,
    flagged: false,
    flagColor: null,
    hasAttachment: false,
    ...over,
  })

  it('leaves the store order alone when sorting by date descending', () => {
    // The store already returns this order, so the common case must do no work at all.
    const rows = [row({ id: 1 }), row({ id: 2 })]
    expect(sortRows(rows, { field: 'date', ascending: false })).toBe(rows)
  })

  it('ignores Re: and Fwd: when sorting by subject', () => {
    const rows = [
      row({ id: 1, subject: 'bananas' }),
      row({ id: 2, subject: 'Re: apples' }),
      row({ id: 3, subject: 'Fwd: apricots' }),
    ]

    const sorted = sortRows(rows, { field: 'subject', ascending: true })
    expect(sorted.map((entry) => entry.id)).toEqual([2, 3, 1])
  })

  it('puts flagged rows first, newest first among them', () => {
    // Flagged messages in arbitrary order would be useless.
    const rows = [
      row({ id: 1, flagged: false, dateReceived: 500 }),
      row({ id: 2, flagged: true, dateReceived: 100 }),
      row({ id: 3, flagged: true, dateReceived: 300 }),
    ]

    const sorted = sortRows(rows, { field: 'flagged', ascending: false })
    expect(sorted.map((entry) => entry.id)).toEqual([3, 2, 1])
  })

  it('sorts by size and by sender', () => {
    const bySize = sortRows([row({ id: 1, size: 10 }), row({ id: 2, size: 900 })], {
      field: 'size',
      ascending: false,
    })
    expect(bySize.map((entry) => entry.id)).toEqual([2, 1])

    const byFrom = sortRows([row({ id: 1, fromName: 'Zoe' }), row({ id: 2, fromName: 'Ada' })], {
      field: 'from',
      ascending: true,
    })
    expect(byFrom.map((entry) => entry.id)).toEqual([2, 1])
  })
})
