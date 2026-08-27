/**
 * The search IPC surface. docs/06 Phase 9.
 *
 * Kept apart from `ipc.ts` and `organise.ts` for the same reason those are apart: one file per
 * seam, so a change to search cannot make a mess of the mail commands.
 */

import { invoke, isTauri } from '@tauri-apps/api/core'

import type { SearchResults } from './generated/SearchResults'
import type { Suggestion } from './generated/Suggestion'

const inTauri: boolean = isTauri()

const NOTHING: SearchResults = {
  hits: [],
  parsed: {
    terms: [],
    from: [],
    to: [],
    subject: [],
    mailbox: [],
    hasAttachment: null,
    isUnread: null,
    isFlagged: null,
    isJunk: null,
    before: null,
    after: null,
    largerThan: null,
    smallerThan: null,
  },
}

export async function runSearch(
  text: string,
  mailboxIds: number[],
  limit: number,
): Promise<SearchResults> {
  if (!inTauri) return NOTHING
  return invoke<SearchResults>('search_run', { text, mailboxIds, limit })
}

/** Runs on every keystroke, so it has a 30ms budget in the core. */
export async function suggestSearch(text: string, limit: number): Promise<Suggestion[]> {
  if (!inTauri) return []
  return invoke<Suggestion[]>('search_suggest', { text, limit })
}

export async function searchHistory(limit: number): Promise<string[]> {
  if (!inTauri) return []
  return invoke<string[]>('search_history', { limit })
}

/**
 * Records a search that was actually run.
 *
 * Called on commit — Enter, or a suggestion chosen — and never on a keystroke. Recording every
 * prefix would fill the history with the twelve fragments typed on the way to one query.
 */
export async function rememberSearch(text: string): Promise<void> {
  if (!inTauri) return
  await invoke('search_remember', { text })
}

export async function clearSearchHistory(): Promise<void> {
  if (!inTauri) return
  await invoke('search_history_clear')
}

export async function saveSearchAsSmartMailbox(name: string, text: string): Promise<number> {
  return invoke<number>('search_save_as_smart', { name, text })
}
