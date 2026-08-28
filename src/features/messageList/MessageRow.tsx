import { memo } from 'react'
import { Archive, CornerUpLeft, Flag, MailOpen, Paperclip } from 'lucide-react'

import type { MessageRow as MessageRowData } from '@/lib/generated/MessageRow'
import { formatRowDate } from '@/lib/date'
import { cx } from '@/lib/cx'
import { Avatar } from '@/ui'

import { receivedAt } from './rows'
import { useSwipe } from './useSwipe'

import styles from './MessageRow.module.css'

export interface MessageRowProps {
  message: MessageRowData
  now: Date
  selected: boolean
  runStart: boolean
  runEnd: boolean
  previewLines: number
  showPhoto: boolean
  onSelect: (id: number, modifiers: { shift: boolean; toggle: boolean }) => void
  onDragStart: (id: number) => number[]
  /** Swipe right. docs/06 gives this to Archive, the one that is easy to undo. */
  onSwipeArchive: (id: number) => void
  /** Swipe left. Read/unread, which is reversible by doing it again. */
  onSwipeToggleRead: (id: number) => void
}

/**
 * One row of the message list. docs/01 §4, docs/02 §6.3.
 *
 * Memoised because this is the surface the 60fps budget in docs/03 §5 is about: scrolling a
 * hundred thousand rows re-renders whatever is not memoised, and the row is the only
 * component in the app that exists in the hundreds.
 *
 * The gutter holding the unread dot is *always* laid out. Marking a message read removes the
 * dot and nothing else moves — docs/01 §9.1 opens with that, and it is the difference
 * between a list that feels solid and one that twitches as mail arrives.
 */
export const MessageRow = memo(function MessageRow({
  message,
  now,
  selected,
  runStart,
  runEnd,
  previewLines,
  showPhoto,
  onSelect,
  onDragStart,
  onSwipeArchive,
  onSwipeToggleRead,
}: MessageRowProps) {
  const unread = !message.seen
  const sender = message.fromName ?? message.fromAddr ?? 'Unknown sender'
  const subject = message.subject ?? '(no subject)'
  const date = formatRowDate(receivedAt(message), now)

  // Both directions are chosen to be recoverable. A swipe is the easiest gesture in the app to
  // perform by accident — a two-finger scroll that drifted — so neither direction may do
  // anything the user cannot immediately undo. Delete is deliberately not offered.
  const swipe = useSwipe({
    onRight: () => {
      onSwipeArchive(message.id)
    },
    onLeft: () => {
      onSwipeToggleRead(message.id)
    },
  })

  return (
    <div className={styles.swipe}>
      {/* The fill behind the row. It is what the row slides off to reveal, so it carries the
          colour and the icon, and it grows opaque as the gesture nears committing — the
          progressive fill docs/06 asks for. */}
      <div
        className={styles.behind}
        data-side={swipe.offset > 0 ? 'right' : 'left'}
        style={{ opacity: swipe.progress }}
        aria-hidden="true"
      >
        {swipe.offset > 0 ? (
          <Archive className={styles.behindIcon} strokeWidth={1.75} />
        ) : (
          <MailOpen className={styles.behindIcon} strokeWidth={1.75} />
        )}
      </div>

      <div
        ref={swipe.ref}
        className={styles.sliding}
        style={swipe.offset === 0 ? undefined : { transform: `translateX(${swipe.offset}px)` }}
        data-swiping={swipe.offset === 0 ? undefined : ''}
      >
        <div
          role="option"
          aria-selected={selected}
          tabIndex={-1}
          className={cx(
            styles.row,
            selected && styles.selected,
            selected && runStart && styles.runStart,
            selected && runEnd && styles.runEnd,
          )}
          onMouseDown={(event) => {
            onSelect(message.id, {
              shift: event.shiftKey,
              toggle: event.ctrlKey || event.metaKey,
            })
          }}
          draggable
          onDragStart={(event) => {
            // Dragging an unselected row drags that row, not the selection — what every list on
            // both platforms does, and what stops a stray drag moving nine messages that were
            // selected earlier.
            const ids = onDragStart(message.id)
            event.dataTransfer.setData('application/x-mailbox-threads', ids.join(' '))
            event.dataTransfer.effectAllowed = 'move'
          }}
        >
          <span className={styles.gutter} aria-hidden="true">
            {unread && <span className={styles.dot} />}
          </span>

          {showPhoto && (
            <Avatar
              {...(message.fromName === null ? {} : { name: message.fromName })}
              {...(message.fromAddr === null ? {} : { email: message.fromAddr })}
              size="md"
              className={styles.photo}
            />
          )}

          <div className={styles.content}>
            <div className={styles.line}>
              <span className={cx(styles.sender, unread && styles.unread)}>{sender}</span>
              <span className={cx(styles.date, 'tabular')}>{date}</span>
            </div>

            <div className={styles.line}>
              <span className={cx(styles.subject, unread && styles.unread)}>{subject}</span>
              <span className={styles.icons} aria-hidden="true">
                {message.answered && <CornerUpLeft className={styles.icon} strokeWidth={1.75} />}
                {message.hasAttachment && <Paperclip className={styles.icon} strokeWidth={1.75} />}
                {message.flagged && (
                  <Flag
                    className={cx(styles.icon, styles.flag)}
                    strokeWidth={1.75}
                    data-flag={message.flagColor ?? 'orange'}
                  />
                )}
              </span>
            </div>

            {previewLines > 0 && (
              <p className={styles.preview} style={{ WebkitLineClamp: previewLines }}>
                {message.preview ?? ''}
              </p>
            )}
          </div>

          {/* One coherent sentence for the screen reader, rather than five fragments read in
          layout order — which is what the visual arrangement above would otherwise produce. */}
          <span className="srOnly">
            {unread ? 'Unread. ' : ''}
            {sender}. {subject}. {message.hasAttachment ? 'Has attachment. ' : ''}
            {message.flagged ? 'Flagged. ' : ''}
            {date}
          </span>
        </div>
      </div>
    </div>
  )
})
