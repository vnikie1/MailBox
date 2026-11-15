//! Date phrases across a change of the clocks, and dates no calendar holds.
//!
//! The app always reads dates in `Local`, which a test cannot move to a zone with a clock change
//! in it: Windows ignores `TZ`. So these use a zone written here that keeps London's 2026 rules.

use super::*;
use chrono::{FixedOffset, MappedLocalTime, NaiveDateTime};

/// GMT, and BST from 01:00 UTC on 29 March 2026 to 01:00 UTC on 25 October 2026.
#[derive(Clone, Copy, Debug)]
struct London;

const HOUR: i32 = 3600;

fn utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|date| date.and_hms_opt(hour, minute, 0))
        .expect("a real date")
}

impl TimeZone for London {
    type Offset = FixedOffset;

    fn from_offset(_: &FixedOffset) -> Self {
        London
    }

    fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<FixedOffset> {
        self.offset_from_local_datetime(&local.and_hms_opt(0, 0, 0).expect("midnight"))
    }

    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> MappedLocalTime<FixedOffset> {
        // Each offset the zone uses, kept when reading the local time with it lands on a moment
        // the zone does give that offset. Two kept: the hour the clocks go back. None: the hour
        // they skip.
        let mut kept: Vec<FixedOffset> = [HOUR, 0]
            .into_iter()
            .filter_map(FixedOffset::east_opt)
            .filter(|offset| {
                let moment =
                    *local - chrono::Duration::seconds(i64::from(offset.local_minus_utc()));
                self.offset_from_utc_datetime(&moment) == *offset
            })
            .collect();

        match kept.len() {
            2 => MappedLocalTime::Ambiguous(kept[0], kept[1]),
            1 => MappedLocalTime::Single(kept.remove(0)),
            _ => MappedLocalTime::None,
        }
    }

    fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
        self.offset_from_utc_datetime(&utc.and_hms_opt(0, 0, 0).expect("midnight"))
    }

    fn offset_from_utc_datetime(&self, moment: &NaiveDateTime) -> FixedOffset {
        let summer = (utc(2026, 3, 29, 1, 0)..utc(2026, 10, 25, 1, 0)).contains(moment);
        FixedOffset::east_opt(if summer { HOUR } else { 0 }).expect("an offset")
    }
}

/// A moment given in London's wall-clock time.
fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
    London
        .from_local_datetime(&utc(year, month, day, hour, minute))
        .earliest()
        .expect("a moment London has")
        .timestamp()
}

/// The London date a moment falls on.
fn date_of(moment: i64) -> NaiveDate {
    London
        .timestamp_opt(moment, 0)
        .single()
        .expect("a moment")
        .date_naive()
}

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("a real date")
}

#[test]
fn the_zone_written_here_changes_its_clocks() {
    // 01:30 on 29 March never happens; 01:30 on 25 October happens twice.
    assert!(London
        .from_local_datetime(&utc(2026, 3, 29, 1, 30))
        .earliest()
        .is_none());
    assert!(matches!(
        London.from_local_datetime(&utc(2026, 10, 25, 1, 30)),
        MappedLocalTime::Ambiguous(..)
    ));
    assert_eq!(
        at(2026, 3, 30, 0, 30) - at(2026, 3, 29, 0, 0),
        23 * 3600 + 1800
    );
}

#[test]
fn yesterday_is_the_day_before_on_the_morning_after_the_clocks_go_forward() {
    // Half past midnight on Monday 30 March. Sunday was 23 hours long, so 24 hours back is
    // Saturday evening, and "yesterday" was Saturday (L14).
    let now = at(2026, 3, 30, 0, 30);

    let found = phrase_in(&London, &["yesterday"], now).expect("a date");

    assert_eq!(found.after.map(date_of), Some(date(2026, 3, 29)));
    assert_eq!(found.before, Some(at(2026, 3, 30, 0, 0)));
    assert_eq!(
        boundary_in(&London, "yesterday", now).map(date_of),
        Some(date(2026, 3, 29))
    );
}

#[test]
fn last_week_is_seven_calendar_days_across_the_change() {
    // Thursday 2 April, early. Seven days back is Thursday 26 March, not Wednesday.
    let now = at(2026, 4, 2, 0, 30);

    let found = phrase_in(&London, &["last", "week"], now).expect("a date");

    assert_eq!(found.after, Some(at(2026, 3, 26, 0, 0)));
    assert_eq!(
        boundary_in(&London, "7 days", now),
        Some(at(2026, 3, 26, 0, 0))
    );
}

#[test]
fn days_either_side_of_the_clocks_going_back_are_whole_days() {
    // Monday 26 October, early: Sunday was 25 hours long.
    let now = at(2026, 10, 26, 0, 30);

    let found = phrase_in(&London, &["yesterday"], now).expect("a date");

    assert_eq!(found.after, Some(at(2026, 10, 25, 0, 0)));
    assert_eq!(found.before, Some(at(2026, 10, 26, 0, 0)));
}

#[test]
fn a_date_no_calendar_holds_is_not_a_date() {
    // Each of these panicked on overflow, and the panic took the search task with it (L15).
    let now = at(2026, 8, 27, 12, 0);

    for absurd in [
        "300000 years",
        "99999999999999 weeks",
        "9223372036854775807 days",
        "-9223372036854775808 days",
        "-300000 years",
    ] {
        assert_eq!(boundary(absurd, now), None, "{absurd}");
    }

    // The last day chrono can hold has no day after it to end on.
    assert_eq!(phrase(&["+262143-12-31"], now), None);
    assert_eq!(phrase(&["262143-12-31"], now), None);
}

#[test]
fn an_absurd_date_in_a_search_is_searched_for_as_text() {
    let query =
        crate::search::query::parse(r#"after:"300000 years" report"#, at(2026, 8, 27, 12, 0));

    assert_eq!(query.after, None);
    assert_eq!(query.terms, vec!["300000 years", "report"]);
}
