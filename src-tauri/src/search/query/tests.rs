//! Tests for the query parser. docs/06 Phase 9.
//!
//! The parser's job is not only to understand what it recognises — it is to leave alone what it
//! does not. Most of these are about the second half, because that is where a query language
//! stops being a feature and starts being something the user fights.

use super::*;
use chrono::{Local, TimeZone};

/// A fixed moment to parse against: Thursday 27 August 2026, 12:00 local.
fn now() -> i64 {
    Local
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("a real moment")
        .timestamp()
}

fn day(year: i32, month: u32, date: u32) -> i64 {
    Local
        .with_ymd_and_hms(year, month, date, 0, 0, 0)
        .single()
        .expect("a real moment")
        .timestamp()
}

#[test]
fn plain_words_are_free_text() {
    let query = parse("quarterly figures", now());
    assert_eq!(query.terms, vec!["quarterly", "figures"]);
    assert!(query.from.is_empty());
}

#[test]
fn fields_are_recognised() {
    let query = parse("from:ada subject:figures", now());
    assert_eq!(query.from, vec!["ada"]);
    assert_eq!(query.subject, vec!["figures"]);
    assert!(query.terms.is_empty());
}

#[test]
fn a_quoted_value_stays_whole() {
    let query = parse("subject:\"the quarterly figures\"", now());
    assert_eq!(query.subject, vec!["the quarterly figures"]);
}

#[test]
fn quoted_free_text_stays_whole() {
    let query = parse("\"quarterly figures\" from:ada", now());
    assert_eq!(query.terms, vec!["quarterly figures"]);
    assert_eq!(query.from, vec!["ada"]);
}

#[test]
fn state_tokens_are_understood_both_ways() {
    assert_eq!(parse("is:unread", now()).is_unread, Some(true));
    assert_eq!(parse("is:read", now()).is_unread, Some(false));
    assert_eq!(parse("is:flagged", now()).is_flagged, Some(true));
    assert_eq!(parse("is:junk", now()).is_junk, Some(true));
    assert_eq!(parse("has:attachment", now()).has_attachment, Some(true));
}

#[test]
fn sizes_use_the_units_the_attachment_list_shows() {
    // Powers of 1024, because that is what the size beside an attachment means. A search that
    // disagreed with the number next to it would look broken.
    assert_eq!(
        parse("larger:5MB", now()).larger_than,
        Some(5 * 1024 * 1024)
    );
    assert_eq!(parse("larger:100kb", now()).larger_than, Some(100 * 1024));
    assert_eq!(parse("smaller:2000", now()).smaller_than, Some(2000));
}

/* ------------------------------------------------------- what it deliberately leaves alone */

#[test]
fn a_url_is_not_a_field() {
    // Only the first colon separates, and only when something precedes it. `http` would
    // otherwise become a field and the rest of the address would vanish from the search.
    let query = parse("http://example.com/report", now());
    assert!(query.terms.iter().any(|term| term.contains("example.com")));
    assert!(query.from.is_empty());
    assert!(query.subject.is_empty());
}

#[test]
fn an_unrecognised_field_becomes_text_rather_than_an_error() {
    // "re: figures" is a thing people type. A parser that rejected it, or silently dropped the
    // half it did not understand, would be worse than no parser.
    let query = parse("re: figures", now());
    assert!(query.terms.iter().any(|term| term.contains("figures")));
    assert!(!query.is_empty());
}

#[test]
fn an_unknown_value_for_a_known_field_becomes_text() {
    // `is:blue` is not a state. Ignoring it would silently return everything; treating it as a
    // filter nobody asked for would silently return nothing.
    let query = parse("is:blue", now());
    assert_eq!(query.is_unread, None);
    assert_eq!(query.is_flagged, None);
    assert_eq!(query.terms, vec!["is:blue"]);
}

#[test]
fn fts_operators_are_carried_as_text_and_never_as_syntax() {
    // Every one of these means something to FTS5. A search for the word AND has to find the
    // word AND, not run a boolean.
    for hostile in ["AND", "NEAR", "OR", "*", "^figures", "\"unclosed"] {
        let query = parse(hostile, now());
        assert!(!query.is_empty(), "{hostile} parsed to nothing at all");
    }
}

