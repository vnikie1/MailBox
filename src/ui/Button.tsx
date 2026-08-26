import type { ComponentPropsWithRef } from 'react'
import type { LucideIcon } from 'lucide-react'

import { cx } from '@/lib/cx'

import styles from './Button.module.css'

export interface ButtonProps extends ComponentPropsWithRef<'button'> {
  variant?: 'filled' | 'bordered' | 'plain' | 'destructive'
  icon?: LucideIcon
  fullWidth?: boolean
}

/**
 * A text button. docs/02 §6.5.
 *
 * `type="button"` by default rather than the HTML default of `submit`: almost every
 * button in this app sits inside compose, which is a form, and a stray submit is how you
 * send a half-written message.
 *
 * Disabled filled buttons go grey rather than dimming the accent. A faded accent still
 * reads as "the primary action", which is exactly the wrong signal, and standing rule 2
 * would rather the saturated colour left the screen altogether.
 */
export function Button({
  variant = 'bordered',
  icon: Icon,
  fullWidth = false,
  className,
  children,
  type = 'button',
  ...rest
}: ButtonProps) {
  return (
    <button
      {...rest}
      type={type}
      className={cx(styles.button, styles[variant], fullWidth && styles.fullWidth, className)}
    >
      {Icon && <Icon className={styles.icon} aria-hidden="true" strokeWidth={1.5} />}
      {children}
    </button>
  )
}
