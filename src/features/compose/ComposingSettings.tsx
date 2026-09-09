import { useEffect, useState } from 'react'

import { Field, Form } from '@/features/settings/SettingsForm'
import { getUndoSeconds, setUndoSeconds } from '@/lib/ipc'
import { Select, type SelectOption } from '@/ui'

import styles from '@/features/settings/settings.module.css'

/**
 * The Composing section of Settings. docs/01 §6, docs/06 Phase 7.
 *
 * One control so far: how long Undo Send holds a message. It lives here rather than in the
 * compose window because it applies to every message, and a per-window control would read as
 * "hold *this* one", which is a different feature.
 */

/** Mail's choices exactly. `0` is Off. */
const CHOICES: SelectOption<number>[] = [
  { value: 0, label: 'Off' },
  { value: 10, label: '10 seconds' },
  { value: 20, label: '20 seconds' },
  { value: 30, label: '30 seconds' },
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
      <h2 className={styles.heading}>Sending</h2>

      <Form>
        <Field
          label="Undo send delay"
          htmlFor="undo-seconds"
          hint={
            seconds === 0
              ? 'Messages are sent as soon as you press Send.'
              : 'A message waits this long in the outbox, so it can be taken back before it goes.'
          }
        >
          {/* Nothing is selected until the stored value has loaded, rather than defaulting to
              one and moving to another a moment later — which reads as the app changing the
              setting by itself. `Select` does that from `value={null}`. */}
          <Select
            id="undo-seconds"
            label="Undo send delay"
            hideLabel
            options={CHOICES}
            value={seconds}
            onValueChange={choose}
          />
        </Field>
      </Form>
    </section>
  )
}
