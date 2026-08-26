import { useId, useRef, useState, type ClipboardEvent, type KeyboardEvent } from 'react'

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

export interface TokenFieldProps {
  /** The row label, "To:" or "Cc:". docs/02 §6.7 — fixed 60 wide, right-aligned. */
  label: string
  tokens: Token[]
  onTokensChange: (tokens: Token[]) => void
  placeholder?: string
  /** Decides whether a committed value is usable. Anything else renders as invalid. */
  validate?: (value: string) => boolean
  /** Show a 16px avatar on each chip, as the compose window does. */
  showAvatars?: boolean
  disabled?: boolean
  className?: string | undefined
}

/**
 * Separators that end a token as you type. Comma and semicolon are what every mail client
 * accepts and what pasted address lists are separated by; whitespace is not one, because
 * "Ada Lovelace <ada@example.com>" has to survive being typed in full.
 */
const SEPARATORS = new Set([',', ';'])

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
  showAvatars = false,
  disabled = false,
  className,
}: TokenFieldProps) {
  const [draft, setDraft] = useState('')
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [announcement, setAnnouncement] = useState('')

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

    if (event.key === 'Enter' || SEPARATORS.has(event.key)) {
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
        className={styles.body}
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
            <li key={token.id} className={styles.token}>
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
          }}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          onBlur={() => {
            commit(draft)
            setSelectedId(null)
          }}
        />
      </div>

      <span id={hintId} className="srOnly">
        Type an address and press Enter. Press Backspace to select the previous recipient.
      </span>
      <span className="srOnly" role="status" aria-live="polite">
        {announcement}
      </span>
    </div>
  )
}
