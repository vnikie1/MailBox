import { useState } from 'react'

import { useMailboxes } from '@/app/queries'
import { Button } from '@/ui'

import { RulesEditor } from './RulesEditor'
import { SmartMailboxEditor } from './SmartMailboxEditor'
import styles from './JunkSettings.module.css'

/**
 * The Rules and Smart Mailboxes entries in Settings. docs/01 §8.
 *
 * Buttons that open their own sheets rather than the editors inlined here, because both are
 * full editors in their own right and a settings panel that grows a condition builder inside
 * it stops being a settings panel.
 *
 * This is where they live until the menu bar exists in Phase 10 — Mail puts Rules under
 * Settings and Smart Mailboxes under the Mailbox menu, and only one of those places is
 * available yet.
 */
export function OrganiseSettings() {
  const [rulesOpen, setRulesOpen] = useState(false)
  const [smartOpen, setSmartOpen] = useState(false)
  const { data: mailboxes = [] } = useMailboxes()

  return (
    <section className={styles.section}>
      <h3 className={styles.heading}>Organising</h3>

      <div className={styles.row}>
        <Button
          variant="bordered"
          onClick={() => {
            setRulesOpen(true)
          }}
        >
          Rules…
        </Button>
        <Button
          variant="bordered"
          onClick={() => {
            setSmartOpen(true)
          }}
        >
          Smart Mailboxes…
        </Button>
      </div>

      <p className={styles.hint}>
        Rules act on mail as it arrives, and can be run over a selection at any time with
        Alt+Ctrl+L. Smart mailboxes are saved searches — they gather mail without moving it.
      </p>

      <RulesEditor
        open={rulesOpen}
        onClose={() => {
          setRulesOpen(false)
        }}
        mailboxes={mailboxes}
      />

      <SmartMailboxEditor
        open={smartOpen}
        onClose={() => {
          setSmartOpen(false)
        }}
      />
    </section>
  )
}
