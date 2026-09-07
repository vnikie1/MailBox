/**
 * The picture under the cursor while mail is dragged. docs/01 §4 — "rows stack into a fanned
 * deck with a count badge".
 *
 * ## Why this exists at all
 *
 * Reported from using the app: dragging a message showed its body text. Nothing was setting a
 * drag image, so Chromium fell back to snapshotting the dragged element — the whole message
 * row, sender and subject and date and icons and however many lines of preview the density
 * calls for. That is up to 329 x 140, and because the default hot spot is wherever inside the
 * row the user happened to grab, roughly half of it hangs to the *left* of the pointer,
 * directly over the 232px sidebar they are aiming at.
 *
 * ## The four rules Blink imposes, none of which are guessable
 *
 * Every one of these was read out of the Chromium source rather than assumed, because each
 * failure mode here is silent:
 *
 * 1. **`setDragImage` rasterizes nothing.** It stores a node pointer. The snapshot is taken
 *    later in the same task, after this handler and its microtasks return, in
 *    `DragController::StartDrag`. So a call from `setTimeout` or `requestAnimationFrame` is
 *    not merely late — Blink flips the `DataTransfer` to read-only the moment the handler
 *    returns, and `setDragImage` then bails without a word.
 * 2. **The element must have a layout object when that snapshot happens.** Detached or
 *    `display: none` yields no layout object, and Blink returns a null image.
 *    `visibility: hidden` and `opacity: 0` do produce one — a correctly sized, completely
 *    transparent one. Off-screen positioning is not clipped by Blink but is widely reported
 *    to be cut off by the platform. Attached, on-screen and opaque is the only option that
 *    works, which is why the deck is added to the document and taken out again a tick later.
 * 3. **A null image means the drag proceeds with _no_ preview at all.** It does not fall back
 *    to the row snapshot, because this call has already overwritten it, and the behaviour
 *    that would restore it sits behind a runtime flag that is off in stable WebView2. So the
 *    visible signature of this file being broken is a drag carrying *nothing* — which reads
 *    like the change did nothing, when it is the opposite.
 * 4. **The offset is in CSS pixels.** Blink multiplies it by the frame's zoom factor, which
 *    already carries the device scale. Doing that arithmetic here would double the offset at
 *    this machine's 200%.
 *
 * ## What cannot be verified
 *
 * No test on any platform can see the finished bitmap: it is composited by the Windows shell,
 * Playwright cannot screenshot it, and the browser tests have no Tauri and no shell.
 * `buildDragDeck` is exported so the *tree* can be asserted, which is as far as automation
 * reaches. The picture itself is checked by eye, in the real app.
 */

import type { MessageRow } from '@/lib/generated/MessageRow'
import { cx } from '@/lib/cx'

import { senderLabel, subjectLabel } from './rows'

import styles from './dragDeck.module.css'

/**
 * Cards drawn, however many messages are being dragged.
 *
 * The deck is a symbol for "several"; the number is carried by the badge. Fifty cards would
 * be two hundred pixels of stripes and would blow past the size at which Windows starts
 * washing a drag image out.
 */
const DECK_CARDS = 3

/** The longest string put on a card. A subject can be megabytes; shaping one costs frames. */
const MAX_TEXT = 120

/** Above this the badge stops counting. Ctrl+A in a large mailbox really does reach 100,000. */
const MAX_COUNT = 999

let deck: HTMLElement | null = null
let frame = 0
let timer = 0
let listening = false

/** Removes the deck and cancels anything still scheduled. Safe at any moment, and twice. */
function teardown(): void {
  if (frame !== 0) cancelAnimationFrame(frame)
  if (timer !== 0) clearTimeout(timer)

  frame = 0
  timer = 0
  deck?.remove()
  deck = null
}

function clamp(value: string): string {
  return value.length > MAX_TEXT ? `${value.slice(0, MAX_TEXT)}…` : value
}

