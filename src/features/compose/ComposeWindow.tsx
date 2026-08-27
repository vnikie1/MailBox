import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Send } from 'lucide-react'

import type { ComposeAddress } from '@/lib/generated/ComposeAddress'
import type { ReplyDraft } from '@/lib/generated/ReplyDraft'
import { composeReply, composeSend, closeThisWindow, accountsList } from '@/lib/ipc'
import { Button, TextField, TokenField, type Token } from '@/ui'

import { Editor } from './Editor'
import styles from './ComposeWindow.module.css'

/**
 * The compose window. docs/01 §6, docs/06 Phase 7.
 *
 * A separate OS window running the same bundle — see `main.tsx`. It keeps its own state while
 * the user types and asks the core at exactly two moments: once at the start, to learn who a
 * reply goes to, and once at the end, to send. A round trip per keystroke would be absurd, and
 * a draft that lives in the core would make every keystroke a database write.
 *
 * **Recipients are computed by the core, not here.** `compose_reply` decides who a reply-all
 * copies, which addresses are the user's own, and what the subject becomes. Those rules are
 * subtle enough to be worth one implementation with tests rather than two — and the one that
 * must never be wrong is that a `Bcc` recipient is never carried into a reply, which is
 * enforced where the envelope is read rather than here where it is displayed.
 */

/** A very loose check. The core validates properly; this only colours the chip. */
function looksLikeAddress(value: string): boolean {
  const at = value.indexOf('@')
  return at > 0 && at < value.length - 1 && !/\s/.test(value)
}

/** Turns a chip back into an address the core understands. */
function toAddress(token: Token): ComposeAddress {
  // "Ada Lovelace <ada@example.test>" as well as a bare address, because both are things
  // people paste into a To field.
  const match = /^(.*?)<([^>]+)>\s*$/.exec(token.value)
  if (match) {
    const name = match[1]?.trim().replace(/^["']|["']$/g, '') ?? ''
    return { name: name === '' ? null : name, email: (match[2] ?? '').trim() }
  }
  return { name: null, email: token.value.trim() }
}

function toToken(address: ComposeAddress): Token {
  return {
    id: `${address.email}-${Math.random().toString(36).slice(2)}`,
    // The chip shows the name where there is one; the value keeps the full form, so an
    // address pasted with a display name survives a round trip through the field.
    label: address.name ?? address.email,
    value: address.name === null ? address.email : `${address.name} <${address.email}>`,
    invalid: !looksLikeAddress(address.email),
  }
}

export function ComposeWindow() {
  const parameters = useMemo(() => new URLSearchParams(window.location.search), [])
  const replyTo = parameters.get('message')
  const replyKind = parameters.get('kind') ?? 'reply'

  const [accountId, setAccountId] = useState<number | null>(null)
  const [to, setTo] = useState<Token[]>([])
  const [cc, setCc] = useState<Token[]>([])
  const [bcc, setBcc] = useState<Token[]>([])
  const [showCopies, setShowCopies] = useState(false)
  const [subject, setSubject] = useState('')
  const [quoted, setQuoted] = useState('')
  const [threading, setThreading] = useState<{ inReplyTo: string | null; references: string[] }>({
    inReplyTo: null,
    references: [],
  })

  const [sending, setSending] = useState(false)
  const [problem, setProblem] = useState<string | null>(null)

  // The editor reports both forms on every change; neither is derived from the other.
  const body = useRef({ html: '', text: '' })

  useEffect(() => {
    let cancelled = false

    const load = async () => {
      if (replyTo !== null) {
        const draft: ReplyDraft = await composeReply(Number(replyTo), replyKind)
        if (cancelled) return

        setAccountId(draft.accountId)
        setTo(draft.to.map(toToken))
        setCc(draft.cc.map(toToken))
        setShowCopies(draft.cc.length > 0)
        setSubject(draft.subject)
        setQuoted(draft.quotedHtml)
        setThreading({ inReplyTo: draft.inReplyTo, references: draft.references })
        return
      }

      // A new message. The first account is the default sender; the picker below only appears
      // when there is more than one, per docs/01 §6.
      const accounts = await accountsList()
      if (!cancelled && accounts.length > 0) setAccountId(accounts[0]?.id ?? null)
    }

    load().catch((cause: unknown) => {
      if (!cancelled) setProblem(String(cause))
    })

    return () => {
      cancelled = true
    }
  }, [replyTo, replyKind])

  const onBodyChange = useCallback((html: string, text: string) => {
    body.current = { html, text }
  }, [])

  const send = useCallback(async () => {
    if (accountId === null) {
      setProblem('No account is selected to send from.')
      return
    }
    if (to.length === 0 && cc.length === 0 && bcc.length === 0) {
      setProblem('Add at least one recipient.')
      return
    }

    setSending(true)
    setProblem(null)

    try {
      await composeSend({
        accountId,
        to: to.map(toAddress),
        cc: cc.map(toAddress),
        bcc: bcc.map(toAddress),
        subject,
        html: body.current.html,
        text: body.current.text,
        inReplyTo: threading.inReplyTo,
        references: threading.references,
      })

      // The core has the message on disk and in the outbox before this resolves, so closing
      // now cannot lose it — and for the length of the undo hold it has not been sent either.
      await closeThisWindow()
    } catch (cause: unknown) {
      setSending(false)
      const message =
        typeof cause === 'object' && cause !== null && 'message' in cause
          ? String(cause.message)
          : 'The message could not be queued.'
      setProblem(message)
    }
  }, [accountId, to, cc, bcc, subject, threading])

  return (
    <div className={styles.window}>
      <header className={styles.toolbar} data-tauri-drag-region>
        <span className={styles.spacer} />
        <Button
          variant="filled"
          icon={Send}
          disabled={sending}
          onClick={() => {
            void send()
          }}
        >
          {sending ? 'Sending…' : 'Send'}
        </Button>
      </header>

      <div className={styles.fields}>
        <TokenField
          label="To:"
          tokens={to}
          onTokensChange={setTo}
          validate={looksLikeAddress}
          showAvatars
        />

        {showCopies ? (
          <>
            <TokenField
              label="Cc:"
              tokens={cc}
              onTokensChange={setCc}
              validate={looksLikeAddress}
              showAvatars
            />
            <TokenField
              label="Bcc:"
              tokens={bcc}
              onTokensChange={setBcc}
              validate={looksLikeAddress}
              showAvatars
            />
          </>
        ) : (
          <button
            type="button"
            className={styles.copiesToggle}
            onClick={() => {
              setShowCopies(true)
            }}
          >
            Cc/Bcc
          </button>
        )}

        <div className={styles.subjectRow}>
          <span className={styles.subjectLabel}>Subject:</span>
          <TextField
            label="Subject"
            hideLabel
            value={subject}
            onChange={(event) => {
              setSubject(event.target.value)
            }}
            placeholder="Subject"
            className={styles.subjectInput}
          />
        </div>
      </div>

      {problem !== null && (
        <p className={styles.problem} role="alert">
          {problem}
        </p>
      )}

      <Editor initialHtml={quoted} onChange={onBodyChange} ariaLabel="Message body" />
    </div>
  )
}
