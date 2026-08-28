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

  useEffect(() => {
    marked.current = new Set()
  }, [thread])

  // Only the unread ones this hook has not already handled.
  const unread = messages
    .filter((message) => !message.seen && !marked.current.has(message.id))
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
