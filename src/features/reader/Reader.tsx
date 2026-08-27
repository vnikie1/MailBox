import { useMemo, useState } from 'react'
import {
  ChevronDown,
  CornerUpRight,
  Forward,
  Mail,
  Paperclip,
  Reply,
  ReplyAll,
  ShieldAlert,
} from 'lucide-react'

import type { AttachmentRow } from '@/lib/generated/AttachmentRow'
import type { MessageFull } from '@/lib/generated/MessageFull'
import { cx } from '@/lib/cx'
import { formatFileSize, formatReaderDate } from '@/lib/date'
import { useThread } from '@/app/queries'

import { AttachmentPreview } from './AttachmentPreview'
import { JunkBanner } from './JunkBanner'
import { MessageBody } from './MessageBody'
import { useThreadBodies } from './useBodyPrefetch'
import { composeOpen, storeNow } from '@/lib/ipc'

import { RedirectSheet } from './RedirectSheet'
import { useMailStore } from '@/store/mail'
import { Avatar, IconButton, ScrollArea, Tooltip, TooltipGroup } from '@/ui'
import { Toolbar } from '@/features/toolbar/Toolbar'

import styles from './Reader.module.css'

interface Address {
  name?: string | null
  address?: string | null
}

/**
 * Recipients arrive as the JSON the store holds, so parsing can fail on anything the sync
 * engine has not normalised yet. Standing rule 13 — degrade visibly, never panic: a header
 * that will not parse shows as undisclosed rather than taking the reader down.
 */
function addressLine(json: string | null): string {
  if (json === null) return 'undisclosed recipients'

  try {
    const parsed: unknown = JSON.parse(json)
    if (!Array.isArray(parsed)) return 'undisclosed recipients'

    const names = (parsed as Address[])
      .map((entry) => entry.name ?? entry.address ?? '')
      .filter((entry) => entry !== '')

    return names.length > 0 ? names.join(', ') : 'undisclosed recipients'
  } catch {
    return 'undisclosed recipients'
  }
}

interface MessageViewProps {
  message: MessageFull
  now: Date
  expanded: boolean
  onToggle: () => void
  /** The newest message in a thread renders expanded and cannot be collapsed away. */
  collapsible: boolean
}

function MessageView({ message, now, expanded, onToggle, collapsible }: MessageViewProps) {
  const [recipientsOpen, setRecipientsOpen] = useState(false)
  // Per message rather than per thread: opening an attachment on one message in a
  // conversation should not close one already open on another.
  const [previewing, setPreviewing] = useState<AttachmentRow | null>(null)
  const [redirecting, setRedirecting] = useState(false)
  const sender = message.fromName ?? message.fromAddr ?? 'Unknown sender'

  return (
    <article className={cx(styles.message, !expanded && styles.collapsed)}>
      <header
        className={styles.messageHeader}
        {...(collapsible
          ? {
              role: 'button',
              tabIndex: 0,
              'aria-expanded': expanded,
              onClick: onToggle,
              onKeyDown: (event: React.KeyboardEvent) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault()
                  onToggle()
                }
              },
            }
          : {})}
      >
        <Avatar
          name={sender}
          {...(message.fromAddr === null ? {} : { email: message.fromAddr })}
          size="lg"
          className={styles.avatar}
        />

        <div className={styles.identity}>
          <div className={styles.identityLine}>
            <span className={styles.sender}>{sender}</span>
            <span className={styles.address}>{message.fromAddr ?? ''}</span>
          </div>

          {expanded && (
            <span className={styles.messageSubject}>{message.subject ?? '(no subject)'}</span>
          )}

          {expanded ? (
            <button
              type="button"
              className={styles.recipients}
              aria-expanded={recipientsOpen}
              onClick={(event) => {
                event.stopPropagation()
                setRecipientsOpen((value) => !value)
              }}
            >
              <span>To: {addressLine(message.toJson)}</span>
              <ChevronDown
                className={cx(styles.recipientsChevron, recipientsOpen && styles.chevronOpen)}
                aria-hidden="true"
                strokeWidth={2}
              />
            </button>
          ) : (
            // The collapsed header borrows the preview so the stack reads as a
            // conversation rather than as a column of anonymous name-and-date rows.
            <span className={styles.collapsedPreview}>{message.preview ?? ''}</span>
          )}
        </div>

        <div className={styles.headerMeta}>
          <span className={styles.date}>
            {formatReaderDate(new Date(message.dateSent * 1000), now)}
          </span>

          {/* docs/01 §5 — reply, reply-all and forward fade in when the header is hovered.
              They are always laid out; making them enter the layout would move the date
              leftward on hover, which standing rule 6 forbids. */}
          <span className={styles.actions}>
            <TooltipGroup>
              <Tooltip
                content="Reply"
                trigger={
                  <IconButton
                    icon={Reply}
                    label="Reply"
                    size="sm"
                    onClick={(event) => {
                      // The header is itself a button that collapses the message.
                      event.stopPropagation()
                      void composeOpen(message.id, 'reply')
                    }}
                  />
                }
              />
              <Tooltip
                content="Reply All"
                trigger={
                  <IconButton
                    icon={ReplyAll}
                    label="Reply All"
                    size="sm"
                    onClick={(event) => {
                      event.stopPropagation()
                      void composeOpen(message.id, 'replyAll')
                    }}
                  />
                }
              />
              <Tooltip
                content="Forward"
                trigger={
                  <IconButton
                    icon={Forward}
                    label="Forward"
                    size="sm"
                    onClick={(event) => {
                      event.stopPropagation()
                      void composeOpen(message.id, 'forward')
                    }}
                  />
                }
              />
              <Tooltip
                content="Redirect"
                trigger={
                  <IconButton
                    icon={CornerUpRight}
                    label="Redirect"
                    size="sm"
                    onClick={(event) => {
                      event.stopPropagation()
                      setRedirecting(true)
                    }}
                  />
                }
              />
            </TooltipGroup>
          </span>
        </div>
      </header>

      {expanded && recipientsOpen && (
        <dl className={styles.recipientDetail}>
          <dt>To</dt>
          <dd>{addressLine(message.toJson)}</dd>
          {message.ccJson !== null && (
            <>
              <dt>Cc</dt>
              <dd>{addressLine(message.ccJson)}</dd>
            </>
          )}
        </dl>
      )}

      {expanded && (
        <>
          {/* Above the body, not below it. The point of the banner is to be read *before* the
              message it is about — a warning underneath a phishing attempt has already lost. */}
          <JunkBanner message={message} />

          {/* docs/03 §6. The body is rendered in a sandboxed frame from HTML the core has
              sanitised — never interpolated into this document, where it would inherit the
              app's own origin and privileges. */}
          <MessageBody messageId={message.id} className={styles.body} />

          {message.attachments.length > 0 && (
            <div className={styles.attachments}>
              {message.attachments.map((attachment) => (
                <button
                  key={attachment.id}
                  type="button"
                  className={styles.attachment}
                  onClick={() => {
                    setPreviewing(attachment)
                  }}
                >
                  <Paperclip
                    className={styles.attachmentIcon}
                    aria-hidden="true"
                    strokeWidth={1.5}
                  />
                  <span className={styles.attachmentText}>
                    <span className={styles.attachmentName}>
                      {attachment.filename ?? 'Attachment'}
                    </span>
                    <span className={styles.attachmentSize}>
                      {formatFileSize(attachment.size ?? 0)}
                    </span>
                  </span>
                </button>
              ))}
            </div>
          )}

          {previewing !== null && (
            <AttachmentPreview
              attachmentId={previewing.id}
              filename={previewing.filename ?? 'Attachment'}
              onClose={() => {
                setPreviewing(null)
              }}
            />
          )}
        </>
      )}

      <RedirectSheet
        open={redirecting}
        onOpenChange={setRedirecting}
        messageId={message.id}
        subject={message.subject ?? ''}
      />
    </article>
  )
}

