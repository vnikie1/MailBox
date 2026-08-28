import { AlertTriangle, CloudOff, RefreshCw } from 'lucide-react'

import type { SyncAccountError } from '@/lib/ipc'
import { syncAll } from '@/lib/ipc'

import styles from './SyncStatus.module.css'

export interface SyncStatusProps {
  errors: Map<number, SyncAccountError>
  busy: boolean
  online: boolean
  /** Account display names by id, for naming the one that failed. */
  accountNames: Map<number, string>
}

/**
 * The strip along the bottom of the sidebar. docs/06 Phase 10.
 *
 * ## Why this exists at all
 *
 * The sync engine has tracked per-account errors since Phase 5 and nothing has ever displayed
 * them. An account whose password expired went on failing every few minutes, and the entire
 * user-visible consequence was that new mail stopped arriving — no message, no icon, nothing.
 * The app looked like it was working and was not, which is the worst of the available states,
 * and it is precisely the shape of the "my mail is stale" report from this project's own
 * testing. That report turned out to have a different cause, but only by luck: had sync
 * genuinely been failing, the app would have been just as silent.
 *
 * ## Why it is quiet
 *
 * Nothing is shown when everything is fine. A permanent "Connected" line is a banner that
 * teaches you to ignore the space it occupies, so that when something does appear there you no
 * longer look at it. The strip is absent, and its presence is the signal.
 *
 * Offline outranks account errors: with no network every account fails, and four rows saying so
 * describe the same fact four times. The message also deliberately says the mail is still
 * *here* — the failure is that it is not current, and a user who reads "cannot connect" while
 * looking at a full mailbox should not be left wondering whether it is about to empty.
 */
export function SyncStatus({ errors, busy, online, accountNames }: SyncStatusProps) {
  if (!online) {
    return (
      <div className={styles.strip} data-tone="warn" role="status" aria-live="polite">
        <CloudOff className={styles.glyph} aria-hidden="true" strokeWidth={1.5} />
        <div className={styles.text}>
          <p className={styles.title}>Offline</p>
          <p className={styles.detail}>Your mail is here, but not up to date.</p>
        </div>
      </div>
    )
  }

  const failed = [...errors.entries()]
  const first = failed[0]

  if (first !== undefined) {
    const [accountId, error] = first
    const name = accountNames.get(accountId) ?? 'An account'

    return (
      <div className={styles.strip} data-tone="error" role="status" aria-live="polite">
        <AlertTriangle className={styles.glyph} aria-hidden="true" strokeWidth={1.5} />
        <div className={styles.text}>
          <p className={styles.title}>
            {failed.length > 1 ? `${String(failed.length)} accounts can’t connect` : name}
          </p>
          {/* The core's own words. Rewriting them here would mean this file has to know every
              failure the sync engine can have, and would drift the moment it gained one. */}
          <p className={styles.detail}>{error.message}</p>
        </div>

        {/* A way out, because a status with no action is a dead end — the gate's words. Retry
            rather than anything cleverer: the common causes (a dropped VPN, a server that was
            briefly down, a laptop that just woke) are all fixed by asking again. */}
        <button
          type="button"
          className={styles.retry}
          onClick={() => {
            void syncAll()
          }}
        >
          Retry
        </button>
      </div>
    )
  }

  if (busy) {
    return (
      <div className={styles.strip} data-tone="quiet" role="status" aria-live="off">
        <RefreshCw className={`${styles.glyph} ${styles.spin}`} aria-hidden="true" />
        <div className={styles.text}>
          <p className={styles.title}>Checking for mail…</p>
        </div>
      </div>
    )
  }

  // Working normally. Nothing to say, so nothing is said.
  return null
}
