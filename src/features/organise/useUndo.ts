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

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.ctrlKey || event.altKey) return
      if (event.key.toLowerCase() !== 'z') return

      // A text field owns its own undo. Stealing Ctrl+Z from a half-written search or a
      // compose body to put a message back in the Inbox would be maddening.
      const target = event.target
      if (target instanceof HTMLElement) {
        const editable =
          target.isContentEditable ||
          target instanceof HTMLInputElement ||
          target instanceof HTMLTextAreaElement
        if (editable) return
      }

      event.preventDefault()
      if (event.shiftKey) redo()
      else undo()
    }

    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
    }
  }, [undo, redo])

  return { available, undo, redo }
}
