/**
 * Dragging messages onto a mailbox. docs/01 §3 — "drag a message onto a mailbox".
 *
 * The contract between the message list, which starts the drag, and the sidebar, which
 * accepts it. It lives here rather than in either feature because both halves have to agree
 * on it exactly, and a wire format spelled out twice is one that drifts.
 *
 * ## Why the account is written into the *type* rather than the data
 *
 * A drop target has to decide, on every `dragover`, whether it can accept what is being
 * dragged — and at that moment `dataTransfer.getData()` returns an **empty string**. That is
 * deliberate on the platform's part: it stops a page reading the contents of a file the user
 * is merely dragging *across* it, before they have decided to drop. Only `types` is readable
 * during the drag; the data itself unlocks on drop.
 *
 * So anything the target must know *before* the drop has to be part of a type name. There are
 * three here:
 *
 * - `application/x-mailbox-threads` carries the message ids, read on drop.
 * - `application/x-halcyon-account-<id>` says which account the messages belong to.
 * - `application/x-halcyon-origin-<id>` says which mailbox they are already in.
 *
 * Both suffixed types are set **only when the whole selection agrees**. A selection spanning
 * two accounts sets no account type, so no mailbox will accept it — which is right, because
 * the core refuses it too: `msg_move` answers `crossAccount`, "A message can only be moved to
 * a folder in its own account." Mail cannot be moved between servers by moving a row; it
 * would have to be uploaded to the other account, which is a different operation with
 * different failure modes. Better to refuse it in the sidebar, where the cursor can say so
 * before the user commits, than in a toast afterwards.
 */

/** The message ids, space separated. Readable on drop only. */
const IDS = 'application/x-mailbox-threads'

/** Suffixed with the account every dragged message belongs to, when they share one. */
const ACCOUNT = 'application/x-halcyon-account-'

/** Suffixed with the mailbox the dragged messages come from, when they share one. */
const ORIGIN = 'application/x-halcyon-origin-'

/** What the sidebar needs to know about a dragged message to decide where it may land. */
export interface DraggedMessage {
  id: number
  accountId: number
  mailboxId: number
}

/** The single value of `field` across `messages`, or null if they disagree or there are none. */
function shared(messages: DraggedMessage[], field: 'accountId' | 'mailboxId'): number | null {
  const first = messages[0]
  if (first === undefined) return null

  const value = first[field]
  return messages.every((message) => message[field] === value) ? value : null
}

/**
 * Describes a drag of `messages` on the event that starts it.
 *
 * Types are lowercase by the time they reach `types`, which is why every name here already is
 * — a mixed-case type would be set under one name and looked up under another.
 */
export function startMessageDrag(transfer: DataTransfer, messages: DraggedMessage[]): void {
  transfer.setData(IDS, messages.map((message) => message.id).join(' '))
  transfer.effectAllowed = 'move'

  const account = shared(messages, 'accountId')
  if (account !== null) transfer.setData(`${ACCOUNT}${String(account)}`, '')

  const origin = shared(messages, 'mailboxId')
  if (origin !== null) transfer.setData(`${ORIGIN}${String(origin)}`, '')
}

/** The dragged message ids. Only meaningful on drop; empty at every other point in the drag. */
export function draggedMessageIds(transfer: DataTransfer): number[] {
  return transfer
    .getData(IDS)
    .split(' ')
    .map(Number)
    .filter((id) => Number.isInteger(id) && id > 0)
}

/**
 * Whether this drag may be dropped on a mailbox.
 *
 * Answerable during `dragover`, which is the whole point of the type encoding above. A
 * mailbox in another account is refused, and so is the one the messages are already in —
 * moving mail to where it already is does nothing, and a row that lights up and then does
 * nothing reads as a bug.
 */
export function canDropInMailbox(
  transfer: DataTransfer,
  mailbox: { id: number; accountId: number },
): boolean {
  const types = transfer.types

  return (
    types.includes(IDS) &&
    types.includes(`${ACCOUNT}${String(mailbox.accountId)}`) &&
    !types.includes(`${ORIGIN}${String(mailbox.id)}`)
  )
}
