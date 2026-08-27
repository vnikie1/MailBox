import {
  useId,
  useRef,
  useState,
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
} from 'react'

import { cx } from '@/lib/cx'

import { Avatar } from './Avatar'
import { Chip } from './Chip'

import styles from './TokenField.module.css'

export interface Token {
  id: string
  /** What the chip shows — a display name where there is one, else the address. */
  label: string
  /** What the token actually is. The address, for a recipient. */
  value: string
  invalid?: boolean
}

/** One row of the suggestion list. */
export interface Suggestion {
  /** What is committed when it is chosen. */
  value: string
  /** The person's name, where there is one. */
  label: string
  /** The address, shown beneath the name so two people with one name can be told apart. */
  detail?: string
}

export interface TokenFieldProps {
  /** The row label, "To:" or "Cc:". docs/02 §6.7 — fixed 60 wide, right-aligned. */
  label: string
  tokens: Token[]
  onTokensChange: (tokens: Token[]) => void
  placeholder?: string
  /** Decides whether a committed value is usable. Anything else renders as invalid. */
  validate?: (value: string) => boolean
  /**
   * Suggestions for what is being typed, newest query first.
   *
   * Optional, and the field is unchanged without it: the mailbox is the only address book this
   * app has, so only compose supplies them. The field asks via  rather than
   * fetching itself, because a shared primitive that knew how to query contacts would be a
   * primitive that could only ever be used for recipients.
   */
  suggestions?: Suggestion[]
  /** Called as the typed text changes, so the caller can fetch suggestions. */
  onDraftChange?: (value: string) => void
  /** Show a 16px avatar on each chip, as the compose window does. */
  showAvatars?: boolean
  /**
   * Lets chips be dragged between fields that share this name.
   *
   * Named rather than a bare boolean so a chip cannot be dropped somewhere unrelated. To, Cc
   * and Bcc are one group because moving a recipient between them is the whole point; a tag
   * field elsewhere in the app is not, and dropping an address into it would be nonsense the
   * user has to undo.
   *
   * Omitted, the field behaves exactly as before and its chips do not drag.
   */
  dragGroup?: string
  disabled?: boolean
  className?: string | undefined
}

/**
 * Separators that end a token as you type. Comma and semicolon are what every mail client
 * accepts and what pasted address lists are separated by; whitespace is not one, because
 * "Ada Lovelace <ada@example.com>" has to survive being typed in full.
 */
const SEPARATORS = new Set([',', ';'])

/**
 * The drag payload's MIME type, scoped to the group.
 *
 * A custom type rather than `text/plain`, and scoped rather than shared: it means `dragover`
 * can ask "is this one of mine?" before offering to accept, so dragging a file or a line of
 * text from another window never lights up a recipient field as a drop target.
 */
function mimeFor(group: string): string {
  return `application/x-halcyon-token+${group.toLowerCase()}`
}

/** Reads a dragged chip back, tolerating anything that is not one. */
function parseDragged(raw: string): Token | null {
  try {
    const parsed: unknown = JSON.parse(raw)

    if (
      typeof parsed !== 'object' ||
      parsed === null ||
      typeof (parsed as { value?: unknown }).value !== 'string' ||
      typeof (parsed as { label?: unknown }).label !== 'string'
    ) {
      return null
    }

    const { value, label } = parsed as { value: string; label: string }

    // A fresh id: the chip is being added to *this* field's list, and reusing the source's id
    // would collide the moment the same address exists in two fields at once.
    return { id: `${label}-${value}-${String(Math.random()).slice(2, 8)}`, value, label }
  } catch {
    return null
  }
}

let nextTokenId = 0

function createToken(raw: string, validate?: (value: string) => boolean): Token | null {
  const value = raw
    .trim()
    .replace(/[,;]+$/, '')
    .trim()
  if (value.length === 0) return null

  nextTokenId += 1
  return {
    id: `token-${nextTokenId}`,
    label: value,
    value,
    invalid: validate ? !validate(value) : false,
  }
}

/**
 * A recipient field. docs/02 §6.7.
 *
 * The selection model is the macOS one and it is worth stating precisely, because it is
 * the part everyone gets wrong: Backspace in an empty field *selects* the last chip, and
 * only a second Backspace deletes it. One keystroke never destroys a recipient, which
 * matters when the field is one Tab away from a Send button.
 *
 * Chips are not individually tabbable — nine recipients would otherwise cost nine tab
 * stops to cross. Arrow keys walk them instead, which is both what macOS does and what
 * `aria-activedescendant`-style widgets are expected to do.
 */
