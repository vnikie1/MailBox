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
fn a_rule_whose_folder_was_deleted_still_applies_its_other_actions() {
    // A rule outlives its folder. Moving into a mailbox that no longer exists used to fail the
    // foreign key, which failed the whole run — so one deleted folder stopped every rule.
    let mut conn = store();
    add_message(&conn, 1, "ada@example.test", "The quarterly figures");

    let tx = conn.transaction().expect("tx");
    run_over(
        &tx,
        &[1],
        &[rule(
            1,
            0,
            Field::From,
            "ada",
            vec![Action::MoveTo(99), Action::MarkRead],
        )],
    )
    .expect("run");

    assert_eq!(mailbox_of(&tx, 1), 1, "the message stays where it is");

    let seen: i64 = tx
        .query_row("SELECT flag_seen FROM message WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("read");
    assert_eq!(seen, 1, "and the rule's other action still happened");
}

#[test]
fn a_rule_never_files_mail_into_another_accounts_folder() {
    // M18. Rules run over every account's mail; a folder belongs to one. The move used to go
    // ahead, filing the row under the other account while its server was asked for a folder it
    // does not have.
    let mut conn = store();
    conn.execute(
        "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
         VALUES (2, 'Other', 'other@halcyon.test', 'other', 'password', 'halcyon:other')",
        [],
    )
    .expect("account");
    conn.execute(
        "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
         VALUES (20, 2, 'Clients', 'Clients', NULL)",
        [],
    )
    .expect("mailbox");
    add_message(&conn, 1, "ada@example.test", "The quarterly figures");

    let tx = conn.transaction().expect("tx");
    run_over(
        &tx,
        &[1],
        &[rule(
            1,
            0,
            Field::From,
            "ada",
            vec![Action::MoveTo(20), Action::MarkRead],
        )],
    )
    .expect("run");

    assert_eq!(
        mailbox_of(&tx, 1),
        1,
        "the message stays in its own account"
    );
    let queued: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM pending_op WHERE kind = 'move'",
            [],
            |row| row.get(0),
        )
        .expect("read");
    assert_eq!(queued, 0, "and no server is asked to move it");

    let seen: i64 = tx
        .query_row("SELECT flag_seen FROM message WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("read");
    assert_eq!(seen, 1, "while the rule's other action still happens");
}

#[test]
fn a_rule_that_marks_mail_read_keeps_the_unread_count_right() {
    // M20. The count the sidebar shows is cached on the mailbox, and the rule changed the
    // message without it, so the count stayed one too high until the next sync.
    let mut conn = store();
    add_message(&conn, 1, "ada@example.test", "The quarterly figures");
    add_message(&conn, 2, "grace@example.test", "Lunch");
    conn.execute("UPDATE mailbox SET unread_count = 2 WHERE id = 1", [])
        .expect("count");

    let tx = conn.transaction().expect("tx");
    run_over(
        &tx,
        &[1, 2],
        &[rule(1, 0, Field::From, "ada", vec![Action::MarkRead])],
    )
    .expect("run");

    let unread: i64 = tx
        .query_row("SELECT unread_count FROM mailbox WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("read");
    assert_eq!(unread, 1);

    run_over(
        &tx,
        &[1],
        &[rule(2, 0, Field::From, "ada", vec![Action::MarkUnread])],
    )
    .expect("run");
    let unread: i64 = tx
        .query_row("SELECT unread_count FROM mailbox WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("read");
    assert_eq!(unread, 2);
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

/* ------------------------------------------- conditions on a body that is not here yet */

/// A rule over the body, the attachment names or "has attachment", with one condition.
fn body_rule(field: Field, op: Op, value: &str) -> Rule {
    Rule {
        id: 1,
        name: "body rule".into(),
        enabled: true,
        predicate: Predicate::Is(Condition {
            field,
            op,
            value: value.into(),
        }),
        actions: vec![Action::Flag],
        sort_order: 0,
    }
}

fn flagged(conn: &Connection, id: i64) -> bool {
    conn.query_row(
        "SELECT flag_flagged FROM message WHERE id = ?1",
        rusqlite::params![id],
        |row| row.get::<_, i64>(0),
    )
    .expect("flag")
        != 0
}

/// H9 in the 2026-11-14 audit: body conditions were judged against a body that was not there.
///
/// Bodies download after the envelope, so on arrival every message had an empty body, no
/// attachment names and no attachment. A rule asking whether the body *contains* something never
/// matched new mail, and one asking whether it does *not* contain something matched all of it.
#[test]
fn a_body_condition_is_not_judged_before_the_body_is_here() {
    let mut conn = store();
    add_message(&conn, 1, "ada@example.test", "Hello");

    for (field, op, value) in [
        (Field::Body, Op::NotContains, "unsubscribe"),
        (Field::AttachmentName, Op::NotContains, ".exe"),
        (Field::HasAttachment, Op::IsFalse, ""),
        (Field::AnyText, Op::NotContains, "lottery"),
    ] {
        let tx = conn.transaction().expect("tx");
        let report = run_over(&tx, &[1], &[body_rule(field, op, value)]).expect("run");
        tx.commit().expect("commit");

        assert_eq!(
            report.matched, 0,
            "{field:?} {op:?} matched a body not yet downloaded"
        );
        assert!(!flagged(&conn, 1));
    }
}

#[test]
fn a_body_condition_is_judged_once_the_body_is_stored() {
    let mut conn = store();
    add_message(&conn, 1, "ada@example.test", "Hello");
    conn.execute(
        "UPDATE message SET body_state = 'full', body_text = 'your invoice is attached'
          WHERE id = 1",
        [],
    )
    .expect("body");

    let tx = conn.transaction().expect("tx");
    let report = run_over(
        &tx,
        &[1],
        &[body_rule(Field::Body, Op::Contains, "invoice")],
    )
    .expect("run");
    tx.commit().expect("commit");

    assert_eq!(report.matched, 1);
    assert!(flagged(&conn, 1));
}

#[test]
fn a_header_condition_still_runs_without_a_body() {
    // The guard is for body conditions only. A rule on the sender must not start waiting for
    // a download it has no use for.
    let mut conn = store();
    add_message(&conn, 1, "ada@example.test", "Hello");

    let tx = conn.transaction().expect("tx");
    let report = run_over(&tx, &[1], &[body_rule(Field::From, Op::Contains, "ada")]).expect("run");
    tx.commit().expect("commit");

    assert_eq!(report.matched, 1);
}

#[test]
fn whether_a_predicate_needs_the_body_is_known_before_running_it() {
    let header = Predicate::Is(Condition {
        field: Field::Subject,
        op: Op::Contains,
        value: "x".into(),
    });
    let nested_body = Predicate::All(vec![
        header.clone(),
        Predicate::Not(Box::new(Predicate::Is(Condition {
            field: Field::HasAttachment,
            op: Op::IsTrue,
            value: String::new(),
        }))),
    ]);

    assert!(!header.needs_body());
    assert!(nested_body.needs_body());
}

/// The kinds of the operations queued for the server, oldest first.
fn queued_kinds(tx: &rusqlite::Transaction<'_>) -> Vec<String> {
    tx.prepare("SELECT kind FROM pending_op ORDER BY id")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
}

#[test]
fn a_rule_over_many_messages_queues_one_flag_change_per_mailbox() {
    // One per message used to be queued, and sent as a STORE each (L4).
    let mut conn = store();
    for id in 1..=5 {
        add_message(&conn, id, "list@example.test", "Digest");
    }

    let tx = conn.transaction().expect("tx");
    run_over(
        &tx,
        &[1, 2, 3, 4, 5],
        &[rule(
            1,
            0,
            Field::From,
            "list@",
            vec![Action::MarkRead, Action::Flag],
        )],
    )
    .expect("run");

    assert_eq!(
        queued_kinds(&tx),
        ["flag", "flag"],
        "read once, flagged once"
    );
}

#[test]
fn a_flag_change_is_queued_before_the_move_that_follows_it() {
    // Queued after the move, it would name a UID the Inbox no longer has.
    let mut conn = store();
    add_message(&conn, 1, "ada@example.test", "Hello");

    let tx = conn.transaction().expect("tx");
    run_over(
        &tx,
        &[1],
        &[rule(
            1,
            0,
            Field::From,
            "ada",
            vec![Action::MarkRead, Action::MoveTo(2)],
        )],
    )
    .expect("run");

    assert_eq!(queued_kinds(&tx), ["flag", "move"]);
}
