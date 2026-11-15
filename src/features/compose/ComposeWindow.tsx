import { type KeyboardEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react'
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
  composeDiscardDraft,
  composeLoadDraft,
  composeDescribeFiles,
  composeSizeLimit,
  onCloseRequested,
  onFileDrop,
  reasonFor,
} from '@/lib/ipc'
import { useAppearanceSync } from '@/app/useAppearanceSync'
import { Button, IconButton, Select, Sheet, TextField, type Token } from '@/ui'

import { formatFileSize as formatSize } from '@/lib/date'

import type { OutgoingMessage } from '@/lib/generated/OutgoingMessage'

import { looksLikeAddress } from './address'
import { cannotAttach } from './attachments'
import { Editor, type ChangeOrigin, type EditorContent, type EditorSignature } from './Editor'
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

/**
 * Text from a `mailto:` body, as HTML.
 *
 * The body of a mailto is plain text by definition, and it comes from whatever page the user
 * clicked. Inserting it into the editor unescaped would let any link put markup — a script tag,
 * a tracking image, an anchor pointing somewhere else — into a message the user is about to
 * send under their own name.
 */
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
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

/** An account's signature as the editor takes it. */
function signatureOf(draft: ReplyDraft): EditorSignature {
  return {
    html: draft.signatureHtml,
    placement: draft.signaturePlacement === 'below' ? 'below' : 'above',
  }
}

/**
 * Assembles the body the editor opens with: the quote either side of the signature, in the order
 * the account asked for, with `lead` — a `mailto:` body — at the top.
 *
 * "Above" puts the signature under what the user just wrote and before the quoted history,
 * which is what people who reply inline expect. "Below" puts it at the very bottom, which is
 * what people who top-post expect. Getting it wrong makes every reply look like a mistake,
 * which is why it is a stored choice rather than a guess.
 *
 * The signature itself goes in separately, so the editor knows which blocks are the signature
 * and can swap them for another account's when From changes (M46, 2026-11-14 audit). It once went
 * in as an empty `<div>` — every message carried the wrapper and none the signature, and nothing
 * said so — so the blank line above it is added only when there is a signature to sit under.
 */
function openingContent(draft: ReplyDraft, lead = ''): EditorContent {
  const blankLine = draft.signatureHtml.trim() === '' ? '' : '<p><br></p>'

  return draft.signaturePlacement === 'below'
    ? { before: lead + draft.quotedHtml + blankLine, after: '' }
    : { before: lead + blankLine, after: draft.quotedHtml }
}

