//! Writing fetched messages into the local store, and threading them.
//!
//! Everything here runs inside one transaction on the writer actor, because a half-written
//! batch is worse than no batch: the list would show messages whose thread rows do not exist,
//! and the cached mailbox counts would disagree with the rows they count.
//!
//! The upsert key is `(mailbox_id, uid)`, which is the schema's own uniqueness constraint.
//! That is what makes a re-run of an interrupted sync idempotent — the exit gate's *killing
//! the network mid-sync recovers without duplicates* is this one decision.

use rusqlite::{params, Transaction};

use crate::db::DbError;

use super::fetch::Fetched;
use super::threading::{thread_messages, Threadable};

/// What one batch did, for progress reporting and for the caller's logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Written {
    pub inserted: usize,
    pub updated: usize,
    /// The lowest UID in the batch, so backfill knows where to continue from.
    pub lowest_uid: u32,
}

/// Inserts or updates a batch of fetched messages.
///
/// Idempotent by `(mailbox_id, uid)`: running the same batch twice writes the same rows and
/// changes no counts. That is the property the "kill the network mid-sync" gate tests.
pub fn write_batch(
    tx: &Transaction<'_>,
    account_id: i64,
    mailbox_id: i64,
    fetched: &[Fetched],
) -> Result<Written, DbError> {
    if fetched.is_empty() {
        return Ok(Written::default());
    }

    let mut written = Written {
        lowest_uid: u32::MAX,
        ..Written::default()
    };

    for message in fetched {
        written.lowest_uid = written.lowest_uid.min(message.uid);

        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM message WHERE mailbox_id = ?1 AND uid = ?2",
                params![mailbox_id, message.uid],
                |row| row.get(0),
            )
            .ok();

        let envelope = &message.envelope;
        let references = message.references.join(" ");

        // INTERNALDATE for ordering, falling back to the Date header only when the server
        // gave none. The header is the *sender's* clock: sorting by it puts a message from a
        // machine with a wrong date permanently at the top of the mailbox.
        let date_received = if message.internal_date > 0 {
            message.internal_date
        } else {
            envelope.date_sent
        };

        if let Some(id) = existing {
            // A message already here: only its mutable parts can have changed. Rewriting the
            // envelope would churn the FTS index for nothing on every incremental sync.
            tx.execute(
                "UPDATE message
                    SET flag_seen = ?2, flag_answered = ?3, flag_flagged = ?4,
                        flag_draft = ?5, flag_deleted = ?6
                  WHERE id = ?1",
                params![
                    id,
                    i64::from(message.flags.seen),
                    i64::from(message.flags.answered),
                    i64::from(message.flags.flagged),
                    i64::from(message.flags.draft),
                    i64::from(message.flags.deleted),
                ],
            )?;

            written.updated += 1;
            continue;
        }

        tx.execute(
            "INSERT INTO message (
                 account_id, mailbox_id, uid, message_id, in_reply_to, references_, gm_msgid,
                 subject, subject_base, from_name, from_addr, to_json, cc_json,
                 date_sent, date_received, size,
                 flag_seen, flag_answered, flag_flagged, flag_draft, flag_deleted,
                 has_attachment, body_state, from_all, to_all
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                 ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, 0, 'none', ?22, ?23
             )",
            params![
                account_id,
                mailbox_id,
                message.uid,
                envelope.message_id,
                envelope.in_reply_to,
                if references.is_empty() {
                    None
                } else {
                    Some(references)
                },
                message.gm_msgid,
                envelope.subject,
                envelope.subject_base,
                envelope.from_name(),
                envelope.from_addr(),
                addresses_json(&envelope.to),
                addresses_json(&envelope.cc),
                envelope.date_sent,
                date_received,
                message.size as i64,
                i64::from(message.flags.seen),
                i64::from(message.flags.answered),
                i64::from(message.flags.flagged),
                i64::from(message.flags.draft),
                i64::from(message.flags.deleted),
                envelope.from_all(),
                envelope.to_all(),
            ],
        )?;

        written.inserted += 1;
    }

    if written.lowest_uid == u32::MAX {
        written.lowest_uid = 0;
    }

    Ok(written)
}

