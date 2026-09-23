import { QueryClient } from '@tanstack/react-query'
import { describe, expect, it } from 'vitest'

import type { Rendered } from '@/lib/generated/Rendered'

/**
 * A rendered body is refetched only while it is still empty.
 *
 * Opening a message marks it read 700ms later, which emits `messages:updated`, which
 * invalidates that message's body — so every message opened was rendered twice. Measured from
 * the install this was found on: **191 of 402 renders were the same message repeated within ten
 * seconds**, at a p25 gap of 0.696 seconds against a `DWELL_MS` of 700. That was 146 MB of the
 * 275 MB ever rendered, and rendering a body is the most expensive thing the core does — it
 * re-reads the `.eml`, re-parses the MIME tree, re-runs html5ever twice and re-reads every
 * cached remote image back into memory as base64.
 *
 * The invalidation still has to happen: a message is selected before its body has downloaded,
 * so the reader shows an empty frame and needs telling when the content lands. The predicate
 * keeps exactly that case.
 *
 * Tested against a real `QueryClient` rather than a mock, because what is being asserted is
 * TanStack's own behaviour — that a predicate can veto an invalidation, and that `staleTime:
 * Infinity` does *not* protect an active query from one. Both are the reason this bug existed.
 */

const KEY = (id: number, loadRemote: boolean) => ['messageBody', id, loadRemote] as const

function body(html: string): Rendered {
  return {
    html,
    blockedRemote: 0,
    inlined: 0,
    loadedRemote: 0,
    failedRemote: 0,
    fromPlainText: false,
    css: '',
  }
}

/** What `useMailEvents` does when the core says a message changed. */
function invalidateBodies(client: QueryClient, ids: number[]): void {
  for (const id of ids) {
    void client.invalidateQueries({
      queryKey: ['messageBody', id],
      predicate: (query) => {
        const cached = query.state.data as Rendered | undefined
        return cached === undefined || cached.html === ''
      },
    })
  }
}

function isStale(client: QueryClient, id: number, loadRemote: boolean): boolean {
  return (
    client
      .getQueryCache()
      .find({ queryKey: KEY(id, loadRemote) })
      ?.isStale() ?? false
  )
}

describe('invalidating a rendered body', () => {
  it('leaves a body that has already arrived alone', () => {
    const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } })
    client.setQueryData(KEY(1, false), body('<p>the message</p>'))

    invalidateBodies(client, [1])

    expect(
      isStale(client, 1, false),
      'a body that has arrived cannot change — body_state is written full in the same statement as body_html',
    ).toBe(false)
  })

  it('still refetches one that has not arrived', () => {
    // The case the invalidation exists for: selected before the body downloaded, so the
    // reader is showing an empty frame and has to be told when the content lands.
    const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } })
    client.setQueryData(KEY(2, false), body(''))

    invalidateBodies(client, [2])

    expect(isStale(client, 2, false)).toBe(true)
  })

  it('covers both the images-on and images-off copies of the same message', () => {
    // The key carries `loadRemote`, so one message has two entries. Naming only the id is a
    // prefix match over both, and the predicate has to judge them separately.
    const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } })
    client.setQueryData(KEY(3, false), body(''))
    client.setQueryData(KEY(3, true), body('<p>with images</p>'))

    invalidateBodies(client, [3])

    expect(isStale(client, 3, false), 'the empty one is still waiting').toBe(true)
    expect(isStale(client, 3, true), 'the loaded one is final').toBe(false)
  })

  it('does not touch a different message', () => {
    const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } })
    client.setQueryData(KEY(4, false), body(''))
    client.setQueryData(KEY(5, false), body(''))

    invalidateBodies(client, [4])

    expect(isStale(client, 4, false)).toBe(true)
    expect(isStale(client, 5, false), 'only the ids the core named').toBe(false)
  })

  it('shows why staleTime alone was not enough', () => {
    // The reason the bug survived review: `useMessageBody` sets `staleTime: Infinity`, which
    // reads like "never refetch this". It is not — an explicit invalidation overrides it, and
    // that is the whole mechanism. Without the predicate, the loaded body goes stale.
    const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } })
    client.setQueryData(KEY(6, false), body('<p>arrived</p>'))

    void client.invalidateQueries({ queryKey: ['messageBody', 6] })

    expect(
      isStale(client, 6, false),
      'staleTime: Infinity does not survive an invalidation — this is what the predicate is for',
    ).toBe(true)
  })
})
