import { useEffect, useRef, useState } from 'react'
import { Check, Lock, MailPlus, Search } from 'lucide-react'

import { Button, Sheet } from '@/ui'

import { AccountAssistant } from './AccountAssistant'
import styles from './FirstRun.module.css'

/**
 * Welcome, add an account, done. docs/06 Phase 11.
 *
 * ## Why this wraps the assistant instead of extending it
 *
 * The assistant is a reducer with five steps and a guard on each transition. Adding two more
 * that carry no data and ask no questions would put them in every `canContinue`, every `back`,
 * and every title — for two screens that are prose and a button. The assistant stays the thing
 * that adds an account; this is the thing that happens around it the first time.
 *
 * ## Why there is a welcome screen at all
 *
 * The gate is *first run to reading mail in under three minutes*, and the risk to that is not
 * the number of screens. It is opening an unexplained credential form as the first thing a
 * person sees. Mail opens its assistant on a first launch too, but macOS users know what Mail
 * is; nobody has heard of this. Three sentences buy the trust to type a password into it.
 *
 * The welcome screen is also the honest place to say what the app does not do, because a mail
 * client asking for a password is exactly when somebody should want to know.
 */

type Stage = 'welcome' | 'account' | 'done'

export interface FirstRunProps {
  /** True once the core has answered and there are no accounts at all. */
  firstRun: boolean
}

export function FirstRun({ firstRun }: FirstRunProps) {
  const [stage, setStage] = useState<Stage>('welcome')
  const [dismissed, setDismissed] = useState(false)

  // How long the whole thing took, for the docs/06 gate. Started when the welcome screen is
  // first shown rather than at process start: the part measured here is the part this component
  // is responsible for, and mixing it with cold start would hide which half was slow.
  const started = useRef<number | null>(null)
  const [seconds, setSeconds] = useState<number | null>(null)

  useEffect(() => {
    if (firstRun && started.current === null) started.current = performance.now()
  }, [firstRun])

  // An account exists now, so the assistant succeeded. `firstRun` going false is the only
  // signal that is true whichever way the account arrived — the assistant's own callback would
  // miss an account added from a second window.
  useEffect(() => {
    if (stage === 'account' && !firstRun) {
      if (started.current !== null) {
        setSeconds(Math.round((performance.now() - started.current) / 1000))
      }
      setStage('done')
    }
  }, [firstRun, stage])

  if (dismissed) return null

  // Nothing to do: the app already has accounts and this is not a first launch.
  if (!firstRun && stage === 'welcome') return null

  if (stage === 'account') {
    return (
      <AccountAssistant
        open
        firstRun
        onOpenChange={() => {
          // Deliberately ignored. The first-run assistant has no cancel — dismissing it would
          // leave somebody in an app with nothing in it and no visible way forward.
        }}
      />
    )
  }

  if (stage === 'done') {
    return (
      <Sheet
        open
        onOpenChange={() => {
          setDismissed(true)
        }}
        title="You're set up"
        className={styles.sheet}
        footer={
          <div className={styles.actions}>
            <Button
              variant="filled"
              onClick={() => {
                setDismissed(true)
              }}
            >
              Start reading
            </Button>
          </div>
        }
      >
        <div className={styles.body}>
          <p className={styles.lead}>
            Your mail is downloading now. It will keep arriving in the background — the first sync
            of a large mailbox takes a few minutes, and you can read what has arrived while the rest
            catches up.
          </p>

          <ul className={styles.points}>
            <li>
              <Search className={styles.icon} aria-hidden />
              Press <kbd className={styles.key}>Ctrl</kbd> + <kbd className={styles.key}>F</kbd> to
              search, or <kbd className={styles.key}>F1</kbd> for every shortcut.
            </li>
            <li>
              <MailPlus className={styles.icon} aria-hidden />
              Add more accounts, set a signature and change how it looks in Settings —{' '}
              <kbd className={styles.key}>Ctrl</kbd> + <kbd className={styles.key}>,</kbd>.
            </li>
          </ul>

          {seconds !== null && (
            // Shown because docs/06 asks for under three minutes and a number nobody can see is
            // a number nobody checks. It is the honest figure: from this screen appearing to the
            // account being saved, on this machine, this time.
            <p className={styles.timing}>Set up in {formatDuration(seconds)}.</p>
          )}
        </div>
      </Sheet>
    )
  }

  return (
    <Sheet
      open
      onOpenChange={() => {
        // No dismissal. There is nothing behind this sheet yet.
      }}
      title="Welcome to Halcyon"
      className={styles.sheet}
      footer={
        <div className={styles.actions}>
          <Button
            variant="filled"
            onClick={() => {
              setStage('account')
            }}
          >
            Add your account
          </Button>
        </div>
      }
    >
      <div className={styles.body}>
        <p className={styles.lead}>
          Halcyon is a mail client that keeps your mail on this computer. It works with the account
          you already have — Gmail, Outlook, iCloud, or any IMAP server.
        </p>

        <ul className={styles.points}>
          <li>
            <Lock className={styles.icon} aria-hidden />
            Your password goes to Windows Credential Manager, not to us. Nothing about you is sent
            anywhere, ever — there is no analytics and no account to make.
          </li>
          <li>
            <Check className={styles.icon} aria-hidden />
            Adding an account takes a minute. Your mail then downloads in the background and stays
            readable offline.
          </li>
        </ul>
      </div>
    </Sheet>
  )
}

/** `2 minutes 5 seconds`, or `45 seconds`. */
function formatDuration(seconds: number): string {
  if (seconds < 60) return `${String(seconds)} seconds`

  const minutes = Math.floor(seconds / 60)
  const rest = seconds % 60
  const minutePart = minutes === 1 ? '1 minute' : `${String(minutes)} minutes`

  return rest === 0 ? minutePart : `${minutePart} ${String(rest)} seconds`
}
