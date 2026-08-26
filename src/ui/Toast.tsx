import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { X } from 'lucide-react'

import { cx } from '@/lib/cx'
import { durationToken } from '@/lib/tokens'

import { Button } from './Button'
import { IconButton } from './IconButton'
import { ToastContext, type ToastApi, type ToastOptions } from './toastContext'

import styles from './Toast.module.css'

/** Only reached when the cascade has not applied; these mirror the authored tokens. */
const DWELL_FALLBACK_MS = 4000
const EXIT_FALLBACK_MS = 200

interface ToastEntry {
  id: string
  options: ToastOptions
  leaving: boolean
}

let nextToastId = 0

export interface ToastProviderProps {
  children: ReactNode
}

/**
 * Transient status messages.
 *
 * Two things are deliberate here:
 *
 *  - The dwell timer pauses while the pointer is over the stack or focus is inside it.
 *    A toast that offers Undo and then withdraws the offer while you are reaching for it
 *    is worse than no toast, and this is the whole reason the primitive exists.
 *  - Unmounting is scheduled from the duration *token*, not from `transitionend`. Under
 *    `prefers-reduced-motion` the token is 0ms and the event never fires, which would
 *    leak every toast for exactly the users who asked for less motion. See lib/tokens.ts.
 */
export function ToastProvider({ children }: ToastProviderProps) {
  const [toasts, setToasts] = useState<ToastEntry[]>([])
  const [paused, setPaused] = useState(false)

  const dwellTimers = useRef(new Map<string, number>())
  const exitTimers = useRef(new Map<string, number>())

  const remove = useCallback((id: string) => {
    setToasts((current) => current.filter((toast) => toast.id !== id))
    exitTimers.current.delete(id)
  }, [])

  const dismiss = useCallback(
    (id: string) => {
      window.clearTimeout(dwellTimers.current.get(id))
      dwellTimers.current.delete(id)

      setToasts((current) =>
        current.map((toast) => (toast.id === id ? { ...toast, leaving: true } : toast)),
      )

      const exit = durationToken('--dur-base', EXIT_FALLBACK_MS)
      exitTimers.current.set(
        id,
        window.setTimeout(() => {
          remove(id)
        }, exit),
      )
    },
    [remove],
  )

  const show = useCallback((options: ToastOptions) => {
    nextToastId += 1
    const id = `toast-${nextToastId}`
    setToasts((current) => [...current, { id, options, leaving: false }])
    return id
  }, [])

  // One effect owns every dwell timer, so pausing is a single re-run rather than a timer
  // per toast that each has to learn about the hover state.
  useEffect(() => {
    if (paused) {
      dwellTimers.current.forEach((timer) => {
        window.clearTimeout(timer)
      })
      dwellTimers.current.clear()
      return
    }

    const fallback = durationToken('--toast-dwell', DWELL_FALLBACK_MS)

    toasts.forEach((toast) => {
      if (toast.leaving || dwellTimers.current.has(toast.id)) return

      dwellTimers.current.set(
        toast.id,
        window.setTimeout(() => {
          dismiss(toast.id)
        }, toast.options.duration ?? fallback),
      )
    })
  }, [toasts, paused, dismiss])

  useEffect(() => {
    const dwell = dwellTimers.current
    const exit = exitTimers.current
    return () => {
      dwell.forEach((timer) => {
        window.clearTimeout(timer)
      })
      exit.forEach((timer) => {
        window.clearTimeout(timer)
      })
    }
  }, [])

  const api = useMemo<ToastApi>(() => ({ show, dismiss }), [show, dismiss])

  return (
    <ToastContext value={api}>
      {children}

      <div
        className={styles.viewport}
        role="region"
        aria-label="Notifications"
        onMouseEnter={() => {
          setPaused(true)
        }}
        onMouseLeave={() => {
          setPaused(false)
        }}
        onFocusCapture={() => {
          setPaused(true)
        }}
        onBlurCapture={() => {
          setPaused(false)
        }}
      >
        {toasts.map((toast) => (
          <Toast
            key={toast.id}
            {...toast.options}
            leaving={toast.leaving}
            onDismiss={() => {
              dismiss(toast.id)
            }}
          />
        ))}
      </div>
    </ToastContext>
  )
}

export interface ToastProps extends ToastOptions {
  leaving?: boolean
  onDismiss?: () => void
  className?: string | undefined
}

/**
 * One toast. Exported on its own so the gallery can show its resting state without
 * running a timer, and so Phase 2 can lay one out statically while checking metrics.
 *
 * `role="status"` rather than `alert`: these report that something the user just asked
 * for has happened. An alert interrupts whatever a screen reader was saying, which is
 * right for a failure and wrong for a confirmation.
 */
export function Toast({
  title,
  description,
  icon: Icon,
  action,
  leaving = false,
  onDismiss,
  className,
}: ToastProps) {
  return (
    <div
      role="status"
      aria-live="polite"
      data-status={leaving ? 'closed' : 'open'}
      className={cx(styles.toast, className)}
    >
      {Icon && <Icon className={styles.icon} aria-hidden="true" strokeWidth={1.5} />}

      <div className={styles.text}>
        <span className={styles.title}>{title}</span>
        {description !== undefined && <span className={styles.description}>{description}</span>}
      </div>

      {action && (
        <Button variant="plain" onClick={action.onAction}>
          {action.label}
        </Button>
      )}

      {onDismiss && <IconButton icon={X} label="Dismiss" size="sm" onClick={onDismiss} />}
    </div>
  )
}
