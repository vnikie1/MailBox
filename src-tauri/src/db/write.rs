//! Mutations. docs/03-architecture.md §3, standing rule 10.
//!
//! Every function here takes a `&Transaction`, because each is called through
//! `Db::write` — the caller cannot forget the transaction and cannot half-apply a change.
//!
//! Each mutation does two things together: it changes `message`, and it enqueues a
//! `pending_op` describing what the server still has to be told. That pairing is the whole
//! optimistic model. The UI is told the change happened as soon as the transaction commits;
//! Phase 5's worker drains the ops with backoff and reconciles.

use rusqlite::Transaction;

use super::model::FlagPatch;
use super::DbError;

/// `?1, ?2, ... ?n`, matching `query::placeholders`. Placeholders only, never values.
fn placeholders(count: usize, start: usize) -> String {
    (0..count)
        .map(|i| format!("?{}", start + i))
        .collect::<Vec<_>>()
        .join(", ")
}

// ## Why nothing here queues a server operation
//
// It used to. `set_flags`, `move_to` and `delete` each wrote a `pending_op` row of their own,
// with a payload like `{"ids":[..],"mailboxId":7}`.
//
// Not one of those rows ever reached a server. `sync::ops::Op` is `#[serde(tag = "kind")]`, so
// a payload without a `kind` field cannot deserialise; the drain logs "pending_op payload could
// not be read; dropping it" and DELETEs the row. Every one, every time, since Phase 5.
//
// It went unnoticed because every command that calls into this module *also* builds a real
// `Op` and enqueues it — so the work got done, and these rows were silent duplicates that were
// thrown away. The two places that relied on them alone, the rules engine and undo, were
// therefore local-only: a rule filed mail here and nowhere else.
//
// A test named `every_mutation_leaves_a_pending_op_for_the_sync_engine` asserted the rows
// existed and never asked whether the sync engine could read one.
//
// Queuing now belongs to the caller, which is the only layer that knows the mailbox path and
// the UIDs a server operation needs.

