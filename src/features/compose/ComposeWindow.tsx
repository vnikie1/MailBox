import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Paperclip, Send, X } from 'lucide-react'

import type { AccountRow } from '@/lib/generated/AccountRow'
import type { ComposeAddress } from '@/lib/generated/ComposeAddress'
import type { PickedFile } from '@/lib/generated/PickedFile'
import type { ReplyDraft } from '@/lib/generated/ReplyDraft'
import {
  accountsList,
  closeThisWindow,
  composePickFiles,
  composeReply,
  composeSend,
  composeBlank,
  composeSizeLimit,
} from '@/lib/ipc'
import { Button, IconButton, TextField, type Token } from '@/ui'

import { formatFileSize as formatSize } from '@/lib/date'

import type { OutgoingMessage } from '@/lib/generated/OutgoingMessage'

import { Editor } from './Editor'
import { RecipientField } from './RecipientField'
import { useAutosave } from './useAutosave'
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

/**
 * Assembles the body the editor opens with: the signature, the quote, in the order the account
 * asked for.
 *
 * "Above" puts the signature under what the user just wrote and before the quoted history,
 * which is what people who reply inline expect. "Below" puts it at the very bottom, which is
 * what people who top-post expect. Getting it wrong makes every reply look like a mistake,
 * which is why it is a stored choice rather than a guess.
 */
function bodyWithSignature(draft: ReplyDraft): string {
  if (draft.signatureHtml.trim() === '') return draft.quotedHtml

  const signature = `<p><br></p><div></div>`

  return draft.signaturePlacement === 'below'
    ? draft.quotedHtml + signature
    : signature + draft.quotedHtml
}

