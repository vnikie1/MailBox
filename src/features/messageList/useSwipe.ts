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

    const limitFor = () => (node.getBoundingClientRect().width || 1) * COMMIT_AT

    /** Applies a displacement and updates what the row shows. Shared by both inputs. */
    const move = (to: number) => {
      const limit = limitFor()
      const past = Math.abs(to) - limit

      // Damped past the commit point, one-to-one before it.
      offset.current = past <= 0 ? to : Math.sign(to) * (limit + past * RUBBER_BAND)

      const travelled = Math.abs(offset.current)
      const progress = Math.min(travelled / limit, 1)

      setState({
        offset: offset.current,
        progress,
        armed:
          travelled < MIN_TRAVEL || progress < 1 ? null : offset.current > 0 ? 'right' : 'left',
      })
    }

    /** Commits or springs back, then returns the row to rest. */
    const release = () => {
      const final = offset.current
      const committed = Math.abs(final) >= limitFor() && Math.abs(final) >= MIN_TRAVEL

      // Reset first. Both actions can remove this row from the list, and leaving the offset
      // set means the next row to occupy this position appears already swiped.
      reset()

      if (!committed) return

      if (final > 0) latest.current.onRight()
      else latest.current.onLeft()
    }

    const onWheel = (event: WheelEvent) => {
      // A vertical scroll that drifts sideways is not a swipe. Requiring the horizontal
      // component to dominate is what keeps the rows still while the list is being scrolled.
      if (Math.abs(event.deltaX) <= Math.abs(event.deltaY)) return

      // Only once it is actually a gesture: preventing default on every stray deltaX would
      // break horizontal scrolling anywhere this row is nested.
      event.preventDefault()

      move(offset.current - event.deltaX)

      // A pan has no end event, so the end is inferred from the deltas stopping. Every event
      // pushes that moment out.
      window.clearTimeout(timer.current)
      timer.current = window.setTimeout(release, SETTLE_MS)
    }

    /**
     * Touch, which docs/06 asks for alongside the touchpad.
     *
     * Simpler than the pan in the one way that matters: a touch gesture has a real end, so
     * `pointerup` commits directly and none of the settle-timer guesswork applies. The
     * arithmetic in between is identical, which is why it is shared above — two implementations
     * of the commit threshold would be two thresholds.
     */
    let start: number | null = null

    const onPointerDown = (event: PointerEvent) => {
      // Touch and pen only. A mouse drag across a row is a text selection or a drag-to-move,
      // both of which already mean something here.
      if (event.pointerType === 'mouse') return
      start = event.clientX
    }

    const onPointerMove = (event: PointerEvent) => {
      if (start === null) return

      const travelled = event.clientX - start

      // Below the threshold this is still possibly a vertical scroll, so the list keeps the
      // gesture. Capturing earlier would make the list impossible to scroll by touch.
      if (Math.abs(travelled) < MIN_TRAVEL) return

      // Once it is a swipe, this row owns the pointer: without capture, moving past the row's
      // own bounds ends the gesture mid-swipe and leaves it displaced.
      if (!node.hasPointerCapture(event.pointerId)) node.setPointerCapture(event.pointerId)

      move(travelled)
    }

    const onPointerUp = (event: PointerEvent) => {
      if (start === null) return
      start = null

      if (node.hasPointerCapture(event.pointerId)) node.releasePointerCapture(event.pointerId)

      release()
    }

    /**
     * The gesture was taken away rather than finished.
     *
     * Springs back without committing, and the distinction matters: `pointercancel` fires when
     * the system claims the gesture for itself or the finger leaves the digitiser, neither of
     * which is the user deciding to archive something. Treating it as a release would archive
     * a message nobody let go of — the precise failure the distance threshold exists to
     * prevent, arriving through the one door that bypasses it.
     */
    const onPointerCancel = (event: PointerEvent) => {
      if (start === null) return
      start = null

      if (node.hasPointerCapture(event.pointerId)) node.releasePointerCapture(event.pointerId)

      reset()
    }

    // Not passive: the handler calls preventDefault once it decides this is a gesture, and a
    // passive listener would make that a no-op with a console warning and no other symptom.
    node.addEventListener('wheel', onWheel, { passive: false })
    node.addEventListener('pointerdown', onPointerDown)
    node.addEventListener('pointermove', onPointerMove)
    node.addEventListener('pointerup', onPointerUp)

    node.addEventListener('pointercancel', onPointerCancel)

    return () => {
      node.removeEventListener('wheel', onWheel)
      node.removeEventListener('pointerdown', onPointerDown)
      node.removeEventListener('pointermove', onPointerMove)
      node.removeEventListener('pointerup', onPointerUp)
      node.removeEventListener('pointercancel', onPointerCancel)
      window.clearTimeout(timer.current)
    }
  }, [reset])

  return { ref: element, ...state }
}