export function TokenField({
  label,
  tokens,
  onTokensChange,
  placeholder,
  validate,
  suggestions = [],
  onDraftChange,
  showAvatars = false,
  dragGroup,
  disabled = false,
  className,
}: TokenFieldProps) {
  const [draft, setDraft] = useState('')
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [dragOver, setDragOver] = useState(false)
  const [announcement, setAnnouncement] = useState('')
  // Which suggestion is highlighted. -1 means none, so Enter commits what was typed rather
  // than whatever happens to be first in a list the user has not looked at.
  const [highlighted, setHighlighted] = useState(-1)

  const inputRef = useRef<HTMLInputElement>(null)
  const labelId = useId()
  const inputId = useId()
  const hintId = useId()

  const selectedIndex = tokens.findIndex((token) => token.id === selectedId)

  const commit = (raw: string) => {
    const token = createToken(raw, validate)
    if (!token) return false

    onTokensChange([...tokens, token])
    setDraft('')
    setSelectedId(null)
    setAnnouncement(`${token.label} added`)
    return true
  }

  const removeAt = (index: number) => {
    const token = tokens[index]
    if (!token) return

    onTokensChange(tokens.filter((_, i) => i !== index))
    setAnnouncement(`${token.label} removed`)

    // Select the one before it, so repeated Backspace walks backwards through the list
    // the way it walks backwards through text.
    const previous = tokens[index - 1]
    setSelectedId(previous ? previous.id : null)
  }

  const select = (index: number | null) => {
    if (index === null) {
      setSelectedId(null)
      inputRef.current?.focus()
      return
    }

    const token = tokens[index]
    if (!token) return
    setSelectedId(token.id)
    setAnnouncement(`${token.label} selected. Press Backspace again to remove.`)
  }

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    const caretAtStart =
      event.currentTarget.selectionStart === 0 && event.currentTarget.selectionEnd === 0
    const atEmptyStart = draft.length === 0

    const open = suggestions.length > 0 && draft.trim().length > 0

    if (open && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
      event.preventDefault()
      setHighlighted((current) => {
        const step = event.key === 'ArrowDown' ? 1 : -1
        const next = current + step
        // Wraps, so holding one arrow key reaches everything without having to change hands.
        if (next < 0) return suggestions.length - 1
        if (next >= suggestions.length) return 0
        return next
      })
      return
    }

    if (open && event.key === 'Escape') {
      event.preventDefault()
      setHighlighted(-1)
      return
    }

    if (event.key === 'Enter' || SEPARATORS.has(event.key)) {
      // A highlighted suggestion wins over the typed text; nothing highlighted means the user
      // is typing an address the mailbox has never seen, which must still work.
      const chosen = open && highlighted >= 0 ? suggestions[highlighted] : undefined
      if (chosen !== undefined) {
        event.preventDefault()
        commit(chosen.value)
        setHighlighted(-1)
        return
      }

      if (commit(draft)) event.preventDefault()
      return
    }

    // Tab commits what is typed and then moves on, rather than silently discarding it.
    if (event.key === 'Tab' && draft.trim().length > 0) {
      commit(draft)
      return
    }

    if (event.key === 'Backspace' && atEmptyStart) {
      event.preventDefault()
      if (selectedIndex >= 0) removeAt(selectedIndex)
      else if (tokens.length > 0) select(tokens.length - 1)
      return
    }

    if (event.key === 'Delete' && atEmptyStart && selectedIndex >= 0) {
      event.preventDefault()
      removeAt(selectedIndex)
      return
    }

    if (event.key === 'ArrowLeft' && caretAtStart) {
      event.preventDefault()
      if (selectedIndex > 0) select(selectedIndex - 1)
      else if (selectedIndex < 0 && tokens.length > 0) select(tokens.length - 1)
      return
    }

    if (event.key === 'ArrowRight' && selectedIndex >= 0) {
      event.preventDefault()
      select(selectedIndex === tokens.length - 1 ? null : selectedIndex + 1)
      return
    }

    if (event.key === 'Escape' && selectedIndex >= 0) {
      // Do not let this reach a surrounding sheet or compose window: clearing the
      // selection is what Escape means here, and closing the window is not.
      event.preventDefault()
      event.stopPropagation()
      select(null)
    }
  }

  /**
   * A pasted address list arrives as one string. Splitting it here is the difference
   * between pasting nine recipients and pasting one nonsense recipient nine addresses
   * long — and it is the same split the separators above perform while typing.
   */
  const handlePaste = (event: ClipboardEvent<HTMLInputElement>) => {
    const text = event.clipboardData.getData('text')
    if (!/[,;\n]/.test(text)) return

    event.preventDefault()
    const created = text
      .split(/[,;\n]+/)
      .map((part) => createToken(part, validate))
      .filter((token): token is Token => token !== null)

    if (created.length === 0) return

    onTokensChange([...tokens, ...created])
    setDraft('')
    setAnnouncement(`${created.length} recipients added`)
  }

  return (
    <div
      className={cx(styles.field, disabled && styles.disabled, className)}
      role="group"
      aria-labelledby={labelId}
    >
      <span id={labelId} className={styles.label}>
        {label}
      </span>

      <div
        className={cx(styles.body, dragOver && styles.dragOver)}
        {...(dragGroup === undefined || disabled
          ? {}
          : {
              onDragOver: (event: DragEvent<HTMLDivElement>) => {
                if (!event.dataTransfer.types.includes(mimeFor(dragGroup))) return
                // Both calls are required. Without `preventDefault` the browser refuses the
                // drop entirely, and without `dropEffect` the source field cannot tell a
                // completed move from an abandoned one.
                event.preventDefault()
                event.dataTransfer.dropEffect = 'move'
                setDragOver(true)
              },
              onDragLeave: (event: DragEvent<HTMLDivElement>) => {
                // Ignore the events fired as the pointer crosses a child. Without this the
                // highlight flickers off and on over every chip the pointer passes.
                if (event.currentTarget.contains(event.relatedTarget as Node | null)) return
                setDragOver(false)
              },
              onDrop: (event: DragEvent<HTMLDivElement>) => {
                event.preventDefault()
                setDragOver(false)

                const raw = event.dataTransfer.getData(mimeFor(dragGroup))
                if (raw === '') return

                const dropped = parseDragged(raw)
                if (dropped === null) return

                // A chip dropped back where it started is a no-op rather than a duplicate.
                if (tokens.some((token) => token.value === dropped.value)) return

                onTokensChange([...tokens, dropped])
              },
            })}
        onMouseDown={(event) => {
          // Clicking the empty space to the right of the last chip should put the caret
          // in the field, exactly as clicking the padding of a text input does.
          if (event.target === event.currentTarget) {
            event.preventDefault()
            inputRef.current?.focus()
          }
        }}
      >
        <ul className={styles.tokens}>
          {tokens.map((token, index) => (
            <li
              key={token.id}
              className={styles.token}
              {...(dragGroup === undefined || disabled
                ? {}
                : {
                    draggable: true,
                    onDragStart: (event: DragEvent<HTMLLIElement>) => {
                      event.dataTransfer.effectAllowed = 'move'
                      event.dataTransfer.setData(
                        mimeFor(dragGroup),
                        JSON.stringify({ value: token.value, label: token.label }),
                      )
                    },
                    onDragEnd: (event: DragEvent<HTMLLIElement>) => {
                      // Removed here rather than in the drop handler, because the drop lands in
                      // a *different* component that has no way to reach back into this one.
                      // `dropEffect` is the only signal that the drag was accepted somewhere;
                      // a drag abandoned over the desktop reports `none` and the chip stays.
                      if (event.dataTransfer.dropEffect !== 'move') return
                      onTokensChange(tokens.filter((other) => other.id !== token.id))
                    },
                  })}
            >
              <Chip
                label={token.label}
                tone={token.invalid === true ? 'invalid' : 'neutral'}
                selected={token.id === selectedId}
                aria-current={token.id === selectedId}
                {...(showAvatars ? { leading: <Avatar name={token.label} size="sm" /> } : {})}
                {...(disabled
                  ? {}
                  : {
                      onRemove: () => {
                        removeAt(index)
                        inputRef.current?.focus()
                      },
                    })}
              />
            </li>
          ))}
        </ul>

        <input
          ref={inputRef}
          id={inputId}
          type="text"
          className={styles.input}
          value={draft}
          disabled={disabled}
          aria-describedby={hintId}
          {...(placeholder === undefined || tokens.length > 0 ? {} : { placeholder })}
          onChange={(event) => {
            setDraft(event.target.value)
            setSelectedId(null)
            setHighlighted(-1)
            onDraftChange?.(event.target.value)
          }}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          onBlur={() => {
            commit(draft)
            setSelectedId(null)
            setHighlighted(-1)
          }}
        />
      </div>

      {suggestions.length > 0 && draft.trim().length > 0 && (
        <ul className={styles.suggestions} role="listbox" aria-label={`${label} suggestions`}>
          {suggestions.map((suggestion, index) => (
            <li key={suggestion.value}>
              <button
                type="button"
                className={cx(styles.suggestion, index === highlighted && styles.highlighted)}
                role="option"
                aria-selected={index === highlighted}
                // Pointer-down rather than click: the input's blur fires first on a click and
                // would commit the half-typed text, closing the list before the choice lands.
                onPointerDown={(event) => {
                  event.preventDefault()
                  commit(suggestion.value)
                  setHighlighted(-1)
                }}
              >
                <span className={styles.suggestionLabel}>{suggestion.label}</span>
                {suggestion.detail !== undefined && (
                  <span className={styles.suggestionDetail}>{suggestion.detail}</span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}

      <span id={hintId} className="srOnly">
        Type an address and press Enter. Press Backspace to select the previous recipient.
      </span>
      <span className="srOnly" role="status" aria-live="polite">
        {announcement}
      </span>
    </div>
  )
}
