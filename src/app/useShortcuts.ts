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

export type Handlers = Partial<Record<Exclude<ShortcutId, 'jumpToMailbox'>, () => void>> & {
  /**
   * Ctrl+1 to Ctrl+9, given the 1-based position in the Favourites list.
   *
   * Typed apart from the rest because it is the one row in the table that is a *range*. That
   * is also why it was broken: `parseChord` returns null for `Ctrl+1-9`, so the table skipped
   * it, no handler existed, and the Help sheet advertised a shortcut that could not fire.
   */
  jumpToMailbox?: (position: number) => void
}

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

/**
 * Whether a modal sheet is open over the application.
 *
 * Every action in the table acts on the message list, and none of them should reach it from
 * behind a sheet. Delete pressed while "Move message to…" was open deleted the selection the
 * picker was about to move — the sheet stayed open over the result, so the only visible effect
 * was that choosing a folder afterwards did nothing.
 *
 * Asked of the document rather than of the event's target. A sheet traps focus, but the target
 * can still be the body — after clicking the overlay, say — and a destructive shortcut must not
 * depend on exactly where focus landed.
 *
 * `Sheet` marks itself through Floating UI's `useRole({ role: 'dialog' })`, so this recognises
 * every sheet in the app without any of them having to register.
 */
function modalIsOpen(): boolean {
  return document.querySelector('[role="dialog"], [role="alertdialog"]') !== null
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

      // Escape and the sheet's own keys belong to the sheet; everything in the table below
      // belongs to the list behind it, and nothing in the table should fire while one is open.
      if (modalIsOpen()) return

      // Ctrl+1 to Ctrl+9, dispatched here rather than from the table because one row stands
      // for nine chords and `parseChord` cannot express that. Handled before the loop so a
      // digit never falls through to a chord that happens to match.
      if (event.ctrlKey && !event.shiftKey && !event.altKey && /^[1-9]$/.test(event.key)) {
        const jump = current.current.jumpToMailbox
        if (jump === undefined) return

        event.preventDefault()
        jump(Number(event.key))
        return
      }

      for (const { shortcut, chord } of bound) {
        if (!matches(event, chord)) continue

        // Handled above, as nine chords rather than one. Excluded here so the lookup below
        // keeps the plain `() => void` handler type the rest of the table uses.
        if (shortcut.id === 'jumpToMailbox') continue

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
