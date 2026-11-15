import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type * as Ipc from '@/lib/ipc'
import type { AddedAccount } from '@/lib/generated/AddedAccount'
import type { ProviderInfo } from '@/lib/generated/ProviderInfo'
import { ToastProvider } from '@/ui'

/**
 * Giving up on a browser sign-in.
 *
 * L30 in the 2026-11-14 audit: the assistant waited five minutes for the browser with nothing to
 * end the wait, and on first run, where the assistant has no Cancel, a Google sign-in abandoned
 * in the browser left only the spinner.
 */

const google: ProviderInfo = {
  id: 'google',
  displayName: 'Google',
  authKind: 'oAuth2',
  needsManualSetup: false,
  setupNote: null,
  setupUrl: null,
  needsOauthClient: false,
  requiresClientSecret: true,
}

// The sign-in waits until it is cancelled, as the core's does.
let endWait: (() => void) | null = null
const addOauth = vi.fn(
  () =>
    new Promise<AddedAccount>((_, reject) => {
      endWait = () => {
        reject(Object.assign(new Error('The sign-in was cancelled.'), { code: 'cancelled' }))
      }
    }),
)
const cancel = vi.fn(() => {
  endWait?.()
  return Promise.resolve()
})

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  accountAddOauth: () => addOauth(),
  oauthCancel: () => cancel(),
  accountDiscover: () => Promise.resolve(null),
}))

vi.mock('@/features/accounts/queries', () => ({
  useProviders: () => ({ data: [google] }),
}))

afterEach(() => {
  addOauth.mockClear()
  cancel.mockClear()
  endWait = null
})

describe('a browser sign-in', () => {
  it('can be given up on, even on first run, and goes back to the address', async () => {
    const { AccountAssistant } = await import('@/features/accounts/AccountAssistant')
    render(
      <ToastProvider>
        <AccountAssistant open firstRun onOpenChange={() => undefined} />
      </ToastProvider>,
    )

    await userEvent.click(screen.getByRole('radio', { name: /Google/ }))
    await userEvent.click(screen.getByRole('button', { name: 'Continue' }))
    await userEvent.type(screen.getByLabelText('Email Address'), 'ada@example.test')
    await userEvent.click(screen.getByRole('button', { name: 'Continue' }))

    await waitFor(() => {
      expect(addOauth).toHaveBeenCalled()
    })
    expect(screen.getByText(/Finish signing in with your browser/)).toBeInTheDocument()

    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }))

    expect(cancel).toHaveBeenCalledTimes(1)
    await waitFor(() => {
      expect(screen.getByLabelText('Email Address')).toHaveValue('ada@example.test')
    })
    // Cancelled, not failed: nothing to apologise for.
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })
})