function card(className: string | undefined): HTMLElement {
  const element = document.createElement('div')
  // `cx`, not a template literal: a CSS module name is typed as possibly absent, and a
  // template would stringify that into a literal "undefined" class.
  element.className = cx(styles.card, className)

  return element
}

/**
 * The deck for `messages`, headed by the row the drag started on.
 *
 * `anchorId` rather than `messages[0]` because the card should name the message physically
 * under the cursor when the user pressed — the one they think they picked up — not whichever
 * of the selection happens to sort first.
 */
export function buildDragDeck(messages: MessageRow[], anchorId: number): HTMLElement | null {
  const anchor = messages.find((row) => row.id === anchorId) ?? messages[0]
  if (anchor === undefined) return null

  const root = document.createElement('div')
  root.className = cx(styles.deck)
  root.setAttribute('aria-hidden', 'true')

  // Appended back-to-front. These two carry no text: they are the edges of cards behind the
  // one being read, which is what makes a stack look like a stack.
  const behind = Math.min(messages.length, DECK_CARDS) - 1
  if (behind >= 2) root.append(card(styles.behind2))
  if (behind >= 1) root.append(card(styles.behind1))

  const front = card(styles.front)
  const text = document.createElement('div')
  text.className = cx(styles.text)

  const sender = document.createElement('div')
  sender.className = cx(styles.sender)
  // `textContent`, never `innerHTML`. A subject is whatever a stranger put in a header.
  sender.textContent = clamp(senderLabel(anchor))

  const subject = document.createElement('div')
  subject.className = cx(styles.subject)
  subject.textContent = clamp(subjectLabel(anchor))

  text.append(sender, subject)
  front.append(text)

  if (messages.length > 1) {
    const count = document.createElement('span')
    count.className = cx(styles.count, 'tabular')
    count.textContent =
      messages.length > MAX_COUNT ? `${String(MAX_COUNT)}+` : String(messages.length)
    front.append(count)
  }

  root.append(front)

  return root
}

/**
 * Draws the drag under the pointer. Call only from inside a `dragstart` handler — see rule 1
 * in the note at the top of this file.
 */
export function setMessageDragImage(
  transfer: DataTransfer,
  messages: MessageRow[],
  anchorId: number,
): void {
  // First, not last. A deck that somehow outlived its drag is removed here, which bounds the
  // number of them in the document at one no matter what went wrong on any previous drag.
  teardown()

  // Older webviews, and every `DataTransfer` stand-in in the unit tests: leave Chromium to
  // its own snapshot rather than mounting an element nothing will ever take a picture of.
  if (typeof transfer.setDragImage !== 'function') return

  if (!listening) {
    listening = true
    // Capture phase, on `window`, and never removed: one pair for the life of the app. A
    // drag can end without a drop, and `dragend` is the only event that always arrives.
    window.addEventListener('dragend', teardown, true)
    window.addEventListener('drop', teardown, true)
  }

  try {
    const built = buildDragDeck(messages, anchorId)
    if (built === null) return

    document.body.append(built)
    deck = built

    // 0,0 puts the pointer at the deck's top-left corner, so it hangs down and to the right
    // and leaves everything above and to the left — which is where the sidebar is — visible.
    // The clear space between pointer and card is the transparent border in the stylesheet,
    // because a negative hot spot is ignored.
    transfer.setDragImage(built, 0, 0)

    // The snapshot happens synchronously once this handler returns, so any macrotask is late
    // enough to clean up and a microtask would be too early. Both a timer and a frame are
    // armed because either can be starved by the OS drag loop; `teardown` is idempotent, so
    // whichever lands first wins and cancels the other.
    timer = window.setTimeout(teardown, 0)
    frame = requestAnimationFrame(teardown)
  } catch {
    // The decoration must never take the drag down with it. Chromium cancels a drag whose
    // `dragstart` handler throws, and moving real mail matters more than the picture of it.
    teardown()
  }
}
