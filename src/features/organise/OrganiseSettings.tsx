import { useState } from 'react'

import { useMailboxes } from '@/app/queries'
import { Field, Form } from '@/features/settings/SettingsForm'
import { Button } from '@/ui'

import { RulesEditor } from './RulesEditor'
import { SmartMailboxEditor } from './SmartMailboxEditor'
import styles from '@/features/settings/settings.module.css'

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
      <h2 className={styles.heading}>Organising</h2>

      {/* One row each rather than two buttons side by side under one note: they open two
          different editors and the note that used to sit under them described both, so
          neither button had an explanation you could read next to it. */}
      <Form>
        <Field
          label="Rules"
          hint="Rules act on mail as it arrives, and can be run over a selection at any time with Alt+Ctrl+L."
        >
          <Button
            variant="bordered"
            onClick={() => {
              setRulesOpen(true)
            }}
          >
            Edit Rules…
          </Button>
        </Field>

        <Field label="Smart mailboxes" hint="Saved searches. They gather mail without moving it.">
          <Button
            variant="bordered"
            onClick={() => {
              setSmartOpen(true)
            }}
          >
            Edit Smart Mailboxes…
          </Button>
        </Field>
      </Form>

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
