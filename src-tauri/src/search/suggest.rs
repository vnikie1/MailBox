//! Turning a prefix into typed token candidates. docs/02 §6.6, docs/06 Phase 9.
//!
//! The dropdown under the search field. It runs on every keystroke and has a 30ms budget, which
//! is the whole design constraint: anything that scans the message table is out, so every
//! lookup here either reads a small table or uses an index whose prefix is the thing being
//! typed.
//!
//! ## Groups, not a flat list
//!
//! docs/02 §6.6 asks for headers. They are not decoration: the same three letters can be a
//! person, a mailbox or a word in a subject, and a flat list makes the user read every row to
//! work out which is which. With headers they read one.

use rusqlite::Connection;
use serde::Serialize;
use ts_rs::TS;

use crate::db::DbError;

use super::query::Query;

/// What kind of thing a suggestion is, which decides its header and its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// A `field:` the user could type next.
    Token,
    Person,
    Mailbox,
    /// Search for the text as written.
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub kind: Kind,
    /// What the row reads as.
    pub label: String,
    /// What the search field becomes when it is chosen.
    pub insert: String,
    /// A second line — an address under a name, say.
    pub detail: Option<String>,
}

/// The fields worth offering, with what each one is for.
const TOKENS: &[(&str, &str)] = &[
    ("from:", "From a person"),
    ("to:", "Sent to a person"),
    ("subject:", "In the subject"),
    ("mailbox:", "In a mailbox"),
    ("has:attachment", "With an attachment"),
    ("is:unread", "Not yet read"),
    ("is:flagged", "Flagged"),
    ("before:", "Received before a date"),
    ("after:", "Received after a date"),
    ("larger:", "Bigger than a size"),
];

/// The word being typed — everything after the last space.
///
/// Suggestions are about what the caret is in, not about the whole field. Someone who has typed
/// `invoice fr` wants `from:` offered, not suggestions for `invoice`.
fn active_word(text: &str) -> &str {
    match text.rfind(char::is_whitespace) {
        Some(at) => &text[at + 1..],
        None => text,
    }
}

/// Replaces the active word, keeping everything before it.
fn with_word(text: &str, replacement: &str) -> String {
    match text.rfind(char::is_whitespace) {
        Some(at) => format!("{}{replacement}", &text[..=at]),
        None => replacement.to_string(),
    }
}

