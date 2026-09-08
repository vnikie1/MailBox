import {
  useCallback,
  useMemo,
  useState,
  type DragEvent,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from 'react'
import { AlertTriangle, ChevronRight, PanelLeft, Settings } from 'lucide-react'

import { cx } from '@/lib/cx'
import { canDropInMailbox, draggedMessageIds } from '@/lib/messageDrag'
import { useLayoutStore } from '@/store/layout'
import {
  useAccounts,
  useFlagNames,
  useMailboxes,
  useMoveMessages,
  useSmartMailboxes,
  useVips,
} from '@/app/queries'
import { useMailStore } from '@/store/mail'
import { Badge, Button, ContextMenu, EmptyState, IconButton, ScrollArea, Tooltip } from '@/ui'

import { useSyncState } from '@/app/useSync'

import { SyncStatus } from './SyncStatus'
import {
  buildSidebar,
  canOpenMailboxMenu,
  selectionForNode,
  visibleRows,
  type SidebarNode,
} from './model'

import styles from './Sidebar.module.css'

/**
 * The absent case, as one array rather than a fresh one per render.
 *
 * `data ?? []` looks equivalent and is not: it allocates on every render, so the memo that
 * builds the tree sees a new dependency every time and rebuilds the whole sidebar. The
 * destructuring default this replaced had the same flaw and eslint could not see it.
 */
const NONE: never[] = []

interface SidebarRowProps {
  node: SidebarNode
  selected: boolean
  collapsed: boolean
  dropTarget: boolean
  onSelect: (node: SidebarNode) => void
  onToggle: (id: string) => void
  onDragOverRow: (node: SidebarNode | null) => void
  /** Puts this row's highlight out — but only if it is the row currently lit. */
  onDragLeaveRow: (node: SidebarNode) => void
  onDropRow: (mailboxId: number, messageIds: number[]) => void
}

function SidebarRow({
  node,
  selected,
  collapsed,
  dropTarget,
  onSelect,
  onToggle,
  onDragOverRow,
  onDragLeaveRow,
  onDropRow,
}: SidebarRowProps) {
  const Icon = node.icon
  const hasChildren = node.children.length > 0

  /**
   * Where a drop on this row would put the mail, or null if it is not a folder.
   *
   * A container row has no mailbox and a unified row has several, so in both cases there is
   * no single destination — and lighting them up would promise something the drop cannot
   * deliver. Predicate rows are excluded by the same test: a smart mailbox is a saved search,
   * and there is nowhere to move mail *to*.
   */
  const single = node.mailboxIds.length === 1 ? node.mailboxIds[0] : undefined
  const destination =
    single !== undefined && node.accountId !== undefined
      ? { id: single, accountId: node.accountId }
      : null

  /**
   * Moves focus to the next or previous row of the tree.
   *
   * The rows use a roving `tabIndex` -- only the selected one is tabbable -- which is the right
   * pattern for a tree and only half of it. The other half is arrow keys, and there were none:
   * a keyboard user could Tab into the sidebar, land on whichever row happened to be selected,
   * and had no way to reach any other. Tab moved straight past the tree to the message list.
   *
   * Focus moves; selection does not. Enter and Space select, which is what ARIA's tree pattern
   * asks for and also avoids loading a different mailbox on every keypress while somebody is
   * simply looking for one.
   *
   * Found in the DOM rather than threaded through props, because the rendered order *is* the
   * answer -- it already accounts for collapsed sections, which a parallel index would have to
   * recompute and could disagree with.
   */
  const moveFocus = (from: HTMLElement, delta: number) => {
    const tree = from.closest('[role="tree"]')
    if (tree === null) return

    const rows = Array.from(tree.querySelectorAll<HTMLElement>('[role="treeitem"]'))
    const next = rows[rows.indexOf(from) + delta]
    next?.focus()
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      moveFocus(event.currentTarget, 1)
      return
    }

    if (event.key === 'ArrowUp') {
      event.preventDefault()
      moveFocus(event.currentTarget, -1)
      return
    }

    // docs/01 §14 — Right and Left expand and collapse, matching the message list's
    // thread expansion and every other tree on both platforms.
    if (event.key === 'ArrowRight' && hasChildren && collapsed) {
      event.preventDefault()
      onToggle(node.id)
    } else if (event.key === 'ArrowLeft' && hasChildren && !collapsed) {
      event.preventDefault()
      onToggle(node.id)
    } else if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      onSelect(node)
    }
  }

  return (
    <div
      role="treeitem"
      aria-selected={selected}
      {...(hasChildren ? { 'aria-expanded': !collapsed } : {})}
      aria-level={node.depth + 1}
      tabIndex={selected ? 0 : -1}
      className={cx(styles.row, selected && styles.selected, dropTarget && styles.dropTarget)}
      // Read by the tree's context menu, which is mounted once around every row rather than
      // once per row and so has only the event target to work out what was clicked.
      data-node-id={node.id}
      style={{
        paddingLeft: `calc(var(--sidebar-row-pad-x) + ${String(node.depth)} * var(--sp-8))`,
      }}
      onClick={() => {
        onSelect(node)
      }}
      onKeyDown={onKeyDown}
      onDragEnter={(event: DragEvent<HTMLDivElement>) => {
        // `dragover` alone is not enough, which is not obvious and was measured rather than
        // reasoned about: approaching a row from the right — the direction every drag out of
        // the message list arrives from — and stopping just inside its edge fires a single
        // `dragenter` and no `dragover` at all. The row then never lit up for as long as the
        // pointer rested there, which is not a flicker but the highlight simply never
        // appearing, on the commonest gesture there is.
        if (destination === null || !canDropInMailbox(event.dataTransfer, destination)) return

        event.preventDefault()
        onDragOverRow(node)
      }}
      onDragOver={(event: DragEvent<HTMLDivElement>) => {
        // Not calling `preventDefault` is how a row refuses: the browser then draws the
        // no-drop cursor and will not deliver a drop here at all. So every reason a move
        // could not work has to be known *now*, before the user lets go — which is why
        // `canDropInMailbox` reads the drag's types rather than its data.
        if (destination === null || !canDropInMailbox(event.dataTransfer, destination)) return

        event.preventDefault()
        event.dataTransfer.dropEffect = 'move'
        onDragOverRow(node)
      }}
      onDragLeave={(event: DragEvent<HTMLDivElement>) => {
        // A `dragleave` fires on this row every time the pointer crosses into one of its own
        // children — the chevron, the icon, the label, the badge — because those are separate
        // elements and the event bubbles back up to here. Clearing unconditionally made the
        // highlight strobe: measured at runs of three and four dark frames while gliding
        // along a row, and a row left dark indefinitely when the pointer came to rest a pixel
        // past a child's edge.
        //
        // `relatedTarget` is where the pointer went. If that is inside this row, it never
        // actually left. It is null when the pointer leaves the window altogether, and
        // falling through to clear is the right answer for that case.
        const next = event.relatedTarget
        if (next instanceof Node && event.currentTarget.contains(next)) return

        onDragLeaveRow(node)
      }}
      onDrop={(event: DragEvent<HTMLDivElement>) => {
        event.preventDefault()
        onDragOverRow(null)

        // Re-checked rather than trusted. A drop only arrives on a row that accepted the
        // drag, but the check is cheap and this is the point of no return for real mail.
        if (destination === null || !canDropInMailbox(event.dataTransfer, destination)) return

        const ids = draggedMessageIds(event.dataTransfer)
        if (ids.length > 0) onDropRow(destination.id, ids)
      }}
    >
      <span className={styles.disclosure}>
        {hasChildren && (
          <button
            type="button"
            tabIndex={-1}
            aria-label={collapsed ? `Expand ${node.label}` : `Collapse ${node.label}`}
            className={cx(styles.chevron, !collapsed && styles.chevronOpen)}
            onClick={(event) => {
              event.stopPropagation()
              onToggle(node.id)
            }}
          >
            <ChevronRight aria-hidden="true" strokeWidth={2} />
          </button>
        )}
      </span>

      <Icon
        className={styles.icon}
        aria-hidden="true"
        strokeWidth={1.75}
        // Spread rather than passed as undefined: with `exactOptionalPropertyTypes` an explicit
        // undefined is not an absent attribute, and the CSS selects on the attribute existing.
        {...(node.flagColor === undefined ? {} : { 'data-flag': node.flagColor })}
      />
      <span className={styles.label}>{node.label}</span>
      <Badge count={node.unreadCount} selected={selected} className={cx(styles.badge)} />
    </div>
  )
}

