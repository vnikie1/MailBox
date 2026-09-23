/**
 * Dragging in the sidebar itself: a favourite to a new place in Favourites, or an account's
 * mailbox into Favourites. docs/01 §3 — "Reorderable by drag".
 *
 * Two types of their own, so neither can be mistaken for a message drag (`messageDrag.ts`) by a
 * row that accepts mail, or the other way round. During a drag the browser shows the page a
 * drag's *types* and nothing else, so the type is the whole of what a row can go on before the
 * drop; which favourite or mailbox is moving is kept by the sidebar from the drag's start.
 */

export const FAVOURITE_DRAG_TYPE = 'application/x-halcyon-favourite'
export const MAILBOX_DRAG_TYPE = 'application/x-halcyon-mailbox'

export type SidebarDrag =
  | { kind: 'favourite'; favouriteId: number; nodeId: string }
  | { kind: 'mailbox'; mailboxId: number; nodeId: string }

/** Which half of a row the pointer is over: where a drop would put the row being dragged. */
export type InsertEdge = 'before' | 'after'

export function startSidebarDrag(dataTransfer: DataTransfer, drag: SidebarDrag): void {
  if (drag.kind === 'favourite') {
    dataTransfer.setData(FAVOURITE_DRAG_TYPE, String(drag.favouriteId))
    dataTransfer.effectAllowed = 'move'
  } else {
    dataTransfer.setData(MAILBOX_DRAG_TYPE, String(drag.mailboxId))
    // Adding to Favourites leaves the mailbox where it is, so this is a copy, and the cursor says
    // so with its plus.
    dataTransfer.effectAllowed = 'copy'
  }
}

/** Whether this drag is one of the sidebar's own. */
export function isSidebarDrag(dataTransfer: DataTransfer | null): boolean {
  if (dataTransfer === null) return false
  const types = Array.from(dataTransfer.types)
  return types.includes(FAVOURITE_DRAG_TYPE) || types.includes(MAILBOX_DRAG_TYPE)
}

/** The half of `element` the pointer is in. */
export function edgeAt(element: Element, clientY: number): InsertEdge {
  const box = element.getBoundingClientRect()
  return clientY < box.top + box.height / 2 ? 'before' : 'after'
}

/**
 * The favourite a drop goes in front of, in the stored order, or `null` for the end.
 *
 * "After" a row is "before whatever the store has next" — which may be a row the sidebar is not
 * drawing, such as VIPs with no VIPs. That is the right answer: the dropped row lands between the
 * two rows the user saw it land between, whatever sits unseen there.
 */
export function beforeFor(
  order: readonly number[],
  target: number,
  edge: InsertEdge,
): number | null {
  if (edge === 'before') return target
  const at = order.indexOf(target)
  return at < 0 ? null : (order[at + 1] ?? null)
}

/** `order` with `moving` in front of `before`, or at the end — what `favourite_move` will make. */
export function reordered(
  order: readonly number[],
  moving: number,
  before: number | null,
): number[] {
  if (before === moving) return [...order]
  const rest = order.filter((id) => id !== moving)
  const at = before === null ? rest.length : rest.indexOf(before)
  if (at < 0) return [...order]
  rest.splice(at, 0, moving)
  return rest
}
