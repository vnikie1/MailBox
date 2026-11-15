import type { State } from './ScopeBar'

/** What a saved search will be: its text, and anything about its scope worth saying. */
export interface SavedSearch {
  text: string
  /** Shown in the sheet when the mailbox it was limited to is kept loosely, or not at all. */
  note: string | null
}

/**
 * The search as it will be saved, with the scope bar's choices written into it.
 *
 * Save Search saved only the typed text, so a search narrowed to Unread in Archive became a
 * smart mailbox of everything, everywhere (L18, 2026-11-14 audit). The scope is written in the
 * search language the core already turns into conditions — `is:unread`, `is:flagged`,
 * `mailbox:"…"` — so the saved mailbox is built the same way as one whose search said so itself.
 *
 * A mailbox goes into a smart mailbox **by name**, which is the only way its conditions can name
 * one, so it means every mailbox of that name: Archive in every account. Several mailboxes with
 * different names (All Inboxes is several mailboxes with one) cannot be said at all, since every
 * condition must hold. Both are told to the user rather than left for them to discover.
 */
export function savedSearch(text: string, state: State, mailboxNames: string[]): SavedSearch {
  const parts = [text.trim()]

  if (state === 'unread') parts.push('is:unread')
  if (state === 'flagged') parts.push('is:flagged')

  const names = [...new Set(mailboxNames)]
  const name = names[0]
  let note: string | null = null

  if (names.length > 1) {
    note = 'It searches every mailbox: a saved search can be limited to one mailbox name only.'
  } else if (name?.includes('"') === true) {
    note = `It searches every mailbox: “${name}” cannot be named in a saved search.`
  } else if (name !== undefined) {
    parts.push(`mailbox:"${name}"`)
    note = `It looks in every mailbox called “${name}”, in every account.`
  }

  return { text: parts.filter((part) => part !== '').join(' '), note }
}
