import { useEffect, useMemo, useRef, useState } from 'react'

import type { AccountRow } from '@/lib/generated/AccountRow'
import type { MailboxRow } from '@/lib/generated/MailboxRow'
import { cx } from '@/lib/cx'
import { Sheet, TextField } from '@/ui'

import styles from './MailboxPicker.module.css'

export interface MailboxPickerProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  mailboxes: MailboxRow[]
  accounts: AccountRow[]
  /** What the chosen mailbox is for — "Move 3 messages to…". */
  title: string
  onChoose: (mailboxId: number) => void
}

/**
 * Ctrl+Shift+M: choose a mailbox by typing. docs/01 §8.
 *
 * Typeahead rather than a tree, because the tree is already in the sidebar and this exists for
 * the case the sidebar is bad at: forty folders, and the one you want is three levels down and
 * scrolled out of sight. Someone who knows the folder's name should not have to find it.
 */

/**
 * Ranks a mailbox against what has been typed.
 *
 * Higher is better, `null` means no match. The ordering matters more than it looks: a prefix
 * match ranks above a contained match, so typing "arch" puts Archive first rather than
 * "Research Notes" — otherwise the top result changes under the user between keystrokes, and
 * Enter sends the mail somewhere they did not look at.
 */
function score(name: string, query: string): number | null {
  const haystack = name.toLowerCase()
  const needle = query.toLowerCase()

  if (needle === '') return 0
  if (haystack === needle) return 4
  if (haystack.startsWith(needle)) return 3

  // A word inside the name — "Bank" in "HDFC Bank" — ranks above a match in the middle of a
  // word, which is usually a coincidence.
  if (haystack.split(/[\s/\\-]+/).some((word) => word.startsWith(needle))) return 2
  if (haystack.includes(needle)) return 1

  return null
}

export function MailboxPicker({
  open,
  onOpenChange,
  mailboxes,
  accounts,
  title,
  onChoose,
}: MailboxPickerProps) {
  const [query, setQuery] = useState('')
  const [highlighted, setHighlighted] = useState(0)
  const listRef = useRef<HTMLUListElement>(null)

  // Cleared each time it opens. A picker that remembers last time's text makes the first
  // keystroke append to a search the user has forgotten about.
  useEffect(() => {
    if (open) {
      setQuery('')
      setHighlighted(0)
    }
  }, [open])

  const matches = useMemo(() => {
    const named = mailboxes.map((mailbox) => ({
      mailbox,
      account: accounts.find((entry) => entry.id === mailbox.accountId),
    }))

    return named
      .map((entry) => ({ ...entry, rank: score(entry.mailbox.displayName, query.trim()) }))
      .filter((entry): entry is typeof entry & { rank: number } => entry.rank !== null)
      .sort((a, b) => b.rank - a.rank || a.mailbox.displayName.localeCompare(b.mailbox.displayName))
      .slice(0, 40)
  }, [mailboxes, accounts, query])

  const choose = (mailboxId: number) => {
    onChoose(mailboxId)
    onOpenChange(false)
  }

  return (
    <Sheet open={open} onOpenChange={onOpenChange} title={title}>
      <TextField
        label="Mailbox"
        hideLabel
        autoFocus
        placeholder="Type a mailbox name"
        value={query}
        onChange={(event) => {
          setQuery(event.target.value)
          setHighlighted(0)
        }}
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown') {
            event.preventDefault()
            setHighlighted((current) => Math.min(current + 1, matches.length - 1))
            return
          }

          if (event.key === 'ArrowUp') {
            event.preventDefault()
            setHighlighted((current) => Math.max(current - 1, 0))
            return
          }

          if (event.key === 'Enter') {
            event.preventDefault()
            const chosen = matches[highlighted]
            // Nothing happens when nothing matches, rather than falling through to the first
            // mailbox in the account. Enter on an empty result list must not move mail.
            if (chosen) choose(chosen.mailbox.id)
          }
        }}
      />

      <ul className={styles.results} ref={listRef}>
        {matches.map((entry, index) => (
          <li key={entry.mailbox.id}>
            <button
              type="button"
              className={cx(styles.result, index === highlighted && styles.highlighted)}
              onMouseEnter={() => {
                setHighlighted(index)
              }}
              onClick={() => {
                choose(entry.mailbox.id)
              }}
            >
              <span className={styles.name}>{entry.mailbox.displayName}</span>
              {/* The account, because folder names repeat across accounts — three "Archive"
                  rows with nothing to tell them apart is worse than no picker. */}
              <span className={styles.account}>{entry.account?.displayName ?? ''}</span>
            </button>
          </li>
        ))}

        {matches.length === 0 && <li className={styles.empty}>No mailbox matches that.</li>}
      </ul>
    </Sheet>
  )
}
