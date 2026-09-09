import { useId, type ComponentPropsWithRef, type ReactNode } from 'react'
import { ChevronDown } from 'lucide-react'

import { cx } from '@/lib/cx'

import styles from './Select.module.css'

export interface SelectOption<T extends string | number> {
  value: T
  label: string
  disabled?: boolean
}

export interface SelectProps<T extends string | number> extends Omit<
  ComponentPropsWithRef<'select'>,
  'onChange' | 'value' | 'children' | 'size'
> {
  /** Required: every control has a name, even where the design hides it. */
  label: string
  hideLabel?: boolean
  options: readonly SelectOption<T>[]
  /** `null` while the stored value is still loading — nothing is selected until it arrives. */
  value: T | null
  onValueChange: (value: T) => void
  description?: ReactNode
  className?: string | undefined
}

/**
 * A popup button. docs/02 §6 — the primitive that was missing.
 *
 * ## Why this exists
 *
 * Six settings were vertical radio stacks: theme, density, translucency, undo-send delay,
 * export format, signature placement. Twenty rows between them, in a window 580px tall, for
 * six answers. Mail uses a popup menu for exactly this shape of choice — several options, one
 * answer, the answer readable at a glance without reading the alternatives — and the radios
 * were making the settings window scroll to say very little.
 *
 * ## Why a native `<select>`
 *
 * The list itself is drawn by Windows, not by us. A hand-built listbox is a keyboard trap
 * waiting to happen and would draw a menu that is nearly the OS one; the native control is
 * already correct for Narrator, for touch, for the keyboard, and for the high-contrast themes.
 * `color-scheme` on `:root` is what makes its popup follow the app's theme, which is why the
 * dark menu appears without anything here asking for it.
 *
 * Only the closed button is ours: `appearance: none`, the house field surface, and a chevron
 * laid over it. `Menu` remains the right primitive for a menu of *commands*; this is for a
 * value.
 */
export function Select<T extends string | number>({
  label,
  hideLabel = false,
  options,
  value,
  onValueChange,
  description,
  className,
  id,
  disabled,
  ...rest
}: SelectProps<T>) {
  const generatedId = useId()
  const selectId = id ?? generatedId
  const descriptionId = `${selectId}-description`

  return (
    <div className={cx(styles.wrap, className)}>
      {/*
        Hidden means *gone*, not visually hidden.

        A `srOnly` label is still a label, and in a settings form the row already has one —
        `Field` renders a `<label for>` pointing at this same select. Two labels for one control
        are concatenated by the accessible-name computation, so the popup announced itself as
        "Translucency Translucency". Dropping this element and naming the control with
        `aria-label` instead also settles the precedence: `aria-label` outranks a native label,
        so the name is this one string whether or not a form row supplied another.
      */}
      {!hideLabel && (
        <label htmlFor={selectId} className={styles.label}>
          {label}
        </label>
      )}

      <div className={styles.control}>
        <select
          {...rest}
          id={selectId}
          className={styles.select}
          {...(hideLabel ? { 'aria-label': label } : {})}
          // The empty string is not one of the options, so the control shows blank rather than
          // the first entry while the stored value is still being read. A popup that shows an
          // answer before it knows one is a popup that appears to change the setting by itself.
          value={value === null ? '' : String(value)}
          disabled={disabled ?? value === null}
          {...(description === undefined ? {} : { 'aria-describedby': descriptionId })}
          onChange={(event) => {
            const chosen = options.find((option) => String(option.value) === event.target.value)
            if (chosen !== undefined) onValueChange(chosen.value)
          }}
        >
          {value === null && <option value="" />}
          {options.map((option) => (
            <option
              key={String(option.value)}
              value={String(option.value)}
              disabled={option.disabled}
            >
              {option.label}
            </option>
          ))}
        </select>

        <ChevronDown className={styles.chevron} aria-hidden="true" strokeWidth={1.75} />
      </div>

      {description !== undefined && (
        <span id={descriptionId} className={styles.description}>
          {description}
        </span>
      )}
    </div>
  )
}
