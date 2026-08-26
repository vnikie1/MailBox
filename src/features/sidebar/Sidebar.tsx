import { useMemo, useState, type DragEvent, type KeyboardEvent } from 'react'
import { ChevronRight, PanelLeft, Settings } from 'lucide-react'

import { cx } from '@/lib/cx'
import { useLayoutStore } from '@/store/layout'
import { useMailboxes, useAccounts, useMoveMessages } from '@/app/queries'
import { useMailStore } from '@/store/mail'
import { Badge, IconButton, ScrollArea, Tooltip } from '@/ui'

import { buildSidebar, visibleRows, type SidebarNode } from './model'

import styles from './Sidebar.module.css'

const DRAG_TYPE = 'application/x-mailbox-threads'

interface SidebarRowProps {
  node: SidebarNode
  selected: boolean
  collapsed: boolean
  dropTarget: boolean
  onSelect: (node: SidebarNode) => void
  onToggle: (id: string) => void
  onDragOverRow: (node: SidebarNode | null) => void
  onDropRow: (node: SidebarNode, messageIds: number[]) => void
}

function SidebarRow({
  node,
  selected,
  collapsed,
  dropTarget,
  onSelect,
  onToggle,
  onDragOverRow,
  onDropRow,
}: SidebarRowProps) {
  const Icon = node.icon
  const hasChildren = node.children.length > 0

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
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
      style={{
        paddingLeft: `calc(var(--sidebar-row-pad-x) + ${String(node.depth)} * var(--sp-8))`,
      }}
      onClick={() => {
        onSelect(node)
      }}
      onKeyDown={onKeyDown}
      onDragOver={(event: DragEvent<HTMLDivElement>) => {
        // Only rows backed by exactly one mailbox are drop targets. A container has none
        // and a unified row has several, and in both cases there is no single destination —
        // highlighting them would promise something the drop cannot deliver.
        if (node.mailboxIds.length !== 1 || !event.dataTransfer.types.includes(DRAG_TYPE)) return
        event.preventDefault()
        event.dataTransfer.dropEffect = 'move'
        onDragOverRow(node)
      }}
      onDragLeave={() => {
        onDragOverRow(null)
      }}
      onDrop={(event: DragEvent<HTMLDivElement>) => {
        event.preventDefault()
        onDragOverRow(null)

        const ids = event.dataTransfer
          .getData(DRAG_TYPE)
          .split(' ')
          .map(Number)
          .filter((id) => Number.isFinite(id))
        if (ids.length > 0) onDropRow(node, ids)
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

      <Icon className={styles.icon} aria-hidden="true" strokeWidth={1.75} />
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
}

export function Sidebar({ onOpenSettings }: SidebarProps) {
  const { data: accounts = [] } = useAccounts()
  const { data: mailboxes = [] } = useMailboxes()

  const selectedNodeId = useMailStore((state) => state.selection.nodeId)
  const selectMailbox = useMailStore((state) => state.selectMailbox)

  const collapsedSections = useLayoutStore((state) => state.collapsedSections)
  const toggleSection = useLayoutStore((state) => state.toggleSection)
  const toggleSidebar = useLayoutStore((state) => state.toggleSidebar)
  const moveMessages = useMoveMessages()

  const [dropTargetId, setDropTargetId] = useState<string | null>(null)

  const sections = useMemo(() => buildSidebar(accounts, mailboxes), [accounts, mailboxes])
  const collapsed = useMemo(() => new Set(collapsedSections), [collapsedSections])

  const onSelect = (node: SidebarNode) => {
    if (node.mailboxIds.length === 0) return
    selectMailbox({ nodeId: node.id, label: node.label, mailboxIds: node.mailboxIds })
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
                  onDropRow={(target, messageIds) => {
                    const destination = target.mailboxIds[0]
                    if (target.mailboxIds.length === 1 && destination !== undefined) {
                      moveMessages.mutate({ ids: messageIds, mailboxId: destination })
                    }
                  }}
                />
              ))}
            </div>
          ))}
        </div>
      </ScrollArea>
    </div>
  )
}
