import { renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

/**
 * Adding an account has to *fetch* it, not merely watch it.
 *
 * A Yahoo account added mid-session showed only its name in the sidebar and never synced: the
 * store held the account row, zero mailboxes and zero messages, and the log had no
 * `sync starting` for it at all. The handler for `accounts:changed` called `sync_watch` and
 * stopped there, and watching only reports what arrives *next* — so the account had a watcher
 * and nothing to show until the app was restarted or Get Mail was pressed by hand.
 *
 * Tested here rather than by observation, because the two paths are indistinguishable once the
 * app has restarted: a launch sync would have fetched the account anyway, and did, which is
 * exactly what made the original fault survive as long as it did.
 */

const listeners: { accountsChanged: (() => void)[] } = { accountsChanged: [] }
// Typed by their implementation rather than `vi.fn()` alone, which is `any` and trips
// the no-unsafe-return rule where the mock module forwards to them.
const syncAll = vi.fn(() => Promise.resolve())
const syncWatch = vi.fn(() => Promise.resolve())

vi.mock('@/lib/ipc', () => ({
  runningInTauri: true,
  syncAll: () => syncAll(),
  syncWatch: () => syncWatch(),
  onAccountsChanged: (handler: () => void) => {
    listeners.accountsChanged.push(handler)
    return Promise.resolve(() => undefined)
  },
  onSyncProgress: () => Promise.resolve(() => undefined),
  onAccountError: () => Promise.resolve(() => undefined),
  onMessagesAdded: () => Promise.resolve(() => undefined),
  onMailboxesChanged: () => Promise.resolve(() => undefined),
}))

vi.mock('@tanstack/react-query', () => ({
  useQueryClient: () => ({ invalidateQueries: () => undefined }),
}))

describe('when an account is added', () => {
  beforeEach(() => {
    listeners.accountsChanged = []
    syncAll.mockClear()
    syncWatch.mockClear()
  })

  it('fetches it as well as watching it', async () => {
    const { useSync } = await import('@/app/useSync')
    renderHook(() => useSync())

    await waitFor(() => {
      expect(listeners.accountsChanged.length).toBeGreaterThan(0)
    })

    // The launch sync runs on mount; this is about what the *event* does.
    syncAll.mockClear()
    syncWatch.mockClear()

    for (const handler of listeners.accountsChanged) handler()

    await waitFor(() => {
      expect(
        syncWatch,
        'the new account needs a watcher for mail that arrives later',
      ).toHaveBeenCalled()
      expect(
        syncAll,
        'watching alone leaves the account with no mailboxes and no mail until a restart',
      ).toHaveBeenCalled()
    })
  })
})
