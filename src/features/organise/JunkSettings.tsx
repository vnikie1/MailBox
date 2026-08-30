import { useEffect, useState } from 'react'

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
      <h3 className={styles.heading}>Junk</h3>

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

      <p className={styles.hint}>
        {status === null
          ? 'Checking what the filter has learned…'
          : status.ready
            ? `Trained on ${String(status.cleanExamples)} ordinary and ${String(status.junkExamples)} junk messages.`
            : `Not enough examples yet — ${String(status.needed)} of each are needed, and there are ${String(status.cleanExamples)} ordinary and ${String(status.junkExamples)} junk. Until then nothing is filed automatically.`}
      </p>
    </section>
  )
}
