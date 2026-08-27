import { useCallback, useEffect, useRef, useState } from 'react'

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
 */

/** Debounced so typing does not run a query per keystroke against 100,000 messages. */
const DEBOUNCE_MS = 180

/** How many results to ask for. More than fits on screen, few enough to rank cheaply. */
const LIMIT = 200

export interface SearchState {
  text: string
  hits: SearchHit[]
  /** The hits left after the scope bar has been applied. */
  visible: SearchHit[]
  place: Place
  state: State
  running: boolean
  setText: (text: string) => void
  setPlace: (place: Place) => void
  setState: (state: State) => void
  /** Runs immediately, skipping the debounce. Enter, or a suggestion chosen. */
  commit: (text: string) => void
  clear: () => void
}

export function useSearch(mailboxIds: number[]): SearchState {
  const [text, setTextRaw] = useState('')
  const [hits, setHits] = useState<SearchHit[]>([])
  const [place, setPlace] = useState<Place>('all')
  const [state, setState] = useState<State>('all')
  const [running, setRunning] = useState(false)

  const timer = useRef<number | undefined>(undefined)
  const latest = useRef(0)

  // The mailbox list is a new array on every render of the caller, so it cannot be a dependency
  // without re-running the search continuously. The identity that matters is the contents.
  const scopeKey = mailboxIds.join(',')

  const execute = useCallback((value: string, within: number[]) => {
    const trimmed = value.trim()

    if (trimmed === '') {
      setHits([])
      setRunning(false)
      return
    }

    const sequence = latest.current + 1
    latest.current = sequence
    setRunning(true)

    void runSearch(trimmed, within, LIMIT)
      .then((results) => {
        // A slow query must not overwrite the answer to a later one, which is how a search
        // box ends up showing results for a prefix the user has already typed past.
        if (sequence !== latest.current) return
        setHits(results.hits)
      })
      .finally(() => {
        if (sequence === latest.current) setRunning(false)
      })
  }, [])

  useEffect(() => {
    window.clearTimeout(timer.current)

    const within = place === 'mailbox' ? scopeKey.split(',').filter(Boolean).map(Number) : []

    timer.current = window.setTimeout(() => {
      execute(text, within)
    }, DEBOUNCE_MS)

    return () => {
      window.clearTimeout(timer.current)
    }
  }, [text, place, scopeKey, execute])

  const commit = useCallback(
    (value: string) => {
      window.clearTimeout(timer.current)
      setTextRaw(value)

      const within = place === 'mailbox' ? scopeKey.split(',').filter(Boolean).map(Number) : []
      execute(value, within)
    },
    [place, scopeKey, execute],
  )

  const clear = useCallback(() => {
    window.clearTimeout(timer.current)
    latest.current += 1
    setTextRaw('')
    setHits([])
    setRunning(false)
  }, [])

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
    setText: setTextRaw,
    setPlace,
    setState,
    commit,
    clear,
  }
}
