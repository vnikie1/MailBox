import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentPropsWithRef,
  type FocusEvent,
  type MouseEvent,
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
  safePolygon,
  shift,
  useClick,
  useDismiss,
  useFloating,
  useFloatingNodeId,
  useFloatingParentNodeId,
  useFloatingTree,
  useHover,
  useInteractions,
  useListItem,
  useListNavigation,
  useMergeRefs,
  useRole,
  useTransitionStatus,
  useTypeahead,
  type Placement,
} from '@floating-ui/react'
import { Check, ChevronRight, X, type LucideIcon } from 'lucide-react'

import { cx } from '@/lib/cx'
import { durationToken, lengthToken } from '@/lib/tokens'

import { MenuContext, useMenuContext } from './menuContext'
import { withTriggerProps, type TriggerElement } from './floatingUtils'

import styles from './Menu.module.css'

/** Only reached when the cascade has not applied; these mirror the authored tokens. */
const OFFSET_FALLBACK_PX = 4
const PAD_FALLBACK_PX = 4
const SUBMENU_DELAY_FALLBACK_MS = 150
const DURATION_FALLBACK_MS = 100

interface SubmenuOpenEvent {
  nodeId: string | undefined
  parentId: string | null
}

export interface MenuProps {
  /** The menu's accessible name, and the row label when this is a submenu. */
  label: string
  children: ReactNode
  /** Root menus only. A submenu draws its own row inside the parent menu. */
  trigger?: TriggerElement
  placement?: Placement
  /** Submenus only: the leading glyph on the row that opens this menu. */
  icon?: LucideIcon
  disabled?: boolean
}

/**
 * A menu. docs/02 §6.9.
 *
 * Nesting is handled by Floating UI's `FloatingTree`, which is why the root wraps itself
 * in one: a submenu three levels down needs to know that choosing an item closes every
 * menu above it, and that opening it closes its siblings. Both are tree events rather
 * than prop drilling.
 *
 * Submenus open on a 150ms hover delay guarded by `safePolygon()` — the safe triangle.
 * Without it, moving the pointer diagonally from a parent row toward the submenu crosses
 * the row below and closes the thing you were aiming at, which is the most common way
 * Windows menus feel worse than macOS ones.
 */
export function Menu(props: MenuProps) {
  const parentId = useFloatingParentNodeId()

  if (parentId === null) {
    return (
      <FloatingTree>
        <MenuInner {...props} />
      </FloatingTree>
    )
  }

  return <MenuInner {...props} />
}

