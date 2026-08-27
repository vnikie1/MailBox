import { create } from 'zustand'

import type { Predicate } from '@/lib/generated/Predicate'

/**
 * What is selected. Nothing else.
 *
 * Phase 2 kept the mail itself in this store, generated from fixtures at module load. Phase
 * 3 moved that behind the IPC contract, where it belongs — the data now lives in TanStack
 * Query (`src/app/queries.ts`) and this store holds only the user's selection, which is UI
 * state and always was.
 *
 * That split is the point of docs/03 §4's seam: swapping where the mail comes from should
 * not disturb what the user has clicked on.
 */

/**
 * What the sidebar has selected.
 *
 * Keyed by the sidebar *row*, not by the mailbox. The same mailbox appears more than once
 * in the tree — an account's inbox is a child of All Inboxes and also a row in that
 * account's section — so keying on the mailbox lit up three rows at once.
 *
 * `mailboxIds` is a list because "All Inboxes" is a real union, not an alias for the first.
 */
export interface MailboxSelection {
  nodeId: string
  label: string
  mailboxIds: number[]
  /**
   * Set when the row is a saved search — a smart mailbox, Flagged, or a flag colour — rather
   * than a folder. The list queries by this instead of by mailbox id.
   *
   * A selection has one or the other, never both. Carrying both would leave two ways to ask
   * the same question with no rule about which wins, which is the bug the `mailboxIds` comment
   * above already records once.
   */
  predicate?: Predicate
}

interface MailState {
  selection: MailboxSelection
  /** In the list's own order, so a multi-selection can be drawn as contiguous runs. */
  selectedMessageIds: number[]
  /** Where a shift-range starts. Ordinary clicks move it; shift-clicks do not. */
  anchorMessageId: number | null

  selectMailbox: (selection: MailboxSelection) => void
  /** A plain click: replaces the selection and moves the anchor. */
  selectMessage: (id: number) => void
  /** Ctrl-click: adds or removes one, and moves the anchor to it. */
  toggleMessage: (id: number) => void
  /**
   * Shift-click: selects the run from the anchor to here, leaving the anchor put.
   *
   * `order` is the ids as the list is currently showing them. The store deliberately does
   * not know how the list is sorted or paged — passing the visible order in is what stops a
   * range selection running down the wrong order.
   */
  extendSelection: (id: number, order: number[]) => void
  /** Arrow keys: moves by one through the visible order and replaces the selection. */
  moveSelection: (delta: number, order: number[]) => void
}

/** Nothing is selected until the mailbox list has loaded and the shell picks a default. */
export const NO_SELECTION: MailboxSelection = {
  nodeId: '',
  label: 'Inbox',
  mailboxIds: [],
}

export const useMailStore = create<MailState>()((set, get) => ({
  selection: NO_SELECTION,
  selectedMessageIds: [],
  anchorMessageId: null,

  selectMailbox: (selection) => {
    // The list decides what to select once its first page arrives; clearing here avoids a
    // frame where the reader shows a message from the mailbox you just left.
    set({ selection, selectedMessageIds: [], anchorMessageId: null })
  },

  selectMessage: (id) => {
    set({ selectedMessageIds: [id], anchorMessageId: id })
  },

  toggleMessage: (id) => {
    const { selectedMessageIds } = get()
    const isSelected = selectedMessageIds.includes(id)

    // Never leave nothing selected by ctrl-clicking the last one away — the reader would
    // empty out under a click that was meant to be additive.
    if (isSelected && selectedMessageIds.length === 1) return

    set({
      selectedMessageIds: isSelected
        ? selectedMessageIds.filter((entry) => entry !== id)
        : [...selectedMessageIds, id],
      anchorMessageId: id,
    })
  },

  extendSelection: (id, order) => {
    const { anchorMessageId } = get()

    const anchorIndex = anchorMessageId === null ? -1 : order.indexOf(anchorMessageId)
    const targetIndex = order.indexOf(id)
    if (targetIndex < 0) return

    if (anchorIndex < 0) {
      set({ selectedMessageIds: [id], anchorMessageId: id })
      return
    }

    const from = Math.min(anchorIndex, targetIndex)
    const to = Math.max(anchorIndex, targetIndex)
    set({ selectedMessageIds: order.slice(from, to + 1) })
  },

  moveSelection: (delta, order) => {
    if (order.length === 0) return

    const { selectedMessageIds } = get()
    const last = selectedMessageIds[selectedMessageIds.length - 1]
    const currentIndex = last === undefined ? -1 : order.indexOf(last)
    const nextIndex = Math.min(Math.max(currentIndex + delta, 0), order.length - 1)

    const next = order[nextIndex]
    if (next === undefined) return
    set({ selectedMessageIds: [next], anchorMessageId: next })
  },
}))
