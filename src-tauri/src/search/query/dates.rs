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

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};

/// A resolved date phrase, and how many words it consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    pub after: Option<i64>,
    pub before: Option<i64>,
    /// How many whitespace-separated words the phrase used, so the caller can skip them.
    pub words: usize,
}

/// Local midnight at the start of the day `offset` days from `now`.
fn midnight(now: i64, offset: i64) -> Option<i64> {
    let moment = Local.timestamp_opt(now, 0).single()?;
    let day = (moment + Duration::days(offset)).date_naive();
    start_of(day)
}

fn start_of(day: NaiveDate) -> Option<i64> {
    let naive = day.and_hms_opt(0, 0, 0)?;
    Local
        .from_local_datetime(&naive)
        .single()
        // A local midnight that does not exist — the hour a DST jump skips — resolves to the
        // moment the day actually begins rather than failing the whole search.
        .or_else(|| Local.from_local_datetime(&naive).earliest())
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
fn month_range(now: i64, month: u32) -> Option<Found> {
    let moment = Local.timestamp_opt(now, 0).single()?;
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
        after: start_of(first),
        before: start_of(next),
        words: 0,
    })
}

/// The whole of one year.
fn year_range(year: i32) -> Option<Found> {
    Some(Found {
        after: start_of(NaiveDate::from_ymd_opt(year, 1, 1)?),
        before: start_of(NaiveDate::from_ymd_opt(year + 1, 1, 1)?),
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
    let lowered = value.trim().to_ascii_lowercase();

    if let Some(day) = explicit(&lowered) {
        return start_of(day);
    }

    match lowered.as_str() {
        "today" => return midnight(now, 0),
        "yesterday" => return midnight(now, -1),
        "tomorrow" => return midnight(now, 1),
        _ => {}
    }

    if let Some(month) = month_number(&lowered) {
        return month_range(now, month)?.after;
    }

    if let Ok(year) = lowered.parse::<i32>() {
        if (1970..=2200).contains(&year) {
            return year_range(year)?.after;
        }
    }

    // "3 days", "2 weeks" — relative to now, looking backwards, which is the only direction a
    // mail search ever means.
    let parts: Vec<&str> = lowered.split_whitespace().collect();
    if parts.len() == 2 {
        if let Ok(count) = parts[0].parse::<i64>() {
            return relative(now, count, parts[1]);
        }
    }

    None
}

/// `n days ago`-style arithmetic, given a unit word.
fn relative(now: i64, count: i64, unit: &str) -> Option<i64> {
    let days = match unit.trim_end_matches('s') {
        "day" => count,
        "week" => count * 7,
        "month" => count * 30,
        "year" => count * 365,
        _ => return None,
    };

    midnight(now, -days)
}

/// Reads a date phrase from the start of `words`, if there is one.
///
/// Returns how many words it used so the caller can skip them. `None` when the first word does
/// not begin a phrase, which is the common case and has to be cheap.
pub fn phrase(words: &[&str], now: i64) -> Option<Found> {
    let first = words.first()?.to_ascii_lowercase();

    // One-word phrases that can only be dates.
    let single = match first.as_str() {
        "today" => Some((midnight(now, 0), midnight(now, 1))),
        "yesterday" => Some((midnight(now, -1), midnight(now, 0))),
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
            after: start_of(day),
            before: start_of(day + Duration::days(1)),
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
                return month_range(now, month).map(|found| Found { words: 2, ..found });
            }

            if let Ok(year) = second.parse::<i32>() {
                if (1970..=2200).contains(&year) {
                    return year_range(year).map(|found| Found { words: 2, ..found });
                }
            }

            None
        }

        "last" | "past" => match second.trim_end_matches('s') {
            "week" => Some(Found {
                after: midnight(now, -7),
                before: midnight(now, 1),
                words: 2,
            }),
            "month" => Some(Found {
                after: midnight(now, -30),
                before: midnight(now, 1),
                words: 2,
            }),
            "year" => Some(Found {
                after: midnight(now, -365),
                before: midnight(now, 1),
                words: 2,
            }),
            "day" => Some(Found {
                after: midnight(now, -1),
                before: midnight(now, 1),
                words: 2,
            }),
            _ => None,
        },

        // "this week" runs from the start of the week to now, unlike "last week" which is a
        // rolling seven days. The difference is what the words mean, and collapsing them into
        // one would make "this week" find mail from before the weekend.
        "this" => match second.as_str() {
            "week" => {
                let moment = Local.timestamp_opt(now, 0).single()?;
                let since_monday = i64::from(moment.weekday().num_days_from_monday());
                Some(Found {
                    after: midnight(now, -since_monday),
                    before: midnight(now, 1),
                    words: 2,
                })
            }
            "month" => {
                let moment = Local.timestamp_opt(now, 0).single()?;
                let first = NaiveDate::from_ymd_opt(moment.year(), moment.month(), 1)?;
                Some(Found {
                    after: start_of(first),
                    before: midnight(now, 1),
                    words: 2,
                })
            }
            "year" => {
                let moment = Local.timestamp_opt(now, 0).single()?;
                let first = NaiveDate::from_ymd_opt(moment.year(), 1, 1)?;
                Some(Found {
                    after: start_of(first),
                    before: midnight(now, 1),
                    words: 2,
                })
            }
            _ => None,
        },

        _ => None,
    }
}
