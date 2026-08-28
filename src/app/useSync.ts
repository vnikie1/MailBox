import { createContext, useContext, useEffect, useMemo, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'

import {
  onAccountError,
  onAccountsChanged,
  onMailboxesChanged,
  onMessagesAdded,
  onSyncProgress,
  runningInTauri,
  syncAll,
  syncWatch,
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
  /**
   * False when the machine has no network at all.
   *
   * Worth separating from a per-account error because the cause and the remedy are different:
   * one account failing to authenticate is that account's problem, but a closed laptop lid is
   * every account's, and showing four identical failures for it would be noise. It is also the
   * one failure where the honest thing to say is "your mail is still here, it just is not
   * current" rather than anything that sounds like the app broke.
   */
  online: boolean
}

export function useSync(): SyncState {
  const client = useQueryClient()
  const [errors, setErrors] = useState<Map<number, SyncAccountError>>(new Map())
  const [busy, setBusy] = useState(false)

  // navigator.onLine is a weak signal — it means "there is a network interface", not "the mail
  // server is reachable", so a captive portal still reports true. It is kept anyway because the
  // case it *does* catch is the common one (no wifi, lid closed, flight mode) and it catches it
  // instantly, where waiting for an IMAP timeout takes half a minute. A server that is
  // unreachable for any subtler reason still arrives as a per-account error.
  const [online, setOnline] = useState(() =>
    typeof navigator === 'undefined' ? true : navigator.onLine,
  )

  useEffect(() => {
    const goOnline = () => {
      setOnline(true)

      // Coming back is the one moment a sync is obviously wanted and cannot be prompted by the
      // server: IDLE connections died with the network, so nothing is going to tell us what
      // arrived while we were away unless we ask.
      if (runningInTauri) {
        void syncAll()
        void syncWatch()
      }
    }

    const goOffline = () => {
      setOnline(false)
    }

    window.addEventListener('online', goOnline)
    window.addEventListener('offline', goOffline)

    return () => {
      window.removeEventListener('online', goOnline)
      window.removeEventListener('offline', goOffline)
    }
  }, [])

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

    // Adding or removing an account changes who should be watched. The core reconciles
    // rather than starting blindly, so calling this more often than necessary is safe.
    track(
      onAccountsChanged(() => {
        void syncWatch()
      }),
    )

    // Sync on launch. Mail that arrived while the app was closed is the first thing anyone
    // opens a mail client to see.
    //
    // Then hand over to IDLE: after this one pass the server tells us when something changes,
    // so there is no timer here and never should be (standing rule 14).
    if (runningInTauri) {
      void syncAll()
      void syncWatch()
    }

    return () => {
      cancelled = true
      window.clearTimeout(timer)
      unlisteners.forEach((off) => {
        off()
      })
    }
  }, [client])

  return useMemo(() => ({ errors, busy, online }), [errors, busy, online])
}

/**
 * The sync state, for anything that needs to report on it.
 *
 * A context rather than props because the one consumer — the strip at the foot of the sidebar
 * — sits three levels below the hook, and threading four fields through Shell and AppShell
 * would make both of them carry state they have no use for. The default is the healthy case,
 * so a component rendered outside the provider (the gallery does this) shows nothing rather
 * than claiming the app is offline.
 */
export const SyncContext = createContext<SyncState>({
  errors: new Map(),
  busy: false,
  online: true,
})

export function useSyncState(): SyncState {
  return useContext(SyncContext)
}