export function ComposeWindow() {
  // Every window has to run this, not just the mailbox. A second OS window is a second React
  // root, and main.tsx only writes the theme and density before first paint — it never sets the
  // accent. Without this the window drew the CSS fallback blue while the rest of the app wore
  // the accent, and a pinned accent would not have reached it at all.
  useAppearanceSync()

  const parameters = useMemo(() => new URLSearchParams(window.location.search), [])
  const replyTo = parameters.get('message')
  const replyKind = parameters.get('kind') ?? 'reply'
  /** A draft reopened from a Drafts mailbox — Edit Draft. */
  const draftOf = parameters.get('draft')
  /** That draft's identity, once loaded, for the autosave to carry on under. */
  const [draftIdentity, setDraftIdentity] = useState<string | null>(null)

  const [accounts, setAccounts] = useState<AccountRow[]>([])
  const [accountId, setAccountId] = useState<number | null>(null)
  const [to, setTo] = useState<Token[]>([])
  const [cc, setCc] = useState<Token[]>([])
  const [bcc, setBcc] = useState<Token[]>([])
  const [showCopies, setShowCopies] = useState(false)
  const [subject, setSubject] = useState('')
  /** What the editor opens with, once known. */
  const [opening, setOpening] = useState<EditorContent | null>(null)
  /**
   * The sending account's signature, which the editor keeps current in the body. `null` for a
   * draft reopened from Drafts, whose signature is already part of its text and cannot be told
   * apart from it.
   */
  const [signature, setSignature] = useState<EditorSignature | null>(null)
  const [threading, setThreading] = useState<{ inReplyTo: string | null; references: string[] }>({
    inReplyTo: null,
    references: [],
  })

  const [attachments, setAttachments] = useState<PickedFile[]>([])
  const [sizeLimit, setSizeLimit] = useState(25 * 1024 * 1024)
  const [closing, setClosing] = useState(false)
  const [sending, setSending] = useState(false)
  const [problem, setProblem] = useState<string | null>(null)

  // The editor reports both forms on every change; neither is derived from the other.
  const body = useRef({ html: '', text: '' })

  /**
   * Whether the user has changed anything since the window opened.
   *
   * A reply arrives with recipients, a subject and a quote, and a new message with a signature,
   * so "is there anything in it" was true of every window the moment it opened: closing an
   * untouched reply asked to save a draft of it, and switching away from one saved it (L25,
   * 2026-11-14 audit). What the window was given is not work the user can lose. Set by the
   * user's own edits only — the loading code writes the state directly, around these.
   */
  const touched = useRef(false)
  const touch = useCallback(() => {
    touched.current = true
  }, [])

  const totalSize = useMemo(
    () => attachments.reduce((sum, file) => sum + file.size, 0),
    [attachments],
  )

  const [dropping, setDropping] = useState(false)

  /**
   * Adds files to the message — from the picker or from a drop, which must behave identically.
   *
   * Appended rather than replaced: attaching twice is how people add a file they forgot, and
   * replacing would silently drop the first set. And de-duplicated by path, because dropping a
   * file that is already attached is an easy thing to do and two chips for one file would send
   * it twice.
   */
  const addAttachments = useCallback((files: PickedFile[]) => {
    if (files.length > 0) touched.current = true
    setAttachments((current) => {
      const seen = new Set(current.map((file) => file.path))
      return [...current, ...files.filter((file) => !seen.has(file.path))]
    })
  }, [])

  /**
   * Files dragged in from Explorer, as Mail allows (asked for on 2026-09-17; docs/01 §6 specifies
   * only that attachments show as chips).
   *
   * The drop arrives as paths from Tauri's native handler — see `onFileDrop` — so it goes
   * through the same description the picker's choices do, and the chips it produces are
   * indistinguishable from picked ones. A folder cannot be attached, and says so; quietly
   * leaving it out would read as the drop not having worked.
   */
  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    void onFileDrop((event) => {
      if (event.type === 'hover') {
        setDropping(true)
        return
      }

      setDropping(false)
      if (event.type !== 'drop' || event.paths.length === 0) return

      composeDescribeFiles(event.paths)
        .then(({ files, skipped }) => {
          if (cancelled) return
          addAttachments(files)
          if (skipped.length > 0) setProblem(cannotAttach(skipped))
        })
        .catch((cause: unknown) => {
          if (!cancelled) setProblem(reasonFor(cause))
        })
    }).then((off) => {
      if (cancelled) off()
      else unlisten = off
    })

    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [addAttachments])

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
      // A saved draft, reopened. Drafts could be saved and never opened again anywhere in the
      // app (C3, 2026-11-14 audit); this is the way back to one.
      if (draftOf !== null) {
        const draft = await composeLoadDraft(Number(draftOf))
        if (!alive.current) return

        setAccountId(draft.accountId)
        setTo(draft.to.map(toToken))
        setCc(draft.cc.map(toToken))
        setBcc(draft.bcc.map(toToken))
        setShowCopies(draft.cc.length > 0 || draft.bcc.length > 0)
        setSubject(draft.subject)
        setOpening({ before: draft.html, after: '' })
        setThreading({ inReplyTo: draft.inReplyTo, references: draft.references })
        setAttachments(draft.attachments)
        setDraftIdentity(draft.messageId)
        return
      }

      if (replyTo !== null) {
        const draft: ReplyDraft = await composeReply(Number(replyTo), replyKind)
        if (!alive.current) return

        setAccountId(draft.accountId)
        setTo(draft.to.map(toToken))
        setCc(draft.cc.map(toToken))
        setShowCopies(draft.cc.length > 0)
        setSubject(draft.subject)
        setSignature(signatureOf(draft))
        setOpening(openingContent(draft))
        setThreading({ inReplyTo: draft.inReplyTo, references: draft.references })
        // Forward as Attachment is the only kind that arrives with a file already on it. The
        // core has already written the copy to disk and named it from the subject, so from
        // here it is indistinguishable from one the user picked.
        setAttachments(draft.attachments)
        return
      }

      // A new message. The first account is the default sender; the picker below only appears
      // when there is more than one, per docs/01 §6.
      const accounts = await accountsList()
      const first = accounts[0]?.id ?? null
      if (first === null) return

      setAccountId(first)

      // A new message still gets the signature, which is usually the only thing in it.
      const blank = await composeBlank(first)
      if (!alive.current) return

      // A mailto link. These four are already sanitised — links.rs dropped Bcc and every other
      // header a web page could have asked for, and compose_open percent-encoded what survived.
      // Nothing is re-filtered here, deliberately: a second filter on this side would look like
      // the boundary and is not one.
      //
      // Read here rather than in the component body so the effect's dependency list can name
      // `parameters` — which never changes, because it is this window's own URL — instead of
      // four derived values that would have to be suppressed.
      const mailtoTo = parameters.getAll('to')
      const mailtoCc = parameters.getAll('cc')
      const mailtoSubject = parameters.get('subject')
      const mailtoBody = parameters.get('body')

      if (mailtoTo.length > 0) setTo(mailtoTo.map((email) => toToken({ name: null, email })))
      if (mailtoCc.length > 0) {
        setCc(mailtoCc.map((email) => toToken({ name: null, email })))
        setShowCopies(true)
      }
      if (mailtoSubject !== null) setSubject(mailtoSubject)

      // The body arrives as plain text — that is all a mailto can carry — so it goes in above
      // the signature as text, not as markup. Treating it as HTML would let a link supply
      // markup into the editor, which is the one thing this whole path exists to prevent.
      setSignature(signatureOf(blank))
      setOpening(
        openingContent(
          blank,
          mailtoBody === null || mailtoBody === '' ? '' : `<p>${escapeHtml(mailtoBody)}</p>`,
        ),
      )
    }

    load().catch((cause: unknown) => {
      if (alive.current) setProblem(reasonFor(cause))
    })

    return () => {
      alive.current = false
    }
  }, [replyTo, replyKind, draftOf, parameters])

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

  const isTouched = useCallback(() => touched.current, [])
  const autosave = useAutosave(buildMessage, isTouched)

  // A reopened draft is saved under its own identity, so each save replaces it rather than
  // filing a second draft beside it.
  const { adopt } = autosave
  useEffect(() => {
    if (draftIdentity !== null) adopt(draftIdentity)
  }, [draftIdentity, adopt])

  /** Whether there is anything in the window worth not losing. */
  const hasContent = useCallback(() => {
    if (!touched.current) return false

    const message = buildMessage()
    if (message === null) return false

    return (
      message.subject.trim() !== '' ||
      (message.text ?? '').trim() !== '' ||
      message.to.length > 0 ||
      message.cc.length > 0 ||
      message.bcc.length > 0 ||
      attachments.length > 0
    )
  }, [buildMessage, attachments])

  /**
   * Set once the user has answered the question, or once there is no question left to ask.
   *
   * `closeThisWindow` calls `window.close()`, which fires `onCloseRequested` again — so every
   * path that closes the window deliberately used to walk straight back into the sheet that
   * asked whether to close it. The fields still hold their text at that point, so `hasContent`
   * was still true, the sheet reopened, and the close was cancelled. Pressing Delete, Save as
   * Draft or Send simply reopened the question, for ever: the window could not be closed at all
   * once anything had been typed in it.
   *
   * A ref rather than state because the handler is registered once and must see the current
   * value — a state variable would be captured at registration and read stale.
   */
  const closeApproved = useRef(false)

  // docs/01 §6 — closing an unsaved compose offers Save as Draft / Delete / Cancel. Closing a
  // window with something typed in it is the one action in a mail client that destroys work
  // with a single click and no undo.
  useEffect(() => {
    let off: (() => void) | undefined
    let stopped = false

    void onCloseRequested(() => {
      // Already answered — by Delete, by Save as Draft, or by the send having gone through.
      if (closeApproved.current) return true
      if (!hasContent()) return true

      setClosing(true)
      return false
    }).then((unlisten) => {
      if (stopped) unlisten()
      else off = unlisten
    })

    return () => {
      stopped = true
      off?.()
    }
  }, [hasContent])

  const onBodyChange = useCallback((html: string, text: string, origin: ChangeOrigin) => {
    body.current = { html, text }
    if (origin === 'user') touched.current = true
  }, [])

  /**
   * Changing From brings that account's signature with it, in place of the one already in the
   * body (M46, 2026-11-14 audit). Not for a reopened draft: its signature is text by now.
   *
   * Numbered, so switching twice quickly cannot leave the first account's signature behind
   * when its answer arrives second.
   */
  const signatureRequest = useRef(0)
  const changeAccount = (next: number) => {
    setAccountId(next)
    if (signature === null) return

    const request = signatureRequest.current + 1
    signatureRequest.current = request

    composeBlank(next)
      .then((blank) => {
        if (request === signatureRequest.current) setSignature(signatureOf(blank))
      })
      .catch((cause: unknown) => {
        console.warn('[compose] the signature for that account could not be read', cause)
      })
  }

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

    // Whether the core took ownership. Everything that can fail inside compose_send fails
    // before the message reaches the outbox, so up to this point the window is the only copy;
    // past it, the message exists on disk and a draft of it would be a duplicate of something
    // already sent.
    let queued = false

    try {
      // Before the send, so a draft that was never autosaved does not reappear afterwards as
      // an unsent copy of a message that has gone.
      autosave.abandon()

      // With the draft it was written as, which the core lets go of — here and on the server —
      // once the message has actually gone. Nothing did, so every sent message stayed in Drafts
      // on every device (H4, 2026-11-14 audit).
      await composeSend(message, autosave.messageId())
      queued = true

      // The core has the message on disk and in the outbox before this resolves, so closing
      // now cannot lose it — and for the length of the undo hold it has not been sent either.
      //
      // Approved before closing, or the close handler asks whether to save a draft of the
      // message that has just been sent — the fields still hold its text.
      closeApproved.current = true
      await closeThisWindow()
    } catch (cause: unknown) {
      // Undoing the abandon is what keeps an unqueued message savable. Without it the timer,
      // the blur save and the Save as Draft button in the close sheet are all dead for the
      // life of the window, and that button silently discards the message instead of saving
      // it. Only when the send itself failed: a window that will not close is a nuisance, but
      // a draft copy of a message already in the outbox is a second message.
      if (!queued) autosave.resume()
      setSending(false)
      const message =
        typeof cause === 'object' && cause !== null && 'message' in cause
          ? String(cause.message)
          : 'The message could not be queued.'
      setProblem(message)
    }
  }, [accountId, to, cc, bcc, buildMessage, autosave])

  /**
   * Ctrl+Enter sends. docs/01 §14, and `shortcuts.ts` lists it as `local: true`.
   *
   * "Local" means the focused control owns the chord and binds it itself — the dispatcher
   * deliberately does not, because this window is a separate OS window with its own React
   * tree. Nothing bound it, so the Help sheet advertised a shortcut that did nothing.
   *
   * A React handler on the root rather than a window listener: it reaches here by bubbling
   * through the tree, it dies with the component, and it keeps the rule that the only
   * `addEventListener('keydown')` in the app is the dispatcher's.
   *
   * **Asked for here, sent after the next render.** The same Enter, in a recipient field, has
   * just committed what was typed there as a chip — a state change this handler's `send` cannot
   * see, because it closed over the recipients as they were. Sending from here sent the message
   * without the address typed last (M45, 2026-11-14 audit), or refused it for having no
   * recipient at all. The effect below runs once that render is done, with that render's `send`.
   */
  const [sendAsked, setSendAsked] = useState(false)
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Enter' || !event.ctrlKey || sending) return
    event.preventDefault()
    setSendAsked(true)
  }

  useEffect(() => {
    if (!sendAsked) return
    setSendAsked(false)
    void send()
  }, [sendAsked, send])

  return (
    <div className={styles.window} onKeyDown={onKeyDown}>
      {dropping && (
        // A pointer affordance, and only that: the drop is announced by the chip it produces.
        <div className={styles.dropTarget} aria-hidden="true">
          <Paperclip className={styles.dropIcon} strokeWidth={1.5} />
          <span className={styles.dropLabel}>Drop to attach</span>
        </div>
      )}

      <header className={styles.toolbar} data-tauri-drag-region>
        <IconButton
          icon={Paperclip}
          label="Attach Files"
          onClick={() => {
            void composePickFiles().then(addAttachments)
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
          onTokensChange={(next) => {
            touch()
            setTo(next)
          }}
          validate={looksLikeAddress}
        />

        {showCopies ? (
          <>
            <RecipientField
              label="Cc:"
              tokens={cc}
              onTokensChange={(next) => {
                touch()
                setCc(next)
              }}
              validate={looksLikeAddress}
            />
            <RecipientField
              label="Bcc:"
              tokens={bcc}
              onTokensChange={(next) => {
                touch()
                setBcc(next)
              }}
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
            {/* The shared popup button, not a `<select>` of its own.

                It was one, styled `background: none` with the light label colour. Windows draws
                the list itself and takes its surface from the control, so in the dark theme the
                list opened white with white text: every account unreadable except the one under
                the pointer, which Windows paints with its own highlight. `ui/Select` had already
                been fixed for exactly that; this control never used it. */}
            <Select
              label="Send from"
              hideLabel
              className={styles.from}
              options={accounts.map((account) => ({
                value: account.id,
                label: `${account.displayName} — ${account.email}`,
              }))}
              value={accountId}
              onValueChange={changeAccount}
            />
          </div>
        )}

        <div className={styles.subjectRow}>
          <span className={styles.subjectLabel}>Subject:</span>
          <TextField
            label="Subject"
            hideLabel
            value={subject}
            onChange={(event) => {
              touch()
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
                  touch()
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

      <Editor
        initialContent={opening}
        signature={signature}
        onChange={onBodyChange}
        ariaLabel="Message body"
      />

      <Sheet
        open={closing}
        onOpenChange={(next) => {
          if (!next) setClosing(false)
        }}
        title="Save this message as a draft?"
        description="It has not been sent."
        footer={
          <>
            <Button
              variant="bordered"
              onClick={() => {
                setClosing(false)
              }}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => {
                // Abandoned first, so the autosave timer cannot write it back on the way out.
                autosave.abandon()

                // And any draft already saved is thrown away, here and on the server. Delete
                // used to close the window and leave the autosaved copy in Drafts everywhere.
                const saved = autosave.messageId()
                const discarded = saved === null ? Promise.resolve() : composeDiscardDraft(saved)

                void discarded
                  .catch((cause: unknown) => {
                    console.warn('[compose] draft not discarded', cause)
                  })
                  .finally(() => {
                    closeApproved.current = true
                    setClosing(false)
                    void closeThisWindow()
                  })
              }}
            >
              Delete
            </Button>
            <Button
              variant="filled"
              onClick={() => {
                // Closed only once the save has finished, and not at all if it failed: this
                // closed straight away, and a save that had not landed — or could not — took the
                // message with the window.
                void autosave.saveNow().then((saved) => {
                  if (!saved) {
                    setClosing(false)
                    setProblem('The draft could not be saved. Try again, or copy the text.')
                    return
                  }
                  closeApproved.current = true
                  setClosing(false)
                  void closeThisWindow()
                })
              }}
            >
              Save as Draft
            </Button>
          </>
        }
      />
    </div>
  )
}
