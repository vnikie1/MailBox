import type { MailboxRow } from '@/lib/generated/MailboxRow'
import type { MatchCounts } from '@/lib/generated/MatchCounts'
import type { MailboxSelection } from '@/store/mail'

/**
 * What the list header counts for the selected row, or null while that is not yet known.
 *
 * A folder, or a unified row such as All Inboxes, adds up its mailboxes' counts. A smart row —
 * Flagged, a flag colour, VIPs, a saved search — has no mailboxes, and adding up none of them
 * is how every smart row read "0 messages" over a full list (found in batch 7 of the
 * 2026-11-14 audit). It takes the core's count of what its predicate matches, and says nothing
 * until that arrives rather than a 0 that is not true.
 */
export function listCounts(
  selection: MailboxSelection,
  mailboxes: MailboxRow[],
  smart: MatchCounts | undefined,
): MatchCounts | null {
  if (selection.predicate !== undefined) return smart ?? null

  const chosen = mailboxes.filter((entry) => selection.mailboxIds.includes(entry.id))
  return {
    total: chosen.reduce((sum, entry) => sum + entry.totalCount, 0),
    unread: chosen.reduce((sum, entry) => sum + entry.unreadCount, 0),
  }
}
