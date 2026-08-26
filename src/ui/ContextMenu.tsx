import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from 'react'
import {
  FloatingFocusManager,
  FloatingList,
  FloatingNode,
  FloatingPortal,
  FloatingTree,
  autoUpdate,
  flip,
  offset,
  shift,
  useDismiss,
  useFloating,
  useFloatingNodeId,
  useFloatingTree,
  useInteractions,
  useListNavigation,
  useRole,
  useTransitionStatus,
  useTypeahead,
} from '@floating-ui/react'

import { cx } from '@/lib/cx'
import { durationToken, lengthToken } from '@/lib/tokens'

import { MenuContext } from './menuContext'

import styles from './Menu.module.css'

/** Only reached when the cascade has not applied; these mirror the authored tokens. */
const OFFSET_FALLBACK_PX = 4
const DURATION_FALLBACK_MS = 100

export interface ContextMenuProps {
  /** The menu's accessible name. */
  label: string
  /** The rows. The same `MenuItem`, `MenuSeparator`, `MenuSection` and nested `Menu`. */
  menu: ReactNode
  /** The region that responds to a right-click. */
  children: ReactNode
  className?: string | undefined
}

/**
 * A right-click menu. docs/02 §6.9 — identical in appearance to `Menu`, and it shares
 * that stylesheet; the only difference is that it is anchored to the pointer rather than
 * to a control.
 *
 * That anchoring is why this is not simply `Menu` with a different trigger: Floating UI
 * positions against a *virtual* element built from the click coordinates, so there is no
 * reference node to hand to `useClick` at all.
 */
export function ContextMenu(props: ContextMenuProps) {
  return (
    <FloatingTree>
      <ContextMenuInner {...props} />
    </FloatingTree>
  )
}

function ContextMenuInner({ label, menu, children, className }: ContextMenuProps) {
  const [isOpen, setIsOpen] = useState(false)
  const [activeIndex, setActiveIndex] = useState<number | null>(null)

  const elementsRef = useRef<(HTMLButtonElement | null)[]>([])
  const labelsRef = useRef<(string | null)[]>([])

  const nodeId = useFloatingNodeId()
  const tree = useFloatingTree()

  const gap = useMemo(() => lengthToken('--menu-offset', OFFSET_FALLBACK_PX), [])
  const duration = useMemo(() => durationToken('--dur-micro', DURATION_FALLBACK_MS), [])

  const { refs, floatingStyles, context } = useFloating({
    nodeId,
    open: isOpen,
    onOpenChange: setIsOpen,
    // Down and to the right of the pointer, flipping near an edge — the platform
    // convention on both platforms, and the one Windows users will expect here.
    placement: 'right-start',
    middleware: [offset({ mainAxis: gap, alignmentAxis: gap }), flip(), shift({ padding: gap })],
    whileElementsMounted: autoUpdate,
  })

  const interactions = useInteractions([
    useRole(context, { role: 'menu' }),
    useDismiss(context, { bubbles: true }),
    useListNavigation(context, {
      listRef: elementsRef,
      activeIndex,
      onNavigate: setActiveIndex,
    }),
    useTypeahead(context, {
      listRef: labelsRef,
      activeIndex,
      ...(isOpen ? { onMatch: setActiveIndex } : {}),
    }),
  ])

  const { isMounted, status } = useTransitionStatus(context, { duration })

  useEffect(() => {
    if (!tree) return

    const closeAll = () => {
      setIsOpen(false)
    }
    tree.events.on('click', closeAll)
    return () => {
      tree.events.off('click', closeAll)
    }
  }, [tree])

  const handleContextMenu = (event: ReactMouseEvent<HTMLDivElement>) => {
    event.preventDefault()

    const { clientX: x, clientY: y } = event
    refs.setPositionReference({
      getBoundingClientRect: () => ({
        width: 0,
        height: 0,
        x,
        y,
        top: y,
        left: x,
        right: x,
        bottom: y,
      }),
    })

    setIsOpen(true)
  }

  const contextValue = useMemo(
    () => ({
      getItemProps: interactions.getItemProps,
      activeIndex,
      setHasFocusInside: () => undefined,
      isOpen,
    }),
    [interactions.getItemProps, activeIndex, isOpen],
  )

  return (
    <FloatingNode id={nodeId}>
      <div className={cx(styles.region, className)} onContextMenu={handleContextMenu}>
        {children}
      </div>

      <MenuContext value={contextValue}>
        {isMounted && (
          <FloatingPortal>
            <FloatingFocusManager context={context} modal={false} initialFocus={0}>
              <div
                ref={refs.setFloating}
                style={floatingStyles}
                data-status={status}
                className={styles.menu}
                {...interactions.getFloatingProps()}
                /* After the spread, and clearing aria-labelledby with it. useRole()
                   labels a menu by its trigger, which silently outranks aria-label —
                   leaving a context menu (which has no trigger text) anonymous and a
                   submenu named after the row that opened it rather than after itself. */
                aria-labelledby={undefined}
                aria-label={label}
              >
                <FloatingList elementsRef={elementsRef} labelsRef={labelsRef}>
                  {menu}
                </FloatingList>
              </div>
            </FloatingFocusManager>
          </FloatingPortal>
        )}
      </MenuContext>
    </FloatingNode>
  )
}
