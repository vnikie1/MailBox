import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { ToastProvider } from '@/ui'

/**
 * Save Search, with the scope bar set.
 *
 * L18 in the 2026-11-14 audit: only the typed text was saved, so a search narrowed to Unread in
 * one mailbox became a smart mailbox of every match everywhere.
 */

const saved = vi.fn<(name: string, text: string) => Promise<number>>(() => Promise.resolve(1))

vi.mock('@/lib/search', () => ({
  saveSearchAsSmartMailbox: (name: string, text: string) => saved(name, text),
}))

afterEach(() => {
  saved.mockClear()
})

async function save(props: { state?: 'all' | 'unread' | 'flagged'; mailboxNames?: string[] }) {
  const { SaveSearchSheet } = await import('@/features/search/SaveSearchSheet')
  render(
    <ToastProvider>
      <SaveSearchSheet open onOpenChange={() => undefined} text="invoice" {...props} />
    </ToastProvider>,
  )
  await userEvent.click(screen.getByRole('button', { name: 'Save' }))
  await waitFor(() => {
    expect(saved).toHaveBeenCalled()
  })
  return saved.mock.calls[0]?.[1]
}

describe('Save Search', () => {
  it('keeps Unread and the mailbox the search was limited to', async () => {
    expect(await save({ state: 'unread', mailboxNames: ['Archive'] })).toBe(
      'invoice is:unread mailbox:"Archive"',
    )
  })

  it('keeps Flagged', async () => {
    expect(await save({ state: 'flagged' })).toBe('invoice is:flagged')
  })

  it('names a mailbox that several accounts share once', async () => {
    // All Inboxes: one Inbox per account, and one name.
    expect(await save({ mailboxNames: ['Inbox', 'Inbox', 'Inbox'] })).toBe(
      'invoice mailbox:"Inbox"',
    )
  })

  it('says so when the mailboxes cannot be kept, rather than saving nothing', async () => {
    expect(await save({ mailboxNames: ['Inbox', 'Archive'] })).toBe('invoice')
    expect(screen.getByText(/limited to one mailbox name only/)).toBeTruthy()
  })
})