/**
 * The mailbox sidebar. docs/01 §3, docs/02 §6.2.
 *
 * Three things here are deliberate and easy to undo by accident:
 *
 *  - **No hover highlight.** docs/01 §3 is explicit, and it is most of why the sidebar
 *    reads as calm rather than as a menu. Windows apps add one by reflex.
 *  - **The badge is laid out even at zero**, where it renders nothing. Reserving the space
 *    means a count arriving does not shove the label leftward — standing rule 6.
 *  - **A drop target is a fill, not a border.** docs/02 §6.2 asks for the accent at 25%
 *    alpha with a 1px inset ring, which is a box-shadow rather than a border — a border
 *    would change the row's size and shove the rows below it as you drag past. Standing
 *    rule 6 again.
 *  - **Selection is keyed by row, not by mailbox.** The same mailbox appears twice in the
 *    tree — under All Inboxes and in its account's section — so keying off the mailbox id
 *    highlighted every copy at once.
 *  - **Selection has three states, not two.** Solid accent when the sidebar itself has
 *    focus, a quiet fill when focus is in another pane, grey when the window is inactive.
 *    The macOS 26 reference captures the middle one, which is what made it look at first
 *    as though the spec's solid-accent selection had been dropped.
 */
export interface SidebarProps {
  /** Opens Settings. Optional so the component gallery can render the sidebar alone. */
  onOpenSettings?: (() => void) | undefined
  /**
   * The right-click menu, built from the row it will act on.
   *
   * A render function for the same reason the message list takes one: only the sidebar knows
   * which row was clicked, and only the shell knows what the actions are.
   */
  contextMenu?: ((node: SidebarNode) => ReactNode) | undefined
}

