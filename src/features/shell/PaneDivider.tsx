import { useCallback, useRef, type KeyboardEvent, type PointerEvent } from 'react'

import styles from './PaneDivider.module.css'

const KEYBOARD_STEP = 16
const KEYBOARD_STEP_LARGE = 64

export interface PaneDividerProps {
  label: string
  /** Current width of the pane to the divider's left. */
  value: number
  min: number
  max: number
  onChange: (width: number) => void
}

/**
 * A draggable pane divider. docs/01 §1 — "divider drag persists per-window".
 *
 * Three things make this feel native rather than like a resizable div:
 *
 *  - Pointer capture. Without it, dragging fast enough to outrun the layout leaves the
 *    pointer outside the divider and the drag simply stops, which is the single most
 *    common way a web resizer feels broken.
 *  - The hit area is wider than the hairline. The rule is 1px because standing rule 3 says
 *    so, but a 1px grab target is unusable; the handle is padded and transparent.
 *  - It is a real `separator` widget with arrow-key support. A mouse-only resizer fails
 *    the definition of done's "fully keyboard operable" line, and Windows users who resize
 *    panes from the keyboard exist.
 */
export function PaneDivider({ label, value, min, max, onChange }: PaneDividerProps) {
  const origin = useRef({ x: 0, width: 0 })

  const onPointerDown = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      event.preventDefault()
      event.currentTarget.setPointerCapture(event.pointerId)
      origin.current = { x: event.clientX, width: value }
    },
    [value],
  )

  const onPointerMove = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      if (!event.currentTarget.hasPointerCapture(event.pointerId)) return
      onChange(origin.current.width + (event.clientX - origin.current.x))
    },
    [onChange],
  )

  const onPointerUp = useCallback((event: PointerEvent<HTMLDivElement>) => {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
  }, [])

  const onKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      const step = event.shiftKey ? KEYBOARD_STEP_LARGE : KEYBOARD_STEP

      if (event.key === 'ArrowLeft') {
        event.preventDefault()
        onChange(value - step)
      } else if (event.key === 'ArrowRight') {
        event.preventDefault()
        onChange(value + step)
      } else if (event.key === 'Home') {
        event.preventDefault()
        onChange(min)
      } else if (event.key === 'End') {
        event.preventDefault()
        onChange(max)
      }
    },
    [onChange, value, min, max],
  )

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={Math.round(value)}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      className={styles.divider}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onKeyDown={onKeyDown}
    >
      <span className={styles.rule} aria-hidden="true" />
    </div>
  )
}
