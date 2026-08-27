//! Tests for the predicate engine, and one property that is the reason it exists.
//!
//! docs/06 Phase 8 says there must be **one** predicate engine shared by Rules and Smart
//! Mailboxes, and property-tested. The property is the point: a predicate compiled to SQL and
//! the same predicate evaluated in memory must agree about every message, always.
//!
//! If they can disagree, the symptom is a rule that files a message which the matching smart
//! mailbox then does not show — a bug nobody can describe, because each half looks correct on
//! its own and neither is obviously the wrong one. A property test is the only way to find
//! that, because the disagreements live in the corners: an empty needle, a `%` the user typed
//! literally, a capital letter, a negation over four columns.

use proptest::prelude::*;
use rusqlite::Connection;

use super::predicate::{Condition, Field, Op, Predicate, Subject, Value};

/// A store holding exactly one message, built from a `Subject`.
///
/// The columns are filled the way the sync engine fills them, so the SQL sees what it would see
/// in production rather than a shape invented for the test.
fn store_with(subject: &Subject<'_>) -> Connection {
    let mut conn = Connection::open_in_memory().expect("open");
    crate::db::migrate::run(&mut conn).expect("migrate");

    conn.execute(
        "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
         VALUES (1, 'Test', 'me@halcyon.test', 'other', 'password', 'halcyon:me')",
        [],
    )
    .expect("account");

    conn.execute(
        "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
         VALUES (1, 1, 'INBOX', ?1, 'inbox')",
        [subject.mailbox],
    )
    .expect("mailbox");

    conn.execute(
        "INSERT INTO message (
             id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
             from_all, to_all, cc_json, body_text, attachment_names,
             has_attachment, flag_seen, flag_flagged, is_junk
         ) VALUES (1, 1, 1, 1, ?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            subject.subject,
            subject.date_received,
            subject.size,
            subject.from,
            subject.to,
            subject.cc,
            subject.body,
            subject.attachment_names,
            i64::from(subject.has_attachment),
            // Stored as "seen", asked as "unread".
            i64::from(!subject.is_unread),
            i64::from(subject.is_flagged),
            i64::from(subject.is_junk),
        ],
    )
    .expect("message");

    conn
}

/// Whether SQL says the one stored message matches.
fn sql_matches(conn: &Connection, predicate: &Predicate) -> bool {
    let compiled = predicate.compile();

    let sql = format!(
        "SELECT COUNT(*) FROM message
           JOIN mailbox ON mailbox.id = message.mailbox_id
          WHERE {}",
        compiled.sql
    );

    let params: Vec<&dyn rusqlite::ToSql> = compiled
        .params
        .iter()
        .map(|value| value as &dyn rusqlite::ToSql)
        .collect();

    let count: i64 = conn
        .query_row(&sql, params.as_slice(), |row| row.get(0))
        .unwrap_or_else(|error| panic!("{sql}\n{error}"));

    count > 0
}

fn condition(field: Field, op: Op, value: &str) -> Predicate {
    Predicate::Is(Condition {
        field,
        op,
        value: value.to_string(),
    })
}

fn sample<'a>() -> Subject<'a> {
    Subject {
        from: "Ada Lovelace <ada@example.test>",
        to: "me@halcyon.test, Grace <grace@example.test>",
        cc: r#"[{"name":"Charles","email":"charles@example.test"}]"#,
        subject: "The quarterly figures",
        body: "Attached are the numbers for Q3. Margin is up 100% on last year.",
        mailbox: "Inbox",
        attachment_names: "figures.pdf",
        date_received: 1_700_000_000,
        size: 4096,
        has_attachment: true,
        is_unread: true,
        is_flagged: false,
        is_junk: false,
    }
}

/* -------------------------------------------------------------------- the ordinary cases */

#[test]
fn a_text_condition_agrees_with_itself_in_both_engines() {
    let subject = sample();
    let conn = store_with(&subject);

    for predicate in [
        condition(Field::From, Op::Contains, "ada"),
        condition(Field::From, Op::Contains, "nobody"),
        condition(Field::Subject, Op::BeginsWith, "The"),
        condition(Field::Subject, Op::EndsWith, "figures"),
        condition(Field::Body, Op::Contains, "margin"),
        condition(Field::AnyText, Op::Contains, "grace"),
        condition(Field::AttachmentName, Op::Contains, ".pdf"),
        condition(Field::Mailbox, Op::Is, "Inbox"),
    ] {
        assert_eq!(
            sql_matches(&conn, &predicate),
            predicate.matches(&subject),
            "{predicate:?}"
        );
    }
}

