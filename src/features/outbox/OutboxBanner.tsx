import { useCallback, useEffect, useRef, useState } from 'react'
import { AlertTriangle, Clock, Undo2 } from 'lucide-react'

import type { OutboxRow } from '@/lib/generated/OutboxRow'
import {
  composeUndo,
  onOutboxProgress,
  soundSent,
  outboxDiscard,
  outboxList,
  outboxRetry,
  outboxSchedule,
  reasonFor,
  runningInTauri,
} from '@/lib/ipc'
import { Button, Sheet, useToast } from '@/ui'

import { SendLaterSheet } from './SendLaterSheet'
import styles from './OutboxBanner.module.css'

/**
 * The Undo Send and send-failure banner. docs/01 §6, docs/06 Phase 7.
 *
 * At the bottom of the main window, as Mail puts it, and it covers two moments that look alike
 * and are not:
 *
 * * **A message is held.** For the length of the undo window it genuinely has not been
 *   transmitted, so Undo is not a cancellation request — it deletes the message before any
 *   connection is opened. The countdown here is a *display* of the core's timer, never the
 *   thing that drives it: a window that was asleep, throttled or closed must not be able to
 *   change when a message goes.
 *
 * * **A message failed.** docs/06 Phase 7 — *never silently drop a message.* So this banner
 *   stays until the user does something about it, and shows what the server actually said
 *   rather than a paraphrase.
 */

/** Redraw the countdown at this rate. Nothing depends on it; it is a clock face. */
const TICK_MS = 250

function secondsLeft(row: OutboxRow): number {
  return Math.max(0, Math.ceil(row.sendAfter - Date.now() / 1000))
}

/** Nine tonight, or nine tomorrow if that has already passed. docs/01 §6. */
function tonight(): number {
  const when = new Date()
  when.setHours(21, 0, 0, 0)
  if (when.getTime() <= Date.now()) when.setDate(when.getDate() + 1)
  return Math.floor(when.getTime() / 1000)
}

/** Eight tomorrow morning. */
function tomorrowMorning(): number {
  const when = new Date()
  when.setDate(when.getDate() + 1)
  when.setHours(8, 0, 0, 0)
  return Math.floor(when.getTime() / 1000)
}

