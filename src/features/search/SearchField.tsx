import { Search } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'

import { cx } from '@/lib/cx'
import type { Suggestion } from '@/lib/generated/Suggestion'
import { rememberSearch, searchHistory, suggestSearch } from '@/lib/search'
import { TextField } from '@/ui'

import styles from './SearchField.module.css'

export interface SearchFieldProps {
  value: string
  onChange: (text: string) => void
  /** Enter, or a suggestion chosen: the search the user actually meant. */
  onCommit: (text: string) => void
}

/**
 * The search field and its dropdown. docs/02 §6.6, docs/06 Phase 9.
 *
 * ## The debounce is about the pointer, not the load
 *
 * Suggestions are cheap — the core answers in under a millisecond. They are debounced anyway,
 * because a list that changes on every keystroke moves under the pointer, and a click aimed at
 * one row lands on another. That is the failure people describe as "it searched for the wrong
 * thing", and it is not a performance problem.
 */

/** Long enough that a fast typist gets one lookup per word, short enough to feel immediate. */
const DEBOUNCE_MS = 90

/** Headers, in the order the groups appear. docs/02 §6.6 asks for them. */
const GROUP_LABELS: Record<Suggestion['kind'], string> = {
  text: 'Search for',
  token: 'Filters',
  person: 'People',
  mailbox: 'Mailboxes',
}

const GROUP_ORDER: Suggestion['kind'][] = ['text', 'token', 'person', 'mailbox']

export function SearchField({ value, onChange, onCommit }: SearchFieldProps) {
  const [suggestions, setSuggestions] = useState<Suggestion[]>([])
  const [history, setHistory] = useState<string[]>([])
  const [open, setOpen] = useState(false)
  const [highlighted, setHighlighted] = useState(-1)

  const timer = useRef<number | undefined>(undefined)
  // Rising sequence number, so a slow lookup cannot overwrite the answer to a later one — the
  // classic way an autocomplete ends up showing results for a prefix already typed past.
  const latest = useRef(0)

  useEffect(() => {
    void searchHistory(5).then(setHistory)
  }, [])

  useEffect(() => {
    window.clearTimeout(timer.current)

    if (value.trim() === '') {
      setSuggestions([])
      return
    }

    const sequence = latest.current + 1
    latest.current = sequence

    timer.current = window.setTimeout(() => {
      void suggestSearch(value, 8).then((found) => {
        if (sequence !== latest.current) return
        setSuggestions(found)
      })
    }, DEBOUNCE_MS)

    return () => {
      window.clearTimeout(timer.current)
    }
  }, [value])

  // Flattened in group order, which is the order the arrow keys walk. Building it once here
  // rather than in the key handler keeps the two from disagreeing about what "next" means.
  const rows: Suggestion[] =
    value.trim() === ''
      ? history.map((text) => ({
          kind: 'text' as const,
          label: text,
          insert: text,
          detail: null,
        }))
      : GROUP_ORDER.flatMap((kind) => suggestions.filter((item) => item.kind === kind))

  const commit = useCallback(
    (text: string) => {
      onChange(text)
      onCommit(text)
      void rememberSearch(text).then(() => searchHistory(5).then(setHistory))
      setOpen(false)
      setHighlighted(-1)
    },
    [onChange, onCommit],
  )

  /** Leaves the search: no text, no results, and nothing added to the recent searches. */
  const clear = () => {
    onChange('')
    onCommit('')
    setOpen(false)
    setHighlighted(-1)
  }

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      setOpen(true)
      setHighlighted((current) => Math.min(current + 1, rows.length - 1))
      return
    }

    if (event.key === 'ArrowUp') {
      event.preventDefault()
      setHighlighted((current) => Math.max(current - 1, -1))
      return
    }

    if (event.key === 'Enter') {
      event.preventDefault()
      // The highlighted row if one is, otherwise what was typed. Enter must never pick a row
      // the user has not moved to: a dropdown that hijacks Enter searches for something they
      // did not ask for.
      const chosen = highlighted >= 0 ? rows[highlighted] : undefined
      commit(chosen ? chosen.insert : value)
      return
    }

    // Escape closes the suggestions if they are up, and otherwise clears the search, as it
    // does in Mail. It only closed them, so leaving a search took selecting the text and
    // deleting it (L43, 2026-11-14 audit). Kept from the window once it has done either, so the
    // same press does not go on to close or deselect anything else.
    if (event.key === 'Escape') {
      if (open && rows.length > 0) {
        event.preventDefault()
        event.stopPropagation()
        setOpen(false)
        setHighlighted(-1)
        return
      }

      if (value !== '') {
        event.preventDefault()
        event.stopPropagation()
        clear()
      }
    }
  }

  let lastKind: Suggestion['kind'] | null = null

  return (
    <div className={styles.wrap}>
      <TextField
        label="Search"
        hideLabel
        variant="search"
        placeholder="Search"
        // How Ctrl+F finds this field. A data attribute rather than the placeholder, which is
        // display text: it changes with wording, it would change with translation, and the
        // selector that used it matched nothing at all — see the note in AppShell.
        data-shortcut="search"
        leadingIcon={Search}
        className={styles.field}
        value={value}
        onChange={(event) => {
          onChange(event.target.value)
          setOpen(true)
          setHighlighted(-1)
        }}
        onFocus={() => {
          setOpen(true)
        }}
        // Delayed, or the blur fires before the click on a row is delivered and choosing a
        // suggestion with the pointer never works.
        onBlur={() => {
          window.setTimeout(() => {
            setOpen(false)
          }, 120)
        }}
        onKeyDown={onKeyDown}
        // The field's own clear button. This passed a `trailing` button, which `TextField` has
        // no such prop for: it reached the input as an attribute, and there was no way to clear
        // a search but to delete it. The button is laid out always and shown only with text in
        // the field, so the field does not resize as the first letter is typed.
        onClear={clear}
        clearLabel="Clear search"
      />

      {open && rows.length > 0 && (
        <ul className={styles.dropdown} role="listbox" aria-label="Search suggestions">
          {rows.map((row, index) => {
            const header = row.kind !== lastKind ? GROUP_LABELS[row.kind] : null
            lastKind = row.kind

            return (
              <li key={`${row.kind}-${row.insert}-${String(index)}`}>
                {header !== null && (
                  <span className={styles.group} aria-hidden>
                    {value.trim() === '' && row.kind === 'text' ? 'Recent' : header}
                  </span>
                )}

                <button
                  type="button"
                  role="option"
                  aria-selected={index === highlighted}
                  className={cx(styles.row, index === highlighted && styles.highlighted)}
                  onMouseEnter={() => {
                    setHighlighted(index)
                  }}
                  onClick={() => {
                    commit(row.insert)
                  }}
                >
                  <span className={styles.label}>{row.label}</span>
                  {row.detail !== null && <span className={styles.detail}>{row.detail}</span>}
                </button>
              </li>
            )
          })}
        </ul>
      )}
    </div>
  )
}
