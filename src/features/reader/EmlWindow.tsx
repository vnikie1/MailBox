import { useEffect, useMemo, useState } from 'react'
import { AlertTriangle, Paperclip } from 'lucide-react'

import type { EmlMessage } from '@/lib/generated/EmlMessage'
import { emlRead } from '@/lib/ipc'
import { EmptyState } from '@/ui'

import { MessageFrame } from './MessageFrame'

import styles from './EmlWindow.module.css'

/**
 * A `.eml` file, opened from Explorer. docs/06 Phase 10.
 *
 * Read-only, and the reasons are in `ipc/eml.rs`: a file on disk is in no mailbox, so there is
 * nowhere for a flag or an archive to be written. Rather than showing controls that do nothing,
 * this window shows a message and says what it is.
 *
 * The three states are all here on purpose. A viewer that renders nothing while it loads and
 * nothing when it fails looks identical in both cases, and the second one is the case where the
 * user most needs to be told something — they double-clicked a file and got a blank window.
 */
export function EmlWindow() {
  const path = useMemo(() => new URLSearchParams(window.location.search).get('eml'), [])

  const [message, setMessage] = useState<EmlMessage | null>(null)
  const [problem, setProblem] = useState<string | null>(null)

  useEffect(() => {
    if (path === null) {
      setProblem('No file was given to open.')
      return
    }

    let alive = true

    emlRead(path)
      .then((loaded) => {
        if (alive) setMessage(loaded)
      })
      .catch((cause: unknown) => {
        // The core's words. It distinguishes "not a message", "could not be read" and "could
        // not be parsed", and flattening those into one sentence would throw away the only
        // information the user has about what to do next.
        if (alive) setProblem(cause instanceof Error ? cause.message : String(cause))
      })

    return () => {
      alive = false
    }
  }, [path])

  if (problem !== null) {
    return (
      <div className={styles.window}>
        <EmptyState
          icon={AlertTriangle}
          tone="error"
          title="This message could not be opened"
          description={problem}
        />
      </div>
    )
  }

  if (message === null) {
    // Deliberately a sentence and not a spinner. Reading a file off a local disk is nearly
    // instant, so a spinner would flash — and on the occasion it does not (a network drive, a
    // very large message) a spinner says less than the sentence does.
    return (
      <div className={styles.window}>
        <EmptyState title="Opening…" />
      </div>
    )
  }

  return (
    <div className={styles.window}>
      <header className={styles.header}>
        <h1 className={styles.subject}>{message.subject}</h1>

        <dl className={styles.fields}>
          <dt className={styles.label}>From</dt>
          <dd className={styles.value}>{message.from}</dd>

          {message.to.length > 0 && (
            <>
              <dt className={styles.label}>To</dt>
              <dd className={styles.value}>{message.to.join(', ')}</dd>
            </>
          )}

          {message.cc.length > 0 && (
            <>
              <dt className={styles.label}>Cc</dt>
              <dd className={styles.value}>{message.cc.join(', ')}</dd>
            </>
          )}

          {message.date !== '' && (
            <>
              <dt className={styles.label}>Date</dt>
              <dd className={styles.value}>{message.date}</dd>
            </>
          )}
        </dl>

        {message.attachments.length > 0 && (
          <p className={styles.attachments}>
            <Paperclip className={styles.clip} aria-hidden="true" strokeWidth={1.75} />
            {/* Named but not openable. Saving an attachment out of a file the user already has
                on disk is a round trip they can make themselves, and offering it would be the
                one place this read-only window wrote something. */}
            {message.attachments.join(', ')}
          </p>
        )}
      </header>

      <div className={styles.body}>
        {/* Straight to the frame, with no "Load Images" banner. That banner offers a choice
            this window cannot honour: the fetch happens in the core against a stored message,
            and a file on disk is not one. Saying what was withheld without offering to undo it
            is the honest version. */}
        <MessageFrame html={message.body.html} fromPlainText={message.body.fromPlainText} />

        {message.body.blockedRemote > 0 && (
          <p className={styles.blocked}>
            {message.body.blockedRemote === 1
              ? '1 remote image was not loaded.'
              : `${String(message.body.blockedRemote)} remote images were not loaded.`}{' '}
            Open this message in your mailbox to load them.
          </p>
        )}
      </div>
    </div>
  )
}
