import { cx } from '@/lib/cx'

import styles from './ScopeBar.module.css'

/** Where to search. */
export type Place = 'all' | 'mailbox'

/** Which of the results to show. */
export type State = 'all' | 'unread' | 'flagged'

export interface ScopeBarProps {
  place: Place
  state: State
  /** The mailbox the user was in when they started searching, for the second tab's label. */
  mailboxLabel: string
  resultCount: number
  onPlaceChange: (place: Place) => void
  onStateChange: (state: State) => void
  onSaveSearch: () => void
}

/**
 * The bar above search results. docs/01 §7, docs/06 Phase 9.
 *
 * Two independent choices, which is why they are two groups rather than one row of five
 * buttons: *where* to look and *which* results to keep are different questions, and a single
 * group would imply picking "Unread" replaces "Inbox".
 *
 * The counts are on the bar because the first thing anybody does with a search result is judge
 * whether it found too much — and a list of fifty with no total does not say.
 */
export function ScopeBar({
  place,
  state,
  mailboxLabel,
  resultCount,
  onPlaceChange,
  onStateChange,
  onSaveSearch,
}: ScopeBarProps) {
  return (
    <div className={styles.bar}>
      <div className={styles.group} role="radiogroup" aria-label="Where to search">
        {(
          [
            ['all', 'All Mailboxes'],
            ['mailbox', mailboxLabel],
          ] as [Place, string][]
        ).map(([id, label]) => (
          <button
            key={id}
            type="button"
            role="radio"
            aria-checked={place === id}
            className={cx(styles.tab, place === id && styles.active)}
            onClick={() => {
              onPlaceChange(id)
            }}
          >
            {label}
          </button>
        ))}
      </div>

      <div className={styles.divider} aria-hidden />

      <div className={styles.group} role="radiogroup" aria-label="Which results">
        {(
          [
            ['all', 'All'],
            ['unread', 'Unread'],
            ['flagged', 'Flagged'],
          ] as [State, string][]
        ).map(([id, label]) => (
          <button
            key={id}
            type="button"
            role="radio"
            aria-checked={state === id}
            className={cx(styles.tab, state === id && styles.active)}
            onClick={() => {
              onStateChange(id)
            }}
          >
            {label}
          </button>
        ))}
      </div>

      <span className={styles.spacer} />

      <span className={styles.count}>
        {resultCount === 0
          ? 'No results'
          : `${String(resultCount)} result${resultCount === 1 ? '' : 's'}`}
      </span>

      <button
        type="button"
        className={styles.save}
        disabled={resultCount === 0}
        onClick={onSaveSearch}
      >
        Save Search…
      </button>
    </div>
  )
}
