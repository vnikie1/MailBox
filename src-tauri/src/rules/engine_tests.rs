//! Tests for rule evaluation. docs/06 Phase 8.
//!
//! The predicate half is property-tested next door; this covers what happens *after* a match,
//! where the subtleties are about order and about what a rule is allowed to do.

use rusqlite::Connection;

use super::engine::{
    is_flag_colour, rule_save, rules_list, run_over, smart_delete, smart_list, smart_save, Action,
    Rule,
};
use super::predicate::{Condition, Field, Op, Predicate};

fn store() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("pragma");
    crate::db::migrate::run(&mut conn).expect("migrate");

    conn.execute(
        "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
         VALUES (1, 'Test', 'me@halcyon.test', 'other', 'password', 'halcyon:me')",
        [],
    )
    .expect("account");

    for (id, path, name, role) in [
        (1, "INBOX", "Inbox", "inbox"),
        (2, "Archive", "Archive", "archive"),
        (3, "Trash", "Trash", "trash"),
    ] {
        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (?1, 1, ?2, ?3, ?4)",
            rusqlite::params![id, path, name, role],
        )
        .expect("mailbox");
    }

    conn
}

fn add_message(conn: &Connection, id: i64, from: &str, subject: &str) {
    conn.execute(
        "INSERT INTO message (
             id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
             from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged, is_junk
         ) VALUES (?1, 1, 1, ?1, ?2, 0, 0, 100, ?3, '', '', 0, 0, 0, 0)",
        rusqlite::params![id, subject, from],
    )
    .expect("message");
}

fn rule(id: i64, order: i64, field: Field, value: &str, actions: Vec<Action>) -> Rule {
    Rule {
        id,
        name: format!("rule {id}"),
        enabled: true,
        predicate: Predicate::Is(Condition {
            field,
            op: Op::Contains,
            value: value.to_string(),
        }),
        actions,
        sort_order: order,
    }
}

fn mailbox_of(conn: &Connection, message_id: i64) -> i64 {
    conn.query_row(
        "SELECT mailbox_id FROM message WHERE id = ?1",
        rusqlite::params![message_id],
        |row| row.get(0),
    )
    .expect("mailbox")
}

#[test]
fn a_matching_rule_applies_its_actions() {
    let mut conn = store();
    add_message(&conn, 1, "ada@example.test", "The quarterly figures");

    let tx = conn.transaction().expect("tx");
    let report = run_over(
        &tx,
        &[1],
        &[rule(
            1,
            0,
            Field::From,
            "ada",
            vec![Action::MarkRead, Action::Flag],
        )],
    )
    .expect("run");

    assert_eq!(report.matched, 1);
    assert_eq!(report.actions_applied, 2);

    let (seen, flagged): (i64, i64) = tx
        .query_row(
            "SELECT flag_seen, flag_flagged FROM message WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read");

    assert_eq!(seen, 1);
    assert_eq!(flagged, 1);
}

#[test]
fn a_rule_that_does_not_match_changes_nothing() {
    let mut conn = store();
    add_message(&conn, 1, "ada@example.test", "The quarterly figures");

    let tx = conn.transaction().expect("tx");
    let report = run_over(
        &tx,
        &[1],
        &[rule(1, 0, Field::From, "grace", vec![Action::MarkRead])],
    )
    .expect("run");

    assert_eq!(report.matched, 0);
    assert_eq!(report.actions_applied, 0);
}

#[test]
fn a_disabled_rule_is_skipped() {
    let mut conn = store();
    add_message(&conn, 1, "ada@example.test", "Subject");

    let mut disabled = rule(1, 0, Field::From, "ada", vec![Action::MarkRead]);
    disabled.enabled = false;

    let tx = conn.transaction().expect("tx");
    let report = run_over(&tx, &[1], &[disabled]).expect("run");

    assert_eq!(report.matched, 0);
}

#[test]
fn stop_evaluating_prevents_later_rules_from_running() {
    // The action exists so a user can say "this one, and never mind the rest". A rule that
    // ignored it would apply changes the user explicitly ruled out.
    let mut conn = store();
    add_message(&conn, 1, "ada@example.test", "Subject");

    let tx = conn.transaction().expect("tx");
    run_over(
        &tx,
        &[1],
        &[
            rule(
                1,
                0,
                Field::From,
                "ada",
                vec![Action::MarkRead, Action::StopEvaluating],
            ),
            rule(2, 1, Field::From, "ada", vec![Action::Flag]),
        ],
    )
    .expect("run");

    let (seen, flagged): (i64, i64) = tx
        .query_row(
            "SELECT flag_seen, flag_flagged FROM message WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read");

    assert_eq!(seen, 1, "the first rule should have run");
    assert_eq!(flagged, 0, "the second rule should not have");
}

