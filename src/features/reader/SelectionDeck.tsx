import { useMemo } from 'react'

import type { MessageFull } from '@/lib/generated/MessageFull'
import { cx } from '@/lib/cx'
import { formatRowDate } from '@/lib/date'
import { senderLabel, subjectLabel } from '@/features/messageList/rows'
import { useMessageBody, useSelectedMessages } from '@/app/queries'
import { storeNow } from '@/lib/ipc'
import { Avatar } from '@/ui'

import { MessageFrame } from './MessageFrame'
import { useThreadBodies } from './useBodyPrefetch'

import styles from './SelectionDeck.module.css'

/**
 * Sheets drawn, however many messages are selected.
 *
 * Three, the same as `messageList/dragDeck.ts`: the stack is a picture of "several", and the
 * number is carried by the caption underneath. It is also the bound on what this pane fetches —
 * a Ctrl+A over a hundred thousand rows costs three `message_get` calls and one body, not a
 * hundred thousand of either.
 */
export const DECK_SHEETS = 3

/** Back to front, so `DEPTH_CLASS[0]` is the sheet on top. */
const DEPTH_CLASS = [styles.front, styles.behind1, styles.behind2]

/**
 * The front sheet's body: the message itself, rendered, with remote images **off**.
 *
 * Off whatever the user's setting, because loading them tells the sender the message was
 * opened, and this one has not been — it has been caught in a selection, very often on its way
 * to the Bin. A preview that reported "opened" for every message somebody shift-clicked past
 * would be the tracking pixel's best friend. Blocked images show as blocked, which is honest
 * for a preview, and opening the message on its own decides the question the usual way.
 *
 * Until the body arrives it shows the stored preview text, so the sheet is never a blank white
 * card — which is what an empty frame looks like, and what reads as the app having broken.
 *
 * Withheld images are drawn as empty space rather than as broken-image glyphs. The reader shows
 * the glyph because the banner above it says what is missing and offers to load it; here there is
 * no banner and no offer, and a preview full of broken glyphs looks like the app failing at
 * something rather than declining to do it.
 */
function FrontBody({ message }: { message: MessageFull }) {
  const { data: rendered } = useMessageBody(message.id, false)

  if (rendered === undefined || rendered.html.trim() === '') {
    return <p className={styles.previewText}>{message.preview ?? ''}</p>
  }

  return (
    <MessageFrame
      html={rendered.html}
      css={rendered.css}
      fromPlainText={rendered.fromPlainText}
      resetKey={message.id}
      hideBlockedImages
    />
  )
}

interface SheetProps {
  message: MessageFull | null
  now: Date
  depth: number
}

/**
 * One sheet: a message laid out as the reader lays it out, header first.
 *
 * Only the front sheet has a body. Every sheet behind is visible only down to the tilted top
 * edge of the one in front of it — a strip the fan step is sized to give its header, and not a
 * line more. The first version gave the sheets behind their preview text "in case a sliver
 * showed", and a sliver is exactly what showed: one line of it cut lengthways by the tilted
 * edge above, which reads as clipped text, not as a page under a page. The same fault the small
 * cards had, for the same reason. So a sheet behind names its message and stops, and all the
 * tilt can cut is blank paper.
 *
 * Laid out before its message arrives, at full size, so the stack does not assemble itself a
 * sheet at a time as three queries land (standing rule 6).
 */
function Sheet({ message, now, depth }: SheetProps) {
  const sender = message === null ? '' : senderLabel(message)

  return (
    <article className={cx(styles.sheet, DEPTH_CLASS[depth])}>
      {message !== null && (
        <>
          <header className={styles.header}>
            <Avatar
              name={sender}
              {...(message.fromAddr === null ? {} : { email: message.fromAddr })}
              size="lg"
              className={styles.avatar}
            />
            <div className={styles.identity}>
              <div className={styles.line}>
                <span className={styles.sender}>{sender}</span>
                <span className={cx(styles.date, 'tabular')}>
                  {formatRowDate(new Date(message.dateReceived * 1000), now)}
                </span>
              </div>
              <span className={styles.subject}>{subjectLabel(message)}</span>
            </div>
          </header>

          {depth === 0 && (
            <div className={styles.body}>
              <FrontBody message={message} />
            </div>
          )}
        </>
      )}
    </article>
  )
}

export interface SelectionDeckProps {
  /** The whole selection, in the list's order. Only the first `DECK_SHEETS` are drawn. */
  ids: number[]
}

/**
 * What the reader shows when several messages are selected. docs/01 §4.
 *
 * The messages themselves, as a stack of sheets the size of the pane: the first selected on top
 * with its body showing, and the next two behind it, lifted and tilted outwards so each one's
 * header — who it is from, what it is about — shows above the sheet in front. The count sits
 * underneath.
 *
 * ## How it got here
 *
 * It was the words "3 Messages Selected" over an envelope glyph, which is accurate and says
 * nothing: a shift-click that caught a row you did not mean is the commonest way to get a
 * multi-selection wrong, and a number cannot show you which rows it caught. Then it was a small
 * fan of cards in the middle of the pane, which could. Reported on seeing that one: "the preview
 * should be big and fill the whole mail window and be tilted to show the different mails." So
 * the cards became sheets, the sheets fill the pane, and the one on top is the real message.
 *
 * ## It is decoration
 *
 * The stack is `aria-hidden` and takes no pointer events. The caption is the live region: three
 * senders, three subjects and a message body read aloud on every change would make
 * shift-arrowing through a mailbox unusable, and a link inside a preview that opened when brushed
 * would be worse. The list is where a message is chosen; this is the picture of what has been.
 */
export function SelectionDeck({ ids }: SelectionDeckProps) {
  const drawn = ids.slice(0, DECK_SHEETS)
  const messages = useSelectedMessages(drawn)

  // One clock for the whole stack, so three sheets cannot disagree about what "Yesterday" is,
  // and taken once rather than on every render — the same as the reader beside it.
  const now = useMemo(storeNow, [])

  const front = messages[0] ?? null

  // The front sheet shows a body, so it has to have been downloaded. Bodies are fetched lazily,
  // on selection, and the list's prefetch follows the single selected row — which a
  // multi-selection does not have.
  useThreadBodies(front === null ? [] : [front])

  // Deepest first, which is the order they have to be appended in.
  const sheets = drawn
    .map((id, depth) => ({ id, depth, message: messages[depth] ?? null }))
    .reverse()

  return (
    <div className={styles.selection}>
      <div className={styles.stage} data-sheets={drawn.length} aria-hidden="true">
        {sheets.map(({ id, depth, message }) => (
          <Sheet key={id} message={message} now={now} depth={depth} />
        ))}
      </div>

      <div className={styles.caption} role="status" aria-live="polite">
        <p className={styles.count}>{`${String(ids.length)} Messages Selected`}</p>
        <p className={styles.hint}>Move, flag and delete act on all of them.</p>
      </div>
    </div>
  )
}
