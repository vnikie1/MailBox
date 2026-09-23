import { useEffect, useId, useRef, useState, type SubmitEvent } from 'react'

import type { EraseTarget } from '@/lib/ipc'
import type { MailboxRow } from '@/lib/generated/MailboxRow'
import { mailboxCreate, mailboxDelete, mailboxErase, mailboxRename, reasonFor } from '@/lib/ipc'
import { mailboxNameProblem } from '@/lib/mailboxName'
import { Button, Select, Sheet, TextField, useToast } from '@/ui'

import styles from './MailboxSheets.module.css'

/**
 * The sheets behind the mailbox menu's four rows that ask something first: New Mailbox and
 * Rename Mailbox ask for a name, Delete Mailbox and the two Erase rows ask to be sure.
 *
 * Each sheet calls the core itself and stays open until the core has answered, so a refusal —
 * a name already taken, a folder that vanished meanwhile — is said in the sheet, next to the
 * thing it is about, rather than in a toast after the sheet has gone.
 */

/** Somewhere a mailbox can be made: the top of an account, or inside one of its mailboxes. */
export interface MailboxLocation {
  /** Unique across accounts: `account-3`, `mailbox-41`. */
  key: string
  accountId: number
  /** The mailbox it goes inside, or null for the top of the account. */
  parentId: number | null
  /** What a refusal calls the place. */
  name: string
  /** What the popup shows, indented to the place's depth. */
  label: string
  depth: number
  /** The server's separator, for checking a name as it is typed. */
  delimiter: string | null
}

/** A name problem is shown only once there is a name to have a problem with. */
function shownProblem(name: string, delimiter: string | null): string | null {
  return name.trim() === '' ? null : mailboxNameProblem(name, delimiter)
}

/* ------------------------------------------------------------------ New Mailbox */

export interface NewMailboxSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  locations: MailboxLocation[]
  /**
   * The location the sheet opens on: the mailbox the menu was opened on when it can hold
   * another, its account otherwise. A `MailboxLocation.key`.
   */
  initial: string | null
}

/**
 * New Mailbox. Mail's sheet: a Location and a Name.
 *
 * The location is an account or a mailbox inside one, listed as the sidebar nests them, and the
 * new mailbox appears there at once.
 */
export function NewMailboxSheet({ open, onOpenChange, locations, initial }: NewMailboxSheetProps) {
  const formId = useId()
  const [location, setLocation] = useState<string | null>(initial)
  const [name, setName] = useState('')
  const [refusal, setRefusal] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    if (!open) return
    setLocation(initial)
    setName('')
    setRefusal(null)
    setBusy(false)
  }, [open, initial])

  const place = locations.find((entry) => entry.key === location)
  const delimiter = place?.delimiter ?? null
  const problem = shownProblem(name, delimiter) ?? refusal
  const ready = place !== undefined && mailboxNameProblem(name, delimiter) === null && !busy

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!ready) return

    setBusy(true)
    mailboxCreate(place.accountId, place.parentId, name.trim())
      .then(() => {
        onOpenChange(false)
      })
      .catch((cause: unknown) => {
        setRefusal(reasonFor(cause))
        setBusy(false)
      })
  }

  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title="New Mailbox"
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
          <Button variant="filled" type="submit" form={formId} disabled={!ready}>
            Create
          </Button>
        </>
      }
    >
      <form id={formId} className={styles.form} onSubmit={submit}>
        <Select
          label="Location"
          options={locations.map((entry) => ({ value: entry.key, label: entry.label }))}
          value={location}
          onValueChange={(value) => {
            setLocation(value)
            setRefusal(null)
          }}
        />
        <TextField
          label="Name"
          value={name}
          // The sheet exists to take this one answer.
          autoFocus
          spellCheck={false}
          maxLength={400}
          invalid={problem !== null}
          // Reserved from the start, so the sheet does not grow when a problem appears.
          description={problem ?? ' '}
          onChange={(event) => {
            setName(event.currentTarget.value)
            setRefusal(null)
          }}
        />
      </form>
    </Sheet>
  )
}

/* --------------------------------------------------------------- Rename Mailbox */

export interface RenameMailboxSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  mailbox: MailboxRow | null
}

export function RenameMailboxSheet({ open, onOpenChange, mailbox }: RenameMailboxSheetProps) {
  const formId = useId()
  const field = useRef<HTMLInputElement>(null)
  const [name, setName] = useState('')
  const [refusal, setRefusal] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const current = mailbox?.displayName ?? ''

  useEffect(() => {
    if (!open) return
    setName(current)
    setRefusal(null)
    setBusy(false)

    // The whole name selected, as a rename field in Explorer and in Mail starts: typing
    // replaces it, and an arrow key keeps it.
    const frame = requestAnimationFrame(() => {
      field.current?.select()
    })
    return () => {
      cancelAnimationFrame(frame)
    }
  }, [open, current])

  const delimiter = mailbox?.delimiter ?? null
  const problem = shownProblem(name, delimiter) ?? refusal
  const ready =
    mailbox !== null &&
    name.trim() !== current &&
    mailboxNameProblem(name, delimiter) === null &&
    !busy

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!ready) return

    setBusy(true)
    mailboxRename(mailbox.id, name.trim())
      .then(() => {
        onOpenChange(false)
      })
      .catch((cause: unknown) => {
        setRefusal(reasonFor(cause))
        setBusy(false)
      })
  }

  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title="Rename Mailbox"
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
          <Button variant="filled" type="submit" form={formId} disabled={!ready}>
            Rename
          </Button>
        </>
      }
    >
      <form id={formId} className={styles.form} onSubmit={submit}>
        <TextField
          ref={field}
          label="Name"
          value={name}
          autoFocus
          spellCheck={false}
          maxLength={400}
          invalid={problem !== null}
          description={problem ?? ' '}
          onChange={(event) => {
            setName(event.currentTarget.value)
            setRefusal(null)
          }}
        />
      </form>
    </Sheet>
  )
}

