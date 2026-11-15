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
  /**
   * Saves now, if anything has changed, and resolves once that save has finished — so Save as
   * Draft can wait for it before closing the window. Resolves false if the save failed, which
   * is the one case where closing would lose the message. A save already under way is waited for and
   * followed by another if the message has changed since.
   */
  saveNow: () => Promise<boolean>
  /**
   * Continues an existing draft — one reopened from a Drafts mailbox — so each save replaces it
   * rather than starting a second draft beside it.
   */
  adopt: (messageId: string) => void
  /** Forgets the draft, so a sent message is not saved again on the way out. */
  abandon: () => void
  /**
   * Undoes an abandon that turned out to be premature, after a send that failed before the
   * message reached the outbox. Only the failure path calls this: on the success path the
   * message has gone and must never be written back as a draft.
   */
  resume: () => void
}

/**
 * `touched` says whether the user has changed anything. Until they have, nothing is saved: a
 * reply opened and looked at is not a draft, and saving one whenever the window lost focus put
 * an unwritten reply in Drafts on every device (L25, 2026-11-14 audit).
 */
export function useAutosave(build: () => OutgoingMessage | null, touched: () => boolean): Autosave {
  const messageId = useRef<string | null>(null)
  const lastSaved = useRef<string>('')
  const abandoned = useRef(false)
  /** The save under way, if any. */
  const inFlight = useRef<Promise<boolean> | null>(null)

  const save = useCallback((): Promise<boolean> => {
    if (!runningInTauri || abandoned.current || !touched()) return Promise.resolve(true)

    // The in-flight guard replaces what used to be an early fingerprint commit. Both stop a
    // second save racing the first; only this one lets a failed save be retried. Waited for
    // rather than skipped, so a save asked for now still saves what is in the window now.
    if (inFlight.current !== null) return inFlight.current.then(save)

    const message = build()
    if (message === null) return Promise.resolve(true)

    // A draft with nothing in it is not a draft. Saving one would put an empty message in the
    // user's Drafts mailbox on every device, for a window they opened and closed.
    const substantive =
      message.subject.trim() !== '' ||
      (message.text ?? '').trim() !== '' ||
      message.to.length > 0 ||
      message.cc.length > 0 ||
      message.bcc.length > 0 ||
      message.attachments.length > 0

    if (!substantive) return Promise.resolve(true)

    // Compared against the last saved shape rather than a dirty flag, because the editor
    // reports a change for a caret move as readily as for a typed character.
    const fingerprint = JSON.stringify([
      message.to,
      message.cc,
      message.bcc,
      message.subject,
      message.html,
      // Drafts keep their files now, so attaching one is a change worth saving.
      message.attachments,
    ])

    if (fingerprint === lastSaved.current) return Promise.resolve(true)

    // Recorded only once the write has actually succeeded. Recording it before the call --
    // which is what this did -- meant the guard above suppressed every later save of unchanged
    // content, so a single transient failure left the draft unsaved for the life of the window
    // and killed the timer, the blur save and Save as Draft together. The comment below
    // promised a retry the code had just made impossible.
    const saving = composeSaveDraft(message, messageId.current)
      .then((saved) => {
        messageId.current = saved.messageId
        lastSaved.current = fingerprint
        return true
      })
      .catch((cause: unknown) => {
        // Worth a line in the console, never an interruption. The next save will try again --
        // and now it genuinely can, because the fingerprint was never committed.
        console.warn('[compose] draft not saved', cause)
        return false
      })
      .finally(() => {
        inFlight.current = null
      })

    inFlight.current = saving
    return saving
  }, [build, touched])

  // Stable, so a window can name it as an effect's dependency without re-running the effect on
  // every render.
  const adopt = useCallback((id: string) => {
    messageId.current = id
  }, [])

  useEffect(() => {
    // The result is not wanted here: a failed timed save is retried by the next one.
    const quietly = () => {
      void save()
    }

    const timer = window.setInterval(quietly, INTERVAL_MS)

    // `blur` on the window rather than on a field: the user clicking to another application is
    // the moment a draft most needs to exist, and a field-level blur would miss it.
    window.addEventListener('blur', quietly)

    return () => {
      window.clearInterval(timer)
      window.removeEventListener('blur', quietly)
    }
  }, [save])

  return {
    messageId: () => messageId.current,
    saveNow: save,
    adopt,
    abandon: () => {
      abandoned.current = true
    },
    resume: () => {
      abandoned.current = false
    },
  }
}
