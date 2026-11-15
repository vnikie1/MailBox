import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'

import type * as Ipc from '@/lib/ipc'
import type * as Search from '@/lib/search'
import type { MessageRow } from '@/lib/generated/MessageRow'
import type { SearchResults } from '@/lib/generated/SearchResults'
import { buildListItems } from '@/features/messageList/rows'

/**
 * The message list and search, from batch 7 of the 2026-11-14 audit.
 *
 * - M34: relevance-ranked results got date headers, which repeated with the same key.
 * - M35: search results never refreshed after the mail changed.
 * - M36: a search that found nothing could never say so.
 * - M37: dates were labelled against the moment the window opened.
 */

// The real clock, rather than the browser fixtures' fixed date, so a test can move it.
vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  storeNow: () => new Date(),
}))

/** Each search answers with whatever this holds when it runs. */
let answer: MessageRow[] = []
let searches = 0

vi.mock('@/lib/search', async (importOriginal) => ({
  ...(await importOriginal<typeof Search>()),
  runSearch: (): Promise<SearchResults> => {
    searches += 1
    return Promise.resolve({
      hits: answer.map((row) => ({
        row,
        score: 1,
        terms: [],
        fromVip: false,
        participated: false,
      })),
      parsed: {
        terms: [],
        from: [],
        to: [],
        subject: [],
        mailbox: [],
        hasAttachment: null,
        isUnread: null,
        isFlagged: null,
        isJunk: null,
        before: null,
        after: null,
        largerThan: null,
        smallerThan: null,
      },
    })
  },
}))

function row(id: number, dateReceived: number, seen = false): MessageRow {
  return {
    id,
    threadId: id,
    mailboxId: 1,
    accountId: 1,
    subject: `Subject ${String(id)}`,
    fromName: null,
    fromAddr: null,
    dateReceived,
    preview: null,
    size: 0,
    seen,
    answered: false,
    flagged: false,
    flagColor: null,
    hasAttachment: false,
    muted: false,
  }
}

beforeEach(() => {
  answer = []
  searches = 0
})

afterEach(() => {
  vi.useRealTimers()
})

describe('date headers', () => {
  it('never give two items the same key, even when a section comes round again', () => {
    // M34. Rows out of date order -- today, last year, today -- made a second "Today" header
    // with the first one's key.
    const now = new Date(2026, 10, 15, 12)
    const today = Math.floor(now.getTime() / 1000) - 60
    const lastYear = Math.floor(new Date(2025, 0, 1).getTime() / 1000)

    const items = buildListItems({
      rows: [row(1, today), row(2, lastYear), row(3, today)],
      now,
      selected: new Set(),
      grouped: true,
    })

    const keys = items.map((item) => item.key)
    expect(new Set(keys).size).toBe(keys.length)
    expect(items.filter((item) => item.kind === 'header')).toHaveLength(3)
  })
})

function wrapper(client: QueryClient) {
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

describe('search results', () => {
  it('are fetched again when the mail changes', async () => {
    // M35. A hit opened and read stayed bold, because the results were state filled once.
    const { useSearch } = await import('@/features/search/useSearch')
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    answer = [row(1, 100, false)]

    const { result } = renderHook(() => useSearch([]), { wrapper: wrapper(client) })
    act(() => {
      result.current.commit('ada')
    })
    await waitFor(() => {
      expect(result.current.hits.map((hit) => hit.row.seen)).toEqual([false])
    })

    // The message is read, and every mutation invalidates `['search']`.
    answer = [row(1, 100, true)]
    await act(async () => {
      await client.invalidateQueries({ queryKey: ['search'] })
    })

    await waitFor(() => {
      expect(result.current.hits.map((hit) => hit.row.seen)).toEqual([true])
    })
    expect(searches).toBe(2)
  })

  it('say they are still running until the search for what is typed has answered', async () => {
    // M36. The list shows "No messages match" only when the search is not running, so running
    // must already be true inside the debounce, before anything has been asked.
    const { useSearch } = await import('@/features/search/useSearch')
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })

    const { result } = renderHook(() => useSearch([]), { wrapper: wrapper(client) })
    act(() => {
      result.current.setText('nothing matches this')
    })
    expect(result.current.running).toBe(true)

    await waitFor(() => {
      expect(result.current.running).toBe(false)
    })
    expect(result.current.hits).toEqual([])
    expect(result.current.failed).toBe(false)
  })

  it('are empty, and not running, once cleared', async () => {
    const { useSearch } = await import('@/features/search/useSearch')
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    answer = [row(1, 100)]

    const { result } = renderHook(() => useSearch([]), { wrapper: wrapper(client) })
    act(() => {
      result.current.commit('ada')
    })
    await waitFor(() => {
      expect(result.current.hits).toHaveLength(1)
    })

    act(() => {
      result.current.clear()
    })
    expect(result.current.hits).toEqual([])
    expect(result.current.running).toBe(false)
  })
})

describe('today', () => {
  it('moves on at midnight', async () => {
    // M37. A window left open overnight filed yesterday's mail under Today.
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] })
    vi.setSystemTime(new Date(2026, 10, 15, 23, 59, 50))
    const { useToday } = await import('@/app/useToday')

    const { result } = renderHook(() => useToday())
    expect(result.current.getDate()).toBe(15)

    act(() => {
      vi.advanceTimersByTime(15_000)
    })
    expect(result.current.getDate()).toBe(16)
  })

  it('catches up when the window is shown again after the PC slept', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] })
    vi.setSystemTime(new Date(2026, 10, 15, 22, 0))
    const { useToday } = await import('@/app/useToday')

    const { result } = renderHook(() => useToday())

    // The machine sleeps through midnight: the clock moves and no timer runs.
    vi.setSystemTime(new Date(2026, 10, 16, 8, 0))
    act(() => {
      document.dispatchEvent(new Event('visibilitychange'))
    })
    expect(result.current.getDate()).toBe(16)
  })
})
