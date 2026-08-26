import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { ArrowDownUp, ListFilter, MoreHorizontal, PanelLeft } from 'lucide-react'

import { useMailboxes, useMessages } from '@/app/queries'
import { useBodyPrefetch } from '@/features/reader/useBodyPrefetch'
import type { Density } from '@/lib/appearance'
import { cx } from '@/lib/cx'
import { storeNow } from '@/lib/ipc'
import { lengthToken } from '@/lib/tokens'
import { useLayoutStore } from '@/store/layout'
import { useMailStore } from '@/store/mail'
import { useSettingsStore } from '@/store/settings'
import { IconButton, Menu, MenuItem, MenuSection, MenuSeparator, Tooltip } from '@/ui'

import { MessageRow } from './MessageRow'
import { buildListItems } from './rows'
import { SORT_LABELS, sortRows, type SortField } from './sort'

import styles from './MessageList.module.css'

/** Only reached when the cascade has not applied; these mirror the authored tokens. */
const ROW_BASE_FALLBACK_PX = 48
const ROW_STEP_FALLBACK_PX = 16
const HEADER_FALLBACK_PX = 26
const OVERSCAN = 8

/**
 * How close to the end the virtualiser must get before the next page is requested.
 *
 * Far enough ahead that the fetch lands before the user reaches the bottom, so scrolling
 * never stops at a spinner — docs/02 §6.10 does not permit one in the list anyway.
 */
const PREFETCH_ROWS = 30

export interface MessageListProps {
  /** True when the sidebar pane is not on screen, so the toggle moves here. */
  showSidebarToggle?: boolean
}

/**
 * The message list. docs/01 §4 — "the single most important surface".
 *
 * Virtualised with TanStack Virtual over a cursor-paged query. The two work together: the
 * store returns a hundred rows at a time keyed on `(dateReceived, id)`, and the virtualiser
 * asks for the next page as it approaches the end. Neither ever counts or skips, which is
 * what holds the 80ms budget in docs/03 §5 at a hundred thousand messages.
 */