/* --------------------------------------------------------------- Delete Mailbox */

export interface DeleteMailboxSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  mailbox: MailboxRow | null
  /** Gmail deletes a label, not the mail under it — the sentence has to say which. */
  gmail: boolean
  /** An imported archive has no server, and the sentence must not claim one. */
  hasServer: boolean
}

/**
 * The confirmation for Delete Mailbox, which says what will actually go.
 *
 * The count is not given. The store holds only the newest messages of a folder, so any number
 * shown here could be smaller than what the server deletes — and an understated warning before
 * a permanent deletion is worse than none.
 */
export function DeleteMailboxSheet({
  open,
  onOpenChange,
  mailbox,
  gmail,
  hasServer,
}: DeleteMailboxSheetProps) {
  const toast = useToast()
  const [refusal, setRefusal] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    if (!open) return
    setRefusal(null)
    setBusy(false)
  }, [open])

  const name = mailbox?.displayName ?? ''
  const inside = mailbox?.descendants ?? 0
  const children =
    inside === 0
      ? ''
      : inside === 1
        ? ' and the mailbox inside it'
        : ` and the ${String(inside)} mailboxes inside it`

  const consequence = gmail
    ? `The label “${name}”${children} will be removed from Gmail. The messages are not deleted: they stay in All Mail, and in any other label they have.`
    : hasServer
      ? `“${name}”${children} and every message in ${inside === 0 ? 'it' : 'them'} will be deleted, on this computer and on the server. This can’t be undone.`
      : `“${name}”${children} and every message in ${inside === 0 ? 'it' : 'them'} will be deleted. This can’t be undone.`

  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title={`Delete “${name}”?`}
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
          <Button
            variant="destructive"
            disabled={mailbox === null || busy}
            onClick={() => {
              if (mailbox === null) return
              setBusy(true)
              mailboxDelete(mailbox.id)
                .then(() => {
                  toast.show({ title: gmail ? `Removed the label “${name}”` : `Deleted “${name}”` })
                  onOpenChange(false)
                })
                .catch((cause: unknown) => {
                  setRefusal(reasonFor(cause))
                  setBusy(false)
                })
            }}
          >
            Delete
          </Button>
        </>
      }
    >
      <p className={styles.body}>{consequence}</p>
      {refusal !== null && (
        <p className={styles.refusal} role="alert">
          {refusal}
        </p>
      )}
    </Sheet>
  )
}

/* ------------------------------------------------------------------------ Erase */

export interface EraseSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  target: EraseTarget
  accountId: number | null
  accountName: string
  /** The Bin or the Junk mailbox being emptied, for its name. */
  mailbox: MailboxRow | undefined
  hasServer: boolean
}

/**
 * The confirmation for Erase Deleted Items and Erase Junk Mail.
 *
 * Named by the mailbox's own name — "Bin", "Deleted Items", "[Gmail]/Trash" shows as "Trash" —
 * because that is the word the user sees in the sidebar, and by the account, because the menu
 * was opened on one account and erases only that one's.
 */
export function EraseSheet({
  open,
  onOpenChange,
  target,
  accountId,
  accountName,
  mailbox,
  hasServer,
}: EraseSheetProps) {
  const toast = useToast()
  const [refusal, setRefusal] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    if (!open) return
    setRefusal(null)
    setBusy(false)
  }, [open])

  const place = mailbox?.displayName ?? (target === 'junk' ? 'Junk' : 'Bin')
  const where = hasServer ? ', on this computer and on the server' : ''

  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title={target === 'junk' ? 'Erase Junk Mail?' : 'Erase Deleted Items?'}
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
          <Button
            variant="destructive"
            disabled={accountId === null || busy}
            onClick={() => {
              if (accountId === null) return
              setBusy(true)
              mailboxErase(accountId, target)
                .then((erased) => {
                  toast.show({
                    title: `Erased “${place}”`,
                    description:
                      erased === 1
                        ? `1 message in ${accountName} was removed.`
                        : `${String(erased)} messages in ${accountName} were removed.`,
                  })
                  onOpenChange(false)
                })
                .catch((cause: unknown) => {
                  setRefusal(reasonFor(cause))
                  setBusy(false)
                })
            }}
          >
            Erase
          </Button>
        </>
      }
    >
      <p className={styles.body}>
        Every message in “{place}” in {accountName} will be permanently erased{where}. This can’t be
        undone.
      </p>
      {refusal !== null && (
        <p className={styles.refusal} role="alert">
          {refusal}
        </p>
      )}
    </Sheet>
  )
}
