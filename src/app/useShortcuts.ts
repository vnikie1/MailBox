import { useEffect, useRef } from 'react'

import { SHORTCUTS, matches, parseChord, type ShortcutId } from './shortcuts'

/**
 * Binds the shortcut table to handlers. docs/01 §14, docs/06 Phase 10.
 *
 * One listener for the whole application rather than one per feature, so the order in which
 * components mount cannot change which shortcut wins.
 *
 * ## Text fields keep their own keys
 *
 * Nothing fires while the caret is in an input, a textarea or the editor. Ctrl+U in a message
 * list means "mark unread"; in a compose body it means underline, and stealing it would be
 * maddening. The exceptions are the few chords that are *about* the field — Ctrl+Enter sends
 * the message being typed — and those are handled by the field itself, which is why they are
 * marked `local` in the table and never bound here.
 */

export type Handlers = Partial<Record<ShortcutId, () => void>>

export interface ShortcutContext {
  /** True when at least one message is selected. */
  hasSelection: boolean
}

function inTextField(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false

  return (
    target.isContentEditable ||
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement
  )
}

export function useShortcuts(handlers: Handlers, context: ShortcutContext): void {
  // Held in a ref so the listener is registered once. Handlers are new closures on every
  // render, and re-binding on each one would tear down and rebuild the listener continuously.
  const current = useRef(handlers)
  current.current = handlers

  const state = useRef(context)
  state.current = context

  useEffect(() => {
    // Parsed once. Doing it inside the listener would re-split every string on every keystroke,
    // which for a fast typist is a few hundred allocations a second for a constant answer.
    const bound = SHORTCUTS.filter((shortcut) => shortcut.local !== true)
      .map((shortcut) => ({ shortcut, chord: parseChord(shortcut.keys) }))
      .filter(
        (entry): entry is typeof entry & { chord: NonNullable<typeof entry.chord> } =>
          entry.chord !== null,
      )

    const onKey = (event: KeyboardEvent) => {
      if (inTextField(event.target)) return

      for (const { shortcut, chord } of bound) {
        if (!matches(event, chord)) continue

        const handler = current.current[shortcut.id]
        if (handler === undefined) return

        // A shortcut that needs a selection and has none does nothing *and still swallows the
        // key*. Letting it fall through would send Delete to the browser's own handling, and
        // the user would see the page navigate rather than nothing happen.
        if (shortcut.requires?.selection === true && !state.current.hasSelection) {
          event.preventDefault()
          return
        }

        event.preventDefault()
        handler()
        return
      }
    }

    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
    }
  }, [])
}
