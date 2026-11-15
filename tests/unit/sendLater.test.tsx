import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type * as Ipc from '@/lib/ipc'
import type { OutboxRow } from '@/lib/generated/OutboxRow'
import { ToastProvider } from '@/ui'

/**
 * Send Later from the Undo Send banner.
 *
 * H17 in the 2026-11-14 audit. "Later…" only opened a sheet, and the ten-second hold went on
 * counting underneath it. Choosing a time takes longer than ten seconds, so by the time Schedule
 * was pressed the message had usually gone — and the reschedule was refused by the core, which
 * the banner did not read, so Schedule appeared to work and silently did nothing.
 */

const heldRow: OutboxRow = {
  id: 9,
  accountId: 1,
  state: 'holding',
  subject: 'Monday plan',
  recipients: 'ada@example.test',
  sendAfter: Math.floor(Date.now() / 1000) + 10,
  attempts: 0,
  lastError: null,
}

const schedule = vi.fn<(id: number, sendAfter: number) => Promise<boolean>>()

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  runningInTauri: true,
  outboxList: () => Promise.resolve([heldRow]),
  outboxSchedule: (id: number, sendAfter: number) => schedule(id, sendAfter),
  onOutboxProgress: () => Promise.resolve(() => undefined),
}))

afterEach(() => {
  schedule.mockReset()
})

async function banner() {
  const { OutboxBanner } = await import('@/features/outbox/OutboxBanner')
  render(
    <ToastProvider>
      <OutboxBanner />
    </ToastProvider>,
  )
  return screen.findByRole('status')
}

describe('Send Later', () => {
  it('stops the hold before the sheet opens, so the message waits while a time is chosen', async () => {
    schedule.mockResolvedValue(true)
    const row = await banner()

    const before = Math.floor(Date.now() / 1000)
    await userEvent.click(within(row).getByRole('button', { name: 'Later…' }))

    await waitFor(() => {
      expect(schedule).toHaveBeenCalled()
    })
    const [id, until] = schedule.mock.calls[0] ?? [0, 0]
    expect(id).toBe(9)
    expect(until).toBeGreaterThan(before + 60 * 60)
  })

  it('says so when the message has already gone, instead of offering a time', async () => {
    schedule.mockResolvedValue(false)
    const row = await banner()

    await userEvent.click(within(row).getByRole('button', { name: 'Tomorrow' }))

    expect(await screen.findByText('Too late to send later')).toBeTruthy()
  })

  it('says why it will not schedule a moment that has passed', async () => {
    // L22: the refusal was passed as a prop the field does not have, so the field went red and
    // said nothing.
    const { SendLaterSheet } = await import('@/features/outbox/SendLaterSheet')
    const onChoose = vi.fn()
    render(<SendLaterSheet open onOpenChange={() => undefined} onChoose={onChoose} />)

    const date = screen.getByLabelText('Date')
    await userEvent.clear(date)
    await userEvent.type(date, '2020-01-01')
    await userEvent.click(screen.getByRole('button', { name: 'Schedule' }))

    expect(await screen.findByText('That moment has already passed.')).toBeTruthy()
    expect(onChoose).not.toHaveBeenCalled()
  })

  it('does not open the sheet for a message that has already gone', async () => {
    schedule.mockResolvedValue(false)
    const row = await banner()

    await userEvent.click(within(row).getByRole('button', { name: 'Later…' }))

    expect(await screen.findByText('Too late to send later')).toBeTruthy()
    expect(screen.queryByRole('dialog')).toBeNull()
  })
})
