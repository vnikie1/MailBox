import { useEffect, useState } from 'react'
import { RefreshCw } from 'lucide-react'

import type { UpdateProblem } from '@/lib/generated/UpdateProblem'
import type { UpdateStatus } from '@/lib/generated/UpdateStatus'
import { updateCheck, updateInstall } from '@/lib/ipc'
import { Button } from '@/ui'

import { Field, Form } from './SettingsForm'
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
/**
 * What to say when an update will not install.
 *
 * ## Why this is not one message
 *
 * A refused signature and a failed download are the same event from the user's side -- the
 * button did nothing -- and completely different events in fact. One means the file was damaged
 * or is not the file we published; the other means the network went away halfway through. Saying
 * "something went wrong" for both wastes the one moment when the distinction is cheap to draw.
 *
 * The signature case deliberately says the app was not changed. Somebody who has just been told
 * an update was rejected as unsigned has every reason to wonder what it did before being caught,
 * and the answer -- nothing, it is verified before it is run -- is the reassuring part.
 */
function describeInstallFailure(error: unknown): string {
  const message =
    typeof error === 'object' && error !== null && 'message' in error
      ? String(error.message)
      : String(error)

  if (/signature|verif/i.test(message)) {
    return (
      'That update was refused: its signature did not match. Halcyon has not been changed. ' +
      'This usually means the download was damaged; it can also mean the file was not the one ' +
      'we published.'
    )
  }

  return 'The update could not be installed. Halcyon has not been changed, so it is safe to try again.'
}

/**
 * What to say when a check produced no answer.
 *
 * ## Why this is five sentences and not one
 *
 * It was one: *"Could not reach the update server. This is usually just being offline."* — said
 * for every failure there is. On this machine that sentence was **wrong 69 times in a row**.
 * The GitHub repository is public and reachable; it simply has no releases published, so
 * `releases/latest/download/latest.json` answers 404. The server was never unreachable, and a
 * mail client that tells you to check your connection when your connection is fine sends you
 * off to restart a router for nothing.
 *
 * The core now says which kind of failure it was (`UpdateProblem`), so each of these describes
 * something that is actually true. None of them mentions a version number or a URL: this line
 * sits in Settings, not in a bug report, and the underlying message is in the log for that.
 */
function describeProblem(problem: UpdateProblem | null): string {
  switch (problem) {
    case 'noRelease':
      return 'The update server answered, and has no release published to compare against. Nothing is wrong with your connection.'
    case 'malformed':
      return 'The update server answered with something Halcyon could not read. Nothing was downloaded.'
    case 'unsupported':
      return 'There is a newer version, but nothing in it for this kind of PC.'
    case 'unavailable':
      return 'This copy of Halcyon has no update server configured, so it cannot check.'
    case 'unreachable':
    case null:
    default:
      return 'Could not reach the update server. This is usually just being offline.'
  }
}

export function UpdateSettings() {
  const [status, setStatus] = useState<UpdateStatus | null>(null)
  const [checking, setChecking] = useState(false)
  const [installing, setInstalling] = useState(false)
  const [installError, setInstallError] = useState<string | null>(null)

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
      <h2 className={styles.heading}>Updates</h2>

      <Form>
        {status !== null && !status.supported ? (
          <Field label="Updates">
            <p className={styles.hint}>
              This copy came from the Microsoft Store, which installs updates for you.
            </p>
          </Field>
        ) : (
          <Field
            label="Updates"
            hint="Checking asks GitHub for a small file listing the latest version. It sends nothing about you or your mail. Updates are signed, and one that is not is refused."
          >
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
                    setInstallError(null)
                    void updateInstall().catch((error: unknown) => {
                      setInstalling(false)
                      setInstallError(describeInstallFailure(error))
                    })
                  }}
                >
                  {installing ? 'Installing…' : `Install ${status.version ?? ''}`}
                </Button>
              )}
            </div>

            {/* Reserved from the start rather than appearing when there is something to say:
                this line sits directly under the buttons, and a line that grows into place
                moves them. Standing rule 6. */}
            <p className={styles.status} aria-live="polite">
              {checking
                ? 'Asking whether there is a newer version…'
                : installing
                  ? 'Halcyon will restart when this finishes. Your accounts, mail and settings stay where they are.'
                  : status === null
                    ? ''
                    : status.error !== null
                      ? // Being offline is not a fault, and neither is a server with nothing
                        // published on it — but they are different, and this said the first
                        // for both. See `describeProblem`.
                        describeProblem(status.problem)
                      : status.available
                        ? `Version ${status.version ?? 'unknown'} is available.`
                        : 'Halcyon is up to date.'}
            </p>

            {installError !== null && (
              <p className={styles.hint} role="alert">
                {installError}
              </p>
            )}

            {status?.available === true && status.notes !== null && (
              <p className={styles.hint}>{status.notes}</p>
            )}
          </Field>
        )}
      </Form>
    </section>
  )
}
