import { memo, useRef } from 'react'
import { Archive, CornerUpLeft, Flag, MailOpen, Paperclip } from 'lucide-react'

import type { MessageRow as MessageRowData } from '@/lib/generated/MessageRow'
import { formatRowDate } from '@/lib/date'
import { cx } from '@/lib/cx'
import { Avatar } from '@/ui'

import { receivedAt, senderLabel, subjectLabel } from './rows'
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
  /** Given the row the drag began on, describes the drag on the transfer. */
  onDragStart: (id: number, transfer: DataTransfer) => void
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
  const sender = senderLabel(message)
  const subject = subjectLabel(message)
  const date = formatRowDate(receivedAt(message), now)

  /**
   * Set when a press landed inside the selection and the selection has not been narrowed yet.
   * A ref, not state: nothing renders differently for it, and a press that re-rendered every
   * visible row would be paid for on the 60fps budget this component exists to protect.
   */
  const collapseOnRelease = useRef(false)

  /**
   * What a screen reader announces for this row.
   *
   * An `aria-label`, not a visually-hidden span. The span was the first attempt and it made
   * things worse: a name computed from content is the *concatenation* of everything inside,
   * so Narrator read the visible sender, date and subject and then read the hidden sentence
   * saying the same things again. Every row was announced twice. `aria-label` replaces the
   * computed name outright, which is the only way to say this once.
   *
   * Verified against the real UI Automation tree, which is where the doubling was found —
   * reading the JSX cannot show it, because both halves look correct on their own.
   *
   * The preview is deliberately left out. It is the longest part of a row and the least useful
   * when every row is being read aloud in sequence.
   */
  const announcement = [
    unread ? 'Unread.' : null,
    `${sender}.`,
    `${subject}.`,
    message.hasAttachment ? 'Has attachment.' : null,
    message.flagged ? 'Flagged.' : null,
    date,
  ]
    .filter(Boolean)
    .join(' ')

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
          aria-label={announcement}
          // Read by the list's context menu, which is mounted once around every row rather
          // than once per row, and so has only the event target to work out what was clicked.
          data-message-id={message.id}
          tabIndex={-1}
          className={cx(
            styles.row,
            selected && styles.selected,
            selected && runStart && styles.runStart,
            selected && runEnd && styles.runEnd,
          )}
          onMouseDown={(event) => {
            // Left button only. A right-click fires `mousedown` too, and running the selection
            // logic on it collapsed a multi-selection to the row under the pointer — so a menu
            // opened on nine selected messages would act on one, having silently thrown the
            // other eight away. The context menu does its own selecting, and it keeps a
            // selection the pointer is already inside.
            if (event.button !== 0) return

            const modifiers = {
              shift: event.shiftKey,
              toggle: event.ctrlKey || event.metaKey,
            }

            /**
             * Pressing inside an existing selection decides nothing yet.
             *
             * This used to select on the way down, unconditionally, and that made a
             * multi-selection impossible to drag: `mousedown` collapsed the selection to the
             * row under the pointer, and the `dragstart` that followed a moment later found
             * one message where the user had picked five. Dropping them on a folder moved
             * one and left the rest behind, which looks like the drop half-failing rather
             * than the selection having been thrown away before it started.
             *
             * So the collapse waits for the button to come back up, and a drag cancels it —
             * which is what Explorer, Finder and Mail all do, and why dragging a group works
             * there. Pressing *outside* the selection still acts immediately, so the row
             * highlights under the finger and the drag carries the row actually grabbed.
             */
            if (!modifiers.shift && !modifiers.toggle && selected) {
              collapseOnRelease.current = true
              return
            }

            onSelect(message.id, modifiers)
          }}
          onMouseUp={() => {
            if (!collapseOnRelease.current) return
            collapseOnRelease.current = false
            onSelect(message.id, { shift: false, toggle: false })
          }}
          draggable
          onDragStart={(event) => {
            // The press turned out to be a drag, so it was never a click and must not narrow
            // the selection. Cleared here rather than left to `mouseup`, which a drag
            // swallows — the flag would otherwise still be set at the next press.
            collapseOnRelease.current = false

            // The list decides what is being dragged and describes it. A row knows its own
            // message and nothing about the others, so it cannot say which account the
            // selection belongs to — and that is what the sidebar needs before the drop.
            onDragStart(message.id, event.dataTransfer)
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
        </div>
      </div>
    </div>
  )
})
