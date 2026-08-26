import { describe, expect, it } from 'vitest'

import {
  formatFileSize,
  formatReaderDate,
  formatRowDate,
  sectionForDate,
  sectionKeyForDate,
} from '@/lib/date'

/**
 * Written before the formatter, per standing rule 19. Date handling is all edge case: the
 * interesting behaviour is at midnight, at the year boundary, and at the seam between
 * "within a week" and "older", none of which you think to handle by writing the happy path
 * first.
 *
 * NOW is a Wednesday, mid-evening, mid-year — far enough from every boundary that a test
 * failing means the logic is wrong rather than the fixture being unlucky.
 */
const NOW = new Date('2026-08-26T19:30:00')

describe('formatRowDate', () => {
  it('shows a time for today', () => {
    expect(formatRowDate(new Date('2026-08-26T09:41:00'), NOW)).toBe('9:41 AM')
    expect(formatRowDate(new Date('2026-08-26T18:05:00'), NOW)).toBe('6:05 PM')
  })

  it('says Yesterday by calendar day, not by elapsed hours', () => {
    // 23:50 last night is under two hours ago and is still Yesterday. An implementation
    // that subtracts hours gets this wrong every evening.
    expect(formatRowDate(new Date('2026-08-25T23:50:00'), NOW)).toBe('Yesterday')
    expect(formatRowDate(new Date('2026-08-25T00:05:00'), NOW)).toBe('Yesterday')
  })

  it('names the weekday within the last week', () => {
    expect(formatRowDate(new Date('2026-08-24T12:00:00'), NOW)).toBe('Monday')
    expect(formatRowDate(new Date('2026-08-21T12:00:00'), NOW)).toBe('Friday')
  })

  it('switches to a date on the seventh day, not the eighth', () => {
    // Six days back is still a weekday name; seven is not, because by then the name is
    // ambiguous with the day you are standing on.
    expect(formatRowDate(new Date('2026-08-20T12:00:00'), NOW)).toBe('Thursday')
    expect(formatRowDate(new Date('2026-08-19T12:00:00'), NOW)).toBe('19/08')
  })

  it('omits the year within the current year and shows it beyond', () => {
    expect(formatRowDate(new Date('2026-03-12T12:00:00'), NOW)).toBe('12/03')
    expect(formatRowDate(new Date('2025-03-12T12:00:00'), NOW)).toBe('12/03/25')
  })
})

describe('formatReaderDate', () => {
  it('spells the date out where there is room', () => {
    expect(formatReaderDate(new Date('2026-08-26T09:41:00'), NOW)).toBe('9:41 AM')
    expect(formatReaderDate(new Date('2026-08-25T22:27:00'), NOW)).toBe('Yesterday at 10:27 PM')
    expect(formatReaderDate(new Date('2026-03-12T14:00:00'), NOW)).toBe('12 March • 2:00 PM')
    expect(formatReaderDate(new Date('2025-03-12T14:00:00'), NOW)).toBe('12 March 2025 • 2:00 PM')
  })
})

describe('sectionForDate', () => {
  it('walks the buckets from docs/01 §4', () => {
    expect(sectionForDate(new Date('2026-08-26T01:00:00'), NOW)).toBe('Today')
    expect(sectionForDate(new Date('2026-08-25T23:00:00'), NOW)).toBe('Yesterday')
    expect(sectionForDate(new Date('2026-08-22T12:00:00'), NOW)).toBe('Previous 7 Days')
    expect(sectionForDate(new Date('2026-08-10T12:00:00'), NOW)).toBe('Previous 30 Days')
    expect(sectionForDate(new Date('2026-05-10T12:00:00'), NOW)).toBe('May')
    expect(sectionForDate(new Date('2024-05-10T12:00:00'), NOW)).toBe('2024')
  })

  it('does not leave a gap between the 7-day and 30-day buckets', () => {
    // Day 7 falls out of "Previous 7 Days" and must land in "Previous 30 Days" rather than
    // in a month name — the boundary an off-by-one puts in the wrong bucket.
    expect(sectionForDate(new Date('2026-08-19T12:00:00'), NOW)).toBe('Previous 30 Days')
    expect(sectionForDate(new Date('2026-07-28T12:00:00'), NOW)).toBe('Previous 30 Days')
    expect(sectionForDate(new Date('2026-07-27T12:00:00'), NOW)).toBe('July')
  })
})

describe('sectionKeyForDate', () => {
  it('distinguishes the same month in different years', () => {
    // Two years of mail reaches "May" twice. Sharing a key would splice the two runs of
    // rows together under one header.
    const key2026 = sectionKeyForDate(new Date('2026-05-10T12:00:00'), NOW)
    const key2025 = sectionKeyForDate(new Date('2025-05-10T12:00:00'), NOW)
    expect(key2026).not.toBe(key2025)
  })
})

describe('formatFileSize', () => {
  it('scales the unit and keeps the number short', () => {
    expect(formatFileSize(512)).toBe('512 bytes')
    expect(formatFileSize(2048)).toBe('2.0 KB')
    expect(formatFileSize(20_480)).toBe('20 KB')
    expect(formatFileSize(5_242_880)).toBe('5.0 MB')
    expect(formatFileSize(2_147_483_648)).toBe('2.0 GB')
  })
})