/**
 * The reader pane. docs/01 §5, docs/02 §6.4, corrected against `assets/reference/`.
 *
 * A thread renders as a vertical stack, oldest at the top, every message collapsed to a
 * single header line except the newest — docs/01 §4. That is the behaviour that makes a
 * fifteen-message thread readable instead of a wall, and it is the one most Windows
 * clients skip in favour of showing only the latest message.
 *
 * The subject is NOT a large title above the header. docs/01 §5 draws it that way and
 * docs/02 §6.4 gives it title2, but macOS 26 puts it as the second line of the header
 * block, under the sender and above the recipients. The reference is unambiguous, so the
 * reference wins; the departure is recorded in docs/PHASE-2-VERIFICATION.md.
 *
 * The pane carries the action toolbar as its own header, at the shared toolbar height, so
 * it lines up with the sidebar's and the list's.
 */
export function Reader() {
  const selectedMessageIds = useMailStore((state) => state.selectedMessageIds)
  const [expandedIds, setExpandedIds] = useState<number[]>([])

  const now = useMemo(storeNow, [])
  const only = selectedMessageIds.length === 1 ? selectedMessageIds[0] : undefined

  // Threading is populated by the sync engine in Phase 5; until then a thread is the one
  // message, and thread_get returns exactly that. The reader is written for the general
  // case either way, so nothing here changes when real threads arrive.
  const { data: messages = [] } = useThread(only ?? null)

  // Whatever the reader shows must be what gets fetched. The list prefetch cannot cover this:
  // a thread reaches across mailboxes, so a message in the conversation may never appear as a
  // row. See `useThreadBodies`.
  useThreadBodies(messages)

  const newestId = messages[messages.length - 1]?.id

  return (
    <div className={styles.pane}>
      <header className={styles.header}>
        <Toolbar />
      </header>

      {messages.length > 0 ? (
        <>
          <div className={styles.countRow}>
            <span className={styles.count}>
              {messages.length} {messages.length === 1 ? 'Message' : 'Messages'}
            </span>
          </div>

          <ScrollArea className={styles.reader}>
            <div className={styles.sheet}>
              {messages.some((message) => message.flagged) && (
                <div className={styles.banner} data-tone="flag">
                  <ShieldAlert className={styles.bannerIcon} aria-hidden="true" strokeWidth={1.5} />
                  <span>This conversation is flagged.</span>
                </div>
              )}

              {messages.map((message) => (
                <MessageView
                  key={message.id}
                  message={message}
                  now={now}
                  expanded={message.id === newestId || expandedIds.includes(message.id)}
                  collapsible={message.id !== newestId}
                  onToggle={() => {
                    setExpandedIds((current) =>
                      current.includes(message.id)
                        ? current.filter((id) => id !== message.id)
                        : [...current, message.id],
                    )
                  }}
                />
              ))}
            </div>
          </ScrollArea>
        </>
      ) : (
        // docs/02 §6.10 — centred glyph plus title, both tertiary. Never a spinner.
        <div className={styles.empty}>
          <Mail className={styles.emptyGlyph} aria-hidden="true" strokeWidth={1.25} />
          <p className={styles.emptyTitle}>
            {selectedMessageIds.length > 1
              ? `${String(selectedMessageIds.length)} Messages Selected`
              : 'No Message Selected'}
          </p>
        </div>
      )}
    </div>
  )
}
