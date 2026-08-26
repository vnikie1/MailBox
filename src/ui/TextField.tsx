import { useId, type ComponentPropsWithRef, type ReactNode } from 'react'
import { X, type LucideIcon } from 'lucide-react'

import { cx } from '@/lib/cx'

import styles from './TextField.module.css'

export interface TextFieldProps extends Omit<ComponentPropsWithRef<'input'>, 'size'> {
  /** Required: every field has a name, even when the design shows only a placeholder. */
  label: string
  /** Hide the label visually, keeping it for assistive technology. */
  hideLabel?: boolean
  /**
   * `search` is the toolbar field of docs/02 §6.6 — 200 wide, expanding to 320 on focus.
   * `field` is the same surface without the width animation.
   */
  variant?: 'field' | 'search'
  leadingIcon?: LucideIcon
  invalid?: boolean
  /**
   * Help or error text. Reserve it from the start if the field can become invalid: the
   * slot appearing later would push everything below it down, which standing rule 6
   * forbids.
   */
  description?: ReactNode
  onClear?: () => void
  clearLabel?: string
  className?: string | undefined
  fieldClassName?: string | undefined
}

/**
 * A single-line text field. docs/02 §6.6.
 *
 * The focus ring is on the container rather than the input, via `:has()`. The input
 * itself has no visible box — the rounded fill belongs to the wrapper — so the global
 * `:focus-visible` ring would otherwise draw a rectangle around an invisible element
 * inside a rounded one.
 */
export function TextField({
  label,
  hideLabel = false,
  variant = 'field',
  leadingIcon: LeadingIcon,
  invalid = false,
  description,
  onClear,
  clearLabel,
  className,
  fieldClassName,
  id,
  value,
  ...rest
}: TextFieldProps) {
  const generatedId = useId()
  const inputId = id ?? generatedId
  const descriptionId = `${inputId}-description`
  const hasValue = value !== undefined && value !== ''

  return (
    <div className={cx(styles.wrap, className)}>
      <label htmlFor={inputId} className={cx(styles.label, hideLabel && 'srOnly')}>
        {label}
      </label>

      <div
        className={cx(
          styles.control,
          variant === 'search' && styles.search,
          invalid && styles.invalid,
          fieldClassName,
        )}
      >
        {LeadingIcon && (
          <LeadingIcon className={styles.leading} aria-hidden="true" strokeWidth={1.5} />
        )}
        <input
          {...rest}
          id={inputId}
          className={styles.input}
          aria-invalid={invalid}
          {...(description ? { 'aria-describedby': descriptionId } : {})}
          {...(value === undefined ? {} : { value })}
        />
        {onClear && (
          <button
            type="button"
            tabIndex={-1}
            className={cx(styles.clear, hasValue && styles.clearVisible)}
            aria-label={clearLabel ?? `Clear ${label}`}
            onMouseDown={(event) => {
              event.preventDefault()
            }}
            onClick={onClear}
          >
            <X aria-hidden="true" strokeWidth={2} />
          </button>
        )}
      </div>

      {description !== undefined && (
        <span
          id={descriptionId}
          className={cx(styles.description, invalid && styles.descriptionInvalid)}
        >
          {description}
        </span>
      )}
    </div>
  )
}