/// Candidates for what is being typed.
///
/// `parsed` is passed in rather than parsed here, because the caller already needed it and
/// parsing twice on every keystroke is the kind of waste a 30ms budget cannot afford.
pub fn suggest(
    conn: &Connection,
    text: &str,
    parsed: &Query,
    limit: usize,
) -> Result<Vec<Suggestion>, DbError> {
    let word = active_word(text);
    let lowered = word.to_ascii_lowercase();

    if word.is_empty() {
        return Ok(Vec::new());
    }

    let mut out: Vec<Suggestion> = Vec::new();

    // Searching for exactly what was typed, always first. Every other row is a guess about
    // what the user meant; this one is what they said.
    if !parsed.is_empty() {
        out.push(Suggestion {
            kind: Kind::Text,
            label: text.trim().to_string(),
            insert: text.to_string(),
            detail: None,
        });
    }

    // Field names. A constant list, so this costs nothing.
    for (token, description) in TOKENS {
        if token.starts_with(&lowered) && *token != lowered {
            out.push(Suggestion {
                kind: Kind::Token,
                label: (*token).to_string(),
                insert: with_word(text, token),
                detail: Some((*description).to_string()),
            });
        }
    }

    // Once a field has been typed, suggest values for *that* field rather than in general.
    let (field, value) = match lowered.split_once(':') {
        Some((field, value)) => (Some(field), value.to_string()),
        None => (None, lowered.clone()),
    };

    if value.len() >= 2 && matches!(field, None | Some("from") | Some("to")) {
        // From the contact index rather than the message table. Scanning 100,000 message rows
        // per keystroke is exactly what the budget forbids.
        // `addr`, not `address`. The first version of this used the wrong column name and the
        // error was swallowed by an `if let Ok`, so person suggestions silently never appeared
        // while every test still passed. The `?` is the point: a query that cannot even be
        // prepared is a bug, and hiding it made a broken feature look like an empty one.
        let mut statement = conn.prepare(
            "SELECT name, addr, seen_count
               FROM contact
              WHERE LOWER(addr) LIKE ?1 ESCAPE '\\'
                 OR LOWER(COALESCE(name, '')) LIKE ?1 ESCAPE '\\'
              ORDER BY seen_count DESC
              LIMIT ?2",
        )?;

        let pattern = format!("{}%", escape_like(&value));

        let rows = statement
            .query_map(rusqlite::params![pattern, limit as i64], |row| {
                Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        for (name, address) in rows {
            let prefix = field.unwrap_or("from");
            out.push(Suggestion {
                kind: Kind::Person,
                label: name.clone().unwrap_or_else(|| address.clone()),
                insert: with_word(text, &format!("{prefix}:{address}")),
                detail: Some(address),
            });
        }
    }

    if value.len() >= 2 && matches!(field, None | Some("mailbox") | Some("in")) {
        let mut statement = conn.prepare(
            "SELECT DISTINCT display_name FROM mailbox
              WHERE LOWER(display_name) LIKE ?1 ESCAPE '\\'
              ORDER BY display_name LIMIT ?2",
        )?;

        let pattern = format!("{}%", escape_like(&value));
        let rows = statement
            .query_map(rusqlite::params![pattern, limit as i64], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        for name in rows {
            out.push(Suggestion {
                kind: Kind::Mailbox,
                label: name.clone(),
                insert: with_word(text, &format!("mailbox:{name}")),
                detail: None,
            });
        }
    }

    out.truncate(limit);
    Ok(out)
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::query;

    fn store() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open");
        crate::db::migrate::run(&mut conn).expect("migrate");
        conn
    }

    fn suggestions(conn: &Connection, text: &str) -> Vec<Suggestion> {
        let parsed = query::parse(text, 1_787_000_000);
        suggest(conn, text, &parsed, 8).expect("suggest")
    }

    #[test]
    fn nothing_typed_suggests_nothing() {
        let conn = store();
        assert!(suggestions(&conn, "").is_empty());
    }

    #[test]
    fn a_prefix_offers_the_matching_fields() {
        let conn = store();
        let found = suggestions(&conn, "fr");

        assert!(found
            .iter()
            .any(|s| s.kind == Kind::Token && s.label == "from:"));
    }

    #[test]
    fn searching_for_what_was_typed_comes_first() {
        // Every other row is a guess about what the user meant. This one is what they said.
        let conn = store();
        let found = suggestions(&conn, "invoice");

        assert_eq!(found.first().map(|s| s.kind), Some(Kind::Text));
    }

    #[test]
    fn suggestions_apply_to_the_word_the_caret_is_in() {
        // Someone who has typed "invoice fr" wants `from:` offered, not suggestions for
        // "invoice" — and choosing one must keep the words already typed.
        let conn = store();
        let found = suggestions(&conn, "invoice fr");

        let token = found
            .iter()
            .find(|s| s.kind == Kind::Token)
            .expect("a field suggestion");

        assert_eq!(token.insert, "invoice from:");
    }

    #[test]
    fn a_completed_field_is_not_offered_again() {
        let conn = store();
        let found = suggestions(&conn, "is:unread");

        assert!(!found
            .iter()
            .any(|s| s.kind == Kind::Token && s.label == "is:unread"));
    }

    #[test]
    fn a_wildcard_in_the_typed_text_is_escaped() {
        // `%` is a LIKE wildcard, and someone typing it means the character.
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("a_b"), "a\\_b");
    }

    #[test]
    fn a_person_prefix_suggests_people() {
        // This is the test the first version did not have, which is why a wrong column name
        // went unnoticed: the query failed to prepare, the error was swallowed, and the
        // feature simply produced nothing.
        let conn = store();
        conn.execute(
            "INSERT INTO contact (addr, name, seen_count) VALUES ('ada@example.test', 'Ada Lovelace', 9)",
            [],
        )
        .expect("contact");

        let found = suggestions(&conn, "ada");

        assert!(
            found
                .iter()
                .any(|s| s.kind == Kind::Person && s.label == "Ada Lovelace"),
            "no person was suggested: {found:?}"
        );

        let person = found
            .iter()
            .find(|s| s.kind == Kind::Person)
            .expect("a person");
        assert_eq!(person.insert, "from:ada@example.test");
    }

    #[test]
    fn a_mailbox_prefix_suggests_mailboxes() {
        let conn = store();
        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'T', 'me@t.test', 'other', 'password', 'halcyon:me')",
            [],
        )
        .expect("account");
        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (1, 1, 'Archive', 'Archive', 'archive')",
            [],
        )
        .expect("mailbox");

        let found = suggestions(&conn, "arch");
        assert!(found
            .iter()
            .any(|s| s.kind == Kind::Mailbox && s.label == "Archive"));
    }
}
