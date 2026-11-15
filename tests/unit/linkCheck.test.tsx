import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type * as Ipc from '@/lib/ipc'
import type { HostMismatch } from '@/lib/generated/HostMismatch'

const openExternalConfirmed = vi.fn<(mismatch: HostMismatch) => Promise<void>>(() =>
  Promise.resolve(),
)

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  openExternalConfirmed: (mismatch: HostMismatch) => openExternalConfirmed(mismatch),
}))

afterEach(() => {
  openExternalConfirmed.mockClear()
})

const mismatch: HostMismatch = {
  shown: 'bank.co.uk',
  actual: 'evil.co.uk',
  url: 'https://evil.co.uk/login?next=%2Faccount',
}

describe('the link check', () => {
  // H15 in the 2026-11-14 audit: the core held such a link back and nothing showed this, so
  // the click did nothing at all.
  it('names both sites and the whole address before anything opens', async () => {
    const { LinkCheck } = await import('@/features/reader/MessageFrame')
    render(<LinkCheck mismatch={mismatch} onClose={() => undefined} />)

    expect(screen.getByRole('dialog', { name: 'Open this link?' })).toBeTruthy()
    expect(
      screen.getByText('The message shows bank.co.uk, but this link goes to evil.co.uk.'),
    ).toBeTruthy()
    expect(screen.getByText(mismatch.url)).toBeTruthy()
    expect(openExternalConfirmed).not.toHaveBeenCalled()
  })

  it('opens the link only when asked to', async () => {
    const { LinkCheck } = await import('@/features/reader/MessageFrame')
    const onClose = vi.fn()
    render(<LinkCheck mismatch={mismatch} onClose={onClose} />)

    await userEvent.click(screen.getByRole('button', { name: 'Open Link' }))

    expect(openExternalConfirmed).toHaveBeenCalledWith(mismatch)
    expect(onClose).toHaveBeenCalled()
  })

  it('opens nothing on Cancel', async () => {
    const { LinkCheck } = await import('@/features/reader/MessageFrame')
    const onClose = vi.fn()
    render(<LinkCheck mismatch={mismatch} onClose={onClose} />)

    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }))

    expect(onClose).toHaveBeenCalled()
    expect(openExternalConfirmed).not.toHaveBeenCalled()
  })

  it('is not there without a link to ask about', async () => {
    const { LinkCheck } = await import('@/features/reader/MessageFrame')
    render(<LinkCheck mismatch={null} onClose={() => undefined} />)

    expect(screen.queryByRole('dialog')).toBeNull()
  })
})
