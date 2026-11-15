import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderHook, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'

import type * as Organise from '@/lib/organise'
import type { MailboxRow } from '@/lib/generated/MailboxRow'
import type { MatchCounts } from '@/lib/generated/MatchCounts'
import type { Predicate } from '@/lib/generated/Predicate'
import { listCounts } from '@/features/messageList/listCounts'
import type { MailboxSelection } from '@/store/mail'

/**
 * The list header's count. Found in batch 7 of the 2026-11-14 audit and fixed after batch 8:
 * every smart row — Flagged, the flag colours, VIPs, saved searches — read "0 messages" over a
 * full list, because the header added up the selection's mailboxes and a smart row has none.
 */

let counted: MatchCounts = { total: 0, unread: 0 }
const smartCount = vi.fn(() => Promise.resolve(counted))

vi.mock('@/lib/organise', async (importOriginal) => ({
  ...(await importOriginal<typeof Organise>()),
  smartCount: () => smartCount(),
}))

const FLAGGED: Predicate = {
  type: 'is',
  value: { field: 'isFlagged', op: 'isTrue', value: '' },
}

function mailbox(id: number, totalCount: number, unreadCount: number): MailboxRow {
  return {
    id,
    accountId: 1,
    displayName: `Mailbox ${String(id)}`,
    parentId: null,
    role: null,
    unreadCount,
    totalCount,
    favouriteOrder: null,
    roleChosen: false,
    delimiter: '/',
    editable: true,
    descendants: 0,
    canContain: true,
  }
}

const MAILBOXES = [mailbox(1, 40, 3), mailbox(2, 60, 2)]

describe('the list header', () => {
  it("counts a smart row's matches, not its mailboxes, of which it has none", () => {
    const flagged: MailboxSelection = {
      nodeId: 'flagged',
      label: 'Flagged',
      mailboxIds: [],
      predicate: FLAGGED,
    }

    expect(listCounts(flagged, MAILBOXES, { total: 12, unread: 4 })).toEqual({
      total: 12,
      unread: 4,
    })
  })

  it('says nothing for a smart row until its count arrives, rather than 0', () => {
    const flagged: MailboxSelection = {
      nodeId: 'flagged',
      label: 'Flagged',
      mailboxIds: [],
      predicate: FLAGGED,
    }

    expect(listCounts(flagged, MAILBOXES, undefined)).toBeNull()
  })

  it('still adds up the mailboxes of a folder or a unified row', () => {
    const inboxes: MailboxSelection = {
      nodeId: 'all-inboxes',
      label: 'All Inboxes',
      mailboxIds: [1, 2],
    }

    expect(listCounts(inboxes, MAILBOXES, undefined)).toEqual({ total: 100, unread: 5 })
  })
})

describe("a smart row's count", () => {
  it('is asked again whenever the list is', async () => {
    const { useSmartCount } = await import('@/app/queries')
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    )

    counted = { total: 12, unread: 4 }
    const { result } = renderHook(() => useSmartCount(FLAGGED), { wrapper })
    await waitFor(() => {
      expect(result.current.data).toEqual({ total: 12, unread: 4 })
    })

    // A message read somewhere else: the events mark `['messages']` stale, and nothing else.
    counted = { total: 12, unread: 3 }
    await client.invalidateQueries({ queryKey: ['messages'] })

    await waitFor(() => {
      expect(result.current.data).toEqual({ total: 12, unread: 3 })
    })
  })
})
