import { Plus, Trash2 } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'

import type { Condition } from '@/lib/generated/Condition'
import type { SmartMailbox } from '@/lib/generated/SmartMailbox'
import { onChanged, smartDelete, smartList, smartSave } from '@/lib/organise'
import { Button, IconButton, Sheet, TextField, useToast } from '@/ui'

import { PredicateEditor } from './PredicateEditor'
import { BLANK_CONDITION, fromFlat, toFlat } from './predicateShape'
import styles from './RulesEditor.module.css'

export interface SmartMailboxEditorProps {
  open: boolean
  onClose: () => void
}

interface Draft {
  id: number | null
  name: string
  matchAll: boolean
  conditions: Condition[]
  /** True when the stored predicate nests deeper than the flat editor can show. */
  tooComplex: boolean
}

function draftFrom(box: SmartMailbox): Draft {
  const flat = toFlat(box.predicate)

  return {
    id: box.id,
    name: box.name,
    matchAll: flat?.matchAll ?? true,
    conditions: flat?.conditions ?? [{ ...BLANK_CONDITION }],
    tooComplex: flat === null,
  }
}

function blankDraft(): Draft {
  return {
    id: null,
    name: 'New Smart Mailbox',
    matchAll: true,
    conditions: [{ ...BLANK_CONDITION }],
    tooComplex: false,
  }
}

/**
 * Smart mailboxes: the list, and the editor for one. docs/01 §8.
 *
 * Shares `PredicateEditor` with the rules editor, which is the point of the shared engine — a
 * smart mailbox and a rule written the same way behave the same way, and there is one place to
 * fix when a field is wrong.
 *
 * The only thing this adds over the rules editor is what it *lacks*: no actions. A smart
 * mailbox is a question about the mailbox, not something that happens to mail.
 */
export function SmartMailboxEditor({ open, onClose }: SmartMailboxEditorProps) {
  const [boxes, setBoxes] = useState<SmartMailbox[]>([])
  const [draft, setDraft] = useState<Draft | null>(null)
  const toast = useToast()

  const load = useCallback(() => {
    void smartList().then(setBoxes)
  }, [])

  useEffect(() => {
    if (!open) return

    load()
    const unlisten = onChanged('smart:changed', load)

    return () => {
      void unlisten.then((stop) => {
        stop()
      })
    }
  }, [open, load])

  const save = () => {
    if (draft === null) return

    void smartSave(
      draft.id,
      draft.name.trim() === '' ? 'Untitled' : draft.name,
      null,
      fromFlat(draft.matchAll, draft.conditions),
    )
      .then(() => {
        setDraft(null)
      })
      .catch((error: unknown) => {
        toast.show({
          title: 'The smart mailbox could not be saved',
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
        title={draft.id === null ? 'New Smart Mailbox' : 'Edit Smart Mailbox'}
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
            label="Name"
            value={draft.name}
            onChange={(event) => {
              setDraft({ ...draft, name: event.target.value })
            }}
          />

          {draft.tooComplex ? (
            <p className={styles.note}>
              This smart mailbox uses nested conditions that this editor cannot show. It still
              matches exactly as saved.
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
      title="Smart Mailboxes"
    >
      <div className={styles.list}>
        {boxes.length === 0 && (
          <p className={styles.note}>
            A smart mailbox is a saved search that stays up to date. It gathers mail from every
            account without moving any of it.
          </p>
        )}

        <ul className={styles.rows}>
          {boxes.map((box) => (
            <li key={box.id} className={styles.row}>
              <span />

              <button
                type="button"
                className={styles.name}
                onClick={() => {
                  setDraft(draftFrom(box))
                }}
              >
                {box.name}
              </button>

              <IconButton
                icon={Trash2}
                label={`Delete ${box.name}`}
                onClick={() => {
                  // Deletes the search, never the mail. Worth saying because a row that looks
                  // like a mailbox and vanishes when deleted invites exactly that fear.
                  void smartDelete(box.id)
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
          Add Smart Mailbox
        </Button>
      </div>
    </Sheet>
  )
}