#[test]
fn a_later_rule_sees_what_an_earlier_one_did() {
    // Rules that depend on each other in order are how people actually build them: "file it,
    // then flag everything in that folder". Evaluating them all against the original state
    // would quietly break that, and the user would blame the second rule.
    let mut conn = store();
    add_message(&conn, 1, "ada@example.test", "Subject");

    let tx = conn.transaction().expect("tx");
    run_over(
        &tx,
        &[1],
        &[
            rule(1, 0, Field::From, "ada", vec![Action::MoveTo(2)]),
            // Matches on the *new* mailbox, which only holds if the second rule re-reads.
            rule(2, 1, Field::Mailbox, "Archive", vec![Action::Flag]),
        ],
    )
    .expect("run");

    assert_eq!(mailbox_of(&tx, 1), 2);

    let flagged: i64 = tx
        .query_row("SELECT flag_flagged FROM message WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("read");

    assert_eq!(flagged, 1, "the second rule did not see the move");
}

#[test]
fn delete_moves_to_trash_rather_than_destroying_anything() {
    // A rule that destroys mail outright, on a predicate written in thirty seconds, is not
    // something anyone recovers from.
    let mut conn = store();
    add_message(&conn, 1, "spam@example.test", "Buy now");

    let tx = conn.transaction().expect("tx");
    run_over(
        &tx,
        &[1],
        &[rule(1, 0, Field::From, "spam", vec![Action::Delete])],
    )
    .expect("run");

    let still_there: i64 = tx
        .query_row("SELECT COUNT(*) FROM message WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("count");

    assert_eq!(still_there, 1, "the message was destroyed");
    assert_eq!(mailbox_of(&tx, 1), 3, "it should be in Trash");
}

#[test]
fn only_the_seven_named_colours_are_accepted() {
    // The value ends up naming a CSS custom property. An unrecognised one is a token that does
    // not resolve, which draws as an invisible flag rather than an error.
    for colour in ["red", "orange", "yellow", "green", "blue", "purple", "gray"] {
        assert!(is_flag_colour(colour), "{colour}");
    }

    for rejected in ["", "chartreuse", "#ff0000", "var(--accent)", "RED"] {
        assert!(!is_flag_colour(rejected), "{rejected}");
    }
}

#[test]
fn an_unrecognised_colour_leaves_the_message_alone() {
    let mut conn = store();
    add_message(&conn, 1, "ada@example.test", "Subject");

    let tx = conn.transaction().expect("tx");
    run_over(
        &tx,
        &[1],
        &[rule(
            1,
            0,
            Field::From,
            "ada",
            vec![Action::SetColour("chartreuse".into())],
        )],
    )
    .expect("run");

    let colour: Option<String> = tx
        .query_row("SELECT flag_color FROM message WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("read");

    assert_eq!(colour, None);
}

#[test]
fn a_rule_whose_json_will_not_parse_matches_nothing_rather_than_everything() {
    // Written by another version, or corrupted. Doing *something* on a predicate that could not
    // be read is far worse than doing nothing, and an inert rule is visible in the editor.
    let mut conn = store();
    conn.execute(
        "INSERT INTO rule (id, name, enabled, match_all, predicate_json, actions_json, sort_order)
         VALUES (1, 'broken', 1, 1, 'not json at all', '[]', 0)",
        [],
    )
    .expect("insert");

    let rules = rules_list(&conn).expect("list");
    assert_eq!(rules.len(), 1);

    add_message(&conn, 1, "ada@example.test", "Subject");
    let tx = conn.transaction().expect("tx");
    let report = run_over(&tx, &[1], &rules).expect("run");

    assert_eq!(report.matched, 0);
}

#[test]
fn a_smart_mailbox_round_trips_through_storage() {
    let mut conn = store();
    let predicate = Predicate::All(vec![Predicate::Is(Condition {
        field: Field::IsFlagged,
        op: Op::IsTrue,
        value: String::new(),
    })]);

    let tx = conn.transaction().expect("tx");
    let id = smart_save(&tx, None, "Flagged", Some("flag"), &predicate).expect("save");
    tx.commit().expect("commit");

    let saved = smart_list(&conn).expect("list");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "Flagged");
    assert_eq!(
        saved[0].predicate, predicate,
        "the predicate changed in storage"
    );

    let tx = conn.transaction().expect("tx");
    smart_delete(&tx, id).expect("delete");
    tx.commit().expect("commit");

    assert!(smart_list(&conn).expect("list").is_empty());
}

#[test]
fn a_rule_round_trips_through_storage() {
    let mut conn = store();
    let predicate = Predicate::Is(Condition {
        field: Field::Subject,
        op: Op::Contains,
        value: "invoice".into(),
    });
    let actions = vec![Action::MoveTo(2), Action::MarkRead];

    let tx = conn.transaction().expect("tx");
    rule_save(&tx, None, "Invoices", true, &predicate, &actions).expect("save");
    tx.commit().expect("commit");

    let saved = rules_list(&conn).expect("list");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].predicate, predicate);
    assert_eq!(saved[0].actions, actions, "the actions changed in storage");
    assert!(saved[0].enabled);
}

