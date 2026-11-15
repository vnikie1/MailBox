import {
  useEffect,
  useId,
  useRef,
  useState,
  type ClipboardEvent,
  type KeyboardEvent,
  type PointerEvent,
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

/** A value as the field shows and compares it. See `TokenFieldProps.read`. */
export interface TokenReading {
  label: string
  key: string
}

function readAsIs(value: string): TokenReading {
  return { label: value, key: value.trim().toLowerCase() }
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
   * What a committed value's chip shows, and what makes two values the same one.
   *
   * Defaults to the value itself, compared without regard to case. The compose window reads a
   * recipient for its display name and its address: `Ada <ada@example.test>` shows as "Ada",
   * and is the same recipient as `ada@example.test`. A value already in the field is not added
   * a second time.
   */
  read?: (value: string) => TokenReading
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
   * Omitted, the field behaves exactly as before and its chips do not drag. See `dropTargets`
   * for how a drag works, and why it is not HTML5 drag and drop.
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
 * Where a separator is not one: inside double quotes or angle brackets.
 *
 * `"Smith, John" <john@example.com>` is one recipient whose name has a comma in it, and quoting
 * the name is how mail says so. Splitting there turned it into two broken chips, a red
 * `"Smith` and a `John" <john@example.com>` (M44, 2026-11-14 audit). A backslash inside quotes
 * escapes the next character, as it does in a header.
 */
function scan(text: string, onSeparator?: (at: number) => void): { open: boolean } {
  let quoted = false
  let bracketed = false
  let escaped = false

  for (let at = 0; at < text.length; at += 1) {
    const char = text.charAt(at)

    if (escaped) {
      escaped = false
    } else if (quoted && char === '\\') {
      escaped = true
    } else if (char === '"' && !bracketed) {
      quoted = !quoted
    } else if (char === '<' && !quoted) {
      bracketed = true
    } else if (char === '>' && !quoted) {
      bracketed = false
    } else if (!quoted && !bracketed && /[,;\n\r]/.test(char)) {
      onSeparator?.(at)
    }
  }

  return { open: quoted || bracketed }
}

/** Pasted text, cut at the separators that are not inside quotes or brackets. */
function splitTokens(text: string): string[] {
  const parts: string[] = []
  let from = 0

  scan(text, (at) => {
    parts.push(text.slice(from, at))
    from = at + 1
  })
  parts.push(text.slice(from))

  return parts
}

let nextTokenId = 0

function createToken(
  raw: string,
  read: (value: string) => TokenReading,
  validate?: (value: string) => boolean,
): Token | null {
  const value = raw
    .trim()
    .replace(/[,;]+$/, '')
    .trim()
  if (value.length === 0) return null

  nextTokenId += 1
  return {
    id: `token-${nextTokenId}`,
    // The name, where there is one. The chip used to show the whole value, so a suggestion
    // committed as `Ada Lovelace <ada@example.test>` showed exactly that, and its avatar took
    // its initials from "Ada" and "<ada@example.test>" (M44).
    label: read(value).label,
    value,
    invalid: validate ? !validate(value) : false,
  }
}

/**
 * Moving a chip to another field of its group, with the pointer.
 *
 * **Pointer events, not HTML5 drag and drop.** The compose window keeps Tauri's native drop
 * handler on, on purpose: a file dragged in from Explorer reaches the attachment code with its
 * path only through that handler, and attachments are sent by path. The handler also takes every
 * HTML5 drag inside the page, so `dragstart` never fired and no chip could be moved in the app at
 * all — while the browser tests, which have no Tauri, passed (M48, 2026-11-14 audit). The same
 * drag dropped back on its own field was worse: the field accepted it, ignored it as a duplicate,
 * and the source, told the move had happened, deleted the chip.
 *
 * So a press on a chip that travels past `DRAG_THRESHOLD_PX` becomes a drag, captured on the
 * field it started in; the field under the pointer is found by `elementFromPoint` and its
 * `data-token-field`; and on release the chip is handed to that field, through this registry, and
 * taken out of its own. Released over its own field, or anywhere else, it stays where it was.
 */
interface DropTarget {
  group: string
  /** Adds a chip from another field, unless this field has its address already. */
  receive: (token: Token) => void
  /** Lights the field while a chip is held over it. */
  highlight: (on: boolean) => void
}

/** Every field that takes chips, by id. The source and the target share nothing else. */
const dropTargets = new Map<string, DropTarget>()

/** How far a press must travel to become a drag, so a click on a chip is still a click. */
const DRAG_THRESHOLD_PX = 4

/** The field of `group` under a point, by id. */
function fieldAt(x: number, y: number, group: string): string | null {
  const field = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-token-field]')
  if (field?.dataset.tokenGroup !== group) return null
  return field.dataset.tokenField ?? null
}

/** A chip being pressed, and once it has moved far enough, dragged. */
interface Held {
  pointerId: number
  x: number
  y: number
  token: Token
  dragging: boolean
  /** The other field it is over, if any. */
  over: string | null
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
  read = readAsIs,
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

  const fieldId = useId()

  /** Whether a value is one the list already holds, by `read`'s key. */
  const holds = (list: Token[], value: string) => {
    const key = read(value).key
    return list.some((token) => read(token.value).key === key)
  }

  const commit = (raw: string) => {
    // Cut where a paste is cut. Typing a separator commits as it goes, so typed text reaches here
    // with none; but text put in whole — dropped in, dictated, set by an input method — keeps
    // its commas, and was committed as one chip holding every address (found driving the
    // release build after batch 8). A comma inside quotes is still part of the name.
    const created: Token[] = []
    let repeated: Token | null = null
    for (const part of splitTokens(raw)) {
      const token = createToken(part, read, validate)
      if (token === null) continue
      // Already there: the typing is taken, and nothing is added. Two chips for one address
      // sent the message to it twice (L21).
      if (holds([...tokens, ...created], token.value)) repeated = token
      else created.push(token)
    }

    const first = created[0]
    if (first === undefined && repeated === null) return false

    setDraft('')
    setSelectedId(null)

    if (first === undefined) {
      if (repeated !== null) setAnnouncement(`${repeated.label} is already in ${label}`)
      return true
    }

    onTokensChange([...tokens, ...created])
    setAnnouncement(
      created.length === 1 ? `${first.label} added` : `${String(created.length)} recipients added`,
    )
    return true
  }

  /* ------------------------------------------------- moving chips between fields */

  const [draggingId, setDraggingId] = useState<string | null>(null)
  const held = useRef<Held | null>(null)

  /**
   * Takes a chip dragged here from another field. Through a ref, because the registry keeps one
   * function for the life of the field and the list it adds to changes with every render.
   */
  const receive = useRef<(token: Token) => void>(() => undefined)
  useEffect(() => {
    receive.current = (dropped) => {
      setAnnouncement(`${dropped.label} moved to ${label}`)
      if (holds(tokens, dropped.value)) return

      // A fresh id: this is now this field's chip, and the address may be in two fields at once.
      nextTokenId += 1
      onTokensChange([...tokens, { ...dropped, id: `token-${String(nextTokenId)}` }])
    }
  })

  useEffect(() => {
    if (dragGroup === undefined || disabled) return

    dropTargets.set(fieldId, {
      group: dragGroup,
      receive: (token) => {
        receive.current(token)
      },
      highlight: setDragOver,
    })
    return () => {
      dropTargets.delete(fieldId)
    }
  }, [fieldId, dragGroup, disabled])

  const pressChip = (event: PointerEvent<HTMLElement>, token: Token) => {
    // The primary button, and not on the chip's remove button, which stays a button.
    if (event.button !== 0) return
    if (event.target instanceof Element && event.target.closest('button') !== null) return

    held.current = {
      pointerId: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      token,
      dragging: false,
      over: null,
    }
  }

  const lightOnly = (over: string | null) => {
    const current = held.current
    if (current === null || current.over === over) return
    if (current.over !== null) dropTargets.get(current.over)?.highlight(false)
    if (over !== null) dropTargets.get(over)?.highlight(true)
    current.over = over
  }

  /** The other field of this group under the pointer, if any. Never this one. */
  const otherFieldAt = (event: PointerEvent<HTMLElement>) => {
    if (dragGroup === undefined) return null
    const over = fieldAt(event.clientX, event.clientY, dragGroup)
    return over === fieldId ? null : over
  }

  const movePointer = (event: PointerEvent<HTMLDivElement>) => {
    const current = held.current
    if (current?.pointerId !== event.pointerId) return

    if (!current.dragging) {
      const travelled = Math.hypot(event.clientX - current.x, event.clientY - current.y)
      if (travelled < DRAG_THRESHOLD_PX) return

      current.dragging = true
      // On the field, which has a box — a chip's <li> is `display: contents` and has none — so
      // the release arrives here wherever in the window it happens.
      event.currentTarget.setPointerCapture(event.pointerId)
      setDraggingId(current.token.id)
    }

    lightOnly(otherFieldAt(event))
  }

  const releasePointer = (event: PointerEvent<HTMLDivElement>, dropped: boolean) => {
    const current = held.current
    if (current?.pointerId !== event.pointerId) return

    const target = dropped && current.dragging ? otherFieldAt(event) : null
    lightOnly(null)
    held.current = null
    if (!current.dragging) return

    setDraggingId(null)

    // Over its own field, or over nothing that takes it: the chip stays where it was.
    if (target === null) return

    dropTargets.get(target)?.receive(current.token)
    onTokensChange(tokens.filter((token) => token.id !== current.token.id))
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

    // A comma inside a quoted name is part of the name. Enter still commits: it is how the
    // user says the address is finished, quotes or not.
    if (event.key === 'Enter' || (SEPARATORS.has(event.key) && !scan(draft).open)) {
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
    const parts = splitTokens(text)
    if (parts.length < 2) return

    event.preventDefault()
    const created: Token[] = []
    for (const part of parts) {
      const token = createToken(part, read, validate)
      if (token !== null && !holds([...tokens, ...created], token.value)) created.push(token)
    }

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
              // Found under the pointer by `fieldAt`. See `dropTargets`.
              'data-token-field': fieldId,
              'data-token-group': dragGroup,
              onPointerMove: movePointer,
              onPointerUp: (event: PointerEvent<HTMLDivElement>) => {
                releasePointer(event, true)
              },
              onPointerCancel: (event: PointerEvent<HTMLDivElement>) => {
                releasePointer(event, false)
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
                    onPointerDown: (event: PointerEvent<HTMLLIElement>) => {
                      pressChip(event, token)
                    },
                  })}
            >
              <Chip
                label={token.label}
                className={cx(
                  dragGroup !== undefined && !disabled && styles.movable,
                  token.id === draggingId && styles.dragging,
                )}
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
