import type { MessageRow } from '@/lib/generated/MessageRow'

/**
 * Sorting the message list. docs/01 §4.
 *
 * **Client-side, over the pages already loaded** — which is a real limitation, stated here
 * rather than hidden. The database returns rows newest-first because that is the ordering
 * `ix_msg_list` supports and the only one `docs/03`'s keyset pagination can page through;
 * sorting by sender across a hundred thousand messages needs either another index or a
 * different cursor, and `docs/PHASE-0-VERIFICATION.md` §4 already flags that as an open
 * question. Until it is answered, choosing "From" sorts what has been fetched.
 *
 * The consequence, so nobody discovers it by surprise: with a partially-loaded mailbox, a
 * non-date sort orders the loaded prefix, not the mailbox.
 */

export type SortField = 'date' | 'from' | 'subject' | 'size' | 'unread' | 'flagged' | 'attachments'

export interface SortOptions {
  field: SortField
  ascending: boolean
}

export const SORT_LABELS: Record<SortField, string> = {
  date: 'Date',
  from: 'From',
  subject: 'Subject',
  size: 'Size',
  unread: 'Unread',
  flagged: 'Flags',
  attachments: 'Attachments',
}

function senderOf(row: MessageRow): string {
  return row.fromName ?? row.fromAddr ?? ''
}

/**
 * Re: and Fwd: are stripped for the comparison only — the row still shows what was sent.
 * Without this a conversation scatters across the alphabet under R, and putting a
 * conversation together is the whole reason to sort by subject.
 */
function strippedSubject(row: MessageRow): string {
  return (row.subject ?? '').replace(/^((re|fwd|fw)\s*:\s*)+/i, '').trim()
}

function compare(a: MessageRow, b: MessageRow, field: SortField): number {
  switch (field) {
    case 'from':
      return senderOf(a).localeCompare(senderOf(b))
    case 'subject':
      return strippedSubject(a).localeCompare(strippedSubject(b))
    case 'size':
      return a.size - b.size
    case 'unread':
      return Number(!a.seen) - Number(!b.seen)
    case 'flagged':
      return Number(a.flagged) - Number(b.flagged)
    case 'attachments':
      return Number(a.hasAttachment) - Number(b.hasAttachment)
    case 'date':
    default:
      return a.dateReceived - b.dateReceived
  }
}

/**
 * The rows in display order.
 *
 * The three boolean fields fall back to newest-first within each group. Sorting by "Flags"
 * and getting the flagged messages in arbitrary order would be useless; what you want is
 * the flagged ones first, newest first among them.
 */
export function sortRows(rows: MessageRow[], options: SortOptions): MessageRow[] {
  // Date descending is what the store already returns, so the common case does no work.
  if (options.field === 'date' && !options.ascending) return rows

  const direction = options.ascending ? 1 : -1

  return [...rows].sort((a, b) => {
    const primary = compare(a, b, options.field) * direction
    if (primary !== 0) return primary
    if (options.field === 'date') return b.id - a.id

    return b.dateReceived - a.dateReceived || b.id - a.id
  })
}