/// Which mailboxes a set of messages currently sits in.
///
/// Used by the commands to work out which mailbox:changed events to emit. Callers that
/// mutate must read this *before* the mutation when the rows may move or disappear.
pub fn mailboxes_of(tx: &Transaction<'_>, ids: &[i64]) -> Result<Vec<i64>, DbError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let in_list = placeholders(ids.len(), 1);
    let sql = format!("SELECT DISTINCT mailbox_id FROM message WHERE id IN ({in_list})");
    let params: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

    let rows = tx
        .prepare(&sql)?
        .query_map(params.as_slice(), |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Unread and total counts *among a given set of messages*, grouped by mailbox.
type CountsByMailbox = std::collections::HashMap<i64, (i64, i64)>;

/// Counts the given messages only, by primary key.
///
/// This is the hot half of count maintenance: it touches the rows named by `ids` and
/// nothing else, so a hundred-message archive costs a hundred index lookups regardless of
/// how large the mailboxes involved are.
fn snapshot(tx: &Transaction<'_>, ids: &[i64]) -> Result<CountsByMailbox, DbError> {
    if ids.is_empty() {
        return Ok(CountsByMailbox::new());
    }

    let in_list = placeholders(ids.len(), 1);
    let sql = format!(
        "SELECT mailbox_id,
                SUM(CASE WHEN flag_seen = 0 THEN 1 ELSE 0 END),
                COUNT(*)
           FROM message
          WHERE id IN ({in_list})
          GROUP BY mailbox_id"
    );

    let params: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

    let mut counts = CountsByMailbox::new();
    let mut stmt = tx.prepare(&sql)?;
    let mut rows = stmt.query(params.as_slice())?;

    while let Some(row) = rows.next()? {
        counts.insert(row.get(0)?, (row.get(1)?, row.get(2)?));
    }

    Ok(counts)
}

/// Applies the difference between two snapshots to the cached counts on `mailbox`.
///
/// Incremental, not a recount. Recounting — `SELECT COUNT(*) FROM message WHERE mailbox_id = ?`
/// — measured **84 ms** against a 70,000-message inbox, and the first version of this ran it
/// after every mutation, so marking one message read would have cost that and blocked the
/// single writer while it did. Standing rule 10 wants the local write instant; these deltas
/// are O(messages touched).
fn apply_delta(
    tx: &Transaction<'_>,
    before: &CountsByMailbox,
    after: &CountsByMailbox,
) -> Result<(), DbError> {
    let mut touched: Vec<i64> = before.keys().chain(after.keys()).copied().collect();
    touched.sort_unstable();
    touched.dedup();

    for mailbox_id in touched {
        let (unread_before, total_before) = before.get(&mailbox_id).copied().unwrap_or((0, 0));
        let (unread_after, total_after) = after.get(&mailbox_id).copied().unwrap_or((0, 0));

        let unread_delta = unread_after - unread_before;
        let total_delta = total_after - total_before;

        if unread_delta == 0 && total_delta == 0 {
            continue;
        }

        // MAX(0, ...) is a guard, not a fix. A negative cached count would mean the deltas
        // had drifted from reality, and showing "-3 unread" is worse than showing zero
        // while an integrity check catches it.
        tx.execute(
            "UPDATE mailbox
                SET unread_count = MAX(0, unread_count + ?2),
                    total_count  = MAX(0, total_count + ?3)
              WHERE id = ?1",
            (mailbox_id, unread_delta, total_delta),
        )?;
    }

    Ok(())
}

/// Recomputes the cached counts from scratch.
///
/// O(size of the mailbox) — around 84 ms per 70,000-message mailbox — so this belongs to
/// the seed tool and to a future integrity check, never to a mutation.
pub fn recount_mailboxes(tx: &Transaction<'_>, mailbox_ids: &[i64]) -> Result<(), DbError> {
    for mailbox_id in mailbox_ids {
        tx.execute(
            "UPDATE mailbox
                SET unread_count = (SELECT COUNT(*) FROM message
                                     WHERE mailbox_id = ?1 AND flag_seen = 0),
                    total_count  = (SELECT COUNT(*) FROM message WHERE mailbox_id = ?1)
              WHERE id = ?1",
            [mailbox_id],
        )?;
    }

    Ok(())
}

/// Sets read and flagged state. `None` in the patch leaves that flag alone, which is what
/// makes it safe on a multi-selection whose members disagree.
pub fn set_flags(tx: &Transaction<'_>, ids: &[i64], patch: FlagPatch) -> Result<usize, DbError> {
    if ids.is_empty() || (patch.seen.is_none() && patch.flagged.is_none()) {
        return Ok(0);
    }

    let mut assignments: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(seen) = patch.seen {
        params.push(Box::new(i64::from(seen)));
        assignments.push(format!("flag_seen = ?{}", params.len()));
    }
    if let Some(flagged) = patch.flagged {
        params.push(Box::new(i64::from(flagged)));
        assignments.push(format!("flag_flagged = ?{}", params.len()));
    }

    let start = params.len() + 1;
    let in_list = placeholders(ids.len(), start);
    for id in ids {
        params.push(Box::new(*id));
    }

    let sql = format!(
        "UPDATE message SET {} WHERE id IN ({in_list})",
        assignments.join(", ")
    );

    let before = snapshot(tx, ids)?;

    let borrowed: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let changed = tx.execute(&sql, borrowed.as_slice())?;

    apply_delta(tx, &before, &snapshot(tx, ids)?)?;

    Ok(changed)
}

/// Moves messages to another mailbox.
///
/// Counts are refreshed on both the source and the destination — a move that updated only
/// the destination would leave the source badge permanently wrong, and a badge nobody can
/// explain is worse than no badge.
pub fn move_to(tx: &Transaction<'_>, ids: &[i64], mailbox_id: i64) -> Result<usize, DbError> {
    if ids.is_empty() {
        return Ok(0);
    }

    let before = snapshot(tx, ids)?;

    let in_list = placeholders(ids.len(), 2);
    let sql = format!("UPDATE message SET mailbox_id = ?1 WHERE id IN ({in_list})");

    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(mailbox_id)];
    for id in ids {
        params.push(Box::new(*id));
    }
    let borrowed: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let changed = tx.execute(&sql, borrowed.as_slice())?;

    // The source mailboxes appear in the first snapshot and the destination in the second,
    // so both ends are corrected from the same pair.
    apply_delta(tx, &before, &snapshot(tx, ids)?)?;

    Ok(changed)
}

/// Deletes messages, or moves them to Trash.
///
/// A non-permanent delete is a move: that is what Mail does, and it is the only version
/// that Undo can reverse. `permanent` really removes the rows, and the FTS triggers unindex
/// them as part of the same transaction.
pub fn delete(
    tx: &Transaction<'_>,
    ids: &[i64],
    permanent: bool,
    trash_mailbox_id: Option<i64>,
) -> Result<usize, DbError> {
    if ids.is_empty() {
        return Ok(0);
    }

    if !permanent {
        if let Some(trash) = trash_mailbox_id {
            return move_to(tx, ids, trash);
        }
        // No Trash to move to — deleting anyway would destroy mail the user expected to be
        // recoverable, so refuse rather than improvise.
        return Ok(0);
    }

    // Taken while the rows still exist: afterwards there is nothing to count.
    let before = snapshot(tx, ids)?;

    let in_list = placeholders(ids.len(), 1);
    let sql = format!("DELETE FROM message WHERE id IN ({in_list})");
    let params: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

    let changed = tx.execute(&sql, params.as_slice())?;

    // The rows are gone, so the second snapshot is empty and every delta is negative.
    apply_delta(tx, &before, &snapshot(tx, ids)?)?;

    Ok(changed)
}
