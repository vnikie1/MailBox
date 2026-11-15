import { useState } from 'react'
import { AlertTriangle, CloudOff, RefreshCw } from 'lucide-react'

import type { SyncAccountError } from '@/lib/ipc'
import { useAccountsDetail } from '@/features/accounts/queries'
import { accountReauth, codeFor, reasonFor, settingsOpen, syncAll } from '@/lib/ipc'
import { useToast } from '@/ui'

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
  // Which accounts sign in with a password: for those, a refusal is fixed by a new password in
  // Settings, not by a browser sign-in, which the core refuses for them (H16).
  const { data: details = [] } = useAccountsDetail()

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

  if (failed.length > 0) {
    const nameOf = (accountId: number) => accountNames.get(accountId) ?? 'An account'
    const authKindOf = (accountId: number) =>
      details.find((account) => account.id === accountId)?.authKind

    // Asking again fixes these, and one Retry asks for every account at once.
    const retryable = failed.some(([, error]) => !error.needsReauth)

    // One account: its name, the core's words, and the way out, as it always was.
    const only = failed.length === 1 ? failed[0] : undefined
    if (only !== undefined) {
      const [accountId, error] = only

      return (
        <div className={styles.strip} data-tone="error" role="status" aria-live="polite">
          <AlertTriangle className={styles.glyph} aria-hidden="true" strokeWidth={1.5} />
          <div className={styles.text}>
            <p className={styles.title}>{nameOf(accountId)}</p>
            {/* The core's own words. Rewriting them here would mean this file has to know every
                failure the sync engine can have, and would drift the moment it gained one. */}
            <p className={styles.detail}>{error.message}</p>
          </div>
          {error.needsReauth ? (
            <SignInAgain accountId={accountId} authKind={authKindOf(accountId)} />
          ) : (
            <RetryButton />
          )}
        </div>
      )
    }

    // Several. Each is named, with its own way out where it needs one.
    //
    // This showed the count, the first account's message and the first account's button, and
    // nothing of the rest: a second account whose sign-in had been refused could not be signed in
    // again from here, and nothing said which account it was (M42, 2026-11-14 audit). The core's
    // words are given once when every account has the same ones, which is the usual case, and
    // under each name when they differ.
    const messages = new Set(failed.map(([, error]) => error.message))
    const shared = messages.size === 1 ? failed[0]?.[1].message : undefined

    return (
      <div className={styles.strip} data-tone="error" role="status" aria-live="polite">
        <AlertTriangle className={styles.glyph} aria-hidden="true" strokeWidth={1.5} />
        <div className={styles.text}>
          <p className={styles.title}>{`${String(failed.length)} accounts can’t connect`}</p>
          {shared !== undefined && <p className={styles.detail}>{shared}</p>}

          <ul className={styles.accounts}>
            {failed.map(([accountId, error]) => (
              <li key={accountId} className={styles.account}>
                <div className={styles.text}>
                  <p className={styles.name}>{nameOf(accountId)}</p>
                  {shared === undefined && <p className={styles.detail}>{error.message}</p>}
                </div>
                {error.needsReauth && (
                  <SignInAgain accountId={accountId} authKind={authKindOf(accountId)} />
                )}
              </li>
            ))}
          </ul>
        </div>
        {retryable && <RetryButton />}
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

/**
 * Retry. The common causes -- a dropped VPN, a server that was briefly down, a laptop that just
 * woke -- are all fixed by asking again.
 */
function RetryButton() {
  return (
    <button
      type="button"
      className={styles.retry}
      onClick={() => {
        void syncAll()
      }}
    >
      Retry
    </button>
  )
}

/**
 * The way back in for an account whose credential the server refused.
 *
 * A way out, because a status with no action is a dead end — the gate's words. The core sets
 * `needsReauth` for a credential the server refused, and asking again cannot fix one of those:
 * the message says "Signing in again will fix it", and Retry would re-run the sync that had just
 * failed. A password account changes its password in Settings (H16); any other signs in again in
 * the browser.
 *
 * Success needs nothing from here. The core announces the new sign-in, which clears the strip
 * (`useSync`), and `accounts:changed`, which fetches the mail. This used to call `syncAll()` as
 * well, so every sign-in synced every account twice over.
 *
 * One per account, each with its own "Signing in…", so a browser sign-in for one account does
 * not grey out the button of another.
 */
function SignInAgain({ accountId, authKind }: { accountId: number; authKind: string | undefined }) {
  // The browser sign-in takes as long as the user takes, so the button has to say it is doing
  // something or it reads as not having worked.
  const [signingIn, setSigningIn] = useState(false)
  const toast = useToast()

  if (authKind === 'password') {
    return (
      <button
        type="button"
        className={styles.retry}
        onClick={() => {
          void settingsOpen('accounts', accountId)
        }}
      >
        Change Password
      </button>
    )
  }

  return (
    <button
      type="button"
      className={styles.retry}
      disabled={signingIn}
      onClick={() => {
        setSigningIn(true)
        accountReauth(accountId)
          .catch((cause: unknown) => {
            // Walking away from the browser is not news to the person who did it, and the strip
            // still says the account cannot connect.
            if (codeFor(cause) === 'timedOut') return

            // Anything else is. This used to swallow every failure, so a sign-in refused for the
            // wrong address — or one the outgoing check threw away — left the strip exactly as it
            // was, and the only thing to do was press the button again with no idea why the last
            // press had not worked.
            toast.show({
              title: 'That sign-in did not complete',
              description: reasonFor(cause),
            })
          })
          .finally(() => {
            setSigningIn(false)
          })
      }}
    >
      {signingIn ? 'Signing in…' : 'Sign In'}
    </button>
  )
}
