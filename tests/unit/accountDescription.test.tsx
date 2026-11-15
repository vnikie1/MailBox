import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import type * as Ipc from '@/lib/ipc'
import type { AccountDetail } from '@/lib/generated/AccountDetail'
import { ToastProvider } from '@/ui'

/**
 * An account's description, cleared.
 *
 * L26 in the 2026-11-14 audit: an empty description was saved, which blanked the account's
 * heading in the sidebar and its entry in the From menu.
 */

const update = vi.fn<(id: number, patch: { displayName: string }) => Promise<void>>(() =>
  Promise.resolve(),
)

const ACCOUNT = {
  id: 1,
  displayName: 'Work',
  email: 'ada@work.test',
  provider: 'other',
  color: null,
} as unknown as AccountDetail

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  accountUpdate: (id: number, patch: { displayName: string }) => update(id, patch),
}))

vi.mock('@/features/accounts/queries', () => ({
  useAccountsDetail: () => ({ data: [ACCOUNT] }),
  useOAuthClient: () => ({ data: undefined }),
  useProviders: () => ({ data: [] }),
}))

vi.mock('@/features/accounts/useAccountsChanged', () => ({
  useAccountsChanged: () => () => undefined,
}))

vi.mock('@/features/accounts/AccountAssistant', () => ({
  AccountAssistant: () => null,
}))

describe('an account’s description', () => {
  it('is put back rather than saved empty', async () => {
    const { AccountsSettings } = await import('@/features/accounts/AccountsSettings')
    render(
      <ToastProvider>
        <AccountsSettings />
      </ToastProvider>,
    )

    const field = screen.getByLabelText('Description')
    await userEvent.clear(field)
    await userEvent.tab()

    expect(update).not.toHaveBeenCalled()
    expect(field).toHaveValue('Work')
  })
})
