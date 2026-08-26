import type { ComponentPropsWithRef } from 'react'
import type { LucideIcon } from 'lucide-react'

import { cx } from '@/lib/cx'

import styles from './IconButton.module.css'

export interface IconButtonProps extends Omit<ComponentPropsWithRef<'button'>, 'children'> {
  icon: LucideIcon
  /** Required. The control has no text, so this is its only accessible name. */
  label: string
  /** A latched state — the sidebar toggle, a filter. Reported as aria-pressed. */
  toggled?: boolean
  size?: 'md' | 'sm'
}

/**
 * An icon-only button. docs/02 §6.1, docs/01 §13.
 *
 * 17px glyph in a 28px target at 1.5 stroke, `--label-2` at rest, `--label-1` on hover,
 * `--accent` when latched, `--label-3` and no fill when disabled.
 *
 * `label` is not optional and is not defaulted. An icon button with no accessible name is
 * an unlabelled control to a screen reader, and the definition of done requires every
 * feature to be screen-reader labelled — making it a required prop is the only way that
 * survives contact with a hurried Phase 2.
 */
export function IconButton({
  icon: Icon,
  label,
  toggled,
  size = 'md',
  className,
  type = 'button',
  ...rest
}: IconButtonProps) {
  return (
    <button
      {...rest}
      type={type}
      aria-label={label}
      {...(toggled === undefined ? {} : { 'aria-pressed': toggled })}
      className={cx(styles.button, styles[size], toggled && styles.toggled, className)}
    >
      <Icon className={styles.icon} aria-hidden="true" strokeWidth={1.5} />
    </button>
  )
}