#[test]
fn matching_ignores_case_in_both_engines() {
    // SQLite's LIKE is case-insensitive for ASCII, and the in-memory side has to be too. A
    // mismatch here would make the two disagree on every capital letter a user typed.
    let subject = sample();
    let conn = store_with(&subject);

    for needle in ["ADA", "ada", "Ada", "QUARTERLY"] {
        let predicate = condition(Field::AnyText, Op::Contains, needle);
        assert_eq!(
            sql_matches(&conn, &predicate),
            predicate.matches(&subject),
            "{needle}"
        );
        assert!(predicate.matches(&subject), "{needle} should match");
    }
}

#[test]
fn a_literal_percent_sign_is_not_a_wildcard() {
    // The body contains "100%". Without escaping, `LIKE '%100%%'` still matches — but a search
    // for "50%" would match too, because `%` means "anything". The user typed a percent sign.
    let subject = sample();
    let conn = store_with(&subject);

    let matching = condition(Field::Body, Op::Contains, "100%");
    assert!(matching.matches(&subject));
    assert!(sql_matches(&conn, &matching));

    let not_matching = condition(Field::Body, Op::Contains, "50%");
    assert!(!not_matching.matches(&subject));
    assert!(
        !sql_matches(&conn, &not_matching),
        "an unescaped % matched anything"
    );
}

#[test]
fn an_underscore_is_not_a_single_character_wildcard() {
    let subject = sample();
    let conn = store_with(&subject);

    // `_` in LIKE matches any one character, so "Q_" would match "Q3" if unescaped.
    let predicate = condition(Field::Body, Op::Contains, "Q_");
    assert!(!predicate.matches(&subject));
    assert!(!sql_matches(&conn, &predicate), "an unescaped _ matched");
}

#[test]
fn unread_is_the_opposite_of_the_stored_column() {
    // The column is `flag_seen`, because that is what the protocol calls it; the predicate is
    // "is unread", because that is what the user calls it. The inversion is easy to get right
    // once and wrong in the other engine.
    let mut subject = sample();
    let conn = store_with(&subject);

    let unread = condition(Field::IsUnread, Op::IsTrue, "");
    assert!(unread.matches(&subject));
    assert!(sql_matches(&conn, &unread));

    subject.is_unread = false;
    let conn = store_with(&subject);
    assert!(!unread.matches(&subject));
    assert!(!sql_matches(&conn, &unread));
}

#[test]
fn a_negation_covers_every_column_it_reads() {
    // "Not from ada" must mean *not anywhere in the from field*. Distributing the NOT over the
    // ORed columns instead would produce "not in from, or in one of the others", which is true
    // of almost every message — a filter that silently matches everything.
    let subject = sample();
    let conn = store_with(&subject);

    let predicate = condition(Field::AnyText, Op::NotContains, "ada");
    assert!(!predicate.matches(&subject));
    assert!(!sql_matches(&conn, &predicate));
}

#[test]
fn an_empty_group_is_true_for_all_and_false_for_any() {
    // Not an error, and it must not compile to an empty string — `WHERE ()` is a syntax error.
    // A smart mailbox with no conditions yet should show everything rather than fail.
    let subject = sample();
    let conn = store_with(&subject);

    let all = Predicate::All(Vec::new());
    assert!(all.matches(&subject));
    assert!(sql_matches(&conn, &all));

    let any = Predicate::Any(Vec::new());
    assert!(!any.matches(&subject));
    assert!(!sql_matches(&conn, &any));
}

