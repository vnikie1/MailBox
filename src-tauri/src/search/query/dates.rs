//! Turning dates people type into the bounds a query needs. docs/06 Phase 9.
//!
//! ## The rule that keeps this from being a nuisance
//!
//! A bare month name is **never** a date. `March` is a person, `May` is a person, `August` is a
//! surname, and a search box that quietly turned a colleague's name into a date range would be
//! infuriating in a way the user could not diagnose — their search would simply stop finding
//! things, with no error and nothing on screen to explain it.
//!
//! So a phrase is only read as a date when it carries an unambiguous temporal marker: a
//! preposition (`in March`), a determiner (`last week`, `this month`), or a word that can only
//! be a date (`yesterday`, `2026-08-01`). Everything else stays as text to search for.
//!
//! ## Ranges, not instants
//!
//! "Yesterday" is a day, not a moment. `after:yesterday` means from the start of yesterday, and
//! `before:yesterday` means up to the start of yesterday — the day itself is excluded, because
//! "before yesterday" that included yesterday would be a lie. A phrase in free text sets both
//! bounds, so "yesterday" alone means the whole of yesterday and nothing else.
//!
//! Everything works in **local** time. A day is the user's day: a message received at 11pm is
//! yesterday's mail to them whatever UTC thinks, and using UTC here would shift every boundary
//! by the offset and make the last hours of each day land in the wrong bucket.
//!
//! And days are counted on the calendar, never as 24 hours. A day is 23 or 25 hours long when the
//! clocks change, so "yesterday" taken as 24 hours back lands two days back on the morning after
//! the clocks go forward (L14). Every function takes the time zone so a test can use one with a
//! change in it; the app's is always `Local`.
//!
//! None of it can panic. A search box takes any text, and `after:"300000 years"` asked for a
//! date chrono cannot hold: the arithmetic overflowed, and the panic took the search task with it,
//! so the search never answered (L15). Out-of-range arithmetic now means "not a date", and the
//! words are searched for as text.

use chrono::{Datelike, Days, Local, NaiveDate, TimeZone};

/// A resolved date phrase, and how many words it consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    pub after: Option<i64>,
    pub before: Option<i64>,
    /// How many whitespace-separated words the phrase used, so the caller can skip them.
    pub words: usize,
}

/// The calendar day `now` falls on, in `zone`.
fn today<Tz: TimeZone>(zone: &Tz, now: i64) -> Option<NaiveDate> {
    Some(zone.timestamp_opt(now, 0).single()?.date_naive())
}

/// `day` moved by `offset` calendar days, or `None` past the range chrono can hold.
fn shift(day: NaiveDate, offset: i64) -> Option<NaiveDate> {
    let days = Days::new(offset.unsigned_abs());
    if offset < 0 {
        day.checked_sub_days(days)
    } else {
        day.checked_add_days(days)
    }
}

/// Midnight at the start of the day `offset` calendar days from `now`.
fn midnight<Tz: TimeZone>(zone: &Tz, now: i64, offset: i64) -> Option<i64> {
    start_of(zone, shift(today(zone, now)?, offset)?)
}

fn start_of<Tz: TimeZone>(zone: &Tz, day: NaiveDate) -> Option<i64> {
    let naive = day.and_hms_opt(0, 0, 0)?;
    zone.from_local_datetime(&naive)
        // A midnight that happens twice resolves to the first. One that does not happen at all,
        // in a zone whose clocks jump at midnight, resolves to the moment the day does begin
        // rather than failing the whole search.
        .earliest()
        .or_else(|| {
            (1..=3).find_map(|hour| {
                zone.from_local_datetime(&day.and_hms_opt(hour, 0, 0)?)
                    .earliest()
            })
        })
        .map(|moment| moment.timestamp())
}

/// The month a name refers to, or `None`. Full names and the usual three-letter forms.
fn month_number(word: &str) -> Option<u32> {
    let lowered = word.trim().to_ascii_lowercase();

    let months = [
        ("january", 1),
        ("february", 2),
        ("march", 3),
        ("april", 4),
        ("may", 5),
        ("june", 6),
        ("july", 7),
        ("august", 8),
        ("september", 9),
        ("october", 10),
        ("november", 11),
        ("december", 12),
    ];

    months
        .iter()
        .find(|(name, _)| *name == lowered || (lowered.len() >= 3 && name.starts_with(&lowered)))
        .map(|(_, number)| *number)
}

/// The whole of one month, as a range. Picks the most recent occurrence not in the future.
fn month_range<Tz: TimeZone>(zone: &Tz, now: i64, month: u32) -> Option<Found> {
    let moment = today(zone, now)?;
    let this_year = moment.year();

    // "In March" said in January means last March, not the March that has not happened. People
    // search their archive, not their calendar.
    let year = if month > moment.month() {
        this_year - 1
    } else {
        this_year
    };

    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };

    Some(Found {
        after: start_of(zone, first),
        before: start_of(zone, next),
        words: 0,
    })
}

/// The whole of one year.
fn year_range<Tz: TimeZone>(zone: &Tz, year: i32) -> Option<Found> {
    Some(Found {
        after: start_of(zone, NaiveDate::from_ymd_opt(year, 1, 1)?),
        before: start_of(zone, NaiveDate::from_ymd_opt(year + 1, 1, 1)?),
        words: 0,
    })
}

