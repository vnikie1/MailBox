import { useCallback, useEffect, useRef, useState } from 'react'

import { contactsSuggest } from '@/lib/ipc'
import { TokenField, type Suggestion, type Token } from '@/ui'

/**
 * A `TokenField` that suggests people from the mailbox. docs/06 Phase 7.
 *
 * A thin wrapper rather than a change to `TokenField` itself. The field is a Phase 1 primitive
 * used wherever a list of things is typed; teaching it to query contacts would make it a
 * component that can only ever be used for recipients.
 *
 * The lookup is debounced, and that is not only about load. Suggestions that change on every
 * keystroke move under the pointer, so a click aimed at one row lands on another — the
 * failure people describe as "it added the wrong person".
 */

/** Long enough that a fast typist gets one query per word, short enough to feel immediate. */
const DEBOUNCE_MS = 120

export interface RecipientFieldProps {
  label: string
  tokens: Token[]
  onTokensChange: (tokens: Token[]) => void
  validate?: (value: string) => boolean
}

export function RecipientField({ label, tokens, onTokensChange, validate }: RecipientFieldProps) {
  const [suggestions, setSuggestions] = useState<Suggestion[]>([])
  const timer = useRef<number | undefined>(undefined)
  // Rising sequence number, so a slow query cannot overwrite the answer to a later one — the
  // classic way an autocomplete ends up showing results for a prefix the user has moved past.
  const latest = useRef(0)

  const onDraftChange = useCallback((value: string) => {
    window.clearTimeout(timer.current)

    const trimmed = value.trim()
    if (trimmed === '') {
      setSuggestions([])
      return
    }

    const sequence = latest.current + 1
    latest.current = sequence

    timer.current = window.setTimeout(() => {
      contactsSuggest(trimmed)
        .then((found) => {
          if (sequence !== latest.current) return

          setSuggestions(
            found.map((contact) => ({
              // Committed in the full form, so the chip keeps the name and the message carries
              // it. A bare address would lose the name the mailbox already knows.
              value:
                contact.name === null || contact.name.trim() === ''
                  ? contact.email
                  : `${contact.name} <${contact.email}>`,
              label:
                contact.name === null || contact.name.trim() === '' ? contact.email : contact.name,
              detail: contact.email,
            })),
          )
        })
        .catch(() => {
          // No suggestions is a perfectly good state; a failed lookup must not stop typing.
          setSuggestions([])
        })
    }, DEBOUNCE_MS)
  }, [])

  useEffect(
    () => () => {
      window.clearTimeout(timer.current)
    },
    [],
  )

  return (
    <TokenField
      label={label}
      tokens={tokens}
      onTokensChange={(next) => {
        // The list is stale the moment a chip is committed, and leaving it open would put a
        // dropdown over the field the user is about to type the next address into.
        setSuggestions([])
        onTokensChange(next)
      }}
      suggestions={suggestions}
      onDraftChange={onDraftChange}
      {...(validate === undefined ? {} : { validate })}
      // To, Cc and Bcc share one group, so a recipient can be moved between them by dragging
      // — which is how people fix "I meant to Cc them" without retyping the address.
      dragGroup="recipients"
      showAvatars
    />
  )
}
