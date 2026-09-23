import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type * as Ipc from '@/lib/ipc'
import type { OutboxRow } from '@/lib/generated/OutboxRow'
import { ToastProvider } from '@/ui'

/**
 * The send-failure banner can be dismissed.
 *
 * Reported on 2026-09-17: Gmail refused a message for an attachment — a zip of DLLs — and the
 * banner's only button was Try Again. A refusal about content fails the same way on every
 * attempt, so the message and its banner stayed for good. The banner is inside Tauri only
 * (`runningInTauri`), so this is a component test with the IPC mocked.
 */

const GMAIL_BLOCK =
  'Gmail blocked this message because of an attachment — usually a program or a file like a ' +
  '.dll, which Gmail refuses even inside a .zip. Sending it again will not help: delete it, and ' +
  'share the file as a Google Drive link instead. Gmail said: permanent error (552): 5.7.0 ...'

const failedRow: OutboxRow = {
  id: 7,
  accountId: 1,
  state: 'failed',
  subject: 'how are you',
  recipients: 'someone@example.test',
  sendAfter: 0,
  attempts: 5,
  lastError: GMAIL_BLOCK,
}

let rows: OutboxRow[] = [failedRow]
const discard = vi.fn<(id: number) => Promise<boolean>>()
const retry = vi.fn<(id: number) => Promise<boolean>>()

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  runningInTauri: true,
  outboxList: () => Promise.resolve(rows),
  outboxDiscard: (id: number) => discard(id),
  outboxRetry: (id: number) => retry(id),
  onOutboxProgress: () => Promise.resolve(() => undefined),
}))

afterEach(() => {
  rows = [failedRow]
  discard.mockReset()
  retry.mockReset()
})

async function renderBanner() {
  const { OutboxBanner } = await import('@/features/outbox/OutboxBanner')
  render(
    <ToastProvider>
      <OutboxBanner />
    </ToastProvider>,
  )
  return screen.findByRole('alert')
}

describe('a failed message', () => {
  it('explains why it failed and offers Delete beside Try Again', async () => {
    const banner = await renderBanner()

    expect(banner.textContent).toContain('“how are you” was not sent.')
    expect(banner.textContent).toContain('Sending it again will not help')
    expect(within(banner).getByRole('button', { name: 'Try Again' })).toBeTruthy()
    expect(within(banner).getByRole('button', { name: 'Delete' })).toBeTruthy()
  })

  it('is deleted only after the deletion is confirmed', async () => {
    discard.mockImplementation(() => {
      rows = []
      return Promise.resolve(true)
    })

    const banner = await renderBanner()
    await userEvent.click(within(banner).getByRole('button', { name: 'Delete' }))

    // Asked first: this is the only copy of a message that was never sent.
    const sheet = await screen.findByRole('dialog', { name: 'Delete this message?' })
    expect(sheet.textContent).toContain('the only copy')
    expect(discard).not.toHaveBeenCalled()

    await userEvent.click(within(sheet).getByRole('button', { name: 'Delete Message' }))

    expect(discard).toHaveBeenCalledWith(7)
    await waitFor(() => {
      expect(screen.queryByRole('alert')).toBeNull()
    })
  })

  it('is kept when the confirmation is cancelled', async () => {
    const banner = await renderBanner()
    await userEvent.click(within(banner).getByRole('button', { name: 'Delete' }))

    const sheet = await screen.findByRole('dialog', { name: 'Delete this message?' })
    await userEvent.click(within(sheet).getByRole('button', { name: 'Cancel' }))

    expect(discard).not.toHaveBeenCalled()
    // Awaited: while the sheet animates closed it still hides the page behind it from the
    // accessibility tree, so the banner is briefly unfindable even though it never left.
    expect((await screen.findByRole('alert')).textContent).toContain('how are you')
  })

  it('says so when the core did not delete it', async () => {
    // False means it stopped being a failed message — retried from another window, say — and
    // reporting a deletion then would be untrue.
    discard.mockResolvedValue(false)

    const banner = await renderBanner()
    await userEvent.click(within(banner).getByRole('button', { name: 'Delete' }))
    const sheet = await screen.findByRole('dialog', { name: 'Delete this message?' })
    await userEvent.click(within(sheet).getByRole('button', { name: 'Delete Message' }))

    expect(await screen.findByText('The message was not deleted')).toBeTruthy()
  })
})
