import { act, render, renderHook, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type * as Ipc from '@/lib/ipc'
import type { SyncAccountError } from '@/lib/ipc'
import { ToastProvider } from '@/ui'

/**
 * Signing in again has to *look* as though it worked when it did.
 *
 * Reported on 2026-10-02: after a Google sign-in expired, signing in again "takes two attempts
 * at least". The log says otherwise — the first sign-in was stored at 13:43:40 and its sync
 * started the same second. What the user saw was the strip at the foot of the sidebar, which
 * went on reading "The saved sign-in for this account was refused. Signing in again will fix
 * it." over a Sign In button, because nothing cleared it except sync progress, and that pass
 * took 18 seconds to connect. So they signed in again at 13:44:29.
 */

const listeners: {
  accountError: ((error: SyncAccountError) => void)[]
  reauthenticated: ((accountId: number) => void)[]
} = { accountError: [], reauthenticated: [] }

const accountReauth = vi.fn<(id: number) => Promise<void>>()
const syncAll = vi.fn(() => Promise.resolve())

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  runningInTauri: true,
  accountReauth: (id: number) => accountReauth(id),
  syncAll: () => syncAll(),
  syncWatch: () => Promise.resolve(),
  onAccountError: (handler: (error: SyncAccountError) => void) => {
    listeners.accountError.push(handler)
    return Promise.resolve(() => undefined)
  },
  onAccountReauthenticated: (handler: (accountId: number) => void) => {
    listeners.reauthenticated.push(handler)
    return Promise.resolve(() => undefined)
  },
  onAccountsChanged: () => Promise.resolve(() => undefined),
  onSyncProgress: () => Promise.resolve(() => undefined),
  onMessagesAdded: () => Promise.resolve(() => undefined),
  onMailboxesChanged: () => Promise.resolve(() => undefined),
}))

vi.mock('@tanstack/react-query', () => ({
  useQueryClient: () => ({ invalidateQueries: () => undefined }),
}))

afterEach(() => {
  listeners.accountError = []
  listeners.reauthenticated = []
  accountReauth.mockReset()
  syncAll.mockClear()
})

const refused = (accountId: number): SyncAccountError => ({
  accountId,
  message: 'The saved sign-in for this account was refused. Signing in again will fix it.',
  retryInSeconds: 0,
  needsReauth: true,
})

describe('a new sign-in', () => {
  it('clears the refusal the moment the core stores it, and only for that account', async () => {
    const { useSync } = await import('@/app/useSync')
    const { result } = renderHook(() => useSync())

    await waitFor(() => {
      expect(listeners.reauthenticated.length).toBeGreaterThan(0)
    })

    act(() => {
      for (const handler of listeners.accountError) handler(refused(1))
      for (const handler of listeners.accountError) handler(refused(2))
    })
    expect([...result.current.errors.keys()]).toEqual([1, 2])

    act(() => {
      for (const handler of listeners.reauthenticated) handler(1)
    })

    expect(
      result.current.errors.has(1),
      'no waiting for sync progress — that is what made a sign-in that worked look as though it had not',
    ).toBe(false)
    expect(result.current.errors.has(2), 'the other account has not been signed in again').toBe(
      true,
    )
  })
})

async function renderStrip() {
  const { SyncStatus } = await import('@/features/sidebar/SyncStatus')
  render(
    <ToastProvider>
      <SyncStatus
        errors={new Map([[1, refused(1)]])}
        busy={false}
        online
        accountNames={new Map([[1, 'Gmail']])}
      />
    </ToastProvider>,
  )
  return screen.getByRole('button', { name: 'Sign In' })
}

describe('the Sign In button on the strip', () => {
  it('leaves fetching to the core rather than syncing every account itself', async () => {
    accountReauth.mockResolvedValue(undefined)

    await userEvent.click(await renderStrip())

    await waitFor(() => {
      expect(accountReauth).toHaveBeenCalledWith(1)
    })
    // `accounts:changed` already makes the window sync. Calling `syncAll()` here as well meant
    // every sign-in synced every account twice — five passes each, on the day this was found.
    expect(syncAll).not.toHaveBeenCalled()
  })

  it('says why a sign-in failed instead of leaving the strip as it was', async () => {
    accountReauth.mockRejectedValue({
      code: 'badRequest',
      message:
        'That sign-in did not work for someone@example.test. Check you signed in as that address and not another.',
    })

    await userEvent.click(await renderStrip())

    expect(await screen.findByText('That sign-in did not complete')).toBeTruthy()
    expect(screen.getByText(/Check you signed in as that address/)).toBeTruthy()
  })

  it('stays quiet when the browser was simply abandoned', async () => {
    accountReauth.mockRejectedValue({
      code: 'timedOut',
      message: 'The browser sign-in was not completed. Starting again will reopen it.',
    })

    const button = await renderStrip()
    await userEvent.click(button)

    // Settled: the button is usable again, so the rejection has been handled.
    await waitFor(() => {
      expect(button).not.toBeDisabled()
    })
    expect(screen.queryByText('That sign-in did not complete')).toBeNull()
  })
})
