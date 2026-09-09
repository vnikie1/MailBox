import type { ReactNode } from 'react'

import { cx } from '@/lib/cx'

import styles from './settings.module.css'

/**
 * The two-column form the whole settings window is laid out on. docs/06 Phase 11.
 *
 * Mail's settings panes are forms: a quiet label right-aligned in a fixed column, the control
 * in the next column, every control down the pane on the same axis. This window stacked
 * label-above-control at one indentation instead, which spends a row of height on each label
 * and — the part that actually hurts — gives the eye no column to run down, so finding one
 * control means reading all of them.
 *
 * ## Why a fragment rather than a wrapper
 *
 * `Field` renders a label and a control cell as *siblings*, with no element between them and
 * the grid. A wrapper per row would make each row its own formatting context, and then the
 * column widths would be whatever each row's own content wanted — which is a stack again,
 * drawn with more markup. The alignment is the entire feature, so the grid has to own it.
 */
export function Form({
  children,
  className,
}: {
  children: ReactNode
  className?: string | undefined
}) {
  return <div className={cx(styles.form, className)}>{children}</div>
}

export interface FieldProps {
  /** Omitted for a row of checkboxes, which Mail leaves unlabelled with the column kept. */
  label?: ReactNode
  /**
   * The id of the control this names, when that control is a single form element. Makes the
   * visible text its real `<label>`, so clicking the text focuses it.
   */
  htmlFor?: string
  /**
   * The id to put *on* the label, for a control named by `aria-labelledby` — a radio group or
   * a segmented control, which have no single element for `htmlFor` to point at.
   */
  labelId?: string
  /** A note about this control, in the control column beneath it. */
  hint?: ReactNode
  children?: ReactNode
}

export function Field({ label, htmlFor, labelId, hint, children }: FieldProps) {
  return (
    <>
      {htmlFor === undefined ? (
        <span
          id={labelId}
          className={styles.label}
          // An empty label cell is a spacer, not a thing to announce. Without this a screen
          // reader walks a blank element before every checkbox group in the window.
          {...(label === undefined ? { 'aria-hidden': true } : {})}
        >
          {label}
        </span>
      ) : (
        <label id={labelId} htmlFor={htmlFor} className={styles.label}>
          {label}
        </label>
      )}

      <div className={styles.control}>
        {children}
        {hint !== undefined && <p className={styles.hint}>{hint}</p>}
      </div>
    </>
  )
}

/** A row that needs the whole width — an editor, a list, a table of its own. */
export function FullRow({
  children,
  className,
}: {
  children: ReactNode
  className?: string | undefined
}) {
  return <div className={cx(styles.full, className)}>{children}</div>
}
