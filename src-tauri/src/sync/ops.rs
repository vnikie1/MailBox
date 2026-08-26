//! Local changes on their way to the server. docs/03 §5's `pending_op` drain.
//!
//! Flagging a message, moving it, deleting it — all of those are written to the local database
//! immediately, because the user is looking at the result and a client that waits for a round
//! trip before redrawing feels broken. The server still has to be told, and it might not be
//! reachable when the change is made.
//!
//! So every mutation records its intent in `pending_op` **inside the same transaction as the
//! local write**. That is the whole design: the local change and the obligation to push it are
//! one atomic unit, so there is no window in which the screen says one thing, the server
//! another, and nothing remembers the difference. Offline mode is not a mode — it is what this
//! queue does when the drain cannot connect.
//!
//! The drain runs at the *start* of a sync, before anything is fetched. The other order loses
//! data: pulling the server's state first would overwrite the local change with the stale
//! value the server still holds, and the queued op would then push a value the user had
//! already seen reverted.

use rusqlite::{params, Transaction};
use serde::{Deserialize, Serialize};

use crate::db::{Db, DbError};

use super::session::{ImapSession, SyncError};

/// How many times to retry one operation before dropping it.
///
/// Dropping is not silent — it logs and clears — but it has to happen. An operation that can
/// never succeed (a message the server has already expunged, a mailbox that has been deleted)
/// would otherwise sit at the head of the queue and block every change made after it.
const MAX_ATTEMPTS: i64 = 5;

/// One queued change, as stored in `pending_op.payload_json`.
///
/// UIDs rather than local row ids: the row id means nothing to the server, and by the time the
/// drain runs the local row may have moved mailbox or been replaced. The mailbox is carried as
/// its remote path for the same reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Op {
    /// `UID STORE` — set or clear `\Seen` and `\Flagged`.
    Flag {
        mailbox: String,
        uids: Vec<u32>,
        seen: Option<bool>,
        flagged: Option<bool>,
    },

    /// `UID MOVE`, falling back to COPY + STORE + EXPUNGE where the server has no MOVE.
    Move {
        from: String,
        to: String,
        uids: Vec<u32>,
    },

    /// `UID STORE \Deleted` then expunge. Only ever a *permanent* delete; moving to Trash is
    /// a `Move`, which is what the delete command does unless the user asked otherwise.
    Delete { mailbox: String, uids: Vec<u32> },
}

impl Op {
    fn kind(&self) -> &'static str {
        match self {
            Op::Flag { .. } => "flag",
            Op::Move { .. } => "move",
            Op::Delete { .. } => "delete",
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Op::Flag {
                uids,
                seen,
                flagged,
                ..
            } => uids.is_empty() || (seen.is_none() && flagged.is_none()),
            Op::Move { uids, .. } | Op::Delete { uids, .. } => uids.is_empty(),
        }
    }
}

/// Formats a UID list as an IMAP sequence set.
///
/// Listed rather than ranged: the UIDs in one operation are whatever the user happened to
/// select, which is rarely contiguous, and a range spanning the gaps would touch messages the
/// user did not choose. That is a correctness point, not a performance one.
fn sequence_set(uids: &[u32]) -> String {
    uids.iter()
        .map(|uid| uid.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Where a set of local rows lives on the server, grouped so one command covers each group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub account_id: i64,
    pub mailbox: String,
    pub uids: Vec<u32>,
}

/// Resolves local message ids to the account, mailbox path and UIDs the server knows.
///
/// **Call before the local write, never after.** A move rewrites `message.mailbox_id`, so
/// afterwards this returns the destination and the queued operation would ask the server to
/// move messages out of the mailbox they were already moved to.
///
/// Grouped by mailbox because IMAP is: one `SELECT` and one command per mailbox, rather than a
/// round trip per message. A selection spanning two accounts is normal — "mark all as read"
/// across All Inboxes — and produces one group per account per mailbox.
pub fn locate(tx: &Transaction<'_>, ids: &[i64]) -> Result<Vec<Located>, DbError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = (0..ids.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "SELECT message.account_id, mailbox.remote_path, message.uid
           FROM message
           JOIN mailbox ON mailbox.id = message.mailbox_id
          WHERE message.id IN ({placeholders})
          ORDER BY message.account_id, mailbox.remote_path, message.uid"
    );

    let params: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

    let rows = tx
        .prepare(&sql)?
        .query_map(params.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut groups: Vec<Located> = Vec::new();

    for (account_id, mailbox, uid) in rows {
        // A UID of 0 is a message that exists locally and has never been on the server —
        // there is nothing to tell the server about, and "0" is not a valid UID to send.
        let Ok(uid) = u32::try_from(uid) else {
            continue;
        };
        if uid == 0 {
            continue;
        }

        match groups.last_mut() {
            Some(last) if last.account_id == account_id && last.mailbox == mailbox => {
                last.uids.push(uid);
            }
            _ => groups.push(Located {
                account_id,
                mailbox,
                uids: vec![uid],
            }),
        }
    }

    Ok(groups)
}

/// The remote path of one mailbox, for naming a move's destination.
pub fn mailbox_path(tx: &Transaction<'_>, mailbox_id: i64) -> Result<Option<String>, DbError> {
    Ok(tx
        .query_row(
            "SELECT remote_path FROM mailbox WHERE id = ?1",
            params![mailbox_id],
            |row| row.get(0),
        )
        .ok())
}

/// Queues one operation. Call inside the transaction that makes the local change.
pub fn enqueue(tx: &Transaction<'_>, account_id: i64, op: &Op) -> Result<(), DbError> {
    if op.is_empty() {
        return Ok(());
    }

    let payload = serde_json::to_string(op).map_err(|error| DbError::Encode {
        what: "pending_op payload",
        detail: error.to_string(),
    })?;

    tx.execute(
        "INSERT INTO pending_op (account_id, kind, payload_json, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            account_id,
            op.kind(),
            payload,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        ],
    )?;

    Ok(())
}

