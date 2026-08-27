import type { Condition } from '@/lib/generated/Condition'
import type { Field } from '@/lib/generated/Field'
import type { Op } from '@/lib/generated/Op'
import type { Predicate } from '@/lib/generated/Predicate'

/**
 * Reading and writing the flat predicate shape the editor offers.
 *
 * Separate from `PredicateEditor.tsx` because these are pure functions, and a module that
 * exports both a component and plain functions breaks Fast Refresh — every keystroke in the
 * editor would remount the whole panel and lose what was typed. The lint rule that flags it is
 * pointing at a real cost, not a style preference.
 *
 * The shape is deliberately **flat**: a list of conditions joined by all-or-any. `Predicate`
 * nests arbitrarily and the core evaluates whatever it is given, but a UI for arbitrary nesting
 * is a UI nobody can use, and Mail makes the same choice. A nested predicate written by some
 * future editor still loads, matches and runs — it simply cannot be edited here, and `toFlat`
 * returns `null` so the caller can say so plainly.
 */

export const BLANK_CONDITION: Condition = { field: 'from', op: 'contains', value: '' }

export const FIELD_LABELS: Record<Field, string> = {
  from: 'From',
  to: 'To',
  cc: 'Cc',
  subject: 'Subject',
  body: 'Message content',
  anyText: 'Any text',
  mailbox: 'Mailbox',
  dateReceived: 'Date received',
  size: 'Size',
  hasAttachment: 'Has attachment',
  isUnread: 'Is unread',
  isFlagged: 'Is flagged',
  isJunk: 'Is junk',
  attachmentName: 'Attachment name',
}

export const OP_LABELS: Record<Op, string> = {
  contains: 'contains',
  notContains: 'does not contain',
  is: 'is',
  isNot: 'is not',
  beginsWith: 'begins with',
  endsWith: 'ends with',
  greaterThan: 'is greater than',
  lessThan: 'is less than',
  isTrue: 'is true',
  isFalse: 'is false',
}

/** Which operators make sense for a field. Offering `contains` on a boolean is noise. */
export function opsFor(field: Field): Op[] {
  switch (field) {
    case 'hasAttachment':
    case 'isUnread':
    case 'isFlagged':
    case 'isJunk':
      return ['isTrue', 'isFalse']
    case 'dateReceived':
    case 'size':
      return ['greaterThan', 'lessThan', 'is', 'isNot']
    default:
      return ['contains', 'notContains', 'is', 'isNot', 'beginsWith', 'endsWith']
  }
}

export function takesValue(op: Op): boolean {
  return op !== 'isTrue' && op !== 'isFalse'
}

/**
 * Reads a stored predicate as a flat list, when it is one.
 *
 * `null` for anything this editor cannot represent, so the caller says so rather than silently
 * flattening and saving back something that means something different from what was opened.
 */
export function toFlat(
  predicate: Predicate,
): { matchAll: boolean; conditions: Condition[] } | null {
  if (predicate.type === 'is') {
    return { matchAll: true, conditions: [predicate.value] }
  }

  if (predicate.type === 'all' || predicate.type === 'any') {
    const conditions: Condition[] = []

    for (const child of predicate.value) {
      if (child.type !== 'is') return null
      conditions.push(child.value)
    }

    return { matchAll: predicate.type === 'all', conditions }
  }

  return null
}

export function fromFlat(matchAll: boolean, conditions: Condition[]): Predicate {
  return {
    type: matchAll ? 'all' : 'any',
    value: conditions.map((condition) => ({ type: 'is', value: condition })),
  }
}
