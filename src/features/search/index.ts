/**
 * Search. docs/01 §7, docs/06 Phase 9.
 *
 * The field, its dropdown, the scope bar, and the hook that runs the query. The parsing,
 * ranking and SQL all live in the core — this half only decides what to ask and how to show
 * the answer.
 */

export { ScopeBar, type ScopeBarProps, type Place, type State } from './ScopeBar'
export { SearchField, type SearchFieldProps } from './SearchField'
export { highlight, type Segment } from './highlight'
export { useSearch, type SearchState } from './useSearch'
