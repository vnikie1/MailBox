import { useEffect } from 'react'

/**
 * The Phase 8 keyboard shortcuts. docs/01 §8, docs/06 Phase 8.
 *
 * | keys | what |
 * |---|---|
 * | Ctrl+Shift+M | move the selection to a mailbox chosen by typing |
 * | Alt+Ctrl+L | run the rules over the selection |
 *
 * Handlers are registered on the window rather than on a focused element, because both act on
 * the *selection*, and the selection stays put while focus moves between the list, the reader
 * and the sidebar. A shortcut that only works while the list has focus is one people learn is
 * unreliable and stop using.
 *
 * Both are ignored while the caret is in a text field. Ctrl+Shift+M in a search box would move
 * mail the user is not looking at, on a keystroke they meant for the text.
 */

export interface OrganiseShortcuts {
  /** Nothing happens when the selection is empty; both actions need something to act on. */
  hasSelection: boolean
  onMoveTo: () => void
  onRunRules: () => void
}

function inTextField(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false

  return (
    target.isContentEditable ||
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement
  )
}

export function useOrganiseShortcuts({
  hasSelection,
  onMoveTo,
  onRunRules,
}: OrganiseShortcuts): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.ctrlKey || inTextField(event.target)) return

      const key = event.key.toLowerCase()

      // Alt+Ctrl+L — run rules. Checked before the Ctrl+Shift+M case because Alt is the
      // discriminator, and a browser that reports both modifiers would otherwise fall through.
      if (event.altKey && key === 'l') {
        if (!hasSelection) return
        event.preventDefault()
        onRunRules()
        return
      }

      if (event.altKey) return

      if (event.shiftKey && key === 'm') {
        if (!hasSelection) return
        event.preventDefault()
        onMoveTo()
      }
    }

    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
    }
  }, [hasSelection, onMoveTo, onRunRules])
}