/// Everything the account has queued, oldest first.
///
/// Order matters and is not merely tidiness: flagging a message and then moving it must reach
/// the server in that order, because after the move the UID in the first operation no longer
/// resolves in the mailbox it names.
pub fn queued(tx: &Transaction<'_>, account_id: i64) -> Result<Vec<(i64, Op)>, DbError> {
    let mut statement = tx
        .prepare("SELECT id, payload_json FROM pending_op WHERE account_id = ?1 ORDER BY id ASC")?;

    let rows = statement.query_map(params![account_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut ops = Vec::new();

    for row in rows {
        let (id, payload) = row?;

        // A payload that will not parse is from a version of this app that wrote a shape we
        // no longer understand. Skipping it keeps the queue moving; the cleanup below drops
        // it so it is not re-read on every sync forever.
        match serde_json::from_str::<Op>(&payload) {
            Ok(op) => ops.push((id, op)),
            Err(error) => {
                tracing::warn!(id, %error, "pending_op payload could not be read; dropping it");
                tx.execute("DELETE FROM pending_op WHERE id = ?1", params![id])?;
            }
        }
    }

    Ok(ops)
}

/// How many operations are waiting. Used to decide whether a drain is worth a connection.
pub fn pending_count(tx: &Transaction<'_>, account_id: i64) -> Result<i64, DbError> {
    Ok(tx.query_row(
        "SELECT COUNT(*) FROM pending_op WHERE account_id = ?1",
        params![account_id],
        |row| row.get(0),
    )?)
}

fn forget(tx: &Transaction<'_>, id: i64) -> Result<(), DbError> {
    tx.execute("DELETE FROM pending_op WHERE id = ?1", params![id])?;
    Ok(())
}

/// Records a failure, and gives up once the operation has had its chances.
///
/// Returns true when the row was dropped rather than kept.
fn record_failure(tx: &Transaction<'_>, id: i64, error: &str) -> Result<bool, DbError> {
    let attempts: i64 = tx.query_row(
        "SELECT attempts + 1 FROM pending_op WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )?;

    if attempts >= MAX_ATTEMPTS {
        forget(tx, id)?;
        return Ok(true);
    }

    tx.execute(
        "UPDATE pending_op SET attempts = ?2, last_error = ?3 WHERE id = ?1",
        params![id, attempts, error],
    )?;

    Ok(false)
}

/// Issues one `UID STORE` and consumes its response.
///
/// A function rather than an inline loop so the response stream — which borrows the session —
/// is dropped before the caller needs the session again. `UID STORE` returns the new flags for
/// every message touched, and leaving those unread would desynchronise the connection: the
/// next command would read them as its own answer. `.SILENT` asks the server not to send them,
/// but a server is free to ignore that, so they are drained either way.
async fn store_flags(
    session: &mut ImapSession,
    set: &str,
    instruction: String,
) -> Result<(), SyncError> {
    use futures::StreamExt;

    let mut stream = session.uid_store(set, instruction).await?;
    while let Some(item) = stream.next().await {
        item?;
    }

    Ok(())
}

/// Applies one operation to the server.
async fn apply(session: &mut ImapSession, op: &Op, has_move: bool) -> Result<(), SyncError> {
    match op {
        Op::Flag {
            mailbox,
            uids,
            seen,
            flagged,
        } => {
            session.select(mailbox).await?;

            // Set and clear are separate commands: IMAP's `+FLAGS` and `-FLAGS` cannot be
            // combined, and `FLAGS` without a sign would replace the whole set — wiping
            // \Answered and every keyword the user never touched.
            let mut add: Vec<&str> = Vec::new();
            let mut remove: Vec<&str> = Vec::new();

            if let Some(value) = seen {
                if *value {
                    add.push("\\Seen");
                } else {
                    remove.push("\\Seen");
                }
            }
            if let Some(value) = flagged {
                if *value {
                    add.push("\\Flagged");
                } else {
                    remove.push("\\Flagged");
                }
            }

            let set = sequence_set(uids);

            if !add.is_empty() {
                store_flags(session, &set, format!("+FLAGS.SILENT ({})", add.join(" "))).await?;
            }
            if !remove.is_empty() {
                store_flags(
                    session,
                    &set,
                    format!("-FLAGS.SILENT ({})", remove.join(" ")),
                )
                .await?;
            }
        }

        Op::Move { from, to, uids } => {
            session.select(from).await?;
            let set = sequence_set(uids);

            if has_move {
                session.uid_mv(&set, to).await?;
            } else {
                // The three-step fallback docs/03 §5 describes. Copy first: if it fails the
                // message is still in one place, whereas deleting first and failing to copy
                // loses it outright.
                session.uid_copy(&set, to).await?;
                store_flags(session, &set, "+FLAGS.SILENT (\\Deleted)".into()).await?;
                expunge(session, &set).await?;
            }
        }

        Op::Delete { mailbox, uids } => {
            session.select(mailbox).await?;
            let set = sequence_set(uids);

            store_flags(session, &set, "+FLAGS.SILENT (\\Deleted)".into()).await?;
            expunge(session, &set).await?;
        }
    }

    Ok(())
}

/// Expunges just the UIDs named, where the server allows it.
///
/// A bare `EXPUNGE` removes **every** message flagged `\Deleted` in the mailbox, including
/// ones another client flagged and has not expunged yet. `UID EXPUNGE` (UIDPLUS) is the
/// narrow version. Falling back to the broad one is still better than leaving the message
/// flagged-but-present, which is what the user sees as "delete did nothing".
async fn expunge(session: &mut ImapSession, set: &str) -> Result<(), SyncError> {
    use futures::StreamExt;

    let narrow = match session.uid_expunge(set).await {
        Ok(stream) => {
            // The expunge response is not `Unpin`, so it has to be pinned before it can be
            // polled. `pin_mut!` pins it on the stack, which is what a stream consumed here
            // and dropped here wants.
            futures::pin_mut!(stream);
            while let Some(item) = stream.next().await {
                item?;
            }
            true
        }
        Err(error) => {
            tracing::debug!(%error, "UID EXPUNGE refused; falling back to EXPUNGE");
            false
        }
    };

    if narrow {
        return Ok(());
    }

    let stream = session.expunge().await?;
    futures::pin_mut!(stream);
    while let Some(item) = stream.next().await {
        item?;
    }

    Ok(())
}

/// Pushes everything queued for an account, oldest first.
///
/// Returns how many operations were sent. Stops at the first operation that fails rather than
/// skipping past it, for the ordering reason in [`queued`].
pub async fn drain(
    db: &Db,
    session: &mut ImapSession,
    account_id: i64,
    has_move: bool,
) -> Result<usize, SyncError> {
    let ops = db.write(move |tx| queued(tx, account_id)).await?;

    if ops.is_empty() {
        return Ok(0);
    }

    tracing::debug!(
        account_id,
        queued = ops.len(),
        "draining pending operations"
    );

    let mut sent = 0usize;

    for (id, op) in ops {
        match apply(session, &op, has_move).await {
            Ok(()) => {
                db.write(move |tx| forget(tx, id)).await?;
                sent += 1;
            }

            Err(error) => {
                let detail = error.to_string();
                let dropped = db.write(move |tx| record_failure(tx, id, &detail)).await?;

                if dropped {
                    tracing::warn!(
                        id,
                        %error,
                        "pending operation failed too many times; dropping it"
                    );
                    // Dropped, so it no longer blocks the queue — carry on with the rest.
                    continue;
                }

                tracing::warn!(id, %error, "pending operation failed; will retry");

                // Stop rather than skip. Later operations may depend on this one having
                // landed, and pushing them out of order can move a message the server still
                // believes is somewhere else.
                return Err(error);
            }
        }
    }

    tracing::debug!(account_id, sent, "pending operations drained");
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate;

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

        conn
    }

    fn flag(uids: Vec<u32>, seen: Option<bool>) -> Op {
        Op::Flag {
            mailbox: "INBOX".into(),
            uids,
            seen,
            flagged: None,
        }
    }

    #[test]
    fn a_sequence_set_lists_uids_rather_than_spanning_them() {
        // A range would touch every UID between the ones selected. The user picked three
        // messages; "4:900" is a different instruction entirely.
        assert_eq!(sequence_set(&[4, 17, 900]), "4,17,900");
        assert_eq!(sequence_set(&[1]), "1");
        assert_eq!(sequence_set(&[]), "");
    }

    #[test]
    fn operations_come_back_in_the_order_they_were_made() {
        // Flag-then-move must reach the server that way round: after the move the UID in the
        // flag operation no longer resolves in the mailbox it names.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        enqueue(&tx, 1, &flag(vec![7], Some(true))).expect("first");
        enqueue(
            &tx,
            1,
            &Op::Move {
                from: "INBOX".into(),
                to: "Archive".into(),
                uids: vec![7],
            },
        )
        .expect("second");

        let ops = queued(&tx, 1).expect("queued");

        assert_eq!(ops.len(), 2);
        assert!(matches!(ops[0].1, Op::Flag { .. }));
        assert!(matches!(ops[1].1, Op::Move { .. }));
    }

    #[test]
    fn an_operation_that_changes_nothing_is_not_queued() {
        // A patch with neither flag set, or an empty selection, would otherwise put a row in
        // the queue that the drain has to connect to the server to discover is a no-op.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        enqueue(&tx, 1, &flag(vec![], Some(true))).expect("no uids");
        enqueue(&tx, 1, &flag(vec![7], None)).expect("no change");

        assert_eq!(pending_count(&tx, 1).expect("count"), 0);
    }

    #[test]
    fn a_round_trip_through_json_keeps_every_field() {
        // The payload outlives the process, so a shape that serialises lossily loses a user's
        // change rather than merely a field.
        let ops = [
            flag(vec![1, 2, 3], Some(false)),
            Op::Flag {
                mailbox: "Archive".into(),
                uids: vec![9],
                seen: None,
                flagged: Some(true),
            },
            Op::Move {
                from: "INBOX".into(),
                to: "[Gmail]/Trash".into(),
                uids: vec![4, 5],
            },
            Op::Delete {
                mailbox: "INBOX".into(),
                uids: vec![6],
            },
        ];

        for op in ops {
            let json = serde_json::to_string(&op).expect("encode");
            let back: Op = serde_json::from_str(&json).expect("decode");
            assert_eq!(back, op, "{json}");
        }
    }

    #[test]
    fn an_operation_is_dropped_once_it_has_had_its_chances() {
        // Otherwise one impossible operation — a message the server already expunged — sits
        // at the head of the queue and blocks every change made after it, forever.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        enqueue(&tx, 1, &flag(vec![7], Some(true))).expect("enqueue");
        let id = queued(&tx, 1).expect("queued")[0].0;

        for attempt in 1..MAX_ATTEMPTS {
            assert!(
                !record_failure(&tx, id, "nope").expect("failure"),
                "dropped early on attempt {attempt}"
            );
            assert_eq!(pending_count(&tx, 1).expect("count"), 1);
        }

        assert!(record_failure(&tx, id, "nope").expect("last"));
        assert_eq!(pending_count(&tx, 1).expect("count"), 0);
    }

    #[test]
    fn an_unreadable_payload_is_dropped_rather_than_read_forever() {
        // Written by a version that stored a different shape. Keeping it would mean parsing
        // and failing on it at the start of every sync for the life of the install.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        tx.execute(
            "INSERT INTO pending_op (account_id, kind, payload_json, created_at)
             VALUES (1, 'flag', '{\"kind\":\"somethingElse\"}', 0)",
            [],
        )
        .expect("insert");

        assert!(queued(&tx, 1).expect("queued").is_empty());
        assert_eq!(pending_count(&tx, 1).expect("count"), 0);
    }
}
