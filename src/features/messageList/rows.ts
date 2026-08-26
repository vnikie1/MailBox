import type { MessageRow } from '@/lib/generated/MessageRow'
import { sectionForDate, sectionKeyForDate } from '@/lib/date'

/**
 * The flat item list the virtualiser walks. docs/01 §4.
 *
 * Section headers are items in the same list as the rows rather than a separate layer. That
 * is what makes their heights part of the virtualiser's own arithmetic, so scrolling to
 * index 900 lands in the right place instead of drifting by however many headers were
 * passed on the way.
 */

export type ListItem =
  | { kind: 'header'; key: string; label: string }
  | { kind: 'row'; key: string; message: MessageRow; runStart: boolean; runEnd: boolean }

export interface BuildOptions {
  rows: MessageRow[]
  now: Date
  selected: ReadonlySet<number>
  /** Date grouping only applies when the list is actually in date order. */
  grouped: boolean
}

/** Epoch seconds to a Date, at the one place rows cross from the wire into the UI. */
export function receivedAt(row: MessageRow): Date {
  return new Date(row.dateReceived * 1000)
}

/**
 * Marks each selected row as the start or end of a contiguous run.
 *
 * docs/01 §4 wants a multi-selection to read as one rounded block rather than a stack of
 * separate pills. Deciding it here, where the neighbours are known, keeps the row component
 * a pure function of its own props — it never has to look at its siblings.
 */
export function buildListItems({ rows, now, selected, grouped }: BuildOptions): ListItem[] {
  const items: ListItem[] = []
  let lastSectionKey: string | null = null

  rows.forEach((message, index) => {
    if (grouped) {
      const key = sectionKeyForDate(receivedAt(message), now)
      if (key !== lastSectionKey) {
        items.push({
          kind: 'header',
          key: `header-${key}`,
          label: sectionForDate(receivedAt(message), now),
        })
        lastSectionKey = key
      }
    }

    const isSelected = selected.has(message.id)
    const previous = rows[index - 1]
    const next = rows[index + 1]

    // A run also breaks across a section header, because the header interrupts the block
    // visually whether or not the rows either side of it are both selected.
    const sameSectionAs = (other: MessageRow | undefined) =>
      other !== undefined &&
      (!grouped ||
        sectionKeyForDate(receivedAt(other), now) === sectionKeyForDate(receivedAt(message), now))

    items.push({
      kind: 'row',
      key: String(message.id),
      message,
      runStart:
        isSelected &&
        !(previous !== undefined && selected.has(previous.id) && sameSectionAs(previous)),
      runEnd: isSelected && !(next !== undefined && selected.has(next.id) && sameSectionAs(next)),
    })
  })

  return items
}