/// Serialises an address list for the `to_json` / `cc_json` columns.
fn addresses_json(addresses: &[super::envelope::Address]) -> String {
    let entries: Vec<serde_json::Value> = addresses
        .iter()
        .map(|address| serde_json::json!({ "name": address.name, "email": address.email }))
        .collect();

    serde_json::Value::Array(entries).to_string()
}

/// Recomputes threads for an account and writes `thread_id` onto every message.
///
/// Runs over the whole account rather than the batch just written, because threading is not
/// local: a message arriving today can bridge two conversations from last year, and docs/03
/// §5 requires that merge. Bounded by `limit` so a first sync of a very large mailbox does
/// not hold the writer for a minute.
pub fn rethread(tx: &Transaction<'_>, account_id: i64, limit: usize) -> Result<usize, DbError> {
    let mut statement = tx.prepare(
        "SELECT id, message_id, in_reply_to, references_, subject, date_received, gm_msgid
           FROM message
          WHERE account_id = ?1
          ORDER BY date_received DESC
          LIMIT ?2",
    )?;

    let rows = statement.query_map(params![account_id, limit as i64], |row| {
        let references: Option<String> = row.get(3)?;
        let gm_msgid: Option<String> = row.get(6)?;

        Ok(Threadable {
            id: row.get(0)?,
            message_id: row.get(1)?,
            in_reply_to: row.get(2)?,
            references: references
                .map(|joined| {
                    joined
                        .split_whitespace()
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            subject: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            date: row.get(5)?,
            // Gmail's message id doubles as the thread key here only when the server also
            // sent a thread id; the fetch stores that separately. Parsed leniently.
            gm_thrid: gm_msgid.and_then(|id| id.parse::<i64>().ok()),
        })
    })?;

    let messages: Vec<Threadable> = rows.collect::<rusqlite::Result<Vec<_>>>()?;

    if messages.is_empty() {
        return Ok(0);
    }

    let assignments = thread_messages(&messages);

    // One `thread` row per distinct key, created before any message points at it — the
    // foreign key on `message.thread_id` refuses the other order, which is the schema
    // catching a mistake rather than allowing a dangling reference.
    let mut thread_ids: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();

    for assignment in &assignments {
        if thread_ids.contains_key(&assignment.thread_key) {
            continue;
        }

        let subject_base: Option<String> = tx
            .query_row(
                "SELECT subject_base FROM message WHERE id = ?1",
                params![assignment.thread_key],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        // Keyed on the smallest message id in the thread, so re-running produces the same
        // row rather than a new one every sync.
        tx.execute(
            "INSERT INTO thread (id, account_id, subject_base, last_date, message_count)
             VALUES (?1, ?2, ?3, 0, 0)
             ON CONFLICT(id) DO UPDATE SET subject_base = excluded.subject_base",
            params![assignment.thread_key, account_id, subject_base],
        )?;

        thread_ids.insert(assignment.thread_key, assignment.thread_key);
    }

    for assignment in &assignments {
        tx.execute(
            "UPDATE message SET thread_id = ?2 WHERE id = ?1",
            params![assignment.message_id, assignment.thread_key],
        )?;
    }

    // Roll the per-thread aggregates up in one statement rather than per message.
    tx.execute(
        "UPDATE thread
            SET last_date = COALESCE((
                    SELECT MAX(date_received) FROM message WHERE message.thread_id = thread.id
                ), 0),
                message_count = (
                    SELECT COUNT(*) FROM message WHERE message.thread_id = thread.id
                ),
                unread_count = (
                    SELECT COUNT(*) FROM message
                     WHERE message.thread_id = thread.id AND message.flag_seen = 0
                )
          WHERE account_id = ?1",
        params![account_id],
    )?;

    Ok(assignments.len())
}

/// Recomputes the cached badge counts for a mailbox.
///
/// `write_batch` deliberately does not do this itself: a backfill writes a batch every few
/// hundred milliseconds and recounting a 50,000-message mailbox each time would dominate the
/// sync. It is called once per batch by the engine, which knows when a batch has landed.
///
/// Leaving it out entirely is what the first real sync did, and the symptom was stark: the
/// mail was all there, and the sidebar and the list header both said zero. The rows are the
/// truth; these columns are a cache, and a cache nobody refreshes is just a wrong number.
pub fn recount(tx: &Transaction<'_>, mailbox_id: i64) -> Result<(), DbError> {
    tx.execute(
        "UPDATE mailbox
            SET total_count = (
                    SELECT COUNT(*) FROM message WHERE message.mailbox_id = mailbox.id
                ),
                unread_count = (
                    SELECT COUNT(*) FROM message
                     WHERE message.mailbox_id = mailbox.id AND message.flag_seen = 0
                )
          WHERE id = ?1",
        params![mailbox_id],
    )?;

    Ok(())
}

/// Records where a mailbox has got to, so the next sync resumes instead of restarting.
pub fn record_mailbox_state(
    tx: &Transaction<'_>,
    mailbox_id: i64,
    uid_validity: u32,
    uid_next: u32,
    highest_modseq: Option<u64>,
) -> Result<(), DbError> {
    tx.execute(
        "UPDATE mailbox
            SET uid_validity = ?2, uid_next = ?3, highest_modseq = COALESCE(?4, highest_modseq)
          WHERE id = ?1",
        params![
            mailbox_id,
            uid_validity,
            uid_next,
            highest_modseq.map(|value| value as i64),
        ],
    )?;

    Ok(())
}

/// Drops every message in a mailbox. docs/03 §5's `UIDVALIDITY` recovery.
///
/// *Drop and re-sync that mailbox. Do not try to be clever.* Row by row rather than by a
/// bulk delete on the parent so the FTS5 triggers fire — a cascade would leave the removed
/// messages searchable with nothing behind them, which Phase 4 hit in the account path.
pub fn drop_mailbox_contents(tx: &Transaction<'_>, mailbox_id: i64) -> Result<usize, DbError> {
    let removed = tx.execute(
        "DELETE FROM message WHERE mailbox_id = ?1",
        params![mailbox_id],
    )?;

    tx.execute(
        "UPDATE mailbox
            SET uid_validity = NULL, uid_next = NULL, highest_modseq = NULL,
                unread_count = 0, total_count = 0
          WHERE id = ?1",
        params![mailbox_id],
    )?;

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate;
    use crate::sync::envelope::{Address, Envelope};
    use crate::sync::fetch::Flags;

    fn store() -> rusqlite::Connection {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragma");
        migrate::run(&mut conn).expect("migrate");

        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'Test', 'ada@example.test', 'other', 'password', 'halcyon:ada')",
            [],
        )
        .expect("account");

        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (1, 1, 'INBOX', 'Inbox', 'inbox')",
            [],
        )
        .expect("mailbox");

        conn
    }

    fn fetched(uid: u32, message_id: &str, subject: &str) -> Fetched {
        Fetched {
            uid,
            envelope: Envelope {
                message_id: Some(message_id.to_string()),
                subject: subject.to_string(),
                subject_base: subject.to_lowercase(),
                from: vec![Address {
                    name: Some("Ada".into()),
                    email: "ada@example.test".into(),
                }],
                date_sent: 1_000_000 + i64::from(uid),
                ..Envelope::default()
            },
            flags: Flags::default(),
            size: 1024,
            internal_date: 2_000_000 + i64::from(uid),
            modseq: None,
            gm_thrid: None,
            gm_msgid: None,
            references: Vec::new(),
        }
    }

    #[test]
    fn a_batch_is_written_once() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        let written = write_batch(
            &tx,
            1,
            1,
            &[fetched(1, "a@x", "one"), fetched(2, "b@x", "two")],
        )
        .expect("write");

        assert_eq!(written.inserted, 2);
        assert_eq!(written.updated, 0);
        assert_eq!(written.lowest_uid, 1);
    }

    #[test]
    fn writing_the_same_batch_twice_creates_no_duplicates() {
        // The exit gate: *killing the network mid-sync recovers without duplicates or loss*.
        // A resumed sync refetches the batch it was in the middle of, and this is what makes
        // that harmless.
        let mut conn = store();
        let batch = vec![fetched(1, "a@x", "one"), fetched(2, "b@x", "two")];

        {
            let tx = conn.transaction().expect("tx");
            write_batch(&tx, 1, 1, &batch).expect("first");
            tx.commit().expect("commit");
        }

        let second = {
            let tx = conn.transaction().expect("tx");
            let written = write_batch(&tx, 1, 1, &batch).expect("second");
            tx.commit().expect("commit");
            written
        };

        assert_eq!(second.inserted, 0, "nothing new on a replay");
        assert_eq!(second.updated, 2);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM message", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 2);
    }

    #[test]
    fn a_replay_updates_flags_without_rewriting_the_envelope() {
        // Flags change constantly; envelopes never do. Rewriting the subject on every
        // incremental sync would churn the FTS index for the whole mailbox.
        let mut conn = store();

        {
            let tx = conn.transaction().expect("tx");
            write_batch(&tx, 1, 1, &[fetched(1, "a@x", "original subject")]).expect("write");
            tx.commit().expect("commit");
        }

        {
            let tx = conn.transaction().expect("tx");
            let mut changed = fetched(1, "a@x", "a different subject");
            changed.flags.seen = true;
            changed.flags.flagged = true;
            write_batch(&tx, 1, 1, &[changed]).expect("write");
            tx.commit().expect("commit");
        }

        let (subject, seen, flagged): (String, i64, i64) = conn
            .query_row(
                "SELECT subject, flag_seen, flag_flagged FROM message WHERE uid = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("row");

        assert_eq!(subject, "original subject", "the envelope is immutable");
        assert_eq!(seen, 1);
        assert_eq!(flagged, 1);
    }

    #[test]
    fn the_list_orders_by_the_servers_clock_not_the_senders() {
        // A machine with a wrong date sends a message dated 2087. Sorting by the Date header
        // would pin it to the top of the mailbox forever.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        let mut liar = fetched(1, "a@x", "from the future");
        liar.envelope.date_sent = 4_000_000_000;
        liar.internal_date = 1_700_000_000;

        write_batch(&tx, 1, 1, &[liar]).expect("write");

        let received: i64 = tx
            .query_row(
                "SELECT date_received FROM message WHERE uid = 1",
                [],
                |row| row.get(0),
            )
            .expect("row");

        assert_eq!(received, 1_700_000_000);
    }

    #[test]
    fn a_message_with_no_internal_date_falls_back_to_its_header() {
        // Degrade visibly rather than filing it at the epoch.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        let mut no_internal = fetched(1, "a@x", "no internaldate");
        no_internal.internal_date = 0;
        no_internal.envelope.date_sent = 1_650_000_000;

        write_batch(&tx, 1, 1, &[no_internal]).expect("write");

        let received: i64 = tx
            .query_row(
                "SELECT date_received FROM message WHERE uid = 1",
                [],
                |row| row.get(0),
            )
            .expect("row");

        assert_eq!(received, 1_650_000_000);
    }

    #[test]
    fn threading_assigns_every_message_a_thread_row() {
        let mut conn = store();

        {
            let tx = conn.transaction().expect("tx");

            let mut reply = fetched(2, "b@x", "Re: one");
            reply.envelope.in_reply_to = Some("a@x".into());
            reply.references = vec!["a@x".into()];

            write_batch(&tx, 1, 1, &[fetched(1, "a@x", "one"), reply]).expect("write");
            rethread(&tx, 1, 1000).expect("rethread");
            tx.commit().expect("commit");
        }

        let threads: i64 = conn
            .query_row("SELECT COUNT(DISTINCT thread_id) FROM message", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(threads, 1, "a reply shares its parent's thread");

        let unthreaded: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message WHERE thread_id IS NULL",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(unthreaded, 0, "every message must have a thread");
    }

    #[test]
    fn rethreading_twice_produces_the_same_thread_ids() {
        // `thread_id` is persisted and the UI keys off it. A key that churned would rebuild
        // every conversation on every sync and invalidate every cached query.
        let mut conn = store();

        let read_ids = |conn: &rusqlite::Connection| -> Vec<(i64, Option<i64>)> {
            let mut statement = conn
                .prepare("SELECT id, thread_id FROM message ORDER BY id")
                .expect("prepare");
            let rows = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .expect("query");
            rows.collect::<rusqlite::Result<Vec<_>>>().expect("rows")
        };

        {
            let tx = conn.transaction().expect("tx");
            write_batch(
                &tx,
                1,
                1,
                &[fetched(1, "a@x", "one"), fetched(2, "b@x", "two")],
            )
            .expect("write");
            rethread(&tx, 1, 1000).expect("rethread");
            tx.commit().expect("commit");
        }

        let first = read_ids(&conn);

        {
            let tx = conn.transaction().expect("tx");
            rethread(&tx, 1, 1000).expect("rethread");
            tx.commit().expect("commit");
        }

        assert_eq!(first, read_ids(&conn));
    }

    #[test]
    fn a_late_message_merges_two_existing_threads() {
        // docs/03 §5's incremental re-threading, end to end through the database.
        let mut conn = store();

        {
            let tx = conn.transaction().expect("tx");
            write_batch(
                &tx,
                1,
                1,
                &[fetched(1, "a@x", "one"), fetched(2, "b@x", "two")],
            )
            .expect("write");
            rethread(&tx, 1, 1000).expect("rethread");
            tx.commit().expect("commit");
        }

        let before: i64 = conn
            .query_row("SELECT COUNT(DISTINCT thread_id) FROM message", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(before, 2);

        {
            let tx = conn.transaction().expect("tx");
            let mut bridge = fetched(3, "c@x", "Re: both");
            bridge.references = vec!["a@x".into(), "b@x".into()];

            write_batch(&tx, 1, 1, &[bridge]).expect("write");
            rethread(&tx, 1, 1000).expect("rethread");
            tx.commit().expect("commit");
        }

        let after: i64 = conn
            .query_row("SELECT COUNT(DISTINCT thread_id) FROM message", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(after, 1, "the bridging message merged both threads");
    }

    #[test]
    fn thread_aggregates_reflect_their_messages() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        let mut read = fetched(1, "a@x", "one");
        read.flags.seen = true;
        let mut reply = fetched(2, "b@x", "Re: one");
        reply.references = vec!["a@x".into()];

        write_batch(&tx, 1, 1, &[read, reply]).expect("write");
        rethread(&tx, 1, 1000).expect("rethread");

        let (count, unread): (i64, i64) = tx
            .query_row(
                "SELECT message_count, unread_count FROM thread LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("thread");

        assert_eq!(count, 2);
        assert_eq!(unread, 1);
    }

    #[test]
    fn dropping_a_mailbox_clears_its_messages_and_its_search_index() {
        // UIDVALIDITY recovery. A cascade would leave the FTS rows behind and the removed
        // mail would still turn up in search with nothing to open — the same bug Phase 4
        // hit when removing an account.
        let mut conn = store();

        {
            let tx = conn.transaction().expect("tx");
            write_batch(&tx, 1, 1, &[fetched(1, "a@x", "Analytical Engine")]).expect("write");
            tx.commit().expect("commit");
        }

        let hits = |conn: &rusqlite::Connection| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM message_fts WHERE message_fts MATCH 'Analytical'",
                [],
                |row| row.get(0),
            )
            .expect("count")
        };
        assert_eq!(hits(&conn), 1);

        {
            let tx = conn.transaction().expect("tx");
            let removed = drop_mailbox_contents(&tx, 1).expect("drop");
            assert_eq!(removed, 1);
            tx.commit().expect("commit");
        }

        assert_eq!(
            hits(&conn),
            0,
            "the search index must not outlive the messages"
        );

        let validity: Option<i64> = conn
            .query_row("SELECT uid_validity FROM mailbox WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("row");
        assert_eq!(validity, None, "the mailbox must resync from scratch");
    }

    #[test]
    fn the_cached_counts_follow_the_rows() {
        // The first real sync downloaded a mailbox correctly and then showed "0 messages" in
        // the header and no badge in the sidebar, because nothing refreshed these columns.
        // The rows are the truth; these are a cache, and an unrefreshed cache is a wrong
        // number in front of the user.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        let mut read = fetched(1, "a@x", "read");
        read.flags.seen = true;

        write_batch(
            &tx,
            1,
            1,
            &[read, fetched(2, "b@x", "unread"), fetched(3, "c@x", "also")],
        )
        .expect("write");
        recount(&tx, 1).expect("recount");

        let (total, unread): (i64, i64) = tx
            .query_row(
                "SELECT total_count, unread_count FROM mailbox WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("row");

        assert_eq!(total, 3);
        assert_eq!(unread, 2);
    }

    #[test]
    fn recounting_after_a_replay_does_not_double_the_total() {
        // A resumed sync refetches the batch it was interrupted in. If the counts were
        // incremented rather than recomputed, every recovery would inflate the badge.
        let mut conn = store();
        let batch = vec![fetched(1, "a@x", "one"), fetched(2, "b@x", "two")];

        for _ in 0..3 {
            let tx = conn.transaction().expect("tx");
            write_batch(&tx, 1, 1, &batch).expect("write");
            recount(&tx, 1).expect("recount");
            tx.commit().expect("commit");
        }

        let total: i64 = conn
            .query_row("SELECT total_count FROM mailbox WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("row");

        assert_eq!(
            total, 2,
            "three passes over the same batch is still two messages"
        );
    }

    #[test]
    fn mailbox_state_is_recorded_so_the_next_sync_resumes() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        record_mailbox_state(&tx, 1, 12345, 900, Some(4242)).expect("record");

        let (validity, next, modseq): (i64, i64, i64) = tx
            .query_row(
                "SELECT uid_validity, uid_next, highest_modseq FROM mailbox WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("row");

        assert_eq!((validity, next, modseq), (12345, 900, 4242));
    }

    #[test]
    fn recording_state_without_a_modseq_keeps_the_stored_one() {
        // A server that stops advertising CONDSTORE mid-session must not wipe the sequence
        // we already hold — the next sync would fall back to a full windowed scan.
        let mut conn = store();

        {
            let tx = conn.transaction().expect("tx");
            record_mailbox_state(&tx, 1, 1, 100, Some(999)).expect("record");
            tx.commit().expect("commit");
        }

        {
            let tx = conn.transaction().expect("tx");
            record_mailbox_state(&tx, 1, 1, 200, None).expect("record");
            tx.commit().expect("commit");
        }

        let modseq: i64 = conn
            .query_row(
                "SELECT highest_modseq FROM mailbox WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("row");

        assert_eq!(modseq, 999);
    }

    #[test]
    fn an_empty_batch_is_a_no_op_rather_than_an_error() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        assert_eq!(
            write_batch(&tx, 1, 1, &[]).expect("write"),
            Written::default()
        );
    }
}
