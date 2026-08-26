import { useEffect, useRef } from 'react'

import { bodiesEnsure } from '@/lib/ipc'
import type { MessageRow } from '@/lib/generated/MessageRow'

/**
 * Downloads the selected message's body, and the next three rows' bodies ahead of time.
 *
 * docs/06 Phase 5 §3 — *lazy body fetch on selection + prefetch of the next 3 rows*.
 *
 * Three, not thirty. The point of prefetching is that arrowing down a list feels instant;
 * fetching further ahead than the user can plausibly move spends their bandwidth and the
 * server's connection budget on messages they will never open. Three covers a key held down
 * for a second, which is as far as anyone reads without stopping.
 *
 * Every id already downloaded is free — the core checks `body_state` before opening a
 * connection — so this deliberately does not track what it has fetched. Duplicating that
 * bookkeeping in the UI is how the two copies drift apart.
 */
const PREFETCH_AHEAD = 3

export function useBodyPrefetch(selectedId: number | undefined, visibleRows: MessageRow[]): void {
  // The last set requested, so that re-renders from unrelated state — a flag change, a
  // window resize — do not re-issue the same fetch.
  const lastRequested = useRef('')

  useEffect(() => {
    if (selectedId === undefined) return

    const index = visibleRows.findIndex((row) => row.id === selectedId)

    const ahead =
      index >= 0
        ? visibleRows.slice(index + 1, index + 1 + PREFETCH_AHEAD).map((row) => row.id)
        : []

    // The selected message first: it is the one someone is waiting for, and the core walks
    // this list in order.
    const wanted = [selectedId, ...ahead]

    const key = wanted.join(',')
    if (key === lastRequested.current) return
    lastRequested.current = key

    const accountId = visibleRows[index >= 0 ? index : 0]?.accountId
    if (accountId === undefined) return

    void bodiesEnsure(accountId, wanted)
  }, [selectedId, visibleRows])
}