function MenuInner({
  label,
  children,
  trigger,
  placement,
  icon: Icon,
  disabled = false,
}: MenuProps) {
  const [isOpen, setIsOpen] = useState(false)
  const [activeIndex, setActiveIndex] = useState<number | null>(null)
  const [hasFocusInside, setHasFocusInside] = useState(false)

  // `HTMLElement`, not `HTMLButtonElement`: every row was a button until the flag strip,
  // which registers its container div as one navigable stop and moves between the swatches
  // itself. See `MenuSwatchRow`.
  const elementsRef = useRef<(HTMLElement | null)[]>([])
  const labelsRef = useRef<(string | null)[]>([])

  const tree = useFloatingTree()
  const nodeId = useFloatingNodeId()
  const parentId = useFloatingParentNodeId()
  const item = useListItem()
  const parent = useMenuContext()
  const isNested = parentId !== null

  const gap = useMemo(() => lengthToken('--menu-offset', OFFSET_FALLBACK_PX), [])
  const pad = useMemo(() => lengthToken('--menu-pad', PAD_FALLBACK_PX), [])
  const submenuDelay = useMemo(
    () => durationToken('--menu-submenu-delay', SUBMENU_DELAY_FALLBACK_MS),
    [],
  )
  const duration = useMemo(() => durationToken('--dur-micro', DURATION_FALLBACK_MS), [])

  const { refs, floatingStyles, context } = useFloating<HTMLButtonElement>({
    nodeId,
    open: isOpen,
    onOpenChange: setIsOpen,
    placement: isNested ? 'right-start' : (placement ?? 'bottom-start'),
    // A submenu sits flush against its parent panel, aligned with the row that opened it,
    // so its offset cancels the parent's own padding instead of adding a gap.
    middleware: [
      offset(isNested ? { mainAxis: 0, alignmentAxis: -pad } : { mainAxis: gap, alignmentAxis: 0 }),
      flip(),
      shift({ padding: gap }),
    ],
    whileElementsMounted: autoUpdate,
  })

  const interactions = useInteractions([
    useHover(context, {
      enabled: isNested && !disabled,
      delay: { open: submenuDelay },
      handleClose: safePolygon({ blockPointerEvents: true }),
    }),
    useClick(context, {
      event: 'mousedown',
      toggle: !isNested,
      ignoreMouse: isNested,
      enabled: !disabled,
    }),
    useRole(context, { role: 'menu' }),
    useDismiss(context, { bubbles: true }),
    useListNavigation(context, {
      listRef: elementsRef,
      activeIndex,
      nested: isNested,
      onNavigate: setActiveIndex,
    }),
    useTypeahead(context, {
      listRef: labelsRef,
      activeIndex,
      ...(isOpen ? { onMatch: setActiveIndex } : {}),
    }),
  ])

  const { isMounted, status } = useTransitionStatus(context, { duration })

  // Choosing an item anywhere in the tree closes the whole tree; opening a submenu closes
  // whichever sibling was open.
  useEffect(() => {
    if (!tree) return

    const closeAll = () => {
      setIsOpen(false)
    }
    const closeSiblings = (event: SubmenuOpenEvent) => {
      if (event.nodeId !== nodeId && event.parentId === parentId) setIsOpen(false)
    }

    tree.events.on('click', closeAll)
    tree.events.on('menuopen', closeSiblings)

    return () => {
      tree.events.off('click', closeAll)
      tree.events.off('menuopen', closeSiblings)
    }
  }, [tree, nodeId, parentId])

  useEffect(() => {
    if (isOpen && tree) {
      const event: SubmenuOpenEvent = { parentId, nodeId }
      tree.events.emit('menuopen', event)
    }
  }, [tree, isOpen, nodeId, parentId])

  const submenuRef = useMergeRefs([refs.setReference, item.ref])

  const referenceProps = interactions.getReferenceProps(
    parent.getItemProps({
      onFocus: () => {
        setHasFocusInside(false)
        parent.setHasFocusInside(true)
      },
    }),
  )

  const contextValue = useMemo(
    () => ({
      getItemProps: interactions.getItemProps,
      activeIndex,
      setHasFocusInside,
      isOpen,
    }),
    [interactions.getItemProps, activeIndex, isOpen],
  )

  return (
    <FloatingNode id={nodeId}>
      {isNested ? (
        <button
          ref={submenuRef}
          type="button"
          role="menuitem"
          disabled={disabled}
          aria-haspopup="menu"
          aria-expanded={isOpen}
          tabIndex={parent.activeIndex === item.index ? 0 : -1}
          data-open={isOpen ? '' : undefined}
          data-focus-inside={hasFocusInside ? '' : undefined}
          className={styles.item}
          {...referenceProps}
        >
          <span className={styles.lead} aria-hidden="true">
            {Icon && <Icon className={styles.icon} strokeWidth={1.5} />}
          </span>
          <span className={styles.label}>{label}</span>
          <ChevronRight className={styles.chevron} aria-hidden="true" strokeWidth={1.5} />
        </button>
      ) : (
        trigger &&
        withTriggerProps(
          trigger,
          (userProps) =>
            interactions.getReferenceProps(
              parent.getItemProps({
                ...userProps,
                onFocus: () => {
                  setHasFocusInside(false)
                  parent.setHasFocusInside(true)
                },
              }),
            ),
          { ref: refs.setReference },
        )
      )}

      <MenuContext value={contextValue}>
        {isMounted && (
          <FloatingPortal>
            <FloatingFocusManager
              context={context}
              modal={false}
              initialFocus={isNested ? -1 : 0}
              returnFocus={!isNested}
            >
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
                  {children}
                </FloatingList>
              </div>
            </FloatingFocusManager>
          </FloatingPortal>
        )}
      </MenuContext>
    </FloatingNode>
  )
}