#[test]
fn the_gates_five_predicate_smart_mailbox_agrees_with_hand_written_sql() {
    // docs/04's exit gate asks for a five-predicate smart mailbox verified against a
    // hand-written query. This is that query, written independently of the compiler.
    let subject = sample();
    let conn = store_with(&subject);

    let predicate = Predicate::All(vec![
        condition(Field::From, Op::Contains, "example.test"),
        condition(Field::Subject, Op::Contains, "quarterly"),
        condition(Field::HasAttachment, Op::IsTrue, ""),
        condition(Field::IsUnread, Op::IsTrue, ""),
        condition(Field::Size, Op::GreaterThan, "1024"),
    ]);

    assert_eq!(predicate.condition_count(), 5);

    let by_hand: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM message
               JOIN mailbox ON mailbox.id = message.mailbox_id
              WHERE message.from_all LIKE '%example.test%'
                AND message.subject LIKE '%quarterly%'
                AND message.has_attachment = 1
                AND message.flag_seen = 0
                AND message.size > 1024",
            [],
            |row| row.get(0),
        )
        .expect("hand-written");

    assert_eq!(
        by_hand, 1,
        "the fixture should satisfy the hand-written form"
    );
    assert!(sql_matches(&conn, &predicate));
    assert!(predicate.matches(&subject));
}

#[test]
fn no_value_is_ever_spliced_into_the_statement() {
    // The one module that assembles SQL rather than writing it out, so the one place the
    // parameterisation rule could be lost. A value that would end the statement must appear as
    // a bound parameter and never in the text.
    let hostile = "'; DROP TABLE message; --";
    let predicate = condition(Field::Subject, Op::Contains, hostile);
    let compiled = predicate.compile();

    assert!(
        !compiled.sql.contains("DROP"),
        "a value reached the statement: {}",
        compiled.sql
    );
    assert!(compiled.params.iter().any(|value| match value {
        Value::Text(text) => text.contains("DROP"),
        Value::Number(_) => false,
    }));

    // And it really runs, matching nothing.
    let subject = sample();
    let conn = store_with(&subject);
    assert!(!sql_matches(&conn, &predicate));
}

/* ------------------------------------------------------------------------- the property */

fn any_field() -> impl Strategy<Value = Field> {
    prop_oneof![
        Just(Field::From),
        Just(Field::To),
        Just(Field::Subject),
        Just(Field::Body),
        Just(Field::AnyText),
        Just(Field::Mailbox),
        Just(Field::AttachmentName),
        Just(Field::DateReceived),
        Just(Field::Size),
        Just(Field::HasAttachment),
        Just(Field::IsUnread),
        Just(Field::IsFlagged),
        Just(Field::IsJunk),
    ]
}

fn any_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::Contains),
        Just(Op::NotContains),
        Just(Op::Is),
        Just(Op::IsNot),
        Just(Op::BeginsWith),
        Just(Op::EndsWith),
        Just(Op::GreaterThan),
        Just(Op::LessThan),
        Just(Op::IsTrue),
        Just(Op::IsFalse),
    ]
}

/// Values drawn from the alphabet the corners live in: the words in the fixture, wildcards the
/// user might type literally, an empty string, and mixed case.
fn any_value() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("ada".to_string()),
        Just("ADA".to_string()),
        Just("example.test".to_string()),
        Just("quarterly".to_string()),
        Just("%".to_string()),
        Just("_".to_string()),
        Just("100%".to_string()),
        Just("Inbox".to_string()),
        Just("4096".to_string()),
        Just("1024".to_string()),
        Just("nothing at all".to_string()),
    ]
}

fn any_predicate() -> impl Strategy<Value = Predicate> {
    let leaf = (any_field(), any_op(), any_value())
        .prop_map(|(field, op, value)| Predicate::Is(Condition { field, op, value }));

    // Three levels is enough to exercise nesting and negation without generating trees that
    // take longer to build than to test.
    leaf.prop_recursive(3, 12, 3, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..3).prop_map(Predicate::All),
            prop::collection::vec(inner.clone(), 0..3).prop_map(Predicate::Any),
            inner.prop_map(|one| Predicate::Not(Box::new(one))),
        ]
    })
}

proptest! {
    /// **The property the whole module exists for.**
    ///
    /// Compiled to SQL and evaluated in memory, a predicate must reach the same verdict about
    /// the same message. Every disagreement is a rule and a smart mailbox that would quietly
    /// contradict each other.
    #[test]
    fn sql_and_memory_always_agree(predicate in any_predicate()) {
        let subject = sample();
        let conn = store_with(&subject);

        prop_assert_eq!(
            sql_matches(&conn, &predicate),
            predicate.matches(&subject),
            "{:?}",
            predicate
        );
    }
}
