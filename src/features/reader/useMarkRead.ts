import { useEffect, useRef } from 'react'

import { useSetFlags } from '@/app/queries'
import type { MessageFull } from '@/lib/generated/MessageFull'

/**
 * Marks a message read once it has actually been looked at. docs/01 §4.
 *
 * This was missing entirely: `useSetFlags` existed and nothing ever called it, so opening a
 * message left it bold for ever and the unread badge never came down. The only way to clear one
 * was from another client.
 *
 * ## Why a delay, when Mail marks read immediately
 *
 * Because arrow keys exist. Holding ↓ through twenty messages would, with no delay, mark all
 * twenty read and queue twenty `UID STORE` commands for messages nobody read — and undoing that
 * means finding each one again. The delay is short enough to feel immediate when a message is
 * actually being read, and long enough that passing over one leaves it alone.
 *
 * Cancelled on every change of selection, so only the message still on screen when the timer
 * fires is marked.
 *
 * ## Why it remembers what it has already marked
 *
 * Without that memory, Ctrl+U on the open message did nothing: marking it unread put it back
 * into this effect's dependency, the timer started again, and 700ms later it was read once
 * more. "Mark unread to deal with later" is the most common triage move there is and it was
 * impossible on the message you were looking at.
 *
 * So each message is auto-marked at most once per selection. A later unread is the user saying
 * so deliberately, and deliberate beats automatic. The memory clears when the selection changes,
 * which means re-opening a message reads it again — the same as Mail, and the right answer:
 * coming back to a message later is reading it, not un-deciding.
 *
 * ## Why remembering what it marked was not enough
 *
 * That memory only ever holds messages **this hook marked**, so it is empty for a message that
 * was already read when the selection opened — and there is nothing to mark in that case, so it
 * stays empty. Press Ctrl+U on such a message and the effect sees an unread id it does not
 * recognise, starts the timer, and 700ms later reads it again.
 *
 * Measured in the packaged app against the database: `flag_seen` went 1 → 0 at +250ms and was
 * back to 1 by +600ms. The message you are actually looking at could not be marked unread at
 * all, which is the same bug the memory was added to fix, reached from the other side.
 *
 * So eligibility is decided **once, when the selection opens**: only messages that were unread
 * at that moment are ever auto-marked. Anything that becomes unread later during the same
 * selection is the user, and the user is left alone.
 */

/** Long enough to skip past a message, short enough to feel like no delay at all. */
const DWELL_MS = 700

export function useMarkRead(messages: MessageFull[]): void {
  const setFlags = useSetFlags()

  // The mutation object is new on every render, so it cannot be an effect dependency without
  // restarting the timer continuously — which would mean it never fires.
  const mutate = useRef(setFlags.mutate)
  mutate.current = setFlags.mutate

  // What this hook has already marked, for the whole of the current selection.
  const marked = useRef(new Set<number>())

  // The thread on screen, as a value that compares by content — so the effect does not re-run
  // because some unrelated field changed.
  const thread = messages.map((message) => message.id).join(',')

  /**
   * The messages that were unread when this selection opened — the only ones ever auto-marked.
   *
   * Captured during render rather than in an effect, and that is load-bearing: an effect runs
   * *after* the render that first shows a new selection, so `unread` below would be computed
   * against the previous selection's eligibility for one render — long enough to start a timer
   * for a message that should never have had one.
   */
  const eligible = useRef<{ key: string; ids: Set<number> }>({ key: '', ids: new Set() })

  if (eligible.current.key !== thread) {
    eligible.current = {
      key: thread,
      ids: new Set(messages.filter((message) => !message.seen).map((message) => message.id)),
    }
    marked.current = new Set()
  }

  // Unread, eligible, and not already handled. The middle condition is what keeps the hook out
  // of a message the user marked unread while looking at it.
  const unread = messages
    .filter(
      (message) =>
        !message.seen && eligible.current.ids.has(message.id) && !marked.current.has(message.id),
    )
    .map((message) => message.id)
    .join(',')

  useEffect(() => {
    if (unread === '') return

    const ids = unread.split(',').map(Number)

    const timer = window.setTimeout(() => {
      // Recorded before the mutation, not after: the refresh that follows re-runs this effect,
      // and an id added later would arrive too late to keep it out.
      ids.forEach((id) => marked.current.add(id))

      // `answered` and `flagged` are left alone. A patch that carried them would overwrite a
      // flag the user set in another client between the message opening and this firing.
      mutate.current({ ids, patch: { seen: true, flagged: null } })
    }, DWELL_MS)

    return () => {
      window.clearTimeout(timer)
    }
  }, [unread])
}