export function Sidebar({ onOpenSettings, contextMenu }: SidebarProps) {
  const accountsQuery = useAccounts()
  const mailboxesQuery = useMailboxes()

  const accounts = accountsQuery.data ?? NONE
  const mailboxes = mailboxesQuery.data ?? NONE

  const selectedNodeId = useMailStore((state) => state.selection.nodeId)
  const selectMailbox = useMailStore((state) => state.selectMailbox)

  const collapsedSections = useLayoutStore((state) => state.collapsedSections)
  const toggleSection = useLayoutStore((state) => state.toggleSection)
  const toggleSidebar = useLayoutStore((state) => state.toggleSidebar)
  const moveMessages = useMoveMessages()

  const [dropTargetId, setDropTargetId] = useState<string | null>(null)
  const [menuNode, setMenuNode] = useState<SidebarNode | null>(null)

  // Not "no accounts" — that is first-run, and AccountsGate handles it. This is the query
  // itself failing, which used to render an empty tree and say nothing at all: the sidebar
  // looked like a fresh install, and the gate did not appear because it requires isSuccess.
  const failed = accountsQuery.isError || mailboxesQuery.isError

  const sync = useSyncState()
  const accountNames = useMemo(
    () => new Map(accounts.map((account) => [account.id, account.displayName])),
    [accounts],
  )

  const { data: smart = [] } = useSmartMailboxes()
  const { data: flagNames = [] } = useFlagNames()
  const { data: vips = [] } = useVips()

  const sections = useMemo(
    () => buildSidebar(accounts, mailboxes, smart, flagNames, vips),
    [accounts, mailboxes, smart, flagNames, vips],
  )
  const collapsed = useMemo(() => new Set(collapsedSections), [collapsedSections])

  /**
   * Which row a right-click acts on, and whether it may open a menu at all.
   *
   * Unlike the message list, this does **not** select the row. Selecting a mailbox loads it,
   * which is a second or two of work and a change to what the user is looking at — far too
   * much to do on the way to a menu they may close again. Every item here names its own
   * mailbox, so the menu does not need the selection to agree with it.
   */
  const onContextMenuOpen = useCallback(
    (event: ReactMouseEvent<HTMLDivElement>): boolean => {
      const element =
        event.target instanceof Element ? event.target.closest('[data-node-id]') : null
      if (element === null) return false

      const id = element.getAttribute('data-node-id')
      const node = sections.flatMap((section) => section.nodes).find((each) => each.id === id)

      // Containers, unified rows, Flagged and its colours, VIPs and smart mailboxes: none has a
      // single mailbox or a single account, and every row of the menu needs both.
      if (node === undefined || !canOpenMailboxMenu(node)) {
        setMenuNode(null)
        return false
      }

      setMenuNode(node)
      return true
    },
    [sections],
  )

  const onSelect = (node: SidebarNode) => {
    // Every predicate row was once dead on click: Flagged, all seven flag colours, VIPs, and
    // every Smart Mailbox the user had made. They rendered, showed counts and highlighted on
    // hover, and selecting one did nothing, because the guard tested only `mailboxIds`.
    // `selectionForNode` is now the single definition of that rule, shared with Ctrl+1-9.
    const selection = selectionForNode(node)
    if (selection !== null) selectMailbox(selection)
  }

  return (
    <div className={styles.pane}>
      {/* The pane's own header, at the shared toolbar height, so the three pane headers
          line up into one band the way assets/reference/ shows. */}
      <header className={styles.header} data-tauri-drag-region>
        {onOpenSettings && (
          <Tooltip
            content="Settings"
            trigger={<IconButton icon={Settings} label="Settings" onClick={onOpenSettings} />}
          />
        )}

        <Tooltip
          content="Hide sidebar"
          trigger={
            <IconButton icon={PanelLeft} label="Hide sidebar" toggled onClick={toggleSidebar} />
          }
        />
      </header>

      {failed ? (
        <EmptyState
          className={styles.failed}
          icon={AlertTriangle}
          tone="error"
          title="Your mailboxes could not be read"
          description="The local database did not answer. Your mail is still on the server."
          action={
            <Button
              variant="bordered"
              onClick={() => {
                void accountsQuery.refetch()
                void mailboxesQuery.refetch()
              }}
            >
              Try Again
            </Button>
          }
        />
      ) : (
        <ContextMenu
          label="Mailbox actions"
          onOpen={onContextMenuOpen}
          menu={menuNode === null ? null : contextMenu?.(menuNode)}
        >
          {/* One menu around the whole tree rather than one per row — the same reasoning as
              the message list. `onOpen` refuses any row that is not a single real mailbox in a
              known account, which is every container, unified and smart row. */}
          <ScrollArea className={styles.sidebar}>
            <div role="tree" aria-label="Mailboxes" className={styles.tree}>
              {sections.map((section) => (
                <div
                  key={section.id}
                  role="group"
                  aria-label={section.title}
                  className={styles.section}
                >
                  <h2 className={styles.sectionTitle}>{section.title}</h2>

                  {visibleRows(section.nodes, collapsed).map((node) => (
                    <SidebarRow
                      key={node.id}
                      node={node}
                      selected={node.id === selectedNodeId}
                      collapsed={collapsed.has(node.id)}
                      dropTarget={node.id === dropTargetId}
                      onSelect={onSelect}
                      onToggle={toggleSection}
                      onDragOverRow={(target) => {
                        setDropTargetId(target?.id ?? null)
                      }}
                      onDragLeaveRow={(target) => {
                        // Only the row that is actually lit may put the light out. Moving from
                        // one row to the next fires enter-then-leave, so an unconditional null
                        // here would wipe the highlight the new row had just set — which works
                        // today only by the order those two events happen to arrive in.
                        setDropTargetId((current) => (current === target.id ? null : current))
                      }}
                      onDropRow={(mailboxId, messageIds) => {
                        moveMessages.mutate({ ids: messageIds, mailboxId })
                      }}
                    />
                  ))}
                </div>
              ))}
            </div>
          </ScrollArea>
        </ContextMenu>
      )}

      {/* Outside the ScrollArea on purpose: a problem that scrolls out of sight is one the
          user stops seeing, and this is the one part of the sidebar that has to stay put. */}
      <SyncStatus
        errors={sync.errors}
        busy={sync.busy}
        online={sync.online}
        accountNames={accountNames}
      />
    </div>
  )
}