#[test]
fn an_empty_search_is_empty() {
    assert!(parse("", now()).is_empty());
    assert!(parse("   ", now()).is_empty());
    assert!(parse("\"\"", now())
        .terms
        .iter()
        .all(|term| term.is_empty()));
}

/* ------------------------------------------------------------------------------- dates */

#[test]
fn yesterday_is_the_whole_of_yesterday() {
    let query = parse("yesterday", now());
    assert_eq!(query.after, Some(day(2026, 8, 26)));
    assert_eq!(query.before, Some(day(2026, 8, 27)));
}

#[test]
fn last_week_is_a_rolling_seven_days() {
    let query = parse("last week", now());
    assert_eq!(query.after, Some(day(2026, 8, 20)));
    assert_eq!(query.before, Some(day(2026, 8, 28)));
}

#[test]
fn this_week_runs_from_monday_rather_than_seven_days_back() {
    // 27 August 2026 is a Thursday, so this week starts on the 24th. Collapsing "this week"
    // into "last week" would find mail from before the weekend, which is not what was asked.
    let query = parse("this week", now());
    assert_eq!(query.after, Some(day(2026, 8, 24)));
}

#[test]
fn in_march_is_a_month_and_march_alone_is_a_name() {
    // The whole reason the preposition is required. `March` is a person's name, and a search
    // box that turned a colleague into a date range would break searches with no error and
    // nothing on screen to explain it.
    let dated = parse("in march", now());
    assert_eq!(dated.after, Some(day(2026, 3, 1)));
    assert_eq!(dated.before, Some(day(2026, 4, 1)));

    let name = parse("march", now());
    assert_eq!(name.after, None);
    assert_eq!(name.before, None);
    assert_eq!(name.terms, vec!["march"]);
}

#[test]
fn a_month_still_to_come_means_last_year() {
    // "In December" said in August means the December that happened. People search their
    // archive, not their calendar.
    let query = parse("in december", now());
    assert_eq!(query.after, Some(day(2025, 12, 1)));
    assert_eq!(query.before, Some(day(2026, 1, 1)));
}

#[test]
fn an_explicit_date_is_that_day_alone() {
    let query = parse("2026-08-01", now());
    assert_eq!(query.after, Some(day(2026, 8, 1)));
    assert_eq!(query.before, Some(day(2026, 8, 2)));
}

#[test]
fn before_and_after_take_the_same_words() {
    assert_eq!(
        parse("before:yesterday", now()).before,
        Some(day(2026, 8, 26))
    );
    assert_eq!(
        parse("after:2026-08-01", now()).after,
        Some(day(2026, 8, 1))
    );
    assert_eq!(parse("after:march", now()).after, Some(day(2026, 3, 1)));
}

#[test]
fn a_date_field_with_nonsense_in_it_becomes_text() {
    let query = parse("before:banana", now());
    assert_eq!(query.before, None);
    assert_eq!(query.terms, vec!["banana"]);
}

#[test]
fn a_date_word_inside_a_field_is_not_a_date() {
    // `subject:yesterday` searches for the word. The phrase parser only looks at unfielded
    // words, or every message whose subject mentioned a day would silently change the range.
    let query = parse("subject:yesterday", now());
    assert_eq!(query.subject, vec!["yesterday"]);
    assert_eq!(query.after, None);
}

#[test]
fn a_date_phrase_combines_with_the_rest_of_the_search() {
    let query = parse("from:ada yesterday figures", now());
    assert_eq!(query.from, vec!["ada"]);
    assert_eq!(query.terms, vec!["figures"]);
    assert_eq!(query.after, Some(day(2026, 8, 26)));
}

#[test]
fn a_quoted_date_word_is_searched_for_rather_than_resolved() {
    // Someone who quoted it meant the word.
    let query = parse("\"yesterday\"", now());
    assert_eq!(query.terms, vec!["yesterday"]);
    assert_eq!(query.after, None);
}

#[test]
fn structured_only_is_recognised() {
    // Worth knowing because such a query cannot use the FTS index at all and has to take a
    // different path.
    assert!(parse("is:unread", now()).is_structured_only());
    assert!(!parse("is:unread figures", now()).is_structured_only());
    assert!(!parse("", now()).is_structured_only());
}
