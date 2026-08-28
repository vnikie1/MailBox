import { useCallback, useEffect, useRef, useState } from 'react'

/**
 * Horizontal swipe on a list row. docs/06 Phase 10.
 *
 * ## What the gesture actually is on Windows
 *
 * There is no touch here. A precision touchpad's two-finger horizontal pan arrives in a WebView
 * as `wheel` events carrying `deltaX`, which is the same channel a mouse's horizontal scroll
 * wheel uses. That is the whole input: a stream of deltas with no start and no end.
 *
 * The lack of an end is the hard part. A touch gesture has `pointerup` to commit on; a pan has
 * nothing, so the end has to be inferred from the deltas stopping. `SETTLE_MS` is that
 * inference — long enough not to fire mid-gesture when a finger pauses, short enough that the
 * row does not sit open waiting.
 *
 * ## Why it rubber-bands
 *
 * Past the commit point, further movement is damped rather than ignored. Ignoring it feels
 * broken — the fingers move and the row does not — and following it one-to-one lets a row slide
 * off its own list. Damping says "you have gone as far as this goes" in the only language a
 * direct-manipulation gesture has, which is resistance.
 *
 * ## Why commit is by distance and not velocity
 *
 * A flick and a slow drag should mean the same thing. Velocity thresholds reward the confident
 * and punish the careful, and on a touchpad the same physical motion produces wildly different
 * deltas depending on the pointer-speed setting. Half the row's width is a distance anyone can
 * see themselves crossing.
 */

/** Past this fraction of the row's width, releasing commits. */
const COMMIT_AT = 0.5

/** Beyond the commit point, movement is damped to this fraction. */
const RUBBER_BAND = 0.35

/** No delta for this long means the fingers have stopped. */
const SETTLE_MS = 140

/** Below this, it is a scroll that wandered, not a swipe. */
const MIN_TRAVEL = 8

export interface SwipeActions {
  /** Dragged left-to-right. */
  onRight: () => void
  /** Dragged right-to-left. */
  onLeft: () => void
}

export interface SwipeState {
  /** Pixels the row is displaced by. Negative is leftward. */
  offset: number
  /** 0 to 1, how far towards committing. Drives the colour fill. */
  progress: number
  /** Which action would fire right now, or null. */
  armed: 'left' | 'right' | null
}

export function useSwipe(actions: SwipeActions) {
  const element = useRef<HTMLDivElement | null>(null)
  const [state, setState] = useState<SwipeState>({ offset: 0, progress: 0, armed: null })

  // The live offset, separate from the rendered one. A wheel event can arrive before React has
  // committed the previous render, and reading state here would drop deltas.
  const offset = useRef(0)
  const timer = useRef<number | undefined>(undefined)

  // Held in a ref so the wheel listener below can be attached once. Re-attaching it whenever a
  // parent re-creates these callbacks would drop the gesture mid-swipe.
  const latest = useRef(actions)
  latest.current = actions

  const reset = useCallback(() => {
    offset.current = 0
    setState({ offset: 0, progress: 0, armed: null })
  }, [])

  useEffect(() => {
    const node = element.current
    if (node === null) return

    const onWheel = (event: WheelEvent) => {
      // A vertical scroll that drifts sideways is not a swipe. Requiring the horizontal
      // component to dominate is what keeps the rows still while the list is being scrolled.
      if (Math.abs(event.deltaX) <= Math.abs(event.deltaY)) return

      // Only once it is actually a gesture: preventing default on every stray deltaX would
      // break horizontal scrolling anywhere this row is nested.
      event.preventDefault()

      const width = node.getBoundingClientRect().width || 1
      const limit = width * COMMIT_AT

      const next = offset.current - event.deltaX
      const past = Math.abs(next) - limit

      // Damped past the commit point, one-to-one before it.
      offset.current = past <= 0 ? next : Math.sign(next) * (limit + past * RUBBER_BAND)

      const travelled = Math.abs(offset.current)
      const progress = Math.min(travelled / limit, 1)

      setState({
        offset: offset.current,
        progress,
        armed:
          travelled < MIN_TRAVEL || progress < 1 ? null : offset.current > 0 ? 'right' : 'left',
      })

      // The gesture ends when the deltas stop. Every event pushes that moment out.
      window.clearTimeout(timer.current)
      timer.current = window.setTimeout(() => {
        const final = offset.current
        const committed = Math.abs(final) >= limit && Math.abs(final) >= MIN_TRAVEL

        // Reset first. Both actions can remove this row from the list, and leaving the offset
        // set means the next row to occupy this position appears already swiped.
        reset()

        if (!committed) return

        if (final > 0) latest.current.onRight()
        else latest.current.onLeft()
      }, SETTLE_MS)
    }

    // Not passive: the handler calls preventDefault once it decides this is a gesture, and a
    // passive listener would make that a no-op with a console warning and no other symptom.
    node.addEventListener('wheel', onWheel, { passive: false })

    return () => {
      node.removeEventListener('wheel', onWheel)
      window.clearTimeout(timer.current)
    }
  }, [reset])

  return { ref: element, ...state }
}
