import { useRef, type KeyboardEvent } from 'react'

import { cx } from '@/lib/cx'

import styles from './Segmented.module.css'

export interface SegmentedOption<T extends string> {
  value: T
  label: string
  /** Announced instead of the label where the label alone is not self-explanatory. */
  hint?: string
}

export interface SegmentedProps<T extends string> {
  /** Required: the group has a name even where the design shows it only as a form label. */
  label: string
  /**
   * The id of the visible label that names this group. Given one, that element becomes the
   * name and `label` is not announced as well — a form row that says "Theme" beside the
   * control should not also make the control say "Theme" a second time.
   */
  labelledBy?: string
  options: readonly SegmentedOption<T>[]
  /** `null` while the stored value is still loading — no segment is drawn as chosen. */
  value: T | null
  onValueChange: (value: T) => void
  disabled?: boolean
  className?: string | undefined
}

/**
 * A segmented control. docs/02 §6 — the second primitive the settings window was missing.
 *
 * Two or three options, side by side, one chosen. The same choice a `Select` makes, laid out
 * for the case where seeing the alternatives *is* the point — Light / Dark / Follow Windows is
 * the example, and it is the control macOS itself uses for that setting. Above three options
 * the segments get too narrow to read and `Select` is the right answer instead.
 *
 * ## Keyboard
 *
 * A radio group, so it behaves like one: one tab stop for the whole control, arrows move
 * *and* choose, Home and End jump to the ends. Roving `tabIndex` rather than `tabIndex={0}`
 * on every segment — a three-segment control that costs three tabs to walk past is how a
 * settings window becomes tedious to operate without a mouse.
 *
 * Buttons with `role="radio"` rather than real `<input type="radio">`: a segment is a filled
 * area with a label inside it and no dot, and hiding an input under each one to get the
 * semantics — the trick `settings.module.css` plays for the accent swatches, where the target
 * genuinely is a circle — buys nothing here that this does not.
 */
export function Segmented<T extends string>({
  label,
  labelledBy,
  options,
  value,
  onValueChange,
  disabled = false,
  className,
}: SegmentedProps<T>) {
  const track = useRef<HTMLDivElement>(null)

  const move = (event: KeyboardEvent<HTMLDivElement>) => {
    const keys = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End']
    if (!keys.includes(event.key)) return

    const current = options.findIndex((option) => option.value === value)
    const last = options.length - 1

    // From nothing selected, an arrow lands on the first option rather than wrapping to the
    // last, which is what `current === -1` would otherwise produce for a backwards step.
    const next =
      event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? last
          : current === -1
            ? 0
            : event.key === 'ArrowLeft' || event.key === 'ArrowUp'
              ? (current + last) % options.length
              : (current + 1) % options.length

    const chosen = options[next]
    if (chosen === undefined) return

    event.preventDefault()
    onValueChange(chosen.value)

    // Focus follows selection, as it does in a radio group. Read back from the DOM rather
    // than kept in a ref array: the segment that has just become selected is the one with
    // `aria-checked="true"` after React commits, and this runs after that.
    requestAnimationFrame(() => {
      track.current?.querySelector<HTMLButtonElement>('[aria-checked="true"]')?.focus()
    })
  }

  return (
    <div
      ref={track}
      role="radiogroup"
      {...(labelledBy === undefined ? { 'aria-label': label } : { 'aria-labelledby': labelledBy })}
      className={cx(styles.track, className)}
      onKeyDown={move}
    >
      {options.map((option) => {
        const chosen = option.value === value

        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={chosen}
            className={cx(styles.segment, chosen && styles.chosen)}
            disabled={disabled}
            // The group is one tab stop. Nothing selected yet — the value is still
            // loading — leaves the first segment reachable, rather than the whole control
            // dropping out of the tab order.
            tabIndex={chosen || (value === null && option === options[0]) ? 0 : -1}
            title={option.hint}
            onClick={() => {
              onValueChange(option.value)
            }}
          >
            {option.label}
          </button>
        )
      })}
    </div>
  )
}
