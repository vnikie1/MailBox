import { useEffect, useState } from 'react'

import { Field, Form } from '@/features/settings/SettingsForm'
import type { JunkStatus } from '@/lib/generated/JunkStatus'
import { junkStatus, junkTrainingMode, setJunkTrainingMode } from '@/lib/organise'

import styles from '@/features/settings/settings.module.css'

/**
 * The Junk section of Settings. docs/01 §8, docs/06 Phase 8.
 *
 * Shows how much the filter has to go on, which matters more here than in most settings
 * panels: a Bayesian classifier with twelve examples behaves nothing like one with two hundred,
 * and without the count "the junk filter does nothing" and "the junk filter has not been taught
 * anything yet" look identical from the outside.
 */
export function JunkSettings() {
  const [status, setStatus] = useState<JunkStatus | null>(null)
  const [training, setTraining] = useState<boolean | null>(null)

  useEffect(() => {
    let live = true

    void junkStatus().then((value) => {
      if (live) setStatus(value)
    })
    void junkTrainingMode().then((value) => {
      if (live) setTraining(value)
    })

    return () => {
      live = false
    }
  }, [])

  return (
    <section className={styles.section}>
      <h2 className={styles.heading}>Junk</h2>

      <Form>
        <Field
          label="Junk filter"
          hint={
            status === null
              ? 'Checking what the filter has learned…'
              : status.ready
                ? `Trained on ${String(status.cleanExamples)} ordinary and ${String(status.junkExamples)} junk messages.`
                : `Not trained yet. It needs ${String(status.needed)} of each and has ${String(status.cleanExamples)} ordinary and ${String(status.junkExamples)} junk, so nothing is filed automatically until then.`
          }
        >
          <label className={styles.choice}>
            <input
              type="checkbox"
              className={styles.checkbox}
              checked={training === true}
              disabled={training === null}
              onChange={(event) => {
                setTraining(event.target.checked)
                void setJunkTrainingMode(event.target.checked)
              }}
            />
            Mark junk without moving it
          </label>
        </Field>
      </Form>
    </section>
  )
}
