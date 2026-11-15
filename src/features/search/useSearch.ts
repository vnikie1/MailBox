import { useCallback, useEffect, useRef, useState } from 'react'
import { keepPreviousData, useQuery } from '@tanstack/react-query'

import type { SearchHit } from '@/lib/generated/SearchHit'
import { runSearch } from '@/lib/search'

import type { Place, State } from './ScopeBar'

/**
 * Running the search and holding its results. docs/06 Phase 9.
 *
 * The scope is applied **here**, in the UI, rather than by re-running the query in the core.
 * "Unread" and "Flagged" narrow a set the core has already ranked, and asking the core again
 * would re-rank a smaller candidate window — so a message that was fourth could become first
 * for no reason the user can see. Filtering the ranked list keeps the order stable as the
 * scope changes, which is what makes the bar feel like a filter rather than a new search.
 *
 * ## Why the results are a query
 *
 * They were component state, filled once per search and never again, so nothing that changed the
 * mail reached them: a hit opened and read stayed bold, and one deleted or archived stayed in
 * the list (M35, 2026-11-14 audit). Every mutation and every change the core announces already
 * invalidates `['search']`; it had nothing to invalidate. As a query under that key, the results
 * are refetched like every other list. The race a slow search used to need a sequence number for
 * is the query's: a result arrives for the key it was asked under, and a later key's wins.
 */

/** Debounced so typing does not run a query per keystroke against 100,000 messages. */
const DEBOUNCE_MS = 180

/** How many results to ask for. More than fits on screen, few enough to rank cheaply. */
const LIMIT = 200

/** One empty list, so "no results" is the same array from render to render. */
const NO_HITS: SearchHit[] = []

export interface SearchState {
  text: string
  hits: SearchHit[]
  /** The hits left after the scope bar has been applied. */
  visible: SearchHit[]
  place: Place
  state: State
  running: boolean
  /** The search for what is typed failed, as opposed to finding nothing. */
  failed: boolean
  setText: (text: string) => void
  setPlace: (place: Place) => void
  setState: (state: State) => void
  /** Runs immediately, skipping the debounce. Enter, or a suggestion chosen. */
  commit: (text: string) => void
  clear: () => void
}

export function useSearch(mailboxIds: number[]): SearchState {
  const [text, setTextRaw] = useState('')
  // What is actually searched for: `text` once typing has paused, or at once on commit.
  const [asked, setAsked] = useState('')
  const [place, setPlace] = useState<Place>('all')
  const [state, setState] = useState<State>('all')

  const timer = useRef<number | undefined>(undefined)

  // The mailbox list is a new array on every render of the caller, so it cannot be a dependency
  // without re-running the search continuously. The identity that matters is the contents.
  const scopeKey = mailboxIds.join(',')
  const within = place === 'mailbox' ? scopeKey : ''

  useEffect(() => {
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => {
      setAsked(text.trim())
    }, DEBOUNCE_MS)

    return () => {
      window.clearTimeout(timer.current)
    }
  }, [text])

  const query = useQuery({
    // Under `['search']`, which is what every invalidation names. See the note above.
    queryKey: ['search', 'ranked', asked, within] as const,
    queryFn: () => runSearch(asked, within.split(',').filter(Boolean).map(Number), LIMIT),
    enabled: asked !== '',
    // The previous results stay up while the next search runs, as they did, rather than the
    // list emptying on every pause in typing.
    placeholderData: keepPreviousData,
  })

  const commit = useCallback((value: string) => {
    window.clearTimeout(timer.current)
    setTextRaw(value)
    setAsked(value.trim())
  }, [])

  const clear = useCallback(() => {
    window.clearTimeout(timer.current)
    setTextRaw('')
    setAsked('')
  }, [])

  // Nothing asked is no results, whatever the query last held.
  const hits: SearchHit[] = asked === '' ? NO_HITS : (query.data?.hits ?? NO_HITS)
  // Waiting for an answer to what is typed: still inside the debounce, the first search, or the
  // previous one's results standing in for the next. Not a refetch after a change to the mail,
  // which must not flash anything. The list shows "No messages match" only when this is false,
  // so it must not be false before the search for this text has answered.
  const typing = text.trim() !== asked
  const running = typing || (asked !== '' && (query.isPending || query.isPlaceholderData))
  const failed = !running && asked !== '' && query.isError

  const visible = hits.filter((hit) => {
    if (state === 'unread') return !hit.row.seen
    if (state === 'flagged') return hit.row.flagged
    return true
  })

  return {
    text,
    hits,
    visible,
    place,
    state,
    running,
    failed,
    setText: setTextRaw,
    setPlace,
    setState,
    commit,
    clear,
  }
}
