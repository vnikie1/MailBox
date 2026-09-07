import { differenceInCalendarDays, format, isSameYear } from 'date-fns'

/**
 * Dates as Mail writes them. docs/01 §4.
 *
 * Everything here takes `now` as an argument rather than reading the clock. Two reasons:
 * a list rendered at 23:59 must not relabel itself at 00:00 while you are looking at it,
 * and a formatter that reads the clock cannot be tested — its behaviour changes at
 * midnight, which is exactly when nobody is watching CI.
 *
 * That is also why date-fns's `isToday` and `isYesterday` are not used here despite being
 * the obvious fit. They compare against the *system clock*, ignoring any reference date,
 * so a formatter built on them silently disregards its own `now` argument. The first
 * version of this file used them and the tests caught it on the first run.
 *
 * Comparisons are by *calendar* day, not elapsed hours. Something sent at 23:50 last night
 * is "Yesterday" at 01:00, not "an hour ago".
 */

/** Whole calendar days between the two, positive when `date` is in the past. */
function daysAgo(date: Date, now: Date): number {
  return differenceInCalendarDays(now, date)
}

/**
 * The right-hand column of a message row.
 *
 *   today            9:41 AM
 *   yesterday        Yesterday
 *   within a week    Monday
 *   this year        12/03
 *   older            12/03/25
 *
 * The last two implement docs/01 §4's "never shows a year within the current year".
 */
export function formatRowDate(date: Date, now: Date): string {
  const days = daysAgo(date, now)

  // A future date means a skewed sender clock, not a bug worth a special case; showing the
  // time is the least misleading thing to do with it.
  if (days <= 0) return format(date, 'h:mm a')
  if (days === 1) return 'Yesterday'
  if (days < 7) return format(date, 'EEEE')

  return isSameYear(date, now) ? format(date, 'dd/MM') : format(date, 'dd/MM/yy')
}

/**
 * The reader header, where there is room for the whole thing.
 *
 *   today            Today, 26 August • 9:41 AM
 *   yesterday        Yesterday, 25 August • 10:27 PM
 *   this year        12 March • 2:00 PM
 *   older            12 March 2025 • 2:00 PM
 *
 * Every branch names a calendar date. The first two did not — today was a bare `9:41 AM`
 * and yesterday was `Yesterday at 10:27 PM` — which was reported from using the app: "the
 * mail preview doesnt show the date of the mail at top, just the time".
 *
 * It reads as an omission rather than a shorthand because this is the header of an *open*
 * message, which is where a date gets copied out, quoted into a reply, or read aloud to
 * somebody. The row in the list is the place for a bare time: there the neighbouring rows
 * and the sticky `Today` header supply the day, and the column is too narrow for more.
 * Open on its own, a message has neither, and `9:41 AM` cannot say which day it was.
 *
 * `Today` and `Yesterday` are kept in front of the date rather than replaced by it. They
 * are the fastest thing to read and the whole reason the relative form exists; the date is
 * added beside them, not instead of them.
 *
 * docs/01 §4's diagram shows `9:41 AM` in this header, but the message it draws is itself
 * from today, so it fixes the time and says nothing about the date — the diagram is equally
 * consistent with either rule. The two dated branches below were already here and already
 * departed from a literal reading of it.
 */
export function formatReaderDate(date: Date, now: Date): string {
  const days = daysAgo(date, now)

  if (days <= 0) return `Today, ${format(date, 'd MMMM • h:mm a')}`
  if (days === 1) return `Yesterday, ${format(date, 'd MMMM • h:mm a')}`

  return isSameYear(date, now)
    ? format(date, 'd MMMM • h:mm a')
    : format(date, 'd MMMM yyyy • h:mm a')
}

/**
 * The sticky section header a row belongs under. docs/01 §4.
 *
 * Older messages collapse to a month, and older still to a year, so scrolling two years of
 * mail passes a few dozen headers rather than seven hundred.
 */
export function sectionForDate(date: Date, now: Date): string {
  const days = daysAgo(date, now)

  if (days <= 0) return 'Today'
  if (days === 1) return 'Yesterday'
  if (days < 7) return 'Previous 7 Days'
  if (days < 30) return 'Previous 30 Days'

  return isSameYear(date, now) ? format(date, 'MMMM') : format(date, 'yyyy')
}

/**
 * Stable key for a section.
 *
 * Two years of mail reaches "May" twice; sharing a key would splice the two runs of rows
 * together under one header. Keying the month buckets by year-and-month keeps them apart,
 * while the fixed buckets keep their own names.
 */
export function sectionKeyForDate(date: Date, now: Date): string {
  const days = daysAgo(date, now)

  if (days <= 0) return 'today'
  if (days === 1) return 'yesterday'
  if (days < 7) return 'previous-7'
  if (days < 30) return 'previous-30'

  return isSameYear(date, now) ? format(date, 'yyyy-MM') : format(date, 'yyyy')
}

const UNITS = ['bytes', 'KB', 'MB', 'GB'] as const

/** Attachment chip sizes. docs/02 §6.8. */
export function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`

  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024
    unit += 1
  }

  return `${value.toFixed(value < 10 ? 1 : 0)} ${UNITS[unit] ?? 'bytes'}`
}
