import { cx } from '@/lib/cx'

import styles from './Skeleton.module.css'

export interface SkeletonProps {
  /**
   * Which of the three widths from docs/02 §6.10 — 60%, 85%, 95%. Cycling these down a
   * list is what makes placeholder rows read as text rather than as a progress bar.
   */
  width?: 1 | 2 | 3
  shape?: 'bar' | 'circle'
  className?: string | undefined
}

/**
 * A loading placeholder. docs/02 §6.10.
 *
 * Reserved space, not a spinner: the doc is explicit that the message list never shows
 * one, and standing rule 6 means the skeleton has to occupy exactly the room the real
 * content will, so nothing moves when it arrives.
 *
 * `aria-hidden`, because a screen reader should hear the container's busy state once, not
 * eight anonymous placeholder bars.
 */
export function Skeleton({ width = 3, shape = 'bar', className }: SkeletonProps) {
  return (
    <div
      aria-hidden="true"
      className={cx(
        styles.skeleton,
        shape === 'circle' ? styles.circle : styles.bar,
        shape === 'bar' && styles[`w${width}`],
        className,
      )}
    />
  )
}
