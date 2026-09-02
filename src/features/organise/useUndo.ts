import { useCallback, useEffect, useState } from 'react'

import type { Available } from '@/lib/generated/Available'
import { onChanged, performRedo, performUndo, undoAvailable } from '@/lib/organise'
import { useToast } from '@/ui'

/**
 * Ctrl+Z and Ctrl+Shift+Z. docs/01 §14.
 *
 * The stack itself lives in the core, not here. Undo has to reverse a *database* change, and a
 * stack held in React would be lost on a reload and would know nothing about changes made by
 * the compose window, which is a separate OS window with its own React tree.
 *
 * The keyboard handler deliberately ignores events from inside a text field. Ctrl+Z in a
 * message list means "put that message back"; Ctrl+Z in a search box means "undo my typing",
 * and taking that away would be maddening.
 */
export function useUndo(): {
  available: Available
  undo: () => void
  redo: () => void
} {
  const [available, setAvailable] = useState<Available>({ undo: null, redo: null })
  const toast = useToast()

  const refresh = useCallback(() => {
    void undoAvailable().then(setAvailable)
  }, [])

  useEffect(() => {
    refresh()
    const unlisten = onChanged('mailbox:changed', refresh)

    return () => {
      void unlisten.then((stop) => {
        stop()
      })
    }
  }, [refresh])

  const run = useCallback(
    (action: () => Promise<string | null>, verb: string) => {
      void action()
        .then((label) => {
          if (label !== null) {
            toast.show({ title: `${verb} ${label}` })
          }
          refresh()
        })
        .catch((error: unknown) => {
          toast.show({
            title: `Nothing could be ${verb.toLowerCase()}`,
            description: error instanceof Error ? error.message : String(error),
          })
        })
    },
    [refresh, toast],
  )

  const undo = useCallback(() => {
    run(performUndo, 'Undid')
  }, [run])

  const redo = useCallback(() => {
    run(performRedo, 'Redid')
  }, [run])

  // Ctrl+Z is bound by `useShortcuts`, not here.
  //
  // This hook used to register a second `window` keydown listener of its own for the same chord.
  // Both fired: they are separate listeners on the same target, and `preventDefault` does not
  // stop a sibling. One keypress ran `undo()` twice, so with two things on the stack a single
  // Ctrl+Z took back two actions — archiving two messages and pressing it once returned both.
  //
  // It hid behind the shape of the stack. With one step, the second call found nothing and did
  // nothing, which is the case anyone testing by hand reaches for first.
  //
  // `shortcuts.ts` is the one table: the dispatcher binds from it and the Help sheet is rendered
  // from it, so a chord that lives anywhere else is invisible to both.

  return { available, undo, redo }
}