/// What a rule tells the *server*, which for a long time was nothing at all.
///
/// A rule that files mail used to change only the local database. The message moved here and
/// stayed in the Inbox on every other device, and the next sync of the destination could delete
/// the local row outright — the server had never been told it belonged there.
///
/// It looked wired up because `db::write::move_to` does enqueue a row. That row's payload has no
/// `kind` field, `ops::Op` is internally tagged, and the drain deletes what it cannot parse. So
/// these tests assert the queued operation **round-trips through `ops::queued`**, which is the
/// step that used to throw the work away.
mod server_side {
    use super::*;
    use crate::sync::ops;

    #[test]
    fn a_rule_that_moves_a_message_tells_the_server() {
        let mut conn = store();
        add_message(&conn, 1, "news@example.test", "Weekly");

        let tx = conn.transaction().expect("tx");
        let rules = vec![rule(
            1,
            0,
            Field::From,
            "news@example.test",
            vec![Action::MoveTo(2)],
        )];
        run_over(&tx, &[1], &rules).expect("run");

        let queued = ops::queued(&tx, 1).expect("queued");

        let moved = queued.iter().any(|(_, op)| {
            matches!(op, ops::Op::Move { from, to, uids }
                if from == "INBOX" && to == "Archive" && uids == &[1])
        });

        assert!(
            moved,
            "a rule moved a message and queued no usable Move for the server: {queued:?}"
        );
    }

    #[test]
    fn a_rule_that_flags_a_message_tells_the_server() {
        let mut conn = store();
        add_message(&conn, 1, "news@example.test", "Weekly");

        let tx = conn.transaction().expect("tx");
        let rules = vec![rule(
            1,
            0,
            Field::From,
            "news@example.test",
            vec![Action::Flag],
        )];
        run_over(&tx, &[1], &rules).expect("run");

        let queued = ops::queued(&tx, 1).expect("queued");

        assert!(
            queued.iter().any(|(_, op)| {
                matches!(op, ops::Op::Flag { flagged: Some(true), uids, .. } if uids == &[1])
            }),
            "a rule flagged a message and queued no usable Flag: {queued:?}"
        );
    }

    #[test]
    fn a_rule_that_deletes_a_message_moves_it_on_the_server_too() {
        let mut conn = store();
        add_message(&conn, 1, "spam@example.test", "Buy");

        let tx = conn.transaction().expect("tx");
        let rules = vec![rule(
            1,
            0,
            Field::From,
            "spam@example.test",
            vec![Action::Delete],
        )];
        run_over(&tx, &[1], &rules).expect("run");

        let queued = ops::queued(&tx, 1).expect("queued");

        assert!(
            queued
                .iter()
                .any(|(_, op)| matches!(op, ops::Op::Move { to, .. } if to == "Trash")),
            "a rule deleted a message and the server was never told: {queued:?}"
        );
    }

    #[test]
    fn the_queue_is_read_the_way_the_drain_reads_it() {
        // The guard on the guard. `ops::queued` is the function that silently deleted the old
        // rows; if these tests read `pending_op` directly instead, they would pass against
        // exactly the payload that was broken.
        let mut conn = store();
        add_message(&conn, 1, "news@example.test", "Weekly");

        let tx = conn.transaction().expect("tx");
        let rules = vec![rule(
            1,
            0,
            Field::From,
            "news@example.test",
            vec![Action::MoveTo(2)],
        )];
        run_over(&tx, &[1], &rules).expect("run");

        let rows: i64 = tx
            .query_row("SELECT COUNT(*) FROM pending_op", [], |row| row.get(0))
            .expect("count");
        let parsed = ops::queued(&tx, 1).expect("queued").len() as i64;

        assert!(rows > 0, "nothing was queued at all");
        assert_eq!(
            parsed,
            rows,
            "{} of {rows} queued rows could not be parsed and would be dropped by the drain",
            rows - parsed
        );
    }
}
