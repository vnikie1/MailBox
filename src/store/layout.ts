import { create } from 'zustand'

import type { SortField } from '@/features/messageList/sort'
import { persist, createJSONStorage } from 'zustand/middleware'

/**
 * Window layout the user has arranged. docs/01 §1.
 *
 * Persisted, because Mail remembers all of it across launches and a client that forgets
 * your pane widths every morning is one of the small ways Windows mail apps feel careless.
 * Same storage story as the appearance settings: localStorage now, the `settings` table in
 * Phase 3.
 *
 * Widths are stored in CSS pixels rather than as fractions. A fraction would make the
 * sidebar grow when you maximise the window, which is not what a sidebar does — it stays
 * put and the reader takes the space.
 */

export const SIDEBAR_MIN = 150
export const SIDEBAR_MAX = 400
export const SIDEBAR_DEFAULT = 232

export const LIST_MIN = 260
export const LIST_MAX = 560
export const LIST_DEFAULT = 360

/** docs/01 §4 — View ▸ Preview, 0 to 5 lines. */
export type PreviewLines = 0 | 1 | 2 | 3 | 4 | 5

interface LayoutState {
  sidebarWidth: number
  listWidth: number
  sidebarCollapsed: boolean
  /** docs/01 §1 "Alternate layout" — list on top, reader below. */
  classicLayout: boolean
  previewLines: PreviewLines
  /** Sidebar sections and mailboxes the user has opened, by id. */
  collapsedSections: string[]

  /** docs/01 §4 — the sort menu at the list header right. */
  sortField: SortField
  sortAscending: boolean
  /** The list header's filter button. Applied by the store, not by the client. */
  unreadOnly: boolean

  setSidebarWidth: (width: number) => void
  setListWidth: (width: number) => void
  toggleSidebar: () => void
  toggleClassicLayout: () => void
  setPreviewLines: (lines: PreviewLines) => void
  toggleSection: (id: string) => void
  setSort: (field: SortField) => void
  toggleSortDirection: () => void
  toggleUnreadOnly: () => void
}

const clamp = (value: number, min: number, max: number) => Math.min(Math.max(value, min), max)

export const useLayoutStore = create<LayoutState>()(
  persist(
    (set) => ({
      sidebarWidth: SIDEBAR_DEFAULT,
      listWidth: LIST_DEFAULT,
      sidebarCollapsed: false,
      classicLayout: false,
      previewLines: 2,
      sortField: 'date',
      sortAscending: false,
      unreadOnly: false,
      // The reference has these two closed on a fresh install; leaving every unified row
      // open fills a third of the sidebar with per-account duplicates before the user has
      // touched anything.
      collapsedSections: ['all-drafts', 'all-sent'],

      setSidebarWidth: (width) => {
        set({ sidebarWidth: clamp(width, SIDEBAR_MIN, SIDEBAR_MAX) })
      },
      setListWidth: (width) => {
        set({ listWidth: clamp(width, LIST_MIN, LIST_MAX) })
      },
      toggleSidebar: () => {
        set((state) => ({ sidebarCollapsed: !state.sidebarCollapsed }))
      },
      toggleClassicLayout: () => {
        set((state) => ({ classicLayout: !state.classicLayout }))
      },
      setPreviewLines: (previewLines) => {
        set({ previewLines })
      },
      // Choosing the field you are already sorted by flips the direction, which is what
      // every list on both platforms does and saves a second menu trip.
      setSort: (field) => {
        set((state) =>
          state.sortField === field
            ? { sortAscending: !state.sortAscending }
            : { sortField: field, sortAscending: field === 'from' || field === 'subject' },
        )
      },
      toggleSortDirection: () => {
        set((state) => ({ sortAscending: !state.sortAscending }))
      },
      toggleUnreadOnly: () => {
        set((state) => ({ unreadOnly: !state.unreadOnly }))
      },
      // Stored as the collapsed set rather than the expanded one, so a mailbox added later
      // starts open — which is what you want from a tree you have never touched.
      toggleSection: (id) => {
        set((state) => ({
          collapsedSections: state.collapsedSections.includes(id)
            ? state.collapsedSections.filter((entry) => entry !== id)
            : [...state.collapsedSections, id],
        }))
      },
    }),
    {
      name: 'halcyon.settings.layout',
      storage: createJSONStorage(() => localStorage),
    },
  ),
)
