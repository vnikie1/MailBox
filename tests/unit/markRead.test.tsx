import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { MessageFull } from '@/lib/generated/MessageFull'
import { useMarkRead } from '@/features/reader/useMarkRead'

/**
 * The dwell that marks an open message read. docs/01 §4.
 *
 * Found by driving the real app from the keyboard during the Phase 10 exit gate: Ctrl+U on the
 * open message appeared to do nothing. It did work — and then this hook put it back 700ms
 * later, because marking it unread returned it to the effect's dependency and started the timer
 * again. "Mark unread to deal with later" is the most common triage move there is, and it was
 * impossible on the message you were actually looking at.
 *
 * No unit test could have caught it, because the bug is a loop between a mutation and an effect
 * that re-runs on its result. These tests pin the fix instead.
 */

const mutate = vi.fn()

vi.mock('@/app/queries', () => ({
  useSetFlags: () => ({ mutate }),
}))

function message(id: number, seen: boolean): MessageFull {
  // Only the two fields the hook reads. The rest of MessageFull is irrelevant here and
  // spelling it out would make this test fail whenever an unrelated column is added.
  return { id, seen } as unknown as MessageFull
}

function Harness({ messages }: { messages: MessageFull[] }) {
  useMarkRead(messages)
  return null
}

function renderWith(messages: MessageFull[]) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })

  return render(
    <QueryClientProvider client={client}>
      <Harness messages={messages} />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  mutate.mockClear()
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('the dwell', () => {
  it('marks an unread message read after the delay', () => {
    renderWith([message(1, false)])

    expect(mutate).not.toHaveBeenCalled()

    vi.advanceTimersByTime(800)

    expect(mutate).toHaveBeenCalledTimes(1)
    expect(mutate.mock.calls[0]?.[0]).toMatchObject({ ids: [1] })
  })

  it('leaves an already-read message alone', () => {
    renderWith([message(1, true)])
    vi.advanceTimersByTime(800)

    expect(mutate).not.toHaveBeenCalled()
  })

  it('does nothing if the selection changes before the delay is up', () => {
    const view = renderWith([message(1, false)])

    vi.advanceTimersByTime(400)
    view.rerender(
      <QueryClientProvider client={new QueryClient()}>
        <Harness messages={[message(2, true)]} />
      </QueryClientProvider>,
    )
    vi.advanceTimersByTime(800)

    // Message 1 was passed over, not read. Holding the down arrow through twenty messages must
    // not mark twenty messages read and queue twenty UID STOREs.
    expect(mutate).not.toHaveBeenCalled()
  })
})

describe('marking the open message unread', () => {
  it('does not undo the user', () => {
    // The regression, in the order the real app produces it. The intermediate step matters and
    // an earlier version of this test omitted it — going straight from unread back to unread
    // leaves the effect's dependency unchanged, so nothing re-runs and the test passes with the
    // bug still present. It has to travel: unread, read, unread.
    const view = renderWith([message(1, false)])

    vi.advanceTimersByTime(800)
    expect(mutate).toHaveBeenCalledTimes(1)

    // The mutation lands and the query refetches: the message is now read.
    view.rerender(
      <QueryClientProvider client={new QueryClient()}>
        <Harness messages={[message(1, true)]} />
      </QueryClientProvider>,
    )
    vi.advanceTimersByTime(800)

    // The user presses Ctrl+U. Same message, same selection, now unread again.
    view.rerender(
      <QueryClientProvider client={new QueryClient()}>
        <Harness messages={[message(1, false)]} />
      </QueryClientProvider>,
    )
    vi.advanceTimersByTime(2000)

    // Once, not twice. A second call is the bug: it puts the message straight back to read and
    // the user cannot mark the thing in front of them unread.
    expect(mutate).toHaveBeenCalledTimes(1)
  })

  it('reads it again when the message is opened afresh', () => {
    // The other half. Leaving and coming back is reading it, not un-deciding — which is what
    // Mail does, and what stops the memory above turning into "never marks read again".
    const view = renderWith([message(1, false)])

    vi.advanceTimersByTime(800)
    expect(mutate).toHaveBeenCalledTimes(1)

    // Away to another thread, which is what clears the memory.
    view.rerender(
      <QueryClientProvider client={new QueryClient()}>
        <Harness messages={[message(9, true)]} />
      </QueryClientProvider>,
    )
    vi.advanceTimersByTime(800)

    // ...and back to the first, still unread.
    view.rerender(
      <QueryClientProvider client={new QueryClient()}>
        <Harness messages={[message(1, false)]} />
      </QueryClientProvider>,
    )
    vi.advanceTimersByTime(800)

    expect(mutate).toHaveBeenCalledTimes(2)
  })
})
