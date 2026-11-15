import { useEffect, useState } from 'react'

import { saveSearchAsSmartMailbox } from '@/lib/search'
import { Button, Sheet, TextField, useToast } from '@/ui'
import { reasonFor } from '@/lib/ipc'

import { savedSearch } from './savedSearch'
import type { State } from './ScopeBar'

export interface SaveSearchSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The search as typed. Converted to a predicate by the core, not stored as text. */
  text: string
  /** The scope bar's Unread or Flagged, which narrows what is saved as it narrowed the results. */
  state?: State
  /**
   * The names of the mailboxes the scope bar limited the search to, or none when it searched
   * everywhere.
   */
  mailboxNames?: string[]
}

/**
 * Save Search as a Smart Mailbox. docs/06 Phase 9.
 *
 * The note about what changes is on screen rather than buried: a saved search matches slightly
 * more than the search did, because a smart mailbox has no full-text index and its text
 * conditions are substring matches. Someone who saves "fig" and later finds "configure" in
 * there should have been told, not left to work it out.
 */
export function SaveSearchSheet({
  open,
  onOpenChange,
  text,
  state = 'all',
  mailboxNames = [],
}: SaveSearchSheetProps) {
  const [name, setName] = useState('')
  const toast = useToast()

  // Seeded from the search itself, which is nearly always what someone would have typed.
  useEffect(() => {
    if (open) setName(text.trim())
  }, [open, text])

  const saving = savedSearch(text, state, mailboxNames)

  const save = () => {
    const trimmed = name.trim()
    if (trimmed === '') return

    void saveSearchAsSmartMailbox(trimmed, saving.text)
      .then(() => {
        toast.show({ title: `Saved “${trimmed}”` })
        onOpenChange(false)
      })
      .catch((error: unknown) => {
        toast.show({
          title: 'The search could not be saved',
          description: reasonFor(error),
        })
      })
  }

  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title="Save Search"
      description={
        saving.note === null
          ? 'It appears in the sidebar and stays up to date.'
          : `It appears in the sidebar and stays up to date. ${saving.note}`
      }
      footer={
        <>
          <Button
            variant="bordered"
            onClick={() => {
              onOpenChange(false)
            }}
          >
            Cancel
          </Button>
          <Button variant="filled" disabled={name.trim() === ''} onClick={save}>
            Save
          </Button>
        </>
      }
    >
      <TextField
        label="Name"
        value={name}
        onChange={(event) => {
          setName(event.target.value)
        }}
        description="Words are matched anywhere in the text, so a saved search finds a little more than the search did."
      />
    </Sheet>
  )
}
