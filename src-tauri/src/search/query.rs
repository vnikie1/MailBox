//! The search query language and its parser. docs/01 §7, docs/06 Phase 9.
//!
//! ## Why a parser rather than a search box
//!
//! Everything typed into the search field is user input from a person in a hurry, and every
//! character of it can also be FTS5 syntax. `NEAR`, `AND`, `*`, `"` and `^` all mean something
//! to the match engine, and a search for `AND` that returns a syntax error is a search box that
//! does not work. So nothing typed reaches FTS5 as syntax: free text is quoted term by term,
//! and the structured half is turned into bound parameters against ordinary columns.
//!
//! ## The shape
//!
//! `field:value`, with free text between. Everything combines with AND, which is what people
//! expect and what Mail does — a search that broadened as you typed would be unusable.
//!
//! | token | matches |
//! |---|---|
//! | `from:ada` | the sender |
//! | `to:ada` | any recipient, To or Cc |
//! | `subject:figures` | the subject alone |
//! | `mailbox:archive` | the mailbox name |
//! | `has:attachment` | messages with a file |
//! | `is:unread` `is:read` `is:flagged` `is:junk` | state |
//! | `before:2026-08-01` `after:yesterday` | received date |
//! | `larger:5MB` `smaller:100KB` | size |
//!
//! An unrecognised `field:` is **not** an error and is not silently dropped: it becomes free
//! text. Someone searching for `http://example.com` or `re: figures` has typed a colon without
//! meaning a field, and a query language that refuses those is worse than no query language.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod dates;

/// A parsed search.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Query {
    /// Words with no field, matched across everything indexed.
    pub terms: Vec<String>,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub subject: Vec<String>,
    pub mailbox: Vec<String>,
    pub has_attachment: Option<bool>,
    pub is_unread: Option<bool>,
    pub is_flagged: Option<bool>,
    pub is_junk: Option<bool>,
    /// Received strictly before this, seconds since the epoch.
    #[ts(type = "number | null")]
    pub before: Option<i64>,
    /// Received at or after this.
    #[ts(type = "number | null")]
    pub after: Option<i64>,
    #[ts(type = "number | null")]
    pub larger_than: Option<i64>,
    #[ts(type = "number | null")]
    pub smaller_than: Option<i64>,
}

impl Query {
    /// True when nothing was typed that could match anything.
    pub fn is_empty(&self) -> bool {
        *self == Query::default()
    }

    /// True when the query has no free text — only structured constraints.
    ///
    /// Worth knowing because the two take different paths: free text goes through FTS5, and a
    /// query without any cannot use the index at all and has to be a plain scan with filters.
    pub fn is_structured_only(&self) -> bool {
        self.terms.is_empty() && !self.is_empty()
    }
}

/// One `field:value` pair or bare word, as the splitter found it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Piece {
    field: Option<String>,
    value: String,
    /// True when the value arrived in quotes, so `subject:"the quarterly figures"` stays whole.
    quoted: bool,
}

/// Splits input into pieces, honouring quotes.
///
/// Hand-written rather than a regex because of one case: a quote can open *after* the colon
/// (`subject:"two words"`) or around the whole thing (`"two words"`), and the two mean the same
/// to a user and different things to most patterns.
fn split(input: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut field: Option<String> = None;
    let mut current = String::new();
    let mut in_quotes = false;
    let mut was_quoted = false;

    let flush = |pieces: &mut Vec<Piece>,
                 field: &mut Option<String>,
                 current: &mut String,
                 was_quoted: &mut bool| {
        if !current.trim().is_empty() || *was_quoted {
            pieces.push(Piece {
                field: field.take(),
                value: current.trim().to_string(),
                quoted: *was_quoted,
            });
        }
        current.clear();
        *field = None;
        *was_quoted = false;
    };

    for character in input.chars() {
        match character {
            '"' => {
                in_quotes = !in_quotes;
                was_quoted = true;
            }

            // Only the *first* colon of a piece separates a field from its value, and only
            // when something precedes it. `http://x` keeps its colons: the second one has a
            // field already claimed, and `//x` is just more value.
            ':' if !in_quotes && field.is_none() && !current.trim().is_empty() => {
                field = Some(current.trim().to_ascii_lowercase());
                current.clear();
            }

            character if character.is_whitespace() && !in_quotes => {
                flush(&mut pieces, &mut field, &mut current, &mut was_quoted);
            }

            character => current.push(character),
        }
    }

    flush(&mut pieces, &mut field, &mut current, &mut was_quoted);
    pieces
}

