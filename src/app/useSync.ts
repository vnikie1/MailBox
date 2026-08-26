import { useEffect, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'

import {
  onAccountError,
  onMailboxesChanged,
  onMessagesAdded,
  onSyncProgress,
  runningInTauri,
  syncAll,
  type SyncAccountError,
} from '@/lib/ipc'

/**
 * Subscribes to the sync engine and keeps the UI in step with it.
 *
 * Every refresh here is driven by an event from the core — standing rule 14 makes that a
 * rule rather than a default, and a sync engine is exactly the place a polling interval
 * would look reasonable and be wrong.
 *
 * Invalidation is debounced. A backfill emits `messages:added` once per 500-message batch,
 * and re-running every list query on each of those would spend the whole sync re-rendering
 * instead of showing the mail that arrived.
 */
const INVALIDATE_DEBOUNCE_MS = 400

export interface SyncState {
  /** Accounts currently reporting an error, newest message per account. */
  errors: Map<number, SyncAccountError>
  /** True while any account is mid-sync. */
  busy: boolean
}

export function useSync(): SyncState {
  const client = useQueryClient()
  const [errors, setErrors] = useState<Map<number, SyncAccountError>>(new Map())
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    let cancelled = false
    const unlisteners: (() => void)[] = []
    let timer: number | undefined

    const invalidateSoon = () => {
      window.clearTimeout(timer)
      timer = window.setTimeout(() => {
        void client.invalidateQueries({ queryKey: ['messages'] })
        void client.invalidateQueries({ queryKey: ['mailboxes'] })
      }, INVALIDATE_DEBOUNCE_MS)
    }

    const track = (promise: Promise<() => void>) => {
      void promise.then((off) => {
        if (cancelled) off()
        else unlisteners.push(off)
      })
    }

    track(
      onMessagesAdded(() => {
        invalidateSoon()
      }),
    )

    track(
      onMailboxesChanged(() => {
        void client.invalidateQueries({ queryKey: ['mailboxes'] })
        void client.invalidateQueries({ queryKey: ['accounts'] })
      }),
    )

    track(
      onSyncProgress((progress) => {
        setBusy(!progress.done)
        if (progress.done) invalidateSoon()
      }),
    )

    track(
      onAccountError((error) => {
        // Kept per account rather than as one banner: with three accounts, "something went
        // wrong" is not useful, and the retry-at time differs for each.
        setErrors((current) => {
          const next = new Map(current)
          next.set(error.accountId, error)
          return next
        })
      }),
    )

    // Sync on launch. Mail that arrived while the app was closed is the first thing anyone
    // opens a mail client to see.
    if (runningInTauri) void syncAll()

    return () => {
      cancelled = true
      window.clearTimeout(timer)
      unlisteners.forEach((off) => {
        off()
      })
    }
  }, [client])

  return { errors, busy }
}
