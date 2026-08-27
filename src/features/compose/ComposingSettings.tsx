import { useEffect, useState } from 'react'

import { getUndoSeconds, setUndoSeconds } from '@/lib/ipc'

import styles from './ComposingSettings.module.css'

/**
 * The Composing section of Settings. docs/01 §6, docs/06 Phase 7.
 *
 * One control so far: how long Undo Send holds a message. It lives here rather than in the
 * compose window because it applies to every message, and a per-window control would read as
 * "hold *this* one", which is a different feature.
 */

/** Mail's choices exactly. `0` is Off. */
const CHOICES: { seconds: number; label: string }[] = [
  { seconds: 0, label: 'Off' },
  { seconds: 10, label: '10 seconds' },
  { seconds: 20, label: '20 seconds' },
  { seconds: 30, label: '30 seconds' },
]

export function ComposingSettings() {
  const [seconds, setSeconds] = useState<number | null>(null)

  useEffect(() => {
    let live = true

    void getUndoSeconds().then((value) => {
      if (live) setSeconds(value)
    })

    return () => {
      live = false
    }
  }, [])

  const choose = (next: number) => {
    // Set optimistically so the radio moves the instant it is clicked, then corrected to
    // whatever the core actually stored — which is the clamped value, not necessarily ours.
    setSeconds(next)
    void setUndoSeconds(next).then(setSeconds)
  }

  return (
    <section className={styles.section}>
      <h3 className={styles.heading}>Composing</h3>

      <fieldset className={styles.group}>
        <legend className={styles.legend}>Undo send delay</legend>

        {CHOICES.map((choice) => (
          <label key={choice.seconds} className={styles.choice}>
            <input
              type="radio"
              name="undo-seconds"
              className={styles.radio}
              value={choice.seconds}
              // Nothing is checked until the stored value has loaded, rather than defaulting to
              // one and flicking to another a moment later — which reads as the app changing
              // the setting by itself.
              checked={seconds === choice.seconds}
              disabled={seconds === null}
              onChange={() => {
                choose(choice.seconds)
              }}
            />
            {choice.label}
          </label>
        ))}
      </fieldset>

      <p className={styles.hint}>
        {seconds === 0
          ? 'Messages are sent as soon as you press Send.'
          : 'A message waits this long in the outbox, so it can be taken back before it goes.'}
      </p>
    </section>
  )
}
