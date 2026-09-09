import { describe, expect, it } from 'vitest'

import { repairShortRows } from '@/features/reader/repairTables'

/**
 * Rows missing their trailing cells.
 *
 * ## The mail this exists for
 *
 * Reported from using the app: a delivery receipt rendered as a column about two words wide
 * with most of the pane blank beside it. The pipeline was measured and exonerated — sanitiser,
 * inlined images, frame stylesheet and stored HTML all render it identically. It is the
 * message's own markup: row one declares three columns, row two supplies two cells, so the
 * body lands in the column the logo pinned to `width='100'` and the empty third column takes
 * the rest. Measured at a 1502px frame the columns came out **18 / 100 / 1380**.
 *
 * With the repair, that same document measured 1493px instead of 100px.
 */

/** Cells of the given row, as "text:colspan" so both are asserted at once. */
function row(html: string, index: number): string[] {
  const doc = new DOMParser().parseFromString(html, 'text/html')
  const tr = doc.querySelectorAll('tr')[index]

  return [...(tr?.children ?? [])].map(
    (cell) => `${cell.textContent.trim()}:${cell.getAttribute('colspan') ?? '1'}`,
  )
}

describe('a row missing its trailing cells', () => {
  it('extends the last cell to reach the table’s column count', () => {
    const out = repairShortRows(
      '<table><tr><td>a</td><td>b</td><td>c</td></tr><tr><td>d</td><td>e</td></tr></table>',
    )

    expect(row(out, 1)).toEqual(['d:1', 'e:2'])
  })

  it('adds to an existing colspan rather than replacing it', () => {
    const out = repairShortRows(
      '<table><tr><td>a</td><td>b</td><td>c</td><td>d</td></tr><tr><td colspan="2">e</td></tr></table>',
    )

    expect(row(out, 1)).toEqual(['e:4'])
  })
})

describe('what it leaves alone', () => {
  it('a table whose rows all agree', () => {
    // The repair only ever acts on a short row, so a well-formed table cannot be touched —
    // which is what bounds this to the malformed mail it exists for.
    const html = '<table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>'

    expect(row(repairShortRows(html), 1)).toEqual(['c:1', 'd:1'])
  })

  it('any table containing a rowspan', () => {
    // A cell spanning rows occupies a column in each without appearing in its markup, so the
    // row beneath legitimately has fewer cells. Extending it would corrupt a table that renders
    // correctly today.
    const out = repairShortRows(
      '<table><tr><td rowspan="2">a</td><td>b</td><td>c</td></tr><tr><td>d</td></tr></table>',
    )

    expect(row(out, 1)).toEqual(['d:1'])
  })

  it('a table with only one row', () => {
    const html = '<table><tr><td>a</td><td>b</td></tr></table>'
    expect(row(repairShortRows(html), 0)).toEqual(['a:1', 'b:1'])
  })

  it('the rows of a nested table, when counting the outer one’s columns', () => {
    // `table.rows` reaches into nested tables. Counting a nested row as one of the outer
    // table's would invent columns that do not exist and stretch cells to reach them.
    const out = repairShortRows(
      '<table><tr><td><table><tr><td>x</td><td>y</td><td>z</td></tr></table></td><td>b</td></tr>' +
        '<tr><td>c</td><td>d</td></tr></table>',
    )

    expect(row(out, 2)).toEqual(['c:1', 'd:1'])
  })
})

describe('the reported message', () => {
  it('lets the body escape the column the logo pinned', () => {
    // The shape of the real thing: three columns declared by the logo row, two supplied by the
    // row carrying the message.
    const out = repairShortRows(
      "<table><tr><td>&nbsp;</td><td width='100'><img src='x'></td><td>&nbsp;</td></tr>" +
        "<tr><td width='5'>&nbsp;</td><td>Greetings from Instamart</td></tr></table>",
    )

    // The body cell now spans the logo's column and the empty one that was taking the width.
    expect(row(out, 1)[1]).toBe('Greetings from Instamart:2')
  })
})
