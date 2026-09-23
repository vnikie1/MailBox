import { Plus, X } from 'lucide-react'

import type { Condition } from '@/lib/generated/Condition'
import type { Field } from '@/lib/generated/Field'
import { IconButton, Select, TextField, type SelectOption } from '@/ui'

import styles from './PredicateEditor.module.css'
import { BLANK_CONDITION, FIELD_LABELS, OP_LABELS, opsFor, takesValue } from './predicateShape'

/**
 * The condition builder shared by rules and smart mailboxes. docs/01 §8.
 *
 * One editor for both, because they are one predicate type in the core. Two editors would
 * diverge — one would learn a field the other did not — and the whole reason the core shares an
 * engine is so a rule and a smart mailbox written the same way behave the same way.
 *
 * The shape it edits, and why that shape is flat, is in `predicateShape.ts`.
 */

const MATCH_OPTIONS: readonly SelectOption<'all' | 'any'>[] = [
  { value: 'all', label: 'all' },
  { value: 'any', label: 'any' },
]

const FIELD_OPTIONS: readonly SelectOption<Field>[] = (Object.keys(FIELD_LABELS) as Field[]).map(
  (field) => ({ value: field, label: FIELD_LABELS[field] }),
)

export interface PredicateEditorProps {
  matchAll: boolean
  conditions: Condition[]
  onChange: (matchAll: boolean, conditions: Condition[]) => void
}

export function PredicateEditor({ matchAll, conditions, onChange }: PredicateEditorProps) {
  const update = (index: number, patch: Partial<Condition>) => {
    onChange(
      matchAll,
      conditions.map((condition, at) => {
        if (at !== index) return condition

        const next = { ...condition, ...patch }

        // Changing the field can strand an operator that no longer applies — "Has attachment
        // begins with" is not a question. Snapping to the first valid operator is the only
        // option that never leaves an unsaveable row on screen.
        if (patch.field !== undefined && !opsFor(next.field).includes(next.op)) {
          // `opsFor` never returns an empty list, but the index signature does not know that,
          // and a silent `undefined` here would be exactly the unsaveable row this avoids.
          next.op = opsFor(next.field)[0] ?? 'contains'
        }

        return next
      }),
    )
  }

  return (
    <div className={styles.editor}>
      <div className={styles.header}>
        <span className={styles.matchLabel}>If</span>
        <Select
          label="Match"
          hideLabel
          className={styles.select}
          options={MATCH_OPTIONS}
          value={matchAll ? 'all' : 'any'}
          onValueChange={(match) => {
            onChange(match === 'all', conditions)
          }}
        />
        <span className={styles.matchLabel}>of the following are true:</span>
      </div>

      <ul className={styles.rows}>
        {conditions.map((condition, index) => (
          // Keyed by index on purpose. These rows have no identity of their own — they are a
          // positional list edited in place — and a synthesised id would be one more thing to
          // keep in step with the array for no gain.
          <li key={index} className={styles.row}>
            <Select
              label="Field"
              hideLabel
              className={styles.select}
              options={FIELD_OPTIONS}
              value={condition.field}
              onValueChange={(field) => {
                update(index, { field })
              }}
            />

            <Select
              label="Condition"
              hideLabel
              className={styles.select}
              options={opsFor(condition.field).map((op) => ({ value: op, label: OP_LABELS[op] }))}
              value={condition.op}
              onValueChange={(op) => {
                update(index, { op })
              }}
            />

            {takesValue(condition.op) ? (
              <TextField
                label="Value"
                hideLabel
                value={condition.value}
                onChange={(event) => {
                  update(index, { value: event.target.value })
                }}
              />
            ) : (
              <span className={styles.spacer} />
            )}

            <IconButton
              icon={X}
              label="Remove condition"
              // Never down to zero. An empty `all` matches everything and an empty `any`
              // matches nothing, and neither is something a user meant to build.
              disabled={conditions.length <= 1}
              onClick={() => {
                onChange(
                  matchAll,
                  conditions.filter((_, at) => at !== index),
                )
              }}
            />

            <IconButton
              icon={Plus}
              label="Add condition"
              onClick={() => {
                const next = [...conditions]
                next.splice(index + 1, 0, { ...BLANK_CONDITION })
                onChange(matchAll, next)
              }}
            />
          </li>
        ))}
      </ul>
    </div>
  )
}
