import { useEffect, useState } from 'react'
import { RefreshCw } from 'lucide-react'

import type { UpdateStatus } from '@/lib/generated/UpdateStatus'
import { updateCheck, updateInstall } from '@/lib/ipc'
import { Button } from '@/ui'

import styles from './settings.module.css'

/**
 * Settings → General → Updates. docs/06 Phase 11, docs/07 §2.3.
 *
 * ## Why there is a button rather than a schedule
 *
 * Standing rule 16 is no telemetry, and an update check is the only outbound request this app
 * makes that is not mail. It is a GET for a static file and it carries nothing about the user —
 * but a timer that fires it unasked is still a background connection somebody did not agree to,
 * and this app's whole claim is that it does not make those.
 *
 * The trade is real and goes the other way too: a mail client that never mentions a security
 * fix is not safe either. So the check is here, visible, one click, and the version on offer is
 * shown before anything is downloaded.
 *
 * ## Why the notes are plain text
 *
 * Release notes come from a JSON file on a server. They are rendered as text and never as
 * markup, for the same reason a message body goes through a sanitiser: it is remote content,
 * and the fact that we published it does not make it safe to inject.
 */
export function UpdateSettings() {
  const [status, setStatus] = useState<UpdateStatus | null>(null)
  const [checking, setChecking] = useState(false)
  const [installing, setInstalling] = useState(false)

  // Asked once when the pane opens, because somebody who has opened Settings is not going to be
  // interrupted by the answer. Nothing checks on a timer.
  useEffect(() => {
    let live = true
    setChecking(true)

    void updateCheck().then((value) => {
      if (!live) return
      setStatus(value)
      setChecking(false)
    })

    return () => {
      live = false
    }
  }, [])

  const check = () => {
    setChecking(true)
    void updateCheck().then((value) => {
      setStatus(value)
      setChecking(false)
    })
  }

  return (
    <section className={styles.section}>
      <h3 className={styles.heading}>Updates</h3>

      {status !== null && !status.supported ? (
        <p className={styles.hint}>
          This copy came from the Microsoft Store, which installs updates for you.
        </p>
      ) : (
        <>
          <div className={styles.row}>
            <Button variant="bordered" disabled={checking || installing} onClick={check}>
              <RefreshCw size={16} aria-hidden />
              {checking ? 'Checking…' : 'Check for updates'}
            </Button>

            {status?.available === true && (
              <Button
                variant="filled"
                disabled={installing}
                onClick={() => {
                  setInstalling(true)
                  void updateInstall().catch(() => {
                    setInstalling(false)
                  })
                }}
              >
                {installing ? 'Installing…' : `Install ${status.version ?? ''}`}
              </Button>
            )}
          </div>

          <p className={styles.hint} aria-live="polite">
            {checking
              ? 'Asking whether there is a newer version…'
              : status === null
                ? ''
                : status.error !== null
                  ? // Being offline is not a fault. Saying so plainly beats a red banner that
                    // teaches people to ignore the one that matters.
                    'Could not reach the update server. This is usually just being offline.'
                  : status.available
                    ? `Version ${status.version ?? 'unknown'} is available.`
                    : 'Halcyon is up to date.'}
          </p>

          {status?.available === true && status.notes !== null && (
            <p className={styles.hint}>{status.notes}</p>
          )}

          <p className={styles.hint}>
            Checking asks GitHub for a small file listing the latest version. It sends nothing about
            you or your mail. Updates are signed, and one that is not is refused.
          </p>

          {installing && (
            <p className={styles.hint}>
              Halcyon will restart when this finishes. Your accounts, mail and settings stay where
              they are.
            </p>
          )}
        </>
      )}
    </section>
  )
}
