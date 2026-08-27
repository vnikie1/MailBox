import { Plus, Trash2 } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'

import type { Action } from '@/lib/generated/Action'
import type { Condition } from '@/lib/generated/Condition'
import type { MailboxRow } from '@/lib/generated/MailboxRow'
import type { Rule } from '@/lib/generated/Rule'
import { onChanged, ruleDelete, ruleSave, rulesList } from '@/lib/organise'
import { Button, IconButton, Sheet, TextField, useToast } from '@/ui'

import { ActionEditor } from './ActionEditor'
import { PredicateEditor } from './PredicateEditor'
import { BLANK_CONDITION, fromFlat, toFlat } from './predicateShape'
import styles from './RulesEditor.module.css'

export interface RulesEditorProps {
  open: boolean
  onClose: () => void
  mailboxes: MailboxRow[]
}

interface Draft {
  id: number | null
  name: string
  enabled: boolean
  matchAll: boolean
  conditions: Condition[]
  actions: Action[]
  /**
   * True when the stored predicate nests deeper than this editor can show.
   *
   * Such a rule is displayed read-only rather than flattened. Flattening would save back
   * something that means something different from what the user opened, and a rule that
   * quietly changes meaning when you glance at it is the worst possible outcome for a feature
   * whose whole job is filing mail unattended.
   */
  tooComplex: boolean
}

function draftFrom(rule: Rule): Draft {
  const flat = toFlat(rule.predicate)

  return {
    id: rule.id,
    name: rule.name,
    enabled: rule.enabled,
    matchAll: flat?.matchAll ?? true,
    conditions: flat?.conditions ?? [{ ...BLANK_CONDITION }],
    actions: rule.actions,
    tooComplex: flat === null,
  }
}

function blankDraft(): Draft {
  return {
    id: null,
    name: 'New Rule',
    enabled: true,
    matchAll: true,
    conditions: [{ ...BLANK_CONDITION }],
    actions: [{ type: 'markRead' }],
    tooComplex: false,
  }
}

/** The rules list and editor. docs/01 §8. */
export function RulesEditor({ open, onClose, mailboxes }: RulesEditorProps) {
  const [rules, setRules] = useState<Rule[]>([])
  const [draft, setDraft] = useState<Draft | null>(null)
  const toast = useToast()

  const load = useCallback(() => {
    void rulesList().then(setRules)
  }, [])

  useEffect(() => {
    if (!open) return

    load()
    const unlisten = onChanged('rules:changed', load)

    return () => {
      void unlisten.then((stop) => {
        stop()
      })
    }
  }, [open, load])

  const save = () => {
    if (draft === null) return

    void ruleSave(
      draft.id,
      draft.name.trim() === '' ? 'Untitled Rule' : draft.name,
      draft.enabled,
      fromFlat(draft.matchAll, draft.conditions),
      draft.actions,
    )
      .then(() => {
        setDraft(null)
      })
      .catch((error: unknown) => {
        toast.show({
          title: 'The rule could not be saved',
          description: error instanceof Error ? error.message : String(error),
        })
      })
  }

  if (draft !== null) {
    return (
      <Sheet
        open={open}
        onOpenChange={(next) => {
          if (!next) setDraft(null)
        }}
        title={draft.id === null ? 'New Rule' : 'Edit Rule'}
        footer={
          <>
            <Button
              variant="bordered"
              onClick={() => {
                setDraft(null)
              }}
            >
              Cancel
            </Button>
            <Button variant="filled" disabled={draft.tooComplex} onClick={save}>
              OK
            </Button>
          </>
        }
      >
        <div className={styles.form}>
          <TextField
            label="Description"
            value={draft.name}
            onChange={(event) => {
              setDraft({ ...draft, name: event.target.value })
            }}
          />

          {draft.tooComplex ? (
            <p className={styles.note}>
              This rule uses nested conditions that this editor cannot show. It still runs exactly
              as saved. Editing it here would change what it means, so it is left alone.
            </p>
          ) : (
            <PredicateEditor
              matchAll={draft.matchAll}
              conditions={draft.conditions}
              onChange={(matchAll, conditions) => {
                setDraft({ ...draft, matchAll, conditions })
              }}
            />
          )}

          <ActionEditor
            actions={draft.actions}
            mailboxes={mailboxes}
            onChange={(actions) => {
              setDraft({ ...draft, actions })
            }}
          />
        </div>
      </Sheet>
    )
  }

  return (
    <Sheet
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose()
      }}
      title="Rules"
    >
      <div className={styles.list}>
        {rules.length === 0 && (
          <p className={styles.note}>
            No rules yet. A rule files, flags or marks mail as it arrives, and can be run over a
            selection at any time with Alt+Ctrl+L.
          </p>
        )}

        <ul className={styles.rows}>
          {rules.map((rule) => (
            <li key={rule.id} className={styles.row}>
              <input
                type="checkbox"
                className={styles.enable}
                checked={rule.enabled}
                aria-label={`Enable ${rule.name}`}
                onChange={(event) => {
                  // Saved immediately rather than on an OK button. Toggling a rule off is what
                  // someone does when it is misfiring right now, and making them open the
                  // editor first to stop it is the wrong shape for that moment.
                  void ruleSave(
                    rule.id,
                    rule.name,
                    event.target.checked,
                    rule.predicate,
                    rule.actions,
                  )
                }}
              />

              <button
                type="button"
                className={styles.name}
                onClick={() => {
                  setDraft(draftFrom(rule))
                }}
              >
                {rule.name}
              </button>

              <IconButton
                icon={Trash2}
                label={`Delete ${rule.name}`}
                onClick={() => {
                  void ruleDelete(rule.id)
                }}
              />
            </li>
          ))}
        </ul>

        <Button
          variant="bordered"
          icon={Plus}
          onClick={() => {
            setDraft(blankDraft())
          }}
        >
          Add Rule
        </Button>
      </div>
    </Sheet>
  )
}
