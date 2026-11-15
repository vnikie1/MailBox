import { renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type * as Ipc from '@/lib/ipc'
import type { MessageRow } from '@/lib/generated/MessageRow'

/**
 * The list's body prefetch asks each account for its own messages only.
 *
 * ## The bug this pins
 *
 * The prefetch sent the selected row and the next three under **the selected row's account**.
 * In All Inboxes, a smart mailbox or a search, the next three rows are routinely another
 * account's, and the core fetched each of them by its own UID and folder path from the selected
 * account's server. `INBOX` exists everywhere and UIDs are small integers, so that usually found
 * *a* message — somebody else's — and stored it as this row's body. The reader showed it, Reply
 * quoted it, and Forward as Attachment sent it on.
 *
 * The core now refuses an id from another account too (`sync::engine::bodies_wanted`); this
 * pins the half that makes the prefetch useful rather than merely harmless.
 */

const ensured: { accountId: number; ids: number[] }[] = []

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  bodiesEnsure: (accountId: number, ids: number[]): Promise<void> => {
    ensured.push({ accountId, ids })
    return Promise.resolve()
  },
}))

function row(id: number, accountId: number): MessageRow {
  return {
    id,
    threadId: id,
    mailboxId: accountId * 10,
    accountId,
    subject: `Subject ${String(id)}`,
    fromName: null,
    fromAddr: null,
    dateReceived: 1_700_000_000 - id,
    preview: null,
    size: 0,
    seen: false,
    answered: false,
    flagged: false,
    flagColor: null,
    hasAttachment: false,
    muted: false,
  }
}

beforeEach(() => {
  ensured.length = 0
})

describe('useBodyPrefetch', () => {
  it('asks each account only for its own messages', async () => {
    const { useBodyPrefetch } = await import('@/features/reader/useBodyPrefetch')

    // All Inboxes, interleaved by date: accounts 1 and 2 alternate.
    const rows = [row(1, 1), row(2, 2), row(3, 1), row(4, 2), row(5, 1)]

    renderHook(() => {
      useBodyPrefetch(1, rows)
    })

    const byAccount = new Map(ensured.map((call) => [call.accountId, call.ids]))

    expect(byAccount.get(1)).toEqual([1, 3])
    expect(byAccount.get(2)).toEqual([2, 4])
    expect(ensured.flatMap((call) => call.ids).sort()).toEqual([1, 2, 3, 4])
  })

  it('asks for the selected message before the ones after it', async () => {
    const { useBodyPrefetch } = await import('@/features/reader/useBodyPrefetch')

    const rows = [row(1, 2), row(2, 1), row(3, 2)]

    renderHook(() => {
      useBodyPrefetch(2, rows)
    })

    // The selected message's account is asked first, and the selected message leads its list.
    expect(ensured[0]).toEqual({ accountId: 1, ids: [2] })
    expect(ensured[1]).toEqual({ accountId: 2, ids: [3] })
  })

  it('asks for nothing it cannot place in an account', async () => {
    const { useBodyPrefetch } = await import('@/features/reader/useBodyPrefetch')

    // The selection is not among the rows, so its account is unknown. It used to be sent under
    // the first row's account. The reader's own fetch, which knows every message's account,
    // covers it.
    renderHook(() => {
      useBodyPrefetch(99, [row(1, 1), row(2, 2)])
    })

    expect(ensured).toEqual([])
  })
})
