import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type * as Ipc from '@/lib/ipc'
import type { ProviderInfo } from '@/lib/generated/ProviderInfo'
import { ToastProvider } from '@/ui'

/**
 * Autodiscovery in the account assistant, when the user does not wait for it.
 *
 * L23 in the 2026-11-14 audit: the address was marked as looked up when the lookup started. Moving
 * on inside the half-second debounce cancelled the lookup but left the mark, so going back to the
 * address — or closing the assistant and starting again — never looked it up.
 */

const other: ProviderInfo = {
  id: 'other',
  displayName: 'Other Mail Account',
  authKind: 'password',
  needsManualSetup: true,
  setupNote: null,
  setupUrl: null,
  needsOauthClient: false,
  requiresClientSecret: false,
}

const discover = vi.fn<(email: string) => Promise<null>>(() => Promise.resolve(null))

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  accountDiscover: (email: string) => discover(email),
}))

vi.mock('@/features/accounts/queries', () => ({
  useProviders: () => ({ data: [other] }),
}))

afterEach(() => {
  discover.mockClear()
})

async function moveOnBeforeTheLookup() {
  await userEvent.click(screen.getByRole('radio', { name: /Other Mail Account/ }))
  await userEvent.click(screen.getByRole('button', { name: 'Continue' }))

  await userEvent.type(screen.getByLabelText('Email Address'), 'ada@example.test')
  await userEvent.type(screen.getByLabelText('Password'), 'secret')
  // Straight on, inside the debounce.
  await userEvent.click(screen.getByRole('button', { name: 'Continue' }))
  expect(discover).not.toHaveBeenCalled()
}

async function assistant(open: boolean) {
  const { AccountAssistant } = await import('@/features/accounts/AccountAssistant')
  return (
    <ToastProvider>
      <AccountAssistant open={open} onOpenChange={() => undefined} />
    </ToastProvider>
  )
}

describe('autodiscovery', () => {
  it('looks the address up when the user comes back to it', async () => {
    render(await assistant(true))
    await moveOnBeforeTheLookup()

    await userEvent.click(screen.getByRole('button', { name: 'Back' }))

    await waitFor(
      () => {
        expect(discover).toHaveBeenCalledWith('ada@example.test')
      },
      { timeout: 2_000 },
    )
  })

  it('looks the address up again in an assistant opened afresh', async () => {
    const { rerender } = render(await assistant(true))
    await moveOnBeforeTheLookup()

    rerender(await assistant(false))
    rerender(await assistant(true))

    await userEvent.click(screen.getByRole('radio', { name: /Other Mail Account/ }))
    await userEvent.click(screen.getByRole('button', { name: 'Continue' }))
    // Pasted whole. Typed a letter at a time, each shorter prefix replaced the mark and hid the
    // bug; an address arrives in one piece from the clipboard or the browser's autofill.
    await userEvent.click(screen.getByLabelText('Email Address'))
    await userEvent.paste('ada@example.test')

    await waitFor(
      () => {
        expect(discover).toHaveBeenCalledWith('ada@example.test')
      },
      { timeout: 2_000 },
    )
  })
})