export interface MenuSwatchRowProps {
  /** The text before the swatches. macOS Mail writes it "Flag:". */
  label: string
  /** The swatches, in the order they are drawn. */
  options: { value: string; name: string }[]
  /**
   * Which option is currently set.
   *
   * `null` means explicitly none — the clear cell takes the tick. `undefined` means the
   * selection disagrees, and then nothing is ticked, because claiming a colour they do not
   * share would be worse than saying nothing. `exactOptionalPropertyTypes` is on, so the
   * undefined has to be written out.
   */
  current?: string | null | undefined
  /** The leading cell that clears the value. Omitted when there is nothing to clear. */
  clearLabel?: string
  onPick: (value: string | null) => void
}

/**
 * A horizontal strip of colour swatches inside a menu. docs/01 §8 — Mail puts the flag
 * colours on one row under a "Flag:" label rather than in a submenu, because picking a
 * colour is done by eye and a vertical list of seven words is slower than a row of dots.
 *
 * ## Why this is one navigable stop rather than eight
 *
 * Floating UI's list navigation moves up and down a menu with the arrow keys. If each swatch
 * registered itself, Down would step sideways through eight colours before reaching Archive,
 * which is not how a menu behaves anywhere. So the *container* is the stop, and Left/Right
 * move between swatches once it has focus — the same shape as a radio group, and the reason
 * `elementsRef` above had to widen from a button to an element.
 *
 * Typeahead is deliberately given nothing: `useListItem` is called without a label, so
 * pressing "r" in the menu reaches "Reply" rather than being swallowed by "Red".
 */
export function MenuSwatchRow({ label, options, current, clearLabel, onPick }: MenuSwatchRowProps) {
  const menu = useMenuContext()
  const tree = useFloatingTree()
  // No `label`: this row is not a typeahead target. See the note above.
  const item = useListItem()
  const isActive = item.index === menu.activeIndex
  const containerRef = useRef<HTMLDivElement | null>(null)

  // `null` is the clear cell, and it leads so the row reads left to right as "none, then
  // these". Typed up front rather than inferred: an empty literal infers `never[]`.
  const cells: (string | null)[] = clearLabel === undefined ? [] : [null]
  cells.push(...options.map((option) => option.value))

  const move = (delta: number, from: HTMLElement) => {
    const buttons = [...(containerRef.current?.querySelectorAll('button') ?? [])]
    const index = buttons.indexOf(from as HTMLButtonElement)
    // Clamped rather than wrapped: running off the end of a row of colours and reappearing at
    // the other end is disorienting when the row is this short.
    const next = buttons[Math.min(Math.max(index + delta, 0), buttons.length - 1)]
    next?.focus()
  }

  return (
    <div
      ref={(node) => {
        containerRef.current = node
        item.ref(node)
      }}
      role="group"
      aria-label={label}
      className={styles.swatchRow}
      tabIndex={isActive ? 0 : -1}
      onFocus={(event) => {
        menu.setHasFocusInside(true)

        // Arrow-keying down the menu focuses this group, because the group is the navigable
        // stop. Focus is handed straight on to a swatch — the current colour if there is one —
        // so what the user sees is a ring around a colour rather than a ring around a strip
        // with no indication of which cell is live. Only when the group itself was the target:
        // a focus event bubbling up from a cell must not bounce focus back to the start.
        if (event.target !== event.currentTarget) return

        const buttons = [...event.currentTarget.querySelectorAll('button')]
        const marked = buttons.find((button) => button.getAttribute('aria-checked') === 'true')
        ;(marked ?? buttons[0])?.focus()
      }}
      onKeyDown={(event) => {
        if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return

        // Stopped before the menu sees it: left and right would otherwise close this menu or
        // open a submenu, which is what those keys mean everywhere else in a menu.
        event.preventDefault()
        event.stopPropagation()
        move(event.key === 'ArrowRight' ? 1 : -1, event.target as HTMLElement)
      }}
    >
      <span className={styles.swatchLabel}>{label}</span>

      {cells.map((value) => {
        const name = value === null ? clearLabel : options.find((o) => o.value === value)?.name

        return (
          <button
            key={value ?? '__clear__'}
            type="button"
            role="menuitemradio"
            aria-checked={current === value}
            aria-label={name}
            tabIndex={-1}
            className={cx(styles.swatchCell, value === null && styles.swatchClear)}
            {...(value === null ? {} : { style: { background: `var(--flag-${value})` } })}
            onClick={() => {
              onPick(value)
              tree?.events.emit('click')
            }}
          >
            {value === null && <X className={styles.swatchClearIcon} strokeWidth={2.5} />}
            {current === value && value !== null && (
              <Check className={styles.swatchTick} strokeWidth={3} />
            )}
          </button>
        )
      })}
    </div>
  )
}

