import { cx } from '@/lib/cx'

import styles from './Badge.module.css'

export interface BadgeProps {
  count: number
  /** Rendered on the selected row, where the surface is the accent. docs/02 §6.2. */
  selected?: boolean
  /** What the number counts, for the screen reader. */
  noun?: string
  className?: string | undefined
}

/**
 * An unread count. docs/02 §6.2.
 *
 * Hidden at zero, tabular figures always. Tabular is not a nicety here: the badge sits
 * against the right edge of a sidebar row, and proportional digits make it twitch
 * sideways every time a count crosses 1 to 2 or 9 to 10, which standing rule 6 forbids.
 */
export function Badge({ count, selected = false, noun = 'unread', className }: BadgeProps) {
  if (count <= 0) return null

  return (
    <span
      className={cx(styles.badge, 'tabular', selected && styles.selected, className)}
      aria-label={`${count} ${noun}`}
    >
      <span aria-hidden="true">{count}</span>
    </span>
  )
}
