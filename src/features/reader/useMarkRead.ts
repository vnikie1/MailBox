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
 */

/** Long enough to skip past a message, short enough to feel like no delay at all. */
const DWELL_MS = 700

export function useMarkRead(messages: MessageFull[]): void {
  const setFlags = useSetFlags()

  // The mutation object is new on every render, so it cannot be an effect dependency without
  // restarting the timer continuously — which would mean it never fires.
  const mutate = useRef(setFlags.mutate)
  mutate.current = setFlags.mutate

  // Only the unread ones, and only their ids: the effect must not re-run because some other
  // field of a message changed. `join` gives a value that compares by content.
  const unread = messages
    .filter((message) => !message.seen)
    .map((message) => message.id)
    .join(',')

  useEffect(() => {
    if (unread === '') return

    const ids = unread.split(',').map(Number)

    const timer = window.setTimeout(() => {
      // `answered` and `flagged` are left alone. A patch that carried them would overwrite a
      // flag the user set in another client between the message opening and this firing.
      mutate.current({ ids, patch: { seen: true, flagged: null } })
    }, DWELL_MS)

    return () => {
      window.clearTimeout(timer)
    }
  }, [unread])
}
