import { cx } from '@/lib/cx'

import styles from './Divider.module.css'

export interface DividerProps {
  orientation?: 'horizontal' | 'vertical'
  /** Inset from both ends, the way a menu separator is — docs/02 §6.9. */
  inset?: boolean
  className?: string | undefined
}

/**
 * A hairline. Standing rule 3: there are no solid borders in this app, so this is the
 * only rule anything is allowed to draw, and it is 1px at 10% alpha in both themes.
 */
export function Divider({ orientation = 'horizontal', inset = false, className }: DividerProps) {
  return (
    <div
      role="separator"
      aria-orientation={orientation}
      className={cx(styles.divider, styles[orientation], inset && styles.inset, className)}
    />
  )
}
