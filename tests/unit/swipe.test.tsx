import { act } from 'react'
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { render } from '@testing-library/react'

import { useSwipe } from '@/features/messageList/useSwipe'

/**
 * Swipe gesture arithmetic. docs/06 Phase 10.
 *
 * The gesture has no explicit end — a precision touchpad pan is a stream of `wheel` deltas and
 * nothing else — so committing is inferred from the deltas stopping. That inference and the
 * commit threshold are the two things worth pinning down: both are invisible in review, and
 * both fail in the same direction, which is a row archiving itself because somebody scrolled.
 */

const WIDTH = 400

function Harness({ onRight, onLeft }: { onRight: () => void; onLeft: () => void }) {
  const swipe = useSwipe({ onRight, onLeft })

  return (
    <div
      ref={swipe.ref}
      data-testid="row"
      data-offset={swipe.offset}
      data-progress={swipe.progress}
    >
      row
    </div>
  )
}

function setup() {
  const onRight = vi.fn()
  const onLeft = vi.fn()
  const view = render(<Harness onRight={onRight} onLeft={onLeft} />)
  const row = view.getByTestId('row')

  // jsdom gives every element a zero-size box, and the hook divides by the width. Without this
  // the commit threshold is zero and every stray delta commits — which is exactly the bug the
  // tests below exist to catch, so it has to be a real number here.
  row.getBoundingClientRect = () => ({ width: WIDTH, height: 60 }) as DOMRect

  const pan = (deltaX: number) => {
    act(() => {
      row.dispatchEvent(
        new WheelEvent('wheel', { deltaX, deltaY: 0, bubbles: true, cancelable: true }),
      )
    })
  }

  const settle = () => {
    act(() => {
      vi.advanceTimersByTime(200)
    })
  }

  return { row, onRight, onLeft, pan, settle }
}

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('committing', () => {
  it('fires nothing for a short drag', () => {
    const { onRight, onLeft, pan, settle } = setup()

    // A twentieth of the row. This is what a vertical scroll that wandered looks like.
    pan(-20)
    settle()

    expect(onRight).not.toHaveBeenCalled()
    expect(onLeft).not.toHaveBeenCalled()
  })

  it('fires only once past half the row', () => {
    const { onRight, pan, settle } = setup()

    // deltaX is negated into offset, so a negative delta moves the row right.
    pan(-(WIDTH * 0.5 + 10))
    settle()

    expect(onRight).toHaveBeenCalledTimes(1)
  })

  it('does not fire at just under the threshold', () => {
    const { onRight, onLeft, pan, settle } = setup()

    pan(-(WIDTH * 0.5 - 10))
    settle()

    expect(onRight).not.toHaveBeenCalled()
    expect(onLeft).not.toHaveBeenCalled()
  })

  it('tells the two directions apart', () => {
    const { onRight, onLeft, pan, settle } = setup()

    pan(WIDTH * 0.6)
    settle()

    expect(onLeft).toHaveBeenCalledTimes(1)
    expect(onRight).not.toHaveBeenCalled()
  })

  it('accumulates a slow drag rather than judging each delta alone', () => {
    const { onRight, pan, settle } = setup()

    // A touchpad delivers a gesture as dozens of small deltas. Judging them individually would
    // mean a slow, deliberate swipe never commits while a single flick does.
    for (let step = 0; step < 30; step += 1) pan(-10)
    settle()

    expect(onRight).toHaveBeenCalledTimes(1)
  })
})

describe('the end of the gesture', () => {
  it('does not fire while the deltas are still coming', () => {
    const { onRight, pan } = setup()

    pan(-(WIDTH * 0.6))

    // Past the threshold but the fingers have not stopped. Firing here would act mid-gesture,
    // before the user has had the chance to pull back.
    act(() => {
      vi.advanceTimersByTime(100)
    })

    expect(onRight).not.toHaveBeenCalled()
  })

  it('returns the row to rest after committing', () => {
    const { row, pan, settle } = setup()

    pan(-(WIDTH * 0.6))
    settle()

    // Both actions can remove the row from the list. Leaving it displaced means the next row to
    // take this position in the virtualiser appears already swiped.
    expect(row.getAttribute('data-offset')).toBe('0')
  })

  it('returns the row to rest after abandoning', () => {
    const { row, pan, settle } = setup()

    pan(-30)
    settle()

    expect(row.getAttribute('data-offset')).toBe('0')
  })
})

describe('what is not a swipe', () => {
  it('ignores a mostly-vertical gesture', () => {
    const { row, onRight, onLeft } = setup()

    act(() => {
      row.dispatchEvent(
        new WheelEvent('wheel', { deltaX: -300, deltaY: -400, bubbles: true, cancelable: true }),
      )
      vi.advanceTimersByTime(200)
    })

    // Scrolling a list is not a swipe, however far sideways the fingers drift on the way.
    expect(onRight).not.toHaveBeenCalled()
    expect(onLeft).not.toHaveBeenCalled()
    expect(row.getAttribute('data-offset')).toBe('0')
  })
})

describe('rubber banding', () => {
  it('damps movement past the commit point', () => {
    const { row, pan } = setup()

    const limit = WIDTH * 0.5
    pan(-limit)
    const atLimit = Number(row.getAttribute('data-offset'))

    pan(-100)
    const past = Number(row.getAttribute('data-offset'))

    // It keeps moving — ignoring the input entirely feels broken — but by less than it was
    // pushed, which is the only way a gesture can say "this is as far as it goes".
    expect(past).toBeGreaterThan(atLimit)
    expect(past - atLimit).toBeLessThan(100)
  })

  it('reports progress as a fraction that stops at one', () => {
    const { row, pan } = setup()

    pan(-(WIDTH * 0.25))
    expect(Number(row.getAttribute('data-progress'))).toBeCloseTo(0.5, 1)

    pan(-(WIDTH * 0.5))
    expect(Number(row.getAttribute('data-progress'))).toBe(1)
  })
})
