import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { MessageRow } from '@/lib/generated/MessageRow'
import { buildDragDeck, setMessageDragImage } from '@/features/messageList/dragDeck'

/**
 * The drag deck. docs/01 §4 — "rows stack into a fanned deck with a count badge".
 *
 * ## What these tests can and cannot reach
 *
 * They assert the **tree**, never the picture. The finished bitmap is rasterized by Blink and
 * composited by the Windows shell; Playwright cannot screenshot it and there is no shell here.
 * So the guarantees available are: the right text is on the card, the wrong text is not, the
 * element is mounted when Blink needs to photograph it, and it is gone afterwards. Whether it
 * *looks* right is checked by eye in the real app, and `assets/reference/` has no capture of a
 * drag to compare against — see CLAUDE.md.
 *
 * `vitest.config.ts` sets `classNameStrategy: 'non-scoped'`, so class names come back readable.
 */

function row(over: Partial<MessageRow> = {}): MessageRow {
  return {
    id: 1,
    threadId: 1,
    mailboxId: 1,
    accountId: 1,
    subject: 'Your hike: the adventure is not over yet',
    fromName: 'Komoot',
    fromAddr: 'notification@komoot.de',
    dateSent: 1_757_260_800,
    dateReceived: 1_757_260_800,
    size: 52_000,
    preview: 'Nice hike! You came, you saw, you conquered. And now it is time for a recap.',
    seen: false,
    answered: false,
    flagged: false,
    flagColor: null,
    hasAttachment: false,
    isJunk: false,
    ...over,
  } as MessageRow
}

/** A `DataTransfer` that records what it was handed, since jsdom has no drag machinery. */
function transfer(): DataTransfer & { image: Element | null } {
  const stub = {
    image: null as Element | null,
    setDragImage(element: Element) {
      stub.image = element
    },
  }

  return stub as unknown as DataTransfer & { image: Element | null }
}

const decks = () => document.querySelectorAll('.deck')

beforeEach(() => {
  vi.useFakeTimers()
  document.body.innerHTML = ''
})

afterEach(() => {
  vi.useRealTimers()
})

describe('buildDragDeck', () => {
  it('carries the heading and leaves the body text behind', () => {
    // The whole complaint, in one assertion. The old ghost was a snapshot of the row, which
    // includes the preview paragraph; the deck is the heading only.
    const deck = buildDragDeck([row()], 1)

    expect(deck?.textContent).toContain('Komoot')
    expect(deck?.textContent).toContain('Your hike')
    expect(deck?.textContent).not.toContain('you conquered')
  })

  it('draws one card and no badge for one message', () => {
    const deck = buildDragDeck([row()], 1)

    expect(deck?.querySelectorAll('.card')).toHaveLength(1)
    expect(deck?.querySelector('.count')).toBeNull()
  })

  it('names the message the drag started on, not the first in the selection', () => {
    // Grabbing the third row of a selection must show the third row. Anything else tells the
    // user they picked up something they did not.
    const deck = buildDragDeck(
      [row({ id: 1, fromName: 'First' }), row({ id: 2, fromName: 'Grabbed' })],
      2,
    )

    expect(deck?.textContent).toContain('Grabbed')
    expect(deck?.textContent).not.toContain('First')
  })

  it('stops at three cards however many are dragged, and counts on the badge', () => {
    const many = Array.from({ length: 50 }, (_, index) => row({ id: index + 1 }))
    const deck = buildDragDeck(many, 1)

    // Fifty cards would be two hundred pixels of stripes, and past the width at which Windows
    // starts washing a drag image out. The deck says "several"; the badge says how many.
    expect(deck?.querySelectorAll('.card')).toHaveLength(3)
    expect(deck?.querySelector('.count')?.textContent).toBe('50')
  })

  it('gives up counting past 999 rather than widening the card', () => {
    const crowd = Array.from({ length: 100_000 }, (_, index) => row({ id: index + 1 }))

    // Ctrl+A in a real mailbox reaches this. A five-digit badge would eat the subject.
    expect(buildDragDeck(crowd, 1)?.querySelector('.count')?.textContent).toBe('999+')
  })

  it('falls back through an empty sender the way the row does', () => {
    // `??` would let a whitespace-only From header through and draw a blank line. Both the
    // row and the deck go through `senderLabel`, so neither can regress alone.
    const blank = buildDragDeck([row({ fromName: '   ' })], 1)
    expect(blank?.textContent).toContain('notification@komoot.de')

    const nameless = buildDragDeck([row({ fromName: null, fromAddr: null, subject: null })], 1)
    expect(nameless?.textContent).toContain('Unknown sender')
    expect(nameless?.textContent).toContain('(no subject)')
  })

  it('clamps a hostile subject instead of shaping megabytes of it', () => {
    const deck = buildDragDeck([row({ subject: 'x'.repeat(5000) })], 1)

    // 120 characters and an ellipsis. The card is 200px wide; the rest would be laid out,
    // measured and thrown away on the frame the user can least afford to lose.
    expect(deck?.querySelector('.subject')?.textContent).toHaveLength(121)
  })

  it('returns null rather than an empty deck when there is nothing to draw', () => {
    expect(buildDragDeck([], 1)).toBeNull()
  })
})

describe('setMessageDragImage', () => {
  it('mounts the deck, because Blink cannot photograph a detached element', () => {
    const data = transfer()
    setMessageDragImage(data, [row()], 1)

    // Attached and laid out, or Blink returns a null image — and then the drag carries no
    // preview at all, since this call has already overwritten the default row snapshot.
    expect(decks()).toHaveLength(1)
    expect(data.image).not.toBeNull()
    expect(document.body.contains(data.image)).toBe(true)
  })

  it('takes it away again once the snapshot has been taken', () => {
    setMessageDragImage(transfer(), [row()], 1)
    expect(decks()).toHaveLength(1)

    vi.runAllTimers()
    expect(decks()).toHaveLength(0)
  })

  it('never leaves two decks in the document, even if a drag never ended', () => {
    // Belt and braces for the case the timer and the frame were both starved: the next drag
    // clears the last one before it does anything else.
    setMessageDragImage(transfer(), [row()], 1)
    setMessageDragImage(transfer(), [row()], 1)

    expect(decks()).toHaveLength(1)
  })

  it('clears up when the drag ends without a drop', () => {
    setMessageDragImage(transfer(), [row()], 1)
    window.dispatchEvent(new Event('dragend'))

    // Escape cancels a drag. `dragend` is the only event that always arrives.
    expect(decks()).toHaveLength(0)
  })

  it('leaves nothing behind when setDragImage throws', () => {
    const hostile = {
      setDragImage() {
        throw new Error('no')
      },
    } as unknown as DataTransfer

    // Chromium cancels a drag whose dragstart handler throws. Moving real mail matters more
    // than the picture of it, so the decoration must never take the drag down with it.
    expect(() => {
      setMessageDragImage(hostile, [row()], 1)
    }).not.toThrow()
    expect(decks()).toHaveLength(0)
  })

  it('mounts nothing at all where setDragImage does not exist', () => {
    setMessageDragImage({} as DataTransfer, [row()], 1)

    expect(decks()).toHaveLength(0)
  })
})
