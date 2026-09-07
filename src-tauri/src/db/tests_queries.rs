//! Query and mutation behaviour, against a real database.
//!
//! These build a small store rather than a hundred thousand messages — correctness does not
//! need scale, and a test suite that takes a minute stops being run. The scale claims are
//! measured by the `seed` binary, which prints its numbers; these prove the queries return
//! the right rows and the counts stay honest.

use rusqlite::Connection;

use super::model::{Cursor, FlagPatch, ListQuery, SearchQuery};
use super::{migrate, query, write, DbError};

/// A store with one account, three mailboxes and a handful of messages.
///
/// Received times are deliberately shared between some messages: timestamp collisions are
/// the case keyset pagination gets wrong when the cursor ignores the id, and they are
/// common in real mail because a sync commits a batch with one clock reading.
fn fixture() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open");
    conn.pragma_update(None, "foreign_keys", "ON").expect("fk");
    migrate::run(&mut conn).expect("migrate");

    conn.execute(
        "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
         VALUES (1, 'Work', 'me@example.test', 'imap', 'password', 'ref')",
        [],
    )
    .expect("account");

    for (id, name, role) in [
        (1, "Inbox", "inbox"),
        (2, "Archive", "archive"),
        (3, "Bin", "trash"),
    ] {
        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (?1, 1, ?2, ?2, ?3)",
            (id, name, role),
        )
        .expect("mailbox");
    }

    // The thread row has to exist before any message points at it — foreign keys are on,
    // and the first version of this fixture referenced thread 7 without creating it.
    conn.execute(
        "INSERT INTO thread (id, account_id, subject_base, message_count) VALUES (7, 1, 'figures', 6)",
        [],
    )
    .expect("thread");

    // (id, mailbox, date_received, seen, subject, body)
    let messages: &[(i64, i64, i64, bool, &str, &str)] = &[
        (
            1,
            1,
            500,
            false,
            "Quarterly figures",
            "the numbers are attached",
        ),
        (
            2,
            1,
            400,
            true,
            "Lunch on Thursday",
            "shall we try the new place",
        ),
        (
            3,
            1,
            400,
            false,
            "Re: Quarterly figures",
            "one correction on page four",
        ),
        (
            4,
            1,
            300,
            true,
            "Invoice 4471",
            "payment is due in seven days",
        ),
        (
            5,
            1,
            200,
            false,
            "Site visit notes",
            "the warehouse audit found nothing",
        ),
        (6, 2, 450, true, "Archived thing", "nothing to see"),
    ];

    for (id, mailbox, date, seen, subject, body) in messages {
        conn.execute(
            "INSERT INTO message
               (id, account_id, mailbox_id, uid, thread_id, subject, from_name, from_addr,
                date_sent, date_received, size, preview, flag_seen, body_text,
                from_all, to_all, attachment_names)
             VALUES (?1, 1, ?2, ?1, 7, ?3, 'Ada', 'ada@example.test', ?4, ?4, 1000, ?5, ?6, ?5,
                     'Ada ada@example.test', 'me@example.test', '')",
            (id, mailbox, subject, date, body, i64::from(*seen)),
        )
        .expect("message");
    }

    let tx = conn.transaction().expect("tx");
    write::recount_mailboxes(&tx, &[1, 2, 3]).expect("recount");
    tx.commit().expect("commit");

    conn
}

fn page(conn: &Connection, cursor: Option<Cursor>, limit: u32) -> Vec<i64> {
    let query = ListQuery {
        mailbox_ids: vec![1],
        cursor,
        limit,
        unread_only: false,
    };

    query::messages_page(conn, &query)
        .expect("page")
        .items
        .into_iter()
        .map(|row| row.id)
        .collect()
}

#[test]
fn the_list_is_newest_first_with_ties_broken_by_id() {
    let conn = fixture();
    assert_eq!(page(&conn, None, 10), vec![1, 3, 2, 4, 5]);
}

