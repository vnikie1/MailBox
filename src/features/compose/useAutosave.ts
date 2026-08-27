import { useCallback, useEffect, useRef } from 'react'

import type { OutgoingMessage } from '@/lib/generated/OutgoingMessage'
import { composeSaveDraft, runningInTauri } from '@/lib/ipc'

/**
 * Autosaves a draft. docs/01 §6 — *drafts autosave every ~30s and on blur.*
 *
 * Both triggers matter and neither is enough alone. A timer alone loses up to thirty seconds
 * when someone closes the window; blur alone loses everything when a machine dies with the
 * window still focused, which is exactly when a long message is being written.
 *
 * Three things this deliberately does **not** do:
 *
 * * **Save on every keystroke.** That is a database write and a queued IMAP append per
 *   character, and the queue would never drain faster than it filled.
 * * **Save when nothing changed.** A window left open overnight would otherwise append a
 *   fresh copy to the Drafts mailbox every thirty seconds, and the user would find hundreds of
 *   identical drafts on their phone.
 * * **Block anything on the result.** A failed save is worth a log line, not an interruption
 *   of someone's typing.
 */

/** docs/01 §6. */
const INTERVAL_MS = 30_000

export interface Autosave {
  /** The stable `Message-ID` for this draft, once one has been assigned. */
  messageId: () => string | null
  /** Saves now, if anything has changed. Used on blur and before sending. */
  saveNow: () => void
  /** Forgets the draft, so a sent message is not saved again on the way out. */
  abandon: () => void
}

export function useAutosave(build: () => OutgoingMessage | null): Autosave {
  const messageId = useRef<string | null>(null)
  const lastSaved = useRef<string>('')
  const abandoned = useRef(false)

  const save = useCallback(() => {
    if (!runningInTauri || abandoned.current) return

    const message = build()
    if (message === null) return

    // A draft with nothing in it is not a draft. Saving one would put an empty message in the
    // user's Drafts mailbox on every device, for a window they opened and closed.
    const substantive =
      message.subject.trim() !== '' ||
      (message.text ?? '').trim() !== '' ||
      message.to.length > 0 ||
      message.cc.length > 0 ||
      message.bcc.length > 0

    if (!substantive) return

    // Compared against the last saved shape rather than a dirty flag, because the editor
    // reports a change for a caret move as readily as for a typed character.
    const fingerprint = JSON.stringify([
      message.to,
      message.cc,
      message.bcc,
      message.subject,
      message.html,
    ])

    if (fingerprint === lastSaved.current) return
    lastSaved.current = fingerprint

    composeSaveDraft(message, messageId.current)
      .then((saved) => {
        messageId.current = saved.messageId
      })
      .catch((cause: unknown) => {
        // Worth a line in the console, never an interruption. The next save will try again.
        console.warn('[compose] draft not saved', cause)
      })
  }, [build])

  useEffect(() => {
    const timer = window.setInterval(save, INTERVAL_MS)

    // `blur` on the window rather than on a field: the user clicking to another application is
    // the moment a draft most needs to exist, and a field-level blur would miss it.
    window.addEventListener('blur', save)

    return () => {
      window.clearInterval(timer)
      window.removeEventListener('blur', save)
    }
  }, [save])

  return {
    messageId: () => messageId.current,
    saveNow: save,
    abandon: () => {
      abandoned.current = true
    },
  }
}
