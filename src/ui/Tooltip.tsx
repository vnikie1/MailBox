import { useId, useMemo, useState, type ReactNode } from 'react'
import {
  FloatingDelayGroup,
  FloatingPortal,
  autoUpdate,
  flip,
  offset,
  shift,
  useDelayGroup,
  useDismiss,
  useFloating,
  useFocus,
  useHover,
  useInteractions,
  useRole,
  useTransitionStatus,
  type Placement,
} from '@floating-ui/react'

import { cx } from '@/lib/cx'
import { durationToken, lengthToken } from '@/lib/tokens'

import { withTriggerProps, type TriggerElement } from './floatingUtils'

import styles from './Tooltip.module.css'

/** Only reached when the cascade has not applied; mirrors the authored token values. */
const OFFSET_FALLBACK_PX = 4
const DELAY_FALLBACK_MS = 500
const GROUP_TIMEOUT_FALLBACK_MS = 400
const DURATION_FALLBACK_MS = 100

export interface TooltipGroupProps {
  children: ReactNode
}

/**
 * Makes every tooltip inside it share one delay.
 *
 * Wrap a toolbar in this and the first tooltip waits its half second, after which moving
 * along the row shows each one instantly. Without it, crossing eight icon buttons means
 * eight separate half-second waits, which is how a toolbar ends up feeling sticky.
 */
export function TooltipGroup({ children }: TooltipGroupProps) {
  const delay = durationToken('--tooltip-delay', DELAY_FALLBACK_MS)
  const timeoutMs = durationToken('--tooltip-group-timeout', GROUP_TIMEOUT_FALLBACK_MS)

  return (
    <FloatingDelayGroup delay={{ open: delay, close: 0 }} timeoutMs={timeoutMs}>
      {children}
    </FloatingDelayGroup>
  )
}

export interface TooltipProps {
  trigger: TriggerElement
  content: ReactNode
  placement?: Placement
  /** Suppress without unmounting — a control whose label is already visible. */
  disabled?: boolean
}

/**
 * A hover/focus label. docs/01 §13 — every icon-only control needs one.
 *
 * `useFocus({ visibleOnly: true })` matters more than it looks: without it, clicking a
 * toolbar button pops its tooltip open over whatever the click just did, because the
 * click leaves the button focused.
 *
 * The tooltip is `role="tooltip"` and referenced by the trigger's `aria-describedby`,
 * which `useRole` wires up. It is never the trigger's only accessible name — IconButton
 * requires its own `label` for that — because a tooltip a screen reader user cannot hover
 * is not a label.
 */
export function Tooltip({
  trigger,
  content,
  placement = 'bottom',
  disabled = false,
}: TooltipProps) {
  const [isOpen, setIsOpen] = useState(false)
  const id = useId()

  const gap = useMemo(() => lengthToken('--tooltip-offset', OFFSET_FALLBACK_PX), [])

  const { refs, floatingStyles, context } = useFloating({
    open: isOpen && !disabled,
    onOpenChange: setIsOpen,
    placement,
    middleware: [offset(gap), flip({ padding: gap }), shift({ padding: gap })],
    whileElementsMounted: autoUpdate,
  })

  const { delay, isInstantPhase } = useDelayGroup(context, { id })

  const interactions = useInteractions([
    useHover(context, { delay, move: false, enabled: !disabled }),
    useFocus(context, { visibleOnly: true, enabled: !disabled }),
    useDismiss(context, { referencePress: true }),
    useRole(context, { role: 'tooltip' }),
  ])

  const duration = useMemo(() => durationToken('--dur-micro', DURATION_FALLBACK_MS), [])
  const { isMounted, status } = useTransitionStatus(context, {
    duration: isInstantPhase ? 0 : duration,
  })

  return (
    <>
      {withTriggerProps(trigger, interactions.getReferenceProps, { ref: refs.setReference })}

      {isMounted && (
        <FloatingPortal>
          <div
            ref={refs.setFloating}
            style={floatingStyles}
            data-status={status}
            className={cx(styles.tooltip)}
            {...interactions.getFloatingProps()}
          >
            {content}
          </div>
        </FloatingPortal>
      )}
    </>
  )
}
