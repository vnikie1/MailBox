/**
 * Repairs table rows that are missing their trailing cells.
 *
 * ## The mail this exists for
 *
 * Reported from using the app: a delivery receipt rendered as a column about two words wide,
 * with most of the message pane blank beside it. Nothing in the pipeline was at fault — the
 * sanitiser, the inlined images, the frame's stylesheet and the stored HTML were each
 * measured and exonerated. It is the message's own markup:
 *
 * ```html
 * <tr> <td>&nbsp;</td> <td width='100'><img …logo…></td> <td>&nbsp;</td> </tr>
 * <tr> <td width='5'>&nbsp;</td> <td>…the entire message…</td> </tr>
 * ```
 *
 * Row one declares three columns; row two supplies two cells. So the message body lands in
 * column two — the column the logo pinned to 100px — and column three, which is empty in every
 * row and carries no width, absorbs the whole surplus. Measured at a 1502px frame, the columns
 * came out **18 / 100 / 1380**, and the body wrapped inside that hundred pixels.
 *
 * The author meant the second cell to span the rest of the row and omitted the `colspan`. Every
 * client that shows this message correctly is being more forgiving than the specification
 * requires, and this is that forgiveness written down.
 *
 * ## Why this cannot break a well-formed table
 *
 * A table whose rows all declare the same number of columns is left exactly as it was: the
 * repair only ever acts on a row that is *short*, and a well-formed table has none. So the
 * blast radius is precisely the malformed tables it exists for.
 *
 * ## Why `rowspan` is a hard stop
 *
 * A cell spanning several rows occupies a column in each of them without appearing in their
 * markup, so a row underneath one legitimately carries fewer cells than the table has columns.
 * Extending its last cell would then be wrong rather than forgiving. Tracking that properly is
 * possible; getting it subtly wrong would corrupt tables that render correctly today. Any table
 * containing a `rowspan` is therefore left alone, which costs nothing — the pattern this
 * repairs uses spacer columns, not spanned rows.
 */

/** `colspan`, defaulting to 1 and refusing anything that is not a positive number. */
function span(cell: Element): number {
  const declared = Number(cell.getAttribute('colspan') ?? '1')
  return Number.isInteger(declared) && declared > 0 ? declared : 1
}

/** The rows belonging to this table — not to a table nested inside one of its cells. */
function ownRows(table: HTMLTableElement): HTMLTableRowElement[] {
  return [...table.rows].filter((row) => row.closest('table') === table)
}

export function repairShortRows(html: string): string {
  // `text/html` parsing runs no script and fetches nothing; this markup has already been
  // sanitised by the core, and this is a layout repair rather than a second sanitising pass.
  const doc = new DOMParser().parseFromString(html, 'text/html')

  for (const table of doc.querySelectorAll('table')) {
    const rows = ownRows(table)
    if (rows.length < 2) continue

    // See the note above: a rowspan makes "short" ambiguous, so leave the table as it is.
    if (rows.some((row) => [...row.cells].some((cell) => cell.hasAttribute('rowspan')))) continue

    const widths = rows.map((row) => [...row.cells].reduce((total, cell) => total + span(cell), 0))
    const columns = Math.max(...widths)

    rows.forEach((row, index) => {
      const missing = columns - (widths[index] ?? 0)
      const last = row.cells[row.cells.length - 1]

      if (missing > 0 && last !== undefined) {
        last.setAttribute('colspan', String(span(last) + missing))
      }
    })
  }

  return doc.body.innerHTML
}