export function MessageList({ showSidebarToggle = false }: MessageListProps) {
  const selection = useMailStore((state) => state.selection)
  const selectedMessageIds = useMailStore((state) => state.selectedMessageIds)
  const selectMessage = useMailStore((state) => state.selectMessage)
  const toggleMessage = useMailStore((state) => state.toggleMessage)
  const extendSelection = useMailStore((state) => state.extendSelection)
  const moveSelection = useMailStore((state) => state.moveSelection)

  const previewLines = useLayoutStore((state) => state.previewLines)
  const setPreviewLines = useLayoutStore((state) => state.setPreviewLines)
  const toggleSidebar = useLayoutStore((state) => state.toggleSidebar)
  const classicLayout = useLayoutStore((state) => state.classicLayout)
  const toggleClassicLayout = useLayoutStore((state) => state.toggleClassicLayout)
  const sortField = useLayoutStore((state) => state.sortField)
  const sortAscending = useLayoutStore((state) => state.sortAscending)
  const unreadOnly = useLayoutStore((state) => state.unreadOnly)
  const setSort = useLayoutStore((state) => state.setSort)
  const toggleSortDirection = useLayoutStore((state) => state.toggleSortDirection)
  const toggleUnreadOnly = useLayoutStore((state) => state.toggleUnreadOnly)

  const density = useSettingsStore((state) => state.density)
  const setDensity = useSettingsStore((state) => state.setDensity)

  const [showPhotos, setShowPhotos] = useState(false)
  const [focused, setFocused] = useState(false)

  const scrollRef = useRef<HTMLDivElement>(null)

  const { data: mailboxes = [] } = useMailboxes()
  const { data, fetchNextPage, hasNextPage, isFetchingNextPage, isPending } = useMessages(
    selection.mailboxIds,
    unreadOnly,
  )

  const now = useMemo(storeNow, [])

  const rows = useMemo(
    () =>
      sortRows(data?.pages.flatMap((page) => page.items) ?? [], {
        field: sortField,
        ascending: sortAscending,
      }),
    [data, sortField, sortAscending],
  )

  const order = useMemo(() => rows.map((row) => row.id), [rows])
  const selected = useMemo(() => new Set(selectedMessageIds), [selectedMessageIds])

  // docs/06 Phase 5 §3. Here rather than in the Reader because the prefetch needs the rows
  // *around* the selection, and the visible order lives in this component.
  useBodyPrefetch(selectedMessageIds.length === 1 ? selectedMessageIds[0] : undefined, rows)

  const items = useMemo(
    // Date headers only mean something while the list is in date order; under any other
    // sort they would label runs that are not contiguous in time.
    () => buildListItems({ rows, now, selected, grouped: sortField === 'date' }),
    [rows, now, selected, sortField],
  )

  // Select the first row once a mailbox's first page arrives, rather than leaving the reader
  // empty — the flash of "No Message Selected" on every mailbox switch is something Mail
  // never shows.
  const firstId = rows[0]?.id
  useEffect(() => {
    if (selectedMessageIds.length === 0 && firstId !== undefined) selectMessage(firstId)
  }, [firstId, selectedMessageIds.length, selectMessage])

  // Read once per render rather than per item: getComputedStyle forces a style recalc, and
  // calling it inside estimateSize would do that a hundred times a frame. Density is in the
  // dependencies because it changes the token.
  const rowHeight = useMemo(() => {
    const base = lengthToken('--list-row-height-0', ROW_BASE_FALLBACK_PX)
    const step =
      lengthToken('--list-row-height-1', ROW_BASE_FALLBACK_PX + ROW_STEP_FALLBACK_PX) - base
    return base + previewLines * (step > 0 ? step : ROW_STEP_FALLBACK_PX)
    // density is not read here, but it changes the CSS custom property that is.
    // eslint-disable-next-line react-hooks/exhaustive-deps -- React cannot see that.
  }, [previewLines, density])

  const headerHeight = useMemo(
    () => lengthToken('--list-date-header-height', HEADER_FALLBACK_PX),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- as above.
    [density],
  )

  const virtualiser = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => (items[index]?.kind === 'header' ? headerHeight : rowHeight),
    overscan: OVERSCAN,
    getItemKey: (index) => items[index]?.key ?? index,
  })

  const virtualItems = virtualiser.getVirtualItems()

  // Fetch ahead of the viewport rather than at the bottom of it.
  const lastVisible = virtualItems[virtualItems.length - 1]?.index ?? 0
  useEffect(() => {
    if (hasNextPage && !isFetchingNextPage && lastVisible >= items.length - PREFETCH_ROWS) {
      void fetchNextPage()
    }
  }, [hasNextPage, isFetchingNextPage, lastVisible, items.length, fetchNextPage])

  /** Whichever section the topmost visible item belongs to, drawn over the list. */
  const stickyLabel = useMemo(() => {
    const first = virtualItems[0]
    if (!first) return null

    for (let index = first.index; index >= 0; index -= 1) {
      const item = items[index]
      if (item?.kind === 'header') return item.label
    }
    return null
  }, [virtualItems, items])

  const onRowSelect = useCallback(
    (id: number, modifiers: { shift: boolean; toggle: boolean }) => {
      if (modifiers.shift) extendSelection(id, order)
      else if (modifiers.toggle) toggleMessage(id)
      else selectMessage(id)
    },
    [extendSelection, toggleMessage, selectMessage, order],
  )

  const onDragStart = useCallback(
    (id: number) => {
      const current = useMailStore.getState().selectedMessageIds
      if (current.includes(id)) return current

      selectMessage(id)
      return [id]
    },
    [selectMessage],
  )

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      moveSelection(1, order)
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      moveSelection(-1, order)
    }
  }

  // A unified row spans several mailboxes and belongs to no single account, so the header
  // names the row rather than trying to name an account.
  const chosen = mailboxes.filter((entry) => selection.mailboxIds.includes(entry.id))
  const total = chosen.reduce((sum, entry) => sum + entry.totalCount, 0)
  const unread = chosen.reduce((sum, entry) => sum + entry.unreadCount, 0)

  return (
    <div className={styles.pane}>
      <header className={styles.header} data-tauri-drag-region>
        {showSidebarToggle && (
          <Tooltip
            content="Show sidebar"
            trigger={
              <IconButton
                icon={PanelLeft}
                label="Show sidebar"
                className={styles.sidebarToggle}
                onClick={toggleSidebar}
              />
            }
          />
        )}

        <div className={styles.titles}>
          <h1 className={styles.title}>{selection.label}</h1>
          <p className={cx(styles.subtitle, 'tabular')}>
            {total} messages
            {unread > 0 ? `, ${String(unread)} unread` : ''}
          </p>
        </div>

        <div className={styles.headerActions}>
          <Tooltip
            content={unreadOnly ? 'Show all messages' : 'Show unread only'}
            trigger={
              <IconButton
                icon={ListFilter}
                label={unreadOnly ? 'Show all messages' : 'Show unread only'}
                size="sm"
                toggled={unreadOnly}
                onClick={toggleUnreadOnly}
              />
            }
          />

          {/* docs/01 §4 — the sort menu at the list header right. */}
          <Menu
            label="Sort messages"
            placement="bottom-end"
            trigger={<IconButton icon={ArrowDownUp} label="Sort" size="sm" />}
          >
            <MenuSection label="Sort by">
              {(Object.keys(SORT_LABELS) as SortField[]).map((field) => (
                <MenuItem
                  key={field}
                  label={SORT_LABELS[field]}
                  checked={sortField === field}
                  onClick={() => {
                    setSort(field)
                  }}
                />
              ))}
            </MenuSection>

            <MenuSeparator />

            <MenuItem
              label={sortAscending ? 'Ascending' : 'Descending'}
              checked
              onClick={toggleSortDirection}
            />
          </Menu>

          <Menu
            label="List options"
            placement="bottom-end"
            trigger={<IconButton icon={MoreHorizontal} label="List options" size="sm" />}
          >
            <MenuItem
              label="Show Contact Photos"
              checked={showPhotos}
              onClick={() => {
                setShowPhotos((value) => !value)
              }}
            />
            <MenuItem
              label="Use Classic Layout"
              checked={classicLayout}
              onClick={toggleClassicLayout}
            />

            <MenuSeparator />

            <MenuSection label="Preview">
              {([0, 1, 2, 3, 4, 5] as const).map((lines) => (
                <MenuItem
                  key={lines}
                  label={lines === 0 ? 'None' : `${String(lines)} Line${lines > 1 ? 's' : ''}`}
                  checked={previewLines === lines}
                  onClick={() => {
                    setPreviewLines(lines)
                  }}
                />
              ))}
            </MenuSection>

            <MenuSeparator />

            <MenuSection label="Density">
              {(['compact', 'default', 'comfortable'] as const).map((mode: Density) => (
                <MenuItem
                  key={mode}
                  label={mode.charAt(0).toUpperCase() + mode.slice(1)}
                  checked={density === mode}
                  onClick={() => {
                    setDensity(mode)
                  }}
                />
              ))}
            </MenuSection>
          </Menu>
        </div>
      </header>

      <div className={styles.listWrap}>
        {stickyLabel !== null && (
          <div className={styles.sticky} aria-hidden="true">
            {stickyLabel}
          </div>
        )}

        <div
          ref={scrollRef}
          role="listbox"
          aria-label="Messages"
          aria-multiselectable
          aria-busy={isPending}
          tabIndex={0}
          className={cx(styles.scroll, focused && 'messageListFocused')}
          onKeyDown={onKeyDown}
          onFocus={() => {
            setFocused(true)
          }}
          onBlur={() => {
            setFocused(false)
          }}
        >
          <div
            className={styles.inner}
            style={{ height: `${String(virtualiser.getTotalSize())}px` }}
          >
            {virtualItems.map((virtualItem) => {
              const item = items[virtualItem.index]
              if (!item) return null

              return (
                <div
                  key={virtualItem.key}
                  className={styles.item}
                  style={{
                    height: `${String(virtualItem.size)}px`,
                    transform: `translateY(${String(virtualItem.start)}px)`,
                  }}
                >
                  {item.kind === 'header' ? (
                    <div className={styles.sectionHeader}>{item.label}</div>
                  ) : (
                    <MessageRow
                      message={item.message}
                      now={now}
                      selected={selected.has(item.message.id)}
                      runStart={item.runStart}
                      runEnd={item.runEnd}
                      previewLines={previewLines}
                      showPhoto={showPhotos}
                      onSelect={onRowSelect}
                      onDragStart={onDragStart}
                    />
                  )}
                </div>
              )
            })}
          </div>
        </div>
      </div>
    </div>
  )
}
