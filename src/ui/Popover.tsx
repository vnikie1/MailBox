import { useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import {
  FloatingArrow,
  FloatingFocusManager,
  FloatingPortal,
  arrow,
  autoUpdate,
  flip,
  offset,
  shift,
  useClick,
  useDismiss,
  useFloating,
  useInteractions,
  useRole,
  useTransitionStatus,
  type Placement,
} from '@floating-ui/react'

import { cx } from '@/lib/cx'
import { durationToken, lengthToken } from '@/lib/tokens'

import { transformOriginFor, withTriggerProps, type TriggerElement } from './floatingUtils'

import styles from './Popover.module.css'

/**
 * Fallbacks for the two tokens this component reads as numbers. They are only reached
 * when the cascade has not been applied — jsdom, essentially — and mirror the authored
 * values of `--popover-offset` and `--dur-base`. The real values always come from CSS.
 */
const OFFSET_FALLBACK_PX = 6
const DURATION_FALLBACK_MS = 200

export interface PopoverProps {
  trigger: TriggerElement
  children: ReactNode
  /** Names the popover for assistive technology. */
  label?: string
  placement?: Placement
  showArrow?: boolean
  /** Controlled open state. Omit both to let the popover manage its own. */
  open?: boolean
  onOpenChange?: (open: boolean) => void
  className?: string | undefined
}

/**
 * A floating panel anchored to a control. docs/02 §4 (shadow), §5 (material).
 *
 * `modal={false}`: a macOS popover does not black out the window behind it. Focus moves
 * into the panel and Escape returns it to the trigger, but the rest of the app stays
 * live and clicking outside dismisses. `Sheet` is the modal counterpart.
 *
 * Shadows are allowed here — standing rule 4 permits them on floating layers and nowhere
 * else.
 */
export function Popover({
  trigger,
  children,
  label,
  placement = 'bottom-start',
  showArrow = false,
  open,
  onOpenChange,
  className,
}: PopoverProps) {
  const [uncontrolledOpen, setUncontrolledOpen] = useState(false)
  const isOpen = open ?? uncontrolledOpen

  const arrowRef = useRef<SVGSVGElement>(null)
  const gap = useMemo(() => lengthToken('--popover-offset', OFFSET_FALLBACK_PX), [])
  const duration = useMemo(() => durationToken('--dur-base', DURATION_FALLBACK_MS), [])

  const { refs, floatingStyles, context } = useFloating({
    open: isOpen,
    onOpenChange: (next) => {
      setUncontrolledOpen(next)
      onOpenChange?.(next)
    },
    placement,
    // flip() then shift() is the order that matters: try the opposite side first, and
    // only slide along the edge if neither side fits. The reverse pins panels to the
    // screen edge while a perfectly good side sits empty.
    middleware: [
      offset(gap),
      flip({ padding: gap }),
      shift({ padding: gap }),
      ...(showArrow ? [arrow({ element: arrowRef })] : []),
    ],
    whileElementsMounted: autoUpdate,
  })

  const interactions = useInteractions([
    useClick(context),
    useDismiss(context),
    useRole(context, { role: 'dialog' }),
  ])

  const { isMounted, status } = useTransitionStatus(context, { duration })

  const style = {
    ...floatingStyles,
    '--popover-origin': transformOriginFor(context.placement),
  } as CSSProperties

  return (
    <>
      {withTriggerProps(trigger, interactions.getReferenceProps, { ref: refs.setReference })}

      {isMounted && (
        <FloatingPortal>
          <FloatingFocusManager context={context} modal={false}>
            <div
              ref={refs.setFloating}
              style={style}
              data-status={status}
              className={cx(styles.popover, className)}
              {...interactions.getFloatingProps()}
              {...(label === undefined ? {} : { 'aria-label': label })}
            >
              {children}
              {showArrow && (
                <FloatingArrow ref={arrowRef} context={context} className={styles.arrow} />
              )}
            </div>
          </FloatingFocusManager>
        </FloatingPortal>
      )}
    </>
  )
}
