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
fn every_mutation_leaves_a_pending_op_for_the_sync_engine() {
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

    let kinds: Vec<String> = conn
        .prepare("SELECT kind FROM pending_op ORDER BY id")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows");

    assert_eq!(kinds, vec!["flag", "move", "expunge"]);
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
fn message_get_returns_none_for_an_id_that_is_not_there() -> Result<(), DbError> {
    let conn = fixture();
    assert!(query::message_get(&conn, 999)?.is_none());
    Ok(())
}
