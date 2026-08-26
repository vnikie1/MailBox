import type { ComponentPropsWithRef, ReactNode } from 'react'
import { X } from 'lucide-react'

import { cx } from '@/lib/cx'

import styles from './Chip.module.css'

export interface ChipProps extends Omit<ComponentPropsWithRef<'span'>, 'children'> {
  label: string
  /**
   * `neutral` is the compose recipient chip (docs/02 §6.7), `accent` the search token
   * (§6.6), `invalid` an address that does not parse. Nothing else may be tinted —
   * standing rule 2.
   */
  tone?: 'neutral' | 'accent' | 'invalid'
  /** Selected by Backspace, before a second press deletes it. docs/02 §6.6. */
  selected?: boolean
  leading?: ReactNode
  onRemove?: () => void
  removeLabel?: string
}

/**
 * A token capsule. docs/02 §6.6 (search) and §6.7 (recipients).
 *
 * The close button is always laid out and only fades in on hover. The doc says it
 * "appears on hover", but making it appear by entering the layout would resize the chip
 * and shove every chip after it sideways, which standing rule 6 forbids outright.
 */
export function Chip({
  label,
  tone = 'neutral',
  selected = false,
  leading,
  onRemove,
  removeLabel,
  className,
  ...rest
}: ChipProps) {
  return (
    <span
      {...rest}
      className={cx(styles.chip, styles[tone], selected && styles.selected, className)}
    >
      {leading}
      <span className={styles.label}>{label}</span>
      {onRemove && (
        <button
          type="button"
          /* Not in the tab order on purpose: a compose window with nine recipients would
             otherwise cost nine tab stops to cross. Backspace on the field is the
             keyboard path, and it is the one macOS uses. */
          tabIndex={-1}
          className={styles.remove}
          aria-label={removeLabel ?? `Remove ${label}`}
          onMouseDown={(event) => {
            // Keep focus where it is; the field must not blur out from under the click.
            event.preventDefault()
          }}
          onClick={(event) => {
            event.stopPropagation()
            onRemove()
          }}
        >
          <X aria-hidden="true" strokeWidth={2} />
        </button>
      )}
    </span>
  )
}