export function ComposeWindow() {
  const parameters = useMemo(() => new URLSearchParams(window.location.search), [])
  const replyTo = parameters.get('message')
  const replyKind = parameters.get('kind') ?? 'reply'

  const [accounts, setAccounts] = useState<AccountRow[]>([])
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

  const [attachments, setAttachments] = useState<PickedFile[]>([])
  const [sizeLimit, setSizeLimit] = useState(25 * 1024 * 1024)
  const [sending, setSending] = useState(false)
  const [problem, setProblem] = useState<string | null>(null)

  // The editor reports both forms on every change; neither is derived from the other.
  const body = useRef({ html: '', text: '' })

  const totalSize = useMemo(
    () => attachments.reduce((sum, file) => sum + file.size, 0),
    [attachments],
  )

  // The limit is the core's to decide, so the warning and the format agree with whatever the
  // builder actually enforces.
  useEffect(() => {
    void composeSizeLimit().then(setSizeLimit)
    // Loaded regardless of how the window opened: the From picker appears whenever there is
    // more than one identity, including on a reply.
    void accountsList().then(setAccounts)
  }, [])

  useEffect(() => {
    // A mutable holder rather than a plain boolean: TypeScript narrows a captured boolean
    // and cannot see the cleanup closure flip it, so it reports the checks after each await as
    // redundant. They are not — the window can close mid-request — and a property read is
    // re-checked after every call, which is exactly the behaviour wanted here.
    const alive = { current: true }

    const load = async () => {
      if (replyTo !== null) {
        const draft: ReplyDraft = await composeReply(Number(replyTo), replyKind)
        if (!alive.current) return

        setAccountId(draft.accountId)
        setTo(draft.to.map(toToken))
        setCc(draft.cc.map(toToken))
        setShowCopies(draft.cc.length > 0)
        setSubject(draft.subject)
        setQuoted(bodyWithSignature(draft))
        setThreading({ inReplyTo: draft.inReplyTo, references: draft.references })
        return
      }

      // A new message. The first account is the default sender; the picker below only appears
      // when there is more than one, per docs/01 §6.
      const accounts = await accountsList()
      const first = accounts[0]?.id ?? null
      if (first === null) return

      setAccountId(first)

      // A new message still gets the signature, which is the only thing in it.
      const blank = await composeBlank(first)
      if (alive.current) setQuoted(bodyWithSignature(blank))
    }

    load().catch((cause: unknown) => {
      if (alive.current) setProblem(String(cause))
    })

    return () => {
      alive.current = false
    }
  }, [replyTo, replyKind])

  /** The message as it stands. Used by both autosave and Send, so they cannot drift. */
  const buildMessage = useCallback((): OutgoingMessage | null => {
    if (accountId === null) return null

    return {
      accountId,
      to: to.map(toAddress),
      cc: cc.map(toAddress),
      bcc: bcc.map(toAddress),
      subject,
      html: body.current.html,
      text: body.current.text,
      inReplyTo: threading.inReplyTo,
      references: threading.references,
      attachments: attachments.map((file) => file.path),
    }
  }, [accountId, to, cc, bcc, subject, threading, attachments])

  const autosave = useAutosave(buildMessage)

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

    const message = buildMessage()
    if (message === null) {
      setSending(false)
      setProblem('No account is selected to send from.')
      return
    }

    try {
      // Before the send, so a draft that was never autosaved does not reappear afterwards as
      // an unsent copy of a message that has gone.
      autosave.abandon()

      await composeSend(message)

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
  }, [accountId, to, cc, bcc, buildMessage, autosave])

  return (
    <div className={styles.window}>
      <header className={styles.toolbar} data-tauri-drag-region>
        <IconButton
          icon={Paperclip}
          label="Attach Files"
          onClick={() => {
            void composePickFiles().then((picked) => {
              // Appended rather than replaced: attaching twice is how people add a file they
              // forgot, and replacing would silently drop the first set.
              setAttachments((current) => {
                const seen = new Set(current.map((file) => file.path))
                return [...current, ...picked.filter((file) => !seen.has(file.path))]
              })
            })
          }}
        />
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
        <RecipientField
          label="To:"
          tokens={to}
          onTokensChange={setTo}
          validate={looksLikeAddress}
        />

        {showCopies ? (
          <>
            <RecipientField
              label="Cc:"
              tokens={cc}
              onTokensChange={setCc}
              validate={looksLikeAddress}
            />
            <RecipientField
              label="Bcc:"
              tokens={bcc}
              onTokensChange={setBcc}
              validate={looksLikeAddress}
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

        {/* docs/01 §6 — the From picker appears only with more than one identity. With one
            account it is a control that can only ever say the same thing. */}
        {accounts.length > 1 && (
          <div className={styles.subjectRow}>
            <span className={styles.subjectLabel}>From:</span>
            <select
              className={styles.from}
              aria-label="Send from"
              value={accountId ?? ''}
              onChange={(event) => {
                setAccountId(Number(event.target.value))
              }}
            >
              {accounts.map((account) => (
                <option key={account.id} value={account.id}>
                  {account.displayName} — {account.email}
                </option>
              ))}
            </select>
          </div>
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

      {attachments.length > 0 && (
        <div className={styles.attachments}>
          {attachments.map((file) => (
            <span key={file.path} className={styles.attachment}>
              <Paperclip className={styles.attachmentIcon} aria-hidden strokeWidth={1.5} />
              <span className={styles.attachmentName}>{file.filename}</span>
              <span className={styles.attachmentSize}>{formatSize(file.size)}</span>
              <IconButton
                icon={X}
                label={`Remove ${file.filename}`}
                size="sm"
                onClick={() => {
                  setAttachments((current) => current.filter((entry) => entry.path !== file.path))
                }}
              />
            </span>
          ))}

          {/* A warning, not a refusal. The user may know their own server takes more than the
              25MB most providers allow — but a message that is silently too large comes back
              as a bounce hours later, addressed to nobody they recognise. */}
          {totalSize > sizeLimit && (
            <span className={styles.tooLarge} role="status">
              {formatSize(totalSize)} of attachments. Most providers refuse more than{' '}
              {formatSize(sizeLimit)}.
            </span>
          )}
        </div>
      )}

      {problem !== null && (
        <p className={styles.problem} role="alert">
          {problem}
        </p>
      )}

      <Editor initialHtml={quoted} onChange={onBodyChange} ariaLabel="Message body" />
    </div>
  )
}
