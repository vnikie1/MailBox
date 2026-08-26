import { useId, useMemo, type ReactNode } from 'react'
import {
  FloatingFocusManager,
  FloatingOverlay,
  FloatingPortal,
  useDismiss,
  useFloating,
  useInteractions,
  useRole,
  useTransitionStatus,
} from '@floating-ui/react'

import { cx } from '@/lib/cx'
import { durationToken } from '@/lib/tokens'

import styles from './Sheet.module.css'

const DURATION_FALLBACK_MS = 200

export interface SheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  children?: ReactNode
  /** Buttons. Laid out right-aligned, primary action last, as macOS does. */
  footer?: ReactNode
  className?: string | undefined
}

/**
 * A modal panel. The counterpart to `Popover`, which is not modal.
 *
 * Focus is trapped and the scrim swallows clicks, because a sheet asks a question that
 * has to be answered before anything else happens — a confirmation, an account
 * credential. Escape and a click outside both dismiss, which means a sheet must never be
 * the only way to avoid losing data.
 *
 * The title is wired to `aria-labelledby` rather than passed as `aria-label`, so the
 * heading a sighted user reads and the name a screen reader announces cannot drift apart.
 */
export function Sheet({
  open,
  onOpenChange,
  title,
  description,
  children,
  footer,
  className,
}: SheetProps) {
  const titleId = useId()
  const descriptionId = useId()

  const { refs, context } = useFloating({ open, onOpenChange })

  const interactions = useInteractions([
    useDismiss(context, { outsidePressEvent: 'mousedown' }),
    useRole(context, { role: 'dialog' }),
  ])

  const duration = useMemo(() => durationToken('--dur-base', DURATION_FALLBACK_MS), [])
  const { isMounted, status } = useTransitionStatus(context, { duration })

  if (!isMounted) return null

  return (
    <FloatingPortal>
      <FloatingOverlay lockScroll className={styles.overlay} data-status={status}>
        <FloatingFocusManager context={context}>
          <div
            ref={refs.setFloating}
            data-status={status}
            className={cx(styles.sheet, className)}
            {...interactions.getFloatingProps()}
            aria-labelledby={titleId}
            {...(description === undefined ? {} : { 'aria-describedby': descriptionId })}
          >
            <h2 id={titleId} className={styles.title}>
              {title}
            </h2>

            {description !== undefined && (
              <p id={descriptionId} className={styles.description}>
                {description}
              </p>
            )}

            {children}

            {footer !== undefined && <div className={styles.footer}>{footer}</div>}
          </div>
        </FloatingFocusManager>
      </FloatingOverlay>
    </FloatingPortal>
  )
}