export function OutboxBanner() {
  const [rows, setRows] = useState<OutboxRow[]>([])
  /** The row a custom Send Later is being chosen for, if any. */
  const [scheduling, setScheduling] = useState<number | null>(null)
  /** The failed message a Delete is being confirmed for, if any. */
  const [deleting, setDeleting] = useState<OutboxRow | null>(null)
  const [, forceTick] = useState(0)
  const timer = useRef<number | undefined>(undefined)
  const toast = useToast()

  const refresh = useCallback(() => {
    outboxList()
      .then(setRows)
      .catch(() => {
        // A banner that cannot read the outbox says nothing rather than something wrong.
        setRows([])
      })
  }, [])

  /**
   * Undo Send, and the one case where it does not work.
   *
   * `compose_undo` returns false when the message has already left `holding` — the sender
   * picked it up in the moment between the click and the write, and bytes are on their way to
   * a server. Its own doc comment says returning false rather than true is the point, because
   * "reporting success there would be a lie the user would discover from the recipient".
   *
   * That boolean was being discarded. The banner refreshed, the row was gone from it because
   * the message was no longer held, and the screen showed exactly what a successful undo shows.
   * The user believed they had caught it; the recipient had it either way.
   */
  const undoSend = useCallback(
    (id: number) => {
      composeUndo(id)
        .then((cancelled) => {
          if (!cancelled) {
            toast.show({
              title: 'Too late to undo',
              description: 'The message had already started sending and is on its way.',
            })
          }
          refresh()
        })
        .catch((cause: unknown) => {
          // Also not a success. Leaving this uncaught meant a failed undo looked identical to
          // one that worked, because nothing on screen changed either way.
          toast.show({
            title: 'The message could not be recalled',
            description: cause instanceof Error ? cause.message : String(cause),
          })
          refresh()
        })
    },
    [refresh, toast],
  )

  useEffect(() => {
    if (!runningInTauri) return

    refresh()

    let cancelled = false
    let off: (() => void) | undefined

    void onOutboxProgress((progress) => {
      // The event says *something* changed; the list says what. Re-reading is one query and
      // keeps this component from having to model the state machine a second time.
      refresh()

      // Except for the sound, which is about the transition rather than the state. Re-reading
      // the list would say a message is sent for as long as it sits there, and playing on that
      // would make a noise on every unrelated outbox change.
      if (progress.state === 'sent') void soundSent(progress.accountId)
    }).then((unlisten) => {
      if (cancelled) unlisten()
      else off = unlisten
    })

    return () => {
      cancelled = true
      off?.()
    }
  }, [refresh])

  // Only while something is counting down. A timer that ran all day to redraw nothing is the
  // kind of thing that shows up as battery drain and never as a bug report.
  const holding = rows.filter((row) => row.state === 'holding')
  const failed = rows.filter((row) => row.state === 'failed')

  useEffect(() => {
    if (holding.length === 0) {
      window.clearInterval(timer.current)
      return
    }

    timer.current = window.setInterval(() => {
      forceTick((value) => value + 1)
    }, TICK_MS)

    return () => {
      window.clearInterval(timer.current)
    }
  }, [holding.length])

  if (holding.length === 0 && failed.length === 0) return null

  return (
    <div className={styles.stack}>
      {holding.map((row) => {
        const left = secondsLeft(row)

        return (
          <div key={row.id} className={styles.banner} role="status">
            <Clock className={styles.icon} aria-hidden strokeWidth={1.5} />
            <span className={styles.text}>
              Sending {row.subject === '' ? 'your message' : `“${row.subject}”`}
              {left > 0 && <span className={styles.count}> in {left}s</span>}
            </span>

            <span className={styles.actions}>
              <Button
                variant="bordered"
                onClick={() => {
                  void outboxSchedule(row.id, tonight()).then(refresh)
                }}
              >
                Tonight
              </Button>
              <Button
                variant="bordered"
                onClick={() => {
                  void outboxSchedule(row.id, tomorrowMorning()).then(refresh)
                }}
              >
                Tomorrow
              </Button>
              <Button
                variant="bordered"
                onClick={() => {
                  setScheduling(row.id)
                }}
              >
                Later…
              </Button>
              <Button
                variant="bordered"
                icon={Undo2}
                onClick={() => {
                  undoSend(row.id)
                }}
              >
                Undo Send
              </Button>
            </span>
          </div>
        )
      })}

      {failed.map((row) => (
        <div key={row.id} className={styles.banner} data-tone="failed" role="alert">
          <AlertTriangle className={styles.icon} aria-hidden strokeWidth={1.5} />
          <span className={styles.text}>
            {row.subject === '' ? 'A message' : `“${row.subject}”`} was not sent.
            {/* The server's own words. "550 mailbox full" is something the user can act on;
                "could not send" is not, and it is what a paraphrase would leave them with. */}
            {row.lastError !== null && <span className={styles.reason}> {row.lastError}</span>}
          </span>

          <span className={styles.actions}>
            <Button
              variant="bordered"
              onClick={() => {
                void outboxRetry(row.id).then(refresh)
              }}
            >
              Try Again
            </Button>
            {/* The way out when trying again cannot work. Gmail refusing an attachment fails
                the same way on every attempt, and with Try Again the only button this banner
                could never be dismissed — the message sat here for good. */}
            <Button
              variant="bordered"
              onClick={() => {
                setDeleting(row)
              }}
            >
              Delete
            </Button>
          </span>
        </div>
      ))}

      <Sheet
        open={deleting !== null}
        onOpenChange={(next) => {
          if (!next) setDeleting(null)
        }}
        title="Delete this message?"
        footer={
          <>
            <Button
              variant="bordered"
              onClick={() => {
                setDeleting(null)
              }}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => {
                if (deleting === null) return
                const row = deleting
                // Closed only once the core has answered, and the answer is read: false means
                // the message stopped being a failure — retried from another window — and
                // saying "deleted" then would be untrue.
                outboxDiscard(row.id)
                  .then((discarded) => {
                    if (!discarded) {
                      toast.show({
                        title: 'The message was not deleted',
                        description: 'It is no longer waiting as a failed message.',
                      })
                    }
                    setDeleting(null)
                    refresh()
                  })
                  .catch((cause: unknown) => {
                    toast.show({
                      title: 'The message was not deleted',
                      description: reasonFor(cause),
                    })
                  })
              }}
            >
              Delete Message
            </Button>
          </>
        }
      >
        {/* Said plainly: this is the only copy. A failed message was never sent, so it is in no
            Sent mailbox, and it was never a draft either. */}
        <p className={styles.confirmBody}>
          {deleting === null || deleting.subject === '' ? 'This message' : `“${deleting.subject}”`}{' '}
          was never sent. Deleting it removes the only copy, attachments included.
        </p>
      </Sheet>

      <SendLaterSheet
        open={scheduling !== null}
        onOpenChange={(next) => {
          if (!next) setScheduling(null)
        }}
        onChoose={(sendAfter) => {
          if (scheduling === null) return
          void outboxSchedule(scheduling, sendAfter).then(refresh)
          setScheduling(null)
        }}
      />
    </div>
  )
}
