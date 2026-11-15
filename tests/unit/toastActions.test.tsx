import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'

import type * as Ipc from '@/lib/ipc'
import { ToastProvider } from '@/ui'
import type { FlagPatch } from '@/lib/generated/FlagPatch'

/**
 * A toast's Mark as Read marks the message read — it never flips it.
 *
 * M33 in the 2026-11-14 audit. The button called the read *toggle*, on the reasoning that a toast
 * is only ever for mail that has just arrived and so is unread. A toast outlives that: it sits in
 * the Action Center for hours, the message is read on the phone in the meantime, and pressing
 * Mark as Read then marked it **unread**, here and on every device.
 */

const toggled: number[][] = []
const set: { ids: number[]; patch: FlagPatch }[] = []
let press: ((event: { action: string; messageId: number }) => void) | null = null
/** What the core held for the window, and whether the listener was in place when it was asked. */
let held: { action: string; messageId: number }[] = []
let takenAfterListening: boolean | null = null

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  onNotificationAction: (handler: (event: { action: string; messageId: number }) => void) => {
    press = handler
    return Promise.resolve(() => undefined)
  },
  onNotificationShow: () => Promise.resolve(() => undefined),
  handoffTake: (event: string) => {
    if (event !== 'notification:action') return Promise.resolve([])
    takenAfterListening = press !== null
    const taken = held
    held = []
    return Promise.resolve(taken)
  },
  msgToggleRead: (ids: number[]): Promise<number> => {
    toggled.push(ids)
    return Promise.resolve(ids.length)
  },
  msgSetFlags: (ids: number[], patch: FlagPatch): Promise<number> => {
    set.push({ ids, patch })
    return Promise.resolve(ids.length)
  },
}))

beforeEach(() => {
  toggled.length = 0
  set.length = 0
  press = null
  held = []
  takenAfterListening = null
})

function mount(useSystemEvents: () => void) {
  const client = new QueryClient()
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>
      <ToastProvider>{children}</ToastProvider>
    </QueryClientProvider>
  )

  renderHook(
    () => {
      useSystemEvents()
    },
    { wrapper },
  )
}

describe('a toast pressed before the window could hear it', () => {
  it('is acted on once the listener is in place', async () => {
    // M30 and H19: the toast that brought a closed main window back was pressed before the new
    // page had subscribed, and the core held it. Taken before the listener existed, a press made
    // in between would fall through the gap.
    held = [{ action: 'read', messageId: 7 }]
    const { useSystemEvents } = await import('@/app/useSystemEvents')
    mount(useSystemEvents)

    await waitFor(() => {
      expect(set).toEqual([{ ids: [7], patch: { seen: true, flagged: null } }])
    })
    expect(takenAfterListening).toBe(true)
  })
})

describe('the toast’s Mark as Read', () => {
  it('sets the message read rather than toggling it', async () => {
    const { useSystemEvents } = await import('@/app/useSystemEvents')
    const client = new QueryClient()
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>
        <ToastProvider>{children}</ToastProvider>
      </QueryClientProvider>
    )

    renderHook(
      () => {
        useSystemEvents()
      },
      { wrapper },
    )

    await waitFor(() => {
      expect(press).not.toBeNull()
    })
    press?.({ action: 'read', messageId: 42 })

    await waitFor(() => {
      expect(set).toEqual([{ ids: [42], patch: { seen: true, flagged: null } }])
    })
    expect(toggled).toEqual([])
  })
})
