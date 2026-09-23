import { Plus, X } from 'lucide-react'

import type { Action } from '@/lib/generated/Action'
import type { MailboxRow } from '@/lib/generated/MailboxRow'
import { IconButton, Select, type SelectOption } from '@/ui'

import styles from './PredicateEditor.module.css'

/**
 * What a rule does once it matches. docs/01 §8.
 *
 * The list of kinds here is exactly the core's `Action`, and there is no "Run Script" —
 * see `rules/engine.rs` for why that one is deliberately missing rather than unfinished.
 */

type Kind = Action['type']

const ACTION_LABELS: Record<Kind, string> = {
  moveTo: 'Move to mailbox',
  markRead: 'Mark as read',
  markUnread: 'Mark as unread',
  flag: 'Flag',
  unflag: 'Remove flag',
  setColour: 'Set flag colour',
  markJunk: 'Mark as junk',
  delete: 'Move to Trash',
  stopEvaluating: 'Stop evaluating rules',
}

const COLOURS = ['red', 'orange', 'yellow', 'green', 'blue', 'purple', 'gray']

const KIND_OPTIONS: readonly SelectOption<Kind>[] = (Object.keys(ACTION_LABELS) as Kind[]).map(
  (kind) => ({ value: kind, label: ACTION_LABELS[kind] }),
)

const COLOUR_OPTIONS: readonly SelectOption<string>[] = COLOURS.map((colour) => ({
  value: colour,
  label: colour,
}))

/** Builds a default action of a kind, so switching kinds never leaves an invalid one. */
function defaultOf(kind: Kind, mailboxes: MailboxRow[]): Action {
  switch (kind) {
    case 'moveTo':
      return { type: 'moveTo', value: mailboxes[0]?.id ?? 0 }
    case 'setColour':
      return { type: 'setColour', value: 'red' }
    default:
      return { type: kind }
  }
}

export interface ActionEditorProps {
  actions: Action[]
  mailboxes: MailboxRow[]
  onChange: (actions: Action[]) => void
}

export function ActionEditor({ actions, mailboxes, onChange }: ActionEditorProps) {
  const replace = (index: number, action: Action) => {
    onChange(actions.map((existing, at) => (at === index ? action : existing)))
  }

  return (
    <div className={styles.editor}>
      <span className={styles.matchLabel}>Perform the following actions:</span>

      <ul className={styles.rows}>
        {actions.map((action, index) => (
          // Positional by nature, like the condition rows next door.
          <li key={index} className={styles.row}>
            <Select
              label="Action"
              hideLabel
              className={styles.select}
              options={KIND_OPTIONS}
              value={action.type}
              onValueChange={(kind) => {
                replace(index, defaultOf(kind, mailboxes))
              }}
            />

            {action.type === 'moveTo' && (
              <Select
                label="Mailbox"
                hideLabel
                className={styles.select}
                options={mailboxes.map((mailbox) => ({
                  value: mailbox.id,
                  label: mailbox.displayName,
                }))}
                value={action.value}
                onValueChange={(mailboxId) => {
                  replace(index, { type: 'moveTo', value: mailboxId })
                }}
              />
            )}

            {action.type === 'setColour' && (
              <Select
                label="Colour"
                hideLabel
                className={styles.select}
                options={COLOUR_OPTIONS}
                value={action.value}
                onValueChange={(colour) => {
                  replace(index, { type: 'setColour', value: colour })
                }}
              />
            )}

            {/* Every row fills all five columns of the grid it shares with the condition rows,
                so the two buttons always land in the last two. An action with no second popup
                used to render four cells, and the grid placed its × in the value column and its
                + in ×'s — invisible only while that column was being laid out 0px wide. */}
            {action.type !== 'moveTo' && action.type !== 'setColour' && (
              <span className={styles.spacer} />
            )}
            <span className={styles.spacer} />

            <IconButton
              icon={X}
              label="Remove action"
              // A rule with no actions matches mail and does nothing to it, which looks
              // exactly like a rule that is broken.
              disabled={actions.length <= 1}
              onClick={() => {
                onChange(actions.filter((_, at) => at !== index))
              }}
            />

            <IconButton
              icon={Plus}
              label="Add action"
              onClick={() => {
                const next = [...actions]
                next.splice(index + 1, 0, { type: 'markRead' })
                onChange(next)
              }}
            />
          </li>
        ))}
      </ul>
    </div>
  )
}