/// Parses one word as an explicit date: `2026-08-01`, `2026/08/01`, or `01/08/2026`.
fn explicit(word: &str) -> Option<NaiveDate> {
    for format in ["%Y-%m-%d", "%Y/%m/%d", "%d/%m/%Y", "%d-%m-%Y"] {
        if let Ok(day) = NaiveDate::parse_from_str(word, format) {
            return Some(day);
        }
    }
    None
}

/// Resolves the value of a `before:` or `after:` token.
///
/// Accepts everything the free-text phrases do, plus a bare date. A bare month name **is**
/// accepted here: `after:march` is unambiguous in a way that `march` on its own is not, because
/// the user has already said they mean a date.
pub fn boundary(value: &str, now: i64) -> Option<i64> {
    boundary_in(&Local, value, now)
}

fn boundary_in<Tz: TimeZone>(zone: &Tz, value: &str, now: i64) -> Option<i64> {
    let lowered = value.trim().to_ascii_lowercase();

    if let Some(day) = explicit(&lowered) {
        return start_of(zone, day);
    }

    match lowered.as_str() {
        "today" => return midnight(zone, now, 0),
        "yesterday" => return midnight(zone, now, -1),
        "tomorrow" => return midnight(zone, now, 1),
        _ => {}
    }

    if let Some(month) = month_number(&lowered) {
        return month_range(zone, now, month)?.after;
    }

    if let Ok(year) = lowered.parse::<i32>() {
        if (1970..=2200).contains(&year) {
            return year_range(zone, year)?.after;
        }
    }

    // "3 days", "2 weeks" — relative to now, looking backwards, which is the only direction a
    // mail search ever means.
    let parts: Vec<&str> = lowered.split_whitespace().collect();
    if parts.len() == 2 {
        if let Ok(count) = parts[0].parse::<i64>() {
            return relative(zone, now, count, parts[1]);
        }
    }

    None
}

/// `n days ago`-style arithmetic, given a unit word.
fn relative<Tz: TimeZone>(zone: &Tz, now: i64, count: i64, unit: &str) -> Option<i64> {
    let days = match unit.trim_end_matches('s') {
        "day" => Some(count),
        "week" => count.checked_mul(7),
        "month" => count.checked_mul(30),
        "year" => count.checked_mul(365),
        _ => return None,
    }?;

    midnight(zone, now, days.checked_neg()?)
}

/// Reads a date phrase from the start of `words`, if there is one.
///
/// Returns how many words it used so the caller can skip them. `None` when the first word does
/// not begin a phrase, which is the common case and has to be cheap.
pub fn phrase(words: &[&str], now: i64) -> Option<Found> {
    phrase_in(&Local, words, now)
}

fn phrase_in<Tz: TimeZone>(zone: &Tz, words: &[&str], now: i64) -> Option<Found> {
    let midnight = |offset| midnight(zone, now, offset);
    let first = words.first()?.to_ascii_lowercase();

    // One-word phrases that can only be dates.
    let single = match first.as_str() {
        "today" => Some((midnight(0), midnight(1))),
        "yesterday" => Some((midnight(-1), midnight(0))),
        _ => None,
    };

    if let Some((after, before)) = single {
        return Some(Found {
            after,
            before,
            words: 1,
        });
    }

    if let Some(day) = explicit(&first) {
        return Some(Found {
            after: start_of(zone, day),
            before: start_of(zone, day.succ_opt()?),
            words: 1,
        });
    }

    // Two-word phrases. The marker word is what makes them unambiguous.
    let second = words.get(1).map(|word| word.to_ascii_lowercase())?;

    match first.as_str() {
        // "in March", "in 2025". The preposition is doing the work: without it, `March` is a
        // name and this module refuses to guess.
        "in" | "during" => {
            if let Some(month) = month_number(&second) {
                return month_range(zone, now, month).map(|found| Found { words: 2, ..found });
            }

            if let Ok(year) = second.parse::<i32>() {
                if (1970..=2200).contains(&year) {
                    return year_range(zone, year).map(|found| Found { words: 2, ..found });
                }
            }

            None
        }

        "last" | "past" => match second.trim_end_matches('s') {
            "week" => Some(Found {
                after: midnight(-7),
                before: midnight(1),
                words: 2,
            }),
            "month" => Some(Found {
                after: midnight(-30),
                before: midnight(1),
                words: 2,
            }),
            "year" => Some(Found {
                after: midnight(-365),
                before: midnight(1),
                words: 2,
            }),
            "day" => Some(Found {
                after: midnight(-1),
                before: midnight(1),
                words: 2,
            }),
            _ => None,
        },

        // "this week" runs from the start of the week to now, unlike "last week" which is a
        // rolling seven days. The difference is what the words mean, and collapsing them into
        // one would make "this week" find mail from before the weekend.
        "this" => match second.as_str() {
            "week" => {
                let since_monday = i64::from(today(zone, now)?.weekday().num_days_from_monday());
                Some(Found {
                    after: midnight(-since_monday),
                    before: midnight(1),
                    words: 2,
                })
            }
            "month" => {
                let moment = today(zone, now)?;
                let first = NaiveDate::from_ymd_opt(moment.year(), moment.month(), 1)?;
                Some(Found {
                    after: start_of(zone, first),
                    before: midnight(1),
                    words: 2,
                })
            }
            "year" => {
                let moment = today(zone, now)?;
                let first = NaiveDate::from_ymd_opt(moment.year(), 1, 1)?;
                Some(Found {
                    after: start_of(zone, first),
                    before: midnight(1),
                    words: 2,
                })
            }
            _ => None,
        },

        _ => None,
    }
}

#[cfg(test)]
#[path = "dates_tests.rs"]
mod tests;