export interface MenuItemProps extends Omit<ComponentPropsWithRef<'button'>, 'children'> {
  label: string
  icon?: LucideIcon
  /** Right-aligned keyboard hint. docs/02 §6.9. */
  shortcut?: string
  /** Renders a checkmark in the leading column and reports `aria-checked`. */
  checked?: boolean
  /**
   * A colour swatch in the trailing column — one of the seven flag colours by name.
   *
   * Here rather than in the feature because the alternative is a feature reaching past this
   * barrel to style a raw element, which is the one thing the design system forbids. It sits
   * in the trailing column so it never competes with the checkmark for the leading one.
   */
  swatch?: string
  destructive?: boolean
}

/**
 * One row of a menu. docs/02 §6.9.
 *
 * The leading column is always laid out, whether or not the item has an icon or a
 * checkmark, so a menu whose items are a mix of the two keeps every label on the same
 * left edge and nothing shifts when a checkable item is toggled — standing rule 6.
 */
export function MenuItem({
  label,
  icon: Icon,
  shortcut,
  checked,
  swatch,
  destructive = false,
  disabled = false,
  className,
  onClick,
  onFocus,
  ...rest
}: MenuItemProps) {
  const menu = useMenuContext()
  const tree = useFloatingTree()
  const item = useListItem({ label: disabled ? null : label })
  const isActive = item.index === menu.activeIndex

  return (
    <button
      {...rest}
      ref={item.ref}
      type="button"
      role={checked === undefined ? 'menuitem' : 'menuitemcheckbox'}
      disabled={disabled}
      tabIndex={isActive ? 0 : -1}
      className={cx(styles.item, destructive && styles.destructive, className)}
      {...(checked === undefined ? {} : { 'aria-checked': checked })}
      {...menu.getItemProps({
        onClick: (event) => {
          onClick?.(event as MouseEvent<HTMLButtonElement>)
          tree?.events.emit('click')
        },
        onFocus: (event) => {
          onFocus?.(event as FocusEvent<HTMLButtonElement>)
          menu.setHasFocusInside(true)
        },
      })}
    >
      <span className={styles.lead} aria-hidden="true">
        {checked === true ? (
          <Check className={styles.icon} strokeWidth={2} />
        ) : (
          Icon && <Icon className={styles.icon} strokeWidth={1.5} />
        )}
      </span>
      <span className={styles.label}>{label}</span>
      {swatch !== undefined && (
        <span
          className={styles.swatch}
          // The only inline colour in the design system. There are exactly seven, they are
          // named by the core, and the alternative is seven near-identical classes plus a
          // lookup mapping strings to them — the same indirection with more places to forget
          // one. The value is a token name, never a colour literal, so standing rule 1 holds.
          style={{ background: `var(--flag-${swatch})` }}
          aria-hidden="true"
        />
      )}
      {shortcut !== undefined && (
        <span className={cx(styles.shortcut, 'tabular')} aria-hidden="true">
          {shortcut}
        </span>
      )}
    </button>
  )
}

/** docs/02 §6.9 — 1px separator, margin 4 8. */
export function MenuSeparator() {
  return <div role="separator" className={styles.separator} />
}

export interface MenuSectionProps {
  label: string
  children: ReactNode
}

/**
 * A titled group of items — the shape docs/02 §6.6 asks for in the search dropdown.
 * `role="group"` rather than a bare heading, so a screen reader announces the group name
 * as focus crosses into it instead of reading a stray line of text.
 */
export function MenuSection({ label, children }: MenuSectionProps) {
  return (
    <div role="group" aria-label={label} className={styles.section}>
      <span className={styles.sectionLabel} aria-hidden="true">
        {label}
      </span>
      {children}
    </div>
  )
}
