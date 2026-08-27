import { describe, expect, it } from 'vitest'

import { highlight } from '@/features/search'

/**
 * Match highlighting. docs/06 Phase 9.
 *
 * The function returns *segments*, never markup, and most of these tests exist to keep it that
 * way: a message subject is hostile input, and the moment highlighting produces HTML it becomes
 * an injection with a friendly name.
 */

describe('highlight', () => {
  it('marks a match and leaves the rest alone', () => {
    expect(highlight('the quarterly figures', ['quarterly'])).toEqual([
      { text: 'the ', match: false },
      { text: 'quarterly', match: true },
      { text: ' figures', match: false },
    ])
  })

  it('ignores case, because the search did', () => {
    const segments = highlight('Quarterly Figures', ['quarterly'])
    expect(segments.find((segment) => segment.match)?.text).toBe('Quarterly')
  })

  it('marks every occurrence', () => {
    const segments = highlight('fig fig fig', ['fig'])
    expect(segments.filter((segment) => segment.match)).toHaveLength(3)
  })

  it('prefers the longest term so a highlight is not left truncated', () => {
    // Searching for "fig figures" must mark the whole word. Taking the shorter term first
    // would consume the start of the longer one and leave "ures" plain, which reads as a bug.
    const segments = highlight('figures', ['fig', 'figures'])
    expect(segments).toEqual([{ text: 'figures', match: true }])
  })

  it('treats a regex metacharacter as a literal', () => {
    // Someone searching for "c++" or "a.b" means those characters. Interpolating them into a
    // pattern unescaped would either throw or match something else entirely.
    expect(highlight('the c++ handbook', ['c++']).some((s) => s.match && s.text === 'c++')).toBe(
      true,
    )
    expect(highlight('a.b', ['a.b'])).toEqual([{ text: 'a.b', match: true }])
    expect(highlight('axb', ['a.b']).some((segment) => segment.match)).toBe(false)
  })

  it('never returns markup, only segments', () => {
    // The property the whole design exists for.
    const hostile = '<img src=x onerror=alert(1)> figures'
    const segments = highlight(hostile, ['figures'])

    expect(segments.map((segment) => segment.text).join('')).toBe(hostile)
    for (const segment of segments) {
      expect(typeof segment.text).toBe('string')
      expect(typeof segment.match).toBe('boolean')
    }
  })

  it('returns the text whole when there is nothing to mark', () => {
    expect(highlight('figures', [])).toEqual([{ text: 'figures', match: false }])
    expect(highlight('figures', ['   '])).toEqual([{ text: 'figures', match: false }])
    expect(highlight('', ['figures'])).toEqual([{ text: '', match: false }])
  })

  it('reassembles into exactly the original text', () => {
    // The invariant that matters most: highlighting must never add, drop or reorder a
    // character of what it was given.
    for (const [text, terms] of [
      ['the quarterly figures', ['quarterly']],
      ['figures', ['figures']],
      ['nothing here', ['absent']],
      ['aaa', ['a']],
      ['', ['a']],
    ] as [string, string[]][]) {
      expect(
        highlight(text, terms)
          .map((segment) => segment.text)
          .join(''),
      ).toBe(text)
    }
  })
})
