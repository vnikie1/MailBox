import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { Rendered } from '@/lib/generated/Rendered'

/**
 * What the reader tells the user about images kept on the sender's server.
 *
 * ## Why the wording is worth pinning
 *
 * This is the app's one genuinely privacy-relevant default. Most marketing email keeps its
 * images on the sender's own server, so showing them tells that sender the message was opened,
 * roughly when, and from which IP address.
 *
 * The banners used to describe the *mechanism* — "Remote images loaded, which tells the sender
 * you opened this" — which is accurate and leaves the reader nothing to decide with. They now
 * name who learned what, and carry both answers: the one for this message and the durable one.
 *
 * The default is to **show** images. Blocking by default is what macOS Mail does and it makes
 * ordinary mail look broken; the choice is put in front of the user rather than made for them.
 * The test at the bottom is what stops that being quietly reversed.
 *
 * These are asserted against the component rather than end to end because the browser build's
 * seeded bodies never carry remote images, so no banner can appear there at all.
 */

const enabled = vi.fn<() => Promise<boolean>>()
const setEnabled = vi.fn<(value: boolean) => Promise<void>>()
const useBody = vi.fn()

vi.mock('@/lib/ipc', () => ({
  remoteImagesEnabled: () => enabled(),
  setRemoteImagesEnabled: (value: boolean) => setEnabled(value),
}))

vi.mock('@/app/queries', () => ({
  useMessageBody: () => useBody() as unknown,
}))

// The frame renders the body in a sandboxed iframe, which jsdom cannot lay out and which is
// not what any of this is about.
vi.mock('@/features/reader/MessageFrame', () => ({
  MessageFrame: () => null,
}))

function rendered(over: Partial<Rendered>): Rendered {
  return {
    html: '<p>hello</p>',
    blockedRemote: 0,
    inlined: 0,
    loadedRemote: 0,
    failedRemote: 0,
    fromPlainText: false,
    ...over,
  }
}

async function show(body: Rendered, imagesOn: boolean) {
  enabled.mockResolvedValue(imagesOn)
  setEnabled.mockResolvedValue(undefined)
  useBody.mockReturnValue({ data: body, isPending: false, isError: false })

  const { MessageBody } = await import('@/features/reader/MessageBody')
  render(<MessageBody messageId={1} />)

  // The preference is read asynchronously on mount; nothing renders correctly before it lands.
  await waitFor(() => {
    expect(screen.getByRole('status')).toBeTruthy()
  })
}

afterEach(() => {
  vi.clearAllMocks()
  vi.resetModules()
})

describe('when images were shown', () => {
  it('names what the sender learned, rather than what a remote image is', async () => {
    await show(rendered({ loadedRemote: 3 }), true)

    const banner = screen.getByRole('status').textContent

    expect(banner).toContain('sender')
    expect(banner).toContain('opened')
    // The word that made the old copy unreadable: it names the mechanism, not the stake.
    expect(banner.toLowerCase()).not.toContain('remote image')
  })

  it('offers the durable answer as well as the one for this message', async () => {
    // Someone who reads that sentence and dislikes it wants to change the rule, not to press
    // a button on every message for the rest of their life.
    await show(rendered({ loadedRemote: 1 }), true)

    expect(screen.getByRole('button', { name: 'Hide Images' })).toBeTruthy()

    await userEvent.click(screen.getByRole('button', { name: 'Always Ask First' }))
    expect(setEnabled).toHaveBeenCalledWith(false)
  })
})

describe('when images were withheld', () => {
  it('says what showing them would cost, before anything is fetched', async () => {
    await show(rendered({ blockedRemote: 2 }), false)

    const banner = screen.getByRole('status').textContent

    expect(banner).toContain('sender')
    expect(screen.getByRole('button', { name: 'Show Images' })).toBeTruthy()
  })

  it('offers to change the setting only when the setting is what is holding them back', async () => {
    // Blocked by hand a moment ago is a different situation from the setting being off, and
    // offering to change the rule there answers a question the user did not ask.
    await show(rendered({ blockedRemote: 2 }), true)

    expect(screen.queryByRole('button', { name: 'Always Show' })).toBeNull()
  })
})

describe('when the fetch failed', () => {
  it('claims nothing about what the sender learned', async () => {
    // The request left this machine and went unanswered. Whether it arrived first is not
    // knowable from here, and "nothing was shared" would be a comfort the app cannot back.
    await show(rendered({ failedRemote: 4 }), true)

    const banner = screen.getByRole('status').textContent

    expect(banner).toContain('could not be loaded')
    expect(banner).not.toContain('opened')
    // Nothing to decide, so nothing to press.
    expect(screen.queryByRole('button')).toBeNull()
  })
})