#[test]
fn paging_walks_every_row_exactly_once_across_a_timestamp_collision() {
    // Messages 2 and 3 share date_received = 400. A cursor on the date alone would either
    // repeat one of them or skip it, depending on which side of the comparison it fell.
    let conn = fixture();

    let mut seen = Vec::new();
    let mut cursor = None;

    loop {
        let query = ListQuery {
            mailbox_ids: vec![1],
            cursor,
            limit: 2,
            unread_only: false,
        };
        let result = query::messages_page(&conn, &query).expect("page");

        seen.extend(result.items.iter().map(|row| row.id));
        match result.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    assert_eq!(seen, vec![1, 3, 2, 4, 5], "every row, in order, once");
}

#[test]
fn the_cursor_is_none_once_the_end_is_reached() {
    let conn = fixture();

    let exactly_all = ListQuery {
        mailbox_ids: vec![1],
        cursor: None,
        limit: 5,
        unread_only: false,
    };
    let result = query::messages_page(&conn, &exactly_all).expect("page");

    assert_eq!(result.items.len(), 5);
    assert!(
        result.next_cursor.is_none(),
        "a full page that happens to be the last one must not promise another"
    );
}

#[test]
fn a_unified_row_merges_its_mailboxes() {
    let conn = fixture();

    let query = ListQuery {
        mailbox_ids: vec![1, 2],
        cursor: None,
        limit: 10,
        unread_only: false,
    };
    let ids: Vec<i64> = query::messages_page(&conn, &query)
        .expect("page")
        .items
        .into_iter()
        .map(|row| row.id)
        .collect();

    // 6 is in Archive at 450, so it sorts between 1 (500) and the pair at 400.
    assert_eq!(ids, vec![1, 6, 3, 2, 4, 5]);
}

#[test]
fn unread_only_filters_without_disturbing_the_order() {
    let conn = fixture();

    let query = ListQuery {
        mailbox_ids: vec![1],
        cursor: None,
        limit: 10,
        unread_only: true,
    };
    let ids: Vec<i64> = query::messages_page(&conn, &query)
        .expect("page")
        .items
        .into_iter()
        .map(|row| row.id)
        .collect();

    assert_eq!(ids, vec![1, 3, 5]);
}

#[test]
fn flags_move_the_cached_unread_count_by_the_right_amount() {
    let mut conn = fixture();

    let unread = |conn: &Connection| -> i64 {
        conn.query_row("SELECT unread_count FROM mailbox WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("count")
    };

    assert_eq!(unread(&conn), 3);

    // Two unread and one already-read message marked read: the count must fall by two,
    // not by three. A naive delta on the number of ids gets this wrong.
    let tx = conn.transaction().expect("tx");
    write::set_flags(
        &tx,
        &[1, 3, 4],
        FlagPatch {
            seen: Some(true),
            flagged: None,
        },
    )
    .expect("flags");
    tx.commit().expect("commit");

    assert_eq!(unread(&conn), 1);

    let tx = conn.transaction().expect("tx");
    write::set_flags(
        &tx,
        &[1],
        FlagPatch {
            seen: Some(false),
            flagged: None,
        },
    )
    .expect("flags");
    tx.commit().expect("commit");

    assert_eq!(unread(&conn), 2, "marking unread puts the count back up");
}

#[test]
fn the_incremental_counts_agree_with_a_full_recount() {
    // The whole risk of maintaining counts by delta is drift. This does both and compares.
    let mut conn = fixture();

    let tx = conn.transaction().expect("tx");
    write::set_flags(
        &tx,
        &[1, 2],
        FlagPatch {
            seen: Some(true),
            flagged: None,
        },
    )
    .expect("flags");
    write::move_to(&tx, &[4, 5], 2).expect("move");
    write::delete(&tx, &[6], true, None).expect("delete");
    tx.commit().expect("commit");

    let incremental: Vec<(i64, i64, i64)> = conn
        .prepare("SELECT id, unread_count, total_count FROM mailbox ORDER BY id")
        .expect("prepare")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows");

    let tx = conn.transaction().expect("tx");
    write::recount_mailboxes(&tx, &[1, 2, 3]).expect("recount");
    tx.commit().expect("commit");

    let recounted: Vec<(i64, i64, i64)> = conn
        .prepare("SELECT id, unread_count, total_count FROM mailbox ORDER BY id")
        .expect("prepare")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows");

    assert_eq!(
        incremental, recounted,
        "deltas must not drift from the truth"
    );
}

#[test]
fn moving_adjusts_both_ends() {
    let mut conn = fixture();

    let tx = conn.transaction().expect("tx");
    write::move_to(&tx, &[1], 2).expect("move");
    tx.commit().expect("commit");

    let counts = |conn: &Connection, id: i64| -> (i64, i64) {
        conn.query_row(
            "SELECT unread_count, total_count FROM mailbox WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("counts")
    };

    assert_eq!(counts(&conn, 1), (2, 4), "source loses an unread message");
    assert_eq!(counts(&conn, 2), (1, 2), "destination gains it");
}

#[test]
fn a_non_permanent_delete_moves_to_trash_rather_than_destroying() {
    let mut conn = fixture();

    let tx = conn.transaction().expect("tx");
    write::delete(&tx, &[1], false, Some(3)).expect("delete");
    tx.commit().expect("commit");

    let mailbox: i64 = conn
        .query_row("SELECT mailbox_id FROM message WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("still there");

    assert_eq!(mailbox, 3, "the message is in Trash, not gone");
}

#[test]
fn a_delete_with_nowhere_to_put_it_refuses_rather_than_improvising() {
    let mut conn = fixture();

    let tx = conn.transaction().expect("tx");
    let changed = write::delete(&tx, &[1], false, None).expect("delete");
    tx.commit().expect("commit");

    assert_eq!(changed, 0);
    let exists: i64 = conn
        .query_row("SELECT COUNT(*) FROM message WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("count");
    assert_eq!(
        exists, 1,
        "mail the user expected to be recoverable is still there"
    );
}

#[test]
fn a_local_write_queues_nothing_by_itself() {
    // This used to be `every_mutation_leaves_a_pending_op_for_the_sync_engine`, and it asserted
    // that `set_flags`, `move_to` and `delete` each left a row in `pending_op` with kinds
    // "flag", "move" and "expunge".
    //
    // The rows were real and the sync engine could not read one of them: their payloads carry no
    // `kind` field, `ops::Op` is internally tagged, and the drain deletes what it cannot parse.
    // The test checked the `kind` *column* and never the payload, so it passed for months while
    // asserting a property — "for the sync engine" — that was false.
    //
    // Queuing is the caller's job now, because only the caller knows the mailbox path and UIDs.
    // What this checks is that the local write no longer invents rows nobody can use.
    let mut conn = fixture();

    let tx = conn.transaction().expect("tx");
    write::set_flags(
        &tx,
        &[1],
        FlagPatch {
            seen: Some(true),
            flagged: None,
        },
    )
    .expect("flags");
    write::move_to(&tx, &[2], 2).expect("move");
    write::delete(&tx, &[5], true, None).expect("delete");
    tx.commit().expect("commit");

    let queued: i64 = conn
        .query_row("SELECT COUNT(*) FROM pending_op", [], |row| row.get(0))
        .expect("count");

    assert_eq!(
        queued, 0,
        "a local write queued {queued} operation(s) of its own; the drain cannot parse them and          deletes them, which is how the rules engine came to file mail locally and nowhere else"
    );
}

#[test]
fn search_matches_subject_and_body_and_respects_the_mailbox_filter() {
    let conn = fixture();

    let hits = |text: &str, mailboxes: Vec<i64>| -> Vec<i64> {
        query::search(
            &conn,
            &SearchQuery {
                text: text.into(),
                mailbox_ids: mailboxes,
                limit: 20,
            },
        )
        .expect("search")
        .into_iter()
        .map(|row| row.id)
        .collect()
    };

    let mut subject_hits = hits("quarterly", Vec::new());
    subject_hits.sort_unstable();
    assert_eq!(subject_hits, vec![1, 3], "matches the subject");

    assert_eq!(hits("warehouse", Vec::new()), vec![5], "matches the body");

    // Prefix matching, so results appear while the user is still typing.
    let mut partial = hits("quar", Vec::new());
    partial.sort_unstable();
    assert_eq!(partial, vec![1, 3]);

    assert!(
        hits("archived", vec![1]).is_empty(),
        "mailbox filter applies"
    );
    assert_eq!(hits("archived", vec![2]), vec![6]);
}

#[test]
fn search_treats_operator_characters_as_text_rather_than_syntax() {
    // FTS5's query language is expressive enough that unescaped user input can error or
    // mean something unintended. A search box must never do either.
    let conn = fixture();

    for text in ["\"unbalanced", "NEAR(", "figures OR", "*", "^quarterly"] {
        let result = query::search(
            &conn,
            &SearchQuery {
                text: text.into(),
                mailbox_ids: Vec::new(),
                limit: 5,
            },
        );
        assert!(result.is_ok(), "search should not error on {text:?}");
    }
}

#[test]
fn the_hot_queries_use_their_indexes() {
    // A plan that degrades to a scan still returns correct rows, so nothing else in this
    // file would notice. It would just miss the budget on a real mailbox.
    let conn = fixture();

    let plan = query::explain(
        &conn,
        "SELECT id FROM message WHERE mailbox_id IN (?1)
           AND (date_received, id) < (?2, ?3)
         ORDER BY date_received DESC, id DESC LIMIT ?4",
    )
    .expect("explain");
    assert!(
        plan.contains("ix_msg_list"),
        "messages_page must use ix_msg_list, got:\n{plan}"
    );
    assert!(
        !plan.contains("TEMP B-TREE"),
        "the index must satisfy the ORDER BY, got:\n{plan}"
    );

    let unread = query::explain(
        &conn,
        "SELECT COUNT(*) FROM message WHERE mailbox_id = ?1 AND flag_seen = 0",
    )
    .expect("explain");
    assert!(
        unread.contains("ix_msg_unread"),
        "the unread count must use the partial index, got:\n{unread}"
    );
}

#[test]
fn an_empty_mailbox_set_returns_nothing_rather_than_everything() {
    // The dangerous failure: an empty IN list compiled to no WHERE clause would return the
    // entire store, and the caller would page through a hundred thousand rows.
    let conn = fixture();

    let query = ListQuery {
        mailbox_ids: Vec::new(),
        cursor: None,
        limit: 10,
        unread_only: false,
    };
    let result = query::messages_page(&conn, &query).expect("page");

    assert!(result.items.is_empty());
    assert!(result.next_cursor.is_none());
}

#[test]
fn a_thread_comes_back_oldest_first() {
    let conn = fixture();
    let ids: Vec<i64> = query::thread_get(&conn, 7)
        .expect("thread")
        .into_iter()
        .map(|message| message.id)
        .collect();

    // Oldest first is the order the reader stacks them in — docs/01 §4. Message 6 is at
    // 450 and message 1 at 500, so 6 comes first; the ids are not in id order and should
    // not be expected to be.
    assert_eq!(ids, vec![5, 4, 2, 3, 6, 1]);
}

#[test]
fn any_message_in_a_thread_opens_the_whole_thread() {
    // The reader has a message id, never a thread id. It was passing that message id into a
    // query that filters on `thread_id`, and `rethread` keys a thread on the smallest message
    // id in it -- so the two numbers agreed for exactly one message per conversation. Opening
    // any other message showed a conversation of one, which looks like threading not working.
    //
    // Every message in this fixture is in thread 7, and no message has id 7, so a caller that
    // still treats the argument as a thread id gets one message back and fails here.
    let conn = fixture();

    for message_id in 1..=6 {
        let ids: Vec<i64> = query::thread_for_message(&conn, message_id)
            .expect("thread")
            .into_iter()
            .map(|message| message.id)
            .collect();

        assert_eq!(
            ids,
            vec![5, 4, 2, 3, 6, 1],
            "opening message {message_id} did not show the whole conversation"
        );
    }
}

#[test]
fn a_message_with_no_thread_still_opens() {
    // Threading is the sync engine's job, so a store can hold messages it has never run over.
    // Standing rule 13: degrade visibly. One message is better than an empty reader.
    let conn = fixture();
    conn.execute("UPDATE message SET thread_id = NULL WHERE id = 3", [])
        .expect("unthread");

    let ids: Vec<i64> = query::thread_for_message(&conn, 3)
        .expect("thread")
        .into_iter()
        .map(|message| message.id)
        .collect();

    assert_eq!(ids, vec![3]);
}

#[test]
fn a_message_that_does_not_exist_is_an_empty_thread() {
    let conn = fixture();
    assert!(query::thread_for_message(&conn, 999)
        .expect("thread")
        .is_empty());
}

#[test]
fn message_get_returns_none_for_an_id_that_is_not_there() -> Result<(), DbError> {
    let conn = fixture();
    assert!(query::message_get(&conn, 999)?.is_none());
    Ok(())
}

/// Remind Me hides a message from the list until it comes due. docs/01 §8.
///
/// Worth a test of its own because the filter is a comparison between an integer column and
/// whatever the clock expression returns, and SQLite does not compare across storage classes the
/// way arithmetic would: an INTEGER is *always* less than a TEXT value, whatever the digits say.
/// A filter written the obvious way is therefore true for every row and hides nothing, and the
/// symptom is not an error — it is Remind Me quietly doing nothing at all.
#[test]
fn a_snoozed_message_is_hidden_until_it_comes_due() {
    let conn = fixture();

    let far_future = i64::MAX / 2;
    conn.execute(
        "UPDATE message SET snooze_until = ?1 WHERE id = 1",
        [far_future],
    )
    .expect("snooze");

    let visible = page(&conn, None, 10);
    assert!(
        !visible.contains(&1),
        "a message snoozed until the far future is still in the list: {visible:?}"
    );

    // And one whose reminder has passed is back, rather than hidden forever.
    conn.execute("UPDATE message SET snooze_until = 1 WHERE id = 1", [])
        .expect("unsnooze");

    assert!(
        page(&conn, None, 10).contains(&1),
        "a message whose reminder has passed did not come back"
    );
}

/// The page cache is set deliberately, not left to SQLite's default. docs/06 Phase 5's soak.
///
/// Asserted because the value is a *budget*: 8MB per connection times the pool is the app's
/// memory ceiling for the store, and the twelve-hour soak's +21.2% growth was the old default
/// filling up by accident. A change here changes what that soak measures, and it should be a
/// decision rather than a drift.
#[test]
fn the_page_cache_is_an_explicit_budget() {
    // A connection put through `configure`, not the bare fixture — `configure` is the thing
    // being asserted, and the fixture deliberately does not call it.
    let conn = Connection::open_in_memory().expect("open");
    super::configure(&conn).expect("configure");

    let cache: i64 = conn
        .query_row("PRAGMA cache_size", [], |row| row.get(0))
        .expect("cache_size");

    assert_eq!(
        cache, -8192,
        "the page cache is no longer the 8MB per connection the soak was measured against"
    );
}

#[test]
fn a_reply_goes_to_the_reply_to_address_when_there_is_one() {
    // The whole point of Reply-To, and the reason docs/06 Phase 7 names it: a mailing list, a
    // ticketing system or a no-reply sender puts the address that should receive replies here,
    // and answering the From address instead sends the message to the wrong place — often to a
    // mailbox nobody reads.
    //
    // `reply::recipients` has always preferred Reply-To over From, and has a test proving it.
    // It never fired in the running app: `persist` did not store the column and `reply_source`
    // did not select it, so the envelope reaching that code always had an empty list. Three
    // correct pieces and no connection between them.
    let conn = fixture();

    conn.execute(
        "UPDATE message
            SET from_name = 'A Person', from_addr = 'person@example.test',
                reply_to_json = ?1
          WHERE id = 1",
        rusqlite::params![r#"[{"name":"The List","email":"list@example.test"}]"#],
    )
    .expect("set reply-to");

    let source = query::reply_source(&conn, 1).expect("source");

    assert_eq!(
        source
            .envelope
            .reply_to
            .iter()
            .map(|address| address.email.as_str())
            .collect::<Vec<_>>(),
        vec!["list@example.test"],
        "reply_source dropped Reply-To, so a reply would go to the From address"
    );

    let recipients =
        crate::mail::reply::recipients(&source.envelope, crate::mail::reply::Kind::Reply, &[]);

    assert_eq!(
        recipients
            .to
            .iter()
            .map(|address| address.email.as_str())
            .collect::<Vec<_>>(),
        vec!["list@example.test"],
        "the reply was addressed to the sender rather than to Reply-To"
    );
}

/// Moving a message into a mailbox that already holds its UID.
///
/// `message` has `UNIQUE(mailbox_id, uid)`, and a move used to carry the source UID across --
/// so this failed the whole transaction. Deleting is a move to Trash, and on the real account
/// this was found on, 23 of 267 Inbox messages shared a UID with something already in the Bin.
/// **Delete did not work for any of them**, and the mutation hook swallowed the error, so
/// nothing was said and the message stayed exactly where it was.
#[test]
fn a_move_into_a_mailbox_already_holding_that_uid_succeeds() {
    let mut conn = fixture();

    // The same UID in two mailboxes, which is entirely normal: a UID belongs to one mailbox.
    for (id, mailbox) in [(100, 1), (101, 2)] {
        conn.execute(
            "INSERT INTO message
               (id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
                from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged, is_junk)
             VALUES (?1, 1, ?2, 4242, 'S', 0, 0, 10, 'a@b.test', '', '', 0, 0, 0, 0)",
            (id, mailbox),
        )
        .expect("message");
    }

    let tx = conn.transaction().expect("tx");
    write::move_to(&tx, &[100], 2).expect("the move must not collide");

    let (mailbox, uid): (i64, i64) = tx
        .query_row(
            "SELECT mailbox_id, uid FROM message WHERE id = 100",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read");

    assert_eq!(mailbox, 2, "the message did not move");
    assert!(
        uid <= 0,
        "a moved message kept a UID belonging to the mailbox it left: {uid}"
    );

    // And the message that was already there is untouched.
    let other: i64 = tx
        .query_row("SELECT uid FROM message WHERE id = 101", [], |row| {
            row.get(0)
        })
        .expect("read");
    assert_eq!(other, 4242);
}

#[test]
fn several_messages_move_at_once_without_colliding_with_each_other() {
    // The placeholder has to be unique per row, or a multi-selection delete would collide
    // with itself rather than with whatever was already in the Bin.
    let mut conn = fixture();
    let tx = conn.transaction().expect("tx");

    write::move_to(&tx, &[1, 2, 3], 2).expect("move");

    let placeholders: i64 = tx
        .query_row(
            "SELECT COUNT(DISTINCT uid) FROM message WHERE id IN (1, 2, 3)",
            [],
            |row| row.get(0),
        )
        .expect("count");

    assert_eq!(placeholders, 3, "the placeholder UIDs were not unique");
}

/* --------------------------------------- moving a message again before the first move syncs */

/// Queues and performs a move the way `msg_move` does: locate, enqueue, then write.
fn queue_move(tx: &rusqlite::Transaction<'_>, ids: &[i64], to_id: i64, to_path: &str) {
    for group in crate::sync::ops::locate(tx, ids).expect("locate") {
        crate::sync::ops::enqueue(
            tx,
            group.account_id,
            &crate::sync::ops::Op::Move {
                from: group.mailbox,
                to: to_path.to_string(),
                uids: group.uids,
            },
        )
        .expect("enqueue");
    }

    write::move_to(tx, ids, to_id).expect("move");
}

/// Every queued move, as `(from, to, uids)`, oldest first.
fn queued_moves(tx: &rusqlite::Transaction<'_>) -> Vec<(String, String, Vec<u32>)> {
    let payloads: Vec<String> = {
        let mut statement = tx
            .prepare("SELECT payload_json FROM pending_op WHERE kind = 'move' ORDER BY id")
            .expect("prepare");

        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query");

        rows.collect::<rusqlite::Result<Vec<_>>>().expect("rows")
    };

    payloads
        .into_iter()
        .map(|payload| {
            match serde_json::from_str::<crate::sync::ops::Op>(&payload).expect("parse") {
                crate::sync::ops::Op::Move { from, to, uids } => (from, to, uids),
                _ => panic!("a row with kind 'move' held something else"),
            }
        })
        .collect()
}

/// Moving a message twice used to tell the server about the first move only.
///
/// A locally moved row parks at a negative UID, and `locate` skips those — so the second move
/// queued nothing while the first sat waiting. The server performed the first move, the UID
/// changed, and the returning message no longer matched the parked row (which had moved on),
/// so it was inserted beside it: one message, two rows, permanently, because `remove_missing`
/// ignores `uid <= 0`.
#[test]
fn a_second_move_replaces_the_first_instead_of_leaving_both_queued() {
    let mut conn = fixture();
    let tx = conn.transaction().expect("tx");

    queue_move(&tx, &[1], 2, "Archive");
    queue_move(&tx, &[1], 3, "Bin");

    let moves = queued_moves(&tx);
    assert_eq!(
        moves.len(),
        1,
        "two moves for one message cannot both be right: the first changes the UID the second names"
    );

    let (from, to, uids) = &moves[0];
    assert_eq!(
        from, "Inbox",
        "the server still has the message where it started, not where it has been moved to locally"
    );
    assert_eq!(
        to, "Bin",
        "the surviving move must be the one asked for last"
    );
    assert_eq!(
        uids,
        &vec![1_u32],
        "the original server UID, not the placeholder"
    );
}

/// Superseding must take out one message, not the batch it happened to be queued with.
#[test]
fn superseding_leaves_the_other_messages_in_the_batch_alone() {
    let mut conn = fixture();
    let tx = conn.transaction().expect("tx");

    queue_move(&tx, &[1, 2, 3], 2, "Archive");
    queue_move(&tx, &[2], 3, "Bin");

    let moves = queued_moves(&tx);
    assert_eq!(moves.len(), 2, "the batch and the message taken out of it");

    let (_, to_first, uids_first) = &moves[0];
    assert_eq!(to_first, "Archive");
    assert_eq!(
        uids_first,
        &vec![1_u32, 3],
        "message 2 left the batch; 1 and 3 are still going to Archive"
    );

    let (from_second, to_second, uids_second) = &moves[1];
    assert_eq!(from_second, "Inbox");
    assert_eq!(to_second, "Bin");
    assert_eq!(uids_second, &vec![2_u32]);
}

/// The origin is where the *server* has it, which does not change as the row moves locally.
#[test]
fn the_origin_survives_a_second_move_and_goes_when_the_uid_comes_back() {
    let mut conn = fixture();
    let tx = conn.transaction().expect("tx");

    queue_move(&tx, &[1], 2, "Archive");
    queue_move(&tx, &[1], 3, "Bin");

    let (mailbox, uid): (i64, i64) = tx
        .query_row(
            "SELECT origin_mailbox_id, origin_uid FROM message WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read");

    assert_eq!(
        mailbox, 1,
        "the origin is the Inbox, not the mailbox it passed through"
    );
    assert_eq!(uid, 1, "the origin UID is the one the server still has");
}

/// The silent half of the same bug: any command against a parked message queued nothing.
#[test]
fn a_parked_message_can_still_be_flagged_on_the_server() {
    let mut conn = fixture();
    let tx = conn.transaction().expect("tx");

    queue_move(&tx, &[1], 2, "Archive");

    let groups = crate::sync::ops::locate(&tx, &[1]).expect("locate");
    assert_eq!(
        groups.len(),
        1,
        "a message with an unfinished move is still somewhere on the server, and skipping it \
         silently dropped every flag and delete aimed at it"
    );
    assert_eq!(groups[0].mailbox, "Inbox");
    assert_eq!(groups[0].uids, vec![1_u32]);
}
