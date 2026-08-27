/**
 * Match highlighting. docs/06 Phase 9.
 *
 * Splits text into alternating plain and matched segments, so the caller can render the matched
 * ones without ever building HTML from user text. That is the whole point of returning segments
 * rather than a marked-up string: a message subject is hostile input, and the moment
 * highlighting produces markup it becomes an injection with a friendly name.
 */

export interface Segment {
  text: string
  match: boolean
}

/** Characters that mean something to a regular expression, escaped so they match literally. */
function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * Marks every occurrence of any term.
 *
 * Case-insensitive, because the search was. Terms are sorted longest-first so that searching
 * for "fig figures" marks the whole word rather than leaving "ures" plain — the shorter term
 * would otherwise consume the start of the longer one and the highlight would look truncated.
 */
export function highlight(text: string, terms: string[]): Segment[] {
  const usable = terms
    .map((term) => term.trim())
    .filter((term) => term.length > 0)
    .sort((a, b) => b.length - a.length)

  if (usable.length === 0 || text === '') return [{ text, match: false }]

  const pattern = new RegExp(`(${usable.map(escapeRegex).join('|')})`, 'gi')
  const segments: Segment[] = []

  let last = 0
  for (const found of text.matchAll(pattern)) {
    const at = found.index
    if (at > last) segments.push({ text: text.slice(last, at), match: false })
    segments.push({ text: found[0], match: true })
    last = at + found[0].length
  }

  if (last < text.length) segments.push({ text: text.slice(last), match: false })

  return segments
}
