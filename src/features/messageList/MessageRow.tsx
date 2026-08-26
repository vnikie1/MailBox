import { memo } from 'react'
import { CornerUpLeft, Flag, Paperclip } from 'lucide-react'

import type { MessageRow as MessageRowData } from '@/lib/generated/MessageRow'
import { formatRowDate } from '@/lib/date'
import { cx } from '@/lib/cx'
import { Avatar } from '@/ui'

import { receivedAt } from './rows'

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
}: MessageRowProps) {
  const unread = !message.seen
  const sender = message.fromName ?? message.fromAddr ?? 'Unknown sender'
  const subject = message.subject ?? '(no subject)'
  const date = formatRowDate(receivedAt(message), now)

  return (
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
  )
})