/// Reads a size like `5MB`, `100kb`, `2000`. Bare numbers are bytes.
fn parse_size(value: &str) -> Option<i64> {
    let trimmed = value.trim().to_ascii_lowercase();
    let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();

    if digits.is_empty() {
        return None;
    }

    let number: i64 = digits.parse().ok()?;
    let unit = trimmed[digits.len()..].trim();

    // Powers of 1024, because that is what every mail client means by MB when it shows an
    // attachment size, and a search that disagreed with the size beside it would look broken.
    let multiplier = match unit {
        "" | "b" => 1,
        "k" | "kb" => 1024,
        "m" | "mb" => 1024 * 1024,
        "g" | "gb" => 1024 * 1024 * 1024,
        _ => return None,
    };

    number.checked_mul(multiplier)
}

/// Parses a search string.
///
/// `now` is passed in rather than read, so "yesterday" is testable and so the whole parse is a
/// pure function of its inputs.
pub fn parse(input: &str, now: i64) -> Query {
    let mut query = Query::default();
    let pieces = split(input);

    let mut index = 0;
    while index < pieces.len() {
        let piece = &pieces[index];

        // A date phrase in free text — "last week", "in March" — becomes a date bound. Tried
        // before anything else, and only on unfielded words, so `subject:yesterday` still
        // searches for the word.
        if piece.field.is_none() && !piece.quoted {
            let rest: Vec<&str> = pieces[index..]
                .iter()
                .filter(|p| p.field.is_none() && !p.quoted)
                .map(|p| p.value.as_str())
                .collect();

            if let Some(found) = dates::phrase(&rest, now) {
                if found.after.is_some() {
                    query.after = found.after;
                }
                if found.before.is_some() {
                    query.before = found.before;
                }
                index += found.words;
                continue;
            }
        }

        match piece.field.as_deref() {
            Some("from") => query.from.push(piece.value.clone()),
            Some("to") => query.to.push(piece.value.clone()),
            Some("subject") => query.subject.push(piece.value.clone()),
            Some("mailbox") | Some("in") => query.mailbox.push(piece.value.clone()),

            Some("has") => match piece.value.to_ascii_lowercase().as_str() {
                "attachment" | "attachments" | "file" => query.has_attachment = Some(true),
                // Unrecognised: free text rather than silently ignored, so the user sees their
                // words searched for instead of a filter they did not ask for.
                _ => query.terms.push(format!("has:{}", piece.value)),
            },

            Some("is") => match piece.value.to_ascii_lowercase().as_str() {
                "unread" | "new" => query.is_unread = Some(true),
                "read" | "seen" => query.is_unread = Some(false),
                "flagged" | "starred" => query.is_flagged = Some(true),
                "unflagged" => query.is_flagged = Some(false),
                "junk" | "spam" => query.is_junk = Some(true),
                _ => query.terms.push(format!("is:{}", piece.value)),
            },

            Some("before") => match dates::boundary(&piece.value, now) {
                Some(at) => query.before = Some(at),
                None => query.terms.push(piece.value.clone()),
            },

            Some("after") | Some("since") => match dates::boundary(&piece.value, now) {
                Some(at) => query.after = Some(at),
                None => query.terms.push(piece.value.clone()),
            },

            Some("larger") | Some("bigger") => match parse_size(&piece.value) {
                Some(bytes) => query.larger_than = Some(bytes),
                None => query.terms.push(piece.value.clone()),
            },

            Some("smaller") => match parse_size(&piece.value) {
                Some(bytes) => query.smaller_than = Some(bytes),
                None => query.terms.push(piece.value.clone()),
            },

            // An unrecognised field is not an error. `re: figures` and `http://example.com`
            // are things people type, and rejecting them would be worse than no parser.
            Some(other) => query.terms.push(format!("{other}:{}", piece.value)),

            None => {
                if !piece.value.is_empty() {
                    query.terms.push(piece.value.clone());
                }
            }
        }

        index += 1;
    }

    query
}

#[cfg(test)]
mod tests;
