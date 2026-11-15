//! The undo stack. docs/01 §8, docs/06 Phase 8.
//!
//! ## Why undo is recorded rather than derived
//!
//! Every entry stores **the state it replaced**, not a description of what happened. "Moved to
//! Archive" cannot be undone without knowing where the message was before, and reconstructing
//! that after the fact means guessing — the Inbox is the usual answer and the usual answer is
//! wrong exactly when the user most wants undo to work.
//!
//! ## Why it lives in memory
//!
//! Undo is a property of a session, not of a mailbox. An undo stack restored from disk would
//! offer to reverse an action from last Tuesday, against a mailbox that has since been synced,
//! moved and re-threaded by other clients. Mail does not do this and neither should we: closing
//! the window is the end of the undo history, which is also what every user already expects.
//!
//! ## What cannot be undone, and why it says so
//!
//! An operation that reached the server and was confirmed by it is not always reversible — an
//! expunged message is gone. Rather than offering an undo that fails, those entries are never
//! pushed, and the menu item greys out. An undo that silently does nothing is worse than one
//! that is visibly unavailable.

use std::collections::VecDeque;
use std::sync::Mutex;

use rusqlite::{params, Transaction};
use serde::Serialize;
use ts_rs::TS;

use crate::db::DbError;

/// How many steps back the stack goes.
///
/// Mail keeps going until you close the window. A bound exists because each entry holds the
/// prior state of every message it touched, and "select all, mark read" over 50,000 messages
/// would otherwise pin a row per message in memory for the rest of the session.
const DEPTH: usize = 50;

/// The prior state of one message, for one field.
#[derive(Debug, Clone)]
enum Prior {
    /// Where it was. No UID: the one it had there is only still right if the server has not
    /// moved it since, and `ops::queue_move` is what knows that. See `restore`.
    Mailbox {
        id: i64,
        mailbox_id: i64,
    },
    Seen {
        id: i64,
        seen: bool,
    },
    Flagged {
        id: i64,
        flagged: bool,
        color: Option<String>,
    },
    Junk {
        id: i64,
        junk: bool,
        by_user: bool,
    },
    Snooze {
        id: i64,
        until: Option<i64>,
    },
    Muted {
        thread_id: i64,
        muted: bool,
    },
}

/// One undoable action, as the user thinks of it: everything a single command changed.
#[derive(Debug, Clone)]
pub struct Step {
    /// What the menu item says — "Undo Move to Archive". Written by the caller, because only
    /// the caller knows whether this was a move, a rule run or a swipe.
    pub label: String,
    priors: Vec<Prior>,
}

#[cfg(test)]
impl Step {
    /// The messages this step would restore, for tests that check what a command captured.
    pub(crate) fn message_ids(&self) -> Vec<i64> {
        self.priors
            .iter()
            .filter_map(|prior| match prior {
                Prior::Mailbox { id, .. }
                | Prior::Seen { id, .. }
                | Prior::Flagged { id, .. }
                | Prior::Junk { id, .. }
                | Prior::Snooze { id, .. } => Some(*id),
                Prior::Muted { .. } => None,
            })
            .collect()
    }
}

/// What the UI needs to draw the menu item.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    pub undo: Option<String>,
    pub redo: Option<String>,
}

/// The session's stack. One per app, behind a mutex.
#[derive(Default)]
pub struct Stack {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    done: VecDeque<Step>,
    undone: Vec<Step>,
}

impl Stack {
    pub fn new() -> Self {
        Self::default()
    }

    fn push_step(&self, step: Step) {
        if step.priors.is_empty() {
            // Nothing changed, so there is nothing to undo. Pushing an empty step would give
            // the user a menu item that does nothing when they press it.
            return;
        }

        let mut inner = self.inner.lock().expect("undo stack poisoned");

        inner.done.push_back(step);
        while inner.done.len() > DEPTH {
            inner.done.pop_front();
        }

        // A new action invalidates the redo branch. Keeping it would let the user redo their way
        // into a state that never existed.
        inner.undone.clear();
    }

    pub fn available(&self) -> Available {
        let inner = self.inner.lock().expect("undo stack poisoned");

        Available {
            undo: inner.done.back().map(|step| step.label.clone()),
            redo: inner.undone.last().map(|step| step.label.clone()),
        }
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().expect("undo stack poisoned");
        inner.done.clear();
        inner.undone.clear();
    }

    fn pop_undo(&self) -> Option<Step> {
        let mut inner = self.inner.lock().expect("undo stack poisoned");
        inner.done.pop_back()
    }

    fn pop_redo(&self) -> Option<Step> {
        let mut inner = self.inner.lock().expect("undo stack poisoned");
        inner.undone.pop()
    }

    fn push_redo(&self, step: Step) {
        let mut inner = self.inner.lock().expect("undo stack poisoned");
        inner.undone.push(step);
    }

    fn push_done(&self, step: Step) {
        let mut inner = self.inner.lock().expect("undo stack poisoned");
        inner.done.push_back(step);
    }
}

/// Reads the current state of the fields an operation is about to change, so it can be put back.
///
/// Called **before** the change, inside the same transaction. Outside it, a concurrent sync
/// could land between the read and the write and undo would restore a state that was already
/// stale — which looks to the user like undo moving a message somewhere it never was.
pub fn capture(
    tx: &Transaction<'_>,
    label: impl Into<String>,
    ids: &[i64],
    fields: &[Field],
) -> Result<Step, DbError> {
    let mut priors = Vec::new();

    for &id in ids {
        let row = tx.query_row(
            "SELECT mailbox_id, flag_seen, flag_flagged, flag_color, is_junk, junk_by_user,
                    snooze_until, thread_id
               FROM message WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)? != 0,
                    row.get::<_, i64>(2)? != 0,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)? != 0,
                    row.get::<_, i64>(5)? != 0,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                ))
            },
        );

        let Ok((mailbox_id, seen, flagged, color, junk, by_user, snooze, thread_id)) = row else {
            // Gone between the click and the write. Skipping it is right: there is nothing to
            // restore, and failing the whole operation because one message vanished would make
            // undo less reliable rather than more.
            continue;
        };

        for field in fields {
            priors.push(match field {
                Field::Mailbox => Prior::Mailbox { id, mailbox_id },
                Field::Seen => Prior::Seen { id, seen },
                Field::Flagged => Prior::Flagged {
                    id,
                    flagged,
                    color: color.clone(),
                },
                Field::Junk => Prior::Junk { id, junk, by_user },
                Field::Snooze => Prior::Snooze { id, until: snooze },
                Field::Muted => match thread_id {
                    Some(thread_id) => {
                        let muted: i64 = tx
                            .query_row(
                                "SELECT muted FROM thread WHERE id = ?1",
                                params![thread_id],
                                |row| row.get(0),
                            )
                            .unwrap_or(0);

                        Prior::Muted {
                            thread_id,
                            muted: muted != 0,
                        }
                    }
                    None => continue,
                },
            });
        }
    }

    Ok(Step {
        label: label.into(),
        priors,
    })
}

/// Which fields an operation is about to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Mailbox,
    Seen,
    Flagged,
    Junk,
    Snooze,
    Muted,
}

impl Stack {
    /// Records a step. Call after the write has succeeded, with the state captured before it.
    pub fn record(&self, step: Step) {
        self.push_step(step);
    }
}

/// Puts one step's worth of state back, and returns the step that would put it back again.
///
/// The inverse is captured *before* restoring, from the same rows, so redo is exact rather than
/// a re-run of the original command. Re-running would be wrong for anything whose effect
/// depends on when it happens — a rule, a snooze, a filter verdict.
fn restore(tx: &Transaction<'_>, step: &Step) -> Result<Step, DbError> {
    let mut inverse = Vec::with_capacity(step.priors.len());

    // The server side of the flags put back. Undo used to restore the local row and tell the
    // server nothing, so the next sync put the server's state back over it; then it queued one
    // operation per message, which for a large undo held up the account's sync for minutes (L4).
    // Batched, and queued before any move. See `FlagBatch`.
    let mut flags = crate::sync::ops::FlagBatch::default();

    for prior in &step.priors {
        match prior {
            Prior::Mailbox { id, mailbox_id } => {
                let current: Option<i64> = tx
                    .query_row(
                        "SELECT mailbox_id FROM message WHERE id = ?1",
                        params![id],
                        |row| row.get(0),
                    )
                    .ok();

                let Some(current) = current else {
                    continue;
                };

                // The folder it came from has been deleted since. There is nowhere to put the
                // message back, and trying would fail the foreign key and every other part of
                // this undo with it — so it stays where it is.
                if crate::sync::ops::mailbox_path(tx, *mailbox_id)?.is_none() {
                    continue;
                }

                inverse.push(Prior::Mailbox {
                    id: *id,
                    mailbox_id: current,
                });

                // The server side through the one function every move uses. This used to queue
                // its own move from `locate`, which named the message's *original* place — so it
                // queued nothing when undo returned the message there, and the archive still
                // waiting in the queue ran anyway; and once the archive had run, it named a UID
                // the server no longer had. Either way the next sync reversed the undo (H7).
                //
                // The UID it gets back is the server's, when the server still has it there; the
                // one recorded before the move was trusted here, and is wrong once the move ran.
                flags.queue(tx)?;
                let homecoming = crate::sync::ops::queue_move(tx, &[*id], *mailbox_id)?;
                crate::db::write::move_to(tx, &[*id], *mailbox_id)?;
                crate::sync::ops::settle(tx, &homecoming)?;
            }
            Prior::Seen { id, seen } => {
                let current: Option<i64> = tx
                    .query_row(
                        "SELECT flag_seen FROM message WHERE id = ?1",
                        params![id],
                        |row| row.get(0),
                    )
                    .ok();

                let Some(current) = current else { continue };

                inverse.push(Prior::Seen {
                    id: *id,
                    seen: current != 0,
                });

                flags.add(*id, Some(*seen), None);

                // Through `set_flags`, which keeps the mailbox's cached unread count in step. A
                // bare UPDATE left the sidebar's count where the undone action had put it, until
                // the next sync recounted (M20, 2026-11-14 audit).
                crate::db::write::set_flags(
                    tx,
                    &[*id],
                    crate::db::model::FlagPatch {
                        seen: Some(*seen),
                        flagged: None,
                    },
                )?;
            }
            Prior::Flagged { id, flagged, color } => {
                let current: Option<(i64, Option<String>)> = tx
                    .query_row(
                        "SELECT flag_flagged, flag_color FROM message WHERE id = ?1",
                        params![id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .ok();

                let Some((current_flagged, current_color)) = current else {
                    continue;
                };

                inverse.push(Prior::Flagged {
                    id: *id,
                    flagged: current_flagged != 0,
                    color: current_color,
                });

                flags.add(*id, None, Some(*flagged));

                tx.execute(
                    "UPDATE message SET flag_flagged = ?2, flag_color = ?3 WHERE id = ?1",
                    params![id, i64::from(*flagged), color],
                )?;
            }
            Prior::Junk { id, junk, by_user } => {
                // The message's text as well as its flags, because undoing a junk judgement has
                // to reach the corpus and the corpus is keyed on the words.
                let current: Option<(i64, i64, String, String, String)> = tx
                    .query_row(
                        "SELECT is_junk, junk_by_user, COALESCE(from_all, ''),
                                COALESCE(subject, ''), COALESCE(body_text, '')
                           FROM message WHERE id = ?1",
                        params![id],
                        |row| {
                            Ok((
                                row.get(0)?,
                                row.get(1)?,
                                row.get(2)?,
                                row.get(3)?,
                                row.get(4)?,
                            ))
                        },
                    )
                    .ok();

                let Some((current_junk, current_by_user, from, subject, body)) = current else {
                    continue;
                };

                inverse.push(Prior::Junk {
                    id: *id,
                    junk: current_junk != 0,
                    by_user: current_by_user != 0,
                });

                // The filter, not only the column.
                //
                // `junk_mark` trains the classifier on every judgement the user makes, and undo
                // restored the flag while leaving the training example in place — so Ctrl+Z put
                // the message back and the filter went on believing what the user had just
                // taken back, and acting on it for every message that followed. `junk_mark`'s
                // own comment says exactly this about the other direction: "their correction is
                // only half applied and the filter keeps the belief that caused the mistake."
                //
                // Symmetric on purpose, so redo is this same arm run the other way: retract the
                // judgement being undone, and reinstate the one being restored if there was one.
                //
                // Whenever the user's judgement changes, not only when the flag does. Marking as
                // junk a message the filter had already put in Junk trains it too — the user's
                // word is a label the filter's own verdict is not — and undo checked the flag
                // alone, saw it unchanged, and left that example in the corpus (L17).
                let same_judgement =
                    current_by_user != 0 && *by_user && (current_junk != 0) == *junk;

                if current_by_user != 0 && !same_judgement {
                    crate::rules::junk::untrain(tx, &from, &subject, &body, current_junk != 0)?;
                }

                if *by_user && !same_judgement {
                    crate::rules::junk::train(tx, &from, &subject, &body, *junk)?;
                }

                tx.execute(
                    "UPDATE message SET is_junk = ?2, junk_by_user = ?3 WHERE id = ?1",
                    params![id, i64::from(*junk), i64::from(*by_user)],
                )?;
            }
            Prior::Snooze { id, until } => {
                let current: Option<Option<i64>> = tx
                    .query_row(
                        "SELECT snooze_until FROM message WHERE id = ?1",
                        params![id],
                        |row| row.get(0),
                    )
                    .ok();

                let Some(current) = current else { continue };

                inverse.push(Prior::Snooze {
                    id: *id,
                    until: current,
                });

                tx.execute(
                    "UPDATE message SET snooze_until = ?2 WHERE id = ?1",
                    params![id, until],
                )?;
            }
            Prior::Muted { thread_id, muted } => {
                let current: Option<i64> = tx
                    .query_row(
                        "SELECT muted FROM thread WHERE id = ?1",
                        params![thread_id],
                        |row| row.get(0),
                    )
                    .ok();

                let Some(current) = current else { continue };

                inverse.push(Prior::Muted {
                    thread_id: *thread_id,
                    muted: current != 0,
                });

                tx.execute(
                    "UPDATE thread SET muted = ?2 WHERE id = ?1",
                    params![thread_id, i64::from(*muted)],
                )?;
            }
        }
    }

    flags.queue(tx)?;

    Ok(Step {
        label: step.label.clone(),
        priors: inverse,
    })
}

/// Reverses the last action. Returns its label, or `None` if there was nothing to undo.
pub fn undo(stack: &Stack, tx: &Transaction<'_>) -> Result<Option<String>, DbError> {
    let Some(step) = stack.pop_undo() else {
        return Ok(None);
    };

    let label = step.label.clone();

    match restore(tx, &step) {
        Ok(inverse) => {
            stack.push_redo(inverse);
            Ok(Some(label))
        }
        Err(error) => {
            // Put it back. Losing the entry because the write failed would leave the user
            // unable to retry the undo they just asked for.
            stack.push_done(step);
            Err(error)
        }
    }
}

/// Reapplies the last undone action.
pub fn redo(stack: &Stack, tx: &Transaction<'_>) -> Result<Option<String>, DbError> {
    let Some(step) = stack.pop_redo() else {
        return Ok(None);
    };

    let label = step.label.clone();

    match restore(tx, &step) {
        Ok(inverse) => {
            stack.push_done(inverse);
            Ok(Some(label))
        }
        Err(error) => {
            stack.push_redo(step);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn store() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open");
        crate::db::migrate::run(&mut conn).expect("migrate");

        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'T', 'me@t.test', 'other', 'password', 'halcyon:me')",
            [],
        )
        .expect("account");

        for (id, path, role) in [(1, "INBOX", "inbox"), (2, "Archive", "archive")] {
            conn.execute(
                "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
                 VALUES (?1, 1, ?2, ?2, ?3)",
                params![id, path, role],
            )
            .expect("mailbox");
        }

        conn.execute(
            "INSERT INTO thread (id, account_id, subject_base, last_date, message_count, muted)
             VALUES (1, 1, 'Subject', 0, 1, 0)",
            [],
        )
        .expect("thread");

        conn.execute(
            "INSERT INTO message (
                 id, account_id, mailbox_id, thread_id, uid, subject, date_sent, date_received,
                 size, from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged,
                 is_junk, junk_by_user
             ) VALUES (1, 1, 1, 1, 1, 'Subject', 0, 0, 10, 'a@b.test', '', '', 0, 0, 0, 0, 0)",
            [],
        )
        .expect("message");

        conn
    }

    fn mailbox_of(tx: &Transaction<'_>, id: i64) -> i64 {
        tx.query_row(
            "SELECT mailbox_id FROM message WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("mailbox")
    }

    /// Archives message 1 the way `msg_archive` does, server side included.
    fn archive(tx: &Transaction<'_>, stack: &Stack) {
        let step = capture(tx, "Archive", &[1], &[Field::Mailbox]).expect("capture");
        let homecoming = crate::sync::ops::queue_move(tx, &[1], 2).expect("queue");
        crate::db::write::move_to(tx, &[1], 2).expect("move");
        crate::sync::ops::settle(tx, &homecoming).expect("settle");
        stack.record(step);
    }

    fn queued(tx: &Transaction<'_>) -> Vec<crate::sync::ops::Op> {
        crate::sync::ops::queued(tx, 1)
            .expect("queue")
            .into_iter()
            .map(|(_, op)| op)
            .collect()
    }

    /// `(mailbox, uid, origin mailbox, origin uid)`.
    fn row(tx: &Transaction<'_>) -> (i64, i64, Option<i64>, Option<i64>) {
        tx.query_row(
            "SELECT mailbox_id, uid, origin_mailbox_id, origin_uid FROM message WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("row")
    }

    /// H7 in the 2026-11-14 audit, before the archive has reached the server.
    ///
    /// Undo's own move named the message's original place, saw it was already "there", and queued
    /// nothing — leaving the archive itself queued. Two seconds later it ran, and the next sync
    /// took the message back out of the Inbox the user had just restored it to.
    #[test]
    fn undo_before_the_archive_reaches_the_server_cancels_it() {
        let mut conn = store();
        let stack = Stack::new();
        let tx = conn.transaction().expect("tx");

        archive(&tx, &stack);
        assert_eq!(queued(&tx).len(), 1, "the archive is queued");

        undo(&stack, &tx).expect("undo");

        assert!(
            queued(&tx).is_empty(),
            "the archive is still queued: {:?}",
            queued(&tx)
        );
        assert_eq!(
            row(&tx),
            (1, 1, None, None),
            "back exactly as it was, UID and all"
        );
    }

    /// H7, after the archive has reached the server — the usual case, since the queue is pushed
    /// two seconds after a change and Undo is pressed from a toast that stays for longer.
    ///
    /// The message is then in the Archive under a UID the app has not learned, and the old undo
    /// asked the server to move the UID it used to have in the Inbox, which no longer exists.
    #[test]
    fn undo_after_the_archive_reached_the_server_moves_it_back_by_message_id() {
        let mut conn = store();
        conn.execute(
            "UPDATE message SET message_id = 'one@example.test' WHERE id = 1",
            [],
        )
        .expect("message id");
        let stack = Stack::new();
        let tx = conn.transaction().expect("tx");

        archive(&tx, &stack);
        // The drain ran it.
        tx.execute("DELETE FROM pending_op", []).expect("pushed");

        undo(&stack, &tx).expect("undo");

        assert_eq!(
            queued(&tx),
            vec![crate::sync::ops::Op::MoveByMessageId {
                from: "Archive".into(),
                to: "INBOX".into(),
                message_id: "one@example.test".into(),
            }]
        );

        let (mailbox, uid, origin, origin_uid) = row(&tx);
        assert_eq!(mailbox, 1, "shown in the Inbox at once");
        assert!(uid < 0, "parked until the server gives it a UID here");
        assert_eq!(
            (origin, origin_uid),
            (Some(2), None),
            "the server has it in the Archive, UID unknown"
        );

        // And redo before that move runs takes it off the queue again: the server never moved
        // the message back, so there is nothing to send.
        redo(&stack, &tx).expect("redo");
        assert!(queued(&tx).is_empty(), "{:?}", queued(&tx));
        assert_eq!(mailbox_of(&tx, 1), 2);
    }

    #[test]
    fn dragging_a_message_straight_back_cancels_the_first_move() {
        // The same bug reached without Undo: Move to Archive, then drag it back to the Inbox.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        for destination in [2, 1] {
            let homecoming = crate::sync::ops::queue_move(&tx, &[1], destination).expect("queue");
            crate::db::write::move_to(&tx, &[1], destination).expect("move");
            crate::sync::ops::settle(&tx, &homecoming).expect("settle");
        }

        assert!(queued(&tx).is_empty(), "{:?}", queued(&tx));
        assert_eq!(row(&tx), (1, 1, None, None));
    }

    #[test]
    fn a_move_of_several_messages_is_still_one_command() {
        // Deciding per message must not cost a command per message: archiving fifty is one
        // `UID MOVE`, as it was.
        let mut conn = store();
        for (id, uid) in [(2, 7), (3, 9)] {
            conn.execute(
                "INSERT INTO message (
                     id, account_id, mailbox_id, thread_id, uid, subject, date_sent,
                     date_received, size, from_all, to_all, body_text, has_attachment,
                     flag_seen, flag_flagged, is_junk, junk_by_user
                 ) VALUES (?1, 1, 1, 1, ?2, 'S', 0, 0, 10, 'a@b.test', '', '', 0, 0, 0, 0, 0)",
                params![id, uid],
            )
            .expect("message");
        }
        let tx = conn.transaction().expect("tx");

        let homecoming = crate::sync::ops::queue_move(&tx, &[1, 2, 3], 2).expect("queue");
        crate::sync::ops::settle(&tx, &homecoming).expect("settle");

        assert_eq!(
            queued(&tx),
            vec![crate::sync::ops::Op::Move {
                from: "INBOX".into(),
                to: "Archive".into(),
                uids: vec![1, 7, 9],
            }]
        );
    }

    #[test]
    fn a_move_is_undone_to_where_it_actually_was() {
        // Not "to the Inbox" — to wherever it was. The usual answer is the Inbox and the usual
        // answer is wrong exactly when the user most wants undo to work.
        let mut conn = store();
        let stack = Stack::new();

        let tx = conn.transaction().expect("tx");
        let step = capture(&tx, "Move to Archive", &[1], &[Field::Mailbox]).expect("capture");
        crate::db::write::move_to(&tx, &[1], 2).expect("move");
        stack.record(step);

        assert_eq!(mailbox_of(&tx, 1), 2);

        let label = undo(&stack, &tx).expect("undo");
        assert_eq!(label.as_deref(), Some("Move to Archive"));
        assert_eq!(mailbox_of(&tx, 1), 1, "did not go back where it came from");
    }

    #[test]
    fn undoing_a_move_out_of_a_deleted_folder_leaves_the_message_where_it_is() {
        // There is nowhere to put it back. The alternative was a foreign-key failure that took
        // the rest of the undo down with it.
        let mut conn = store();
        let stack = Stack::new();

        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (3, 1, 'Old', 'Old', NULL)",
            [],
        )
        .expect("folder");
        conn.execute("UPDATE message SET mailbox_id = 3 WHERE id = 1", [])
            .expect("file it there");

        let tx = conn.transaction().expect("tx");
        let step = capture(&tx, "Move to Archive", &[1], &[Field::Mailbox]).expect("capture");
        crate::db::write::move_to(&tx, &[1], 2).expect("move");
        stack.record(step);

        crate::sync::folders::delete(&tx, 3).expect("delete the folder");

        let label = undo(&stack, &tx).expect("undo does not fail");
        assert_eq!(label.as_deref(), Some("Move to Archive"));
        assert_eq!(mailbox_of(&tx, 1), 2);
    }

    #[test]
    fn redo_puts_it_back() {
        let mut conn = store();
        let stack = Stack::new();

        let tx = conn.transaction().expect("tx");
        let step = capture(&tx, "Move to Archive", &[1], &[Field::Mailbox]).expect("capture");
        crate::db::write::move_to(&tx, &[1], 2).expect("move");
        stack.record(step);

        undo(&stack, &tx).expect("undo");
        assert_eq!(mailbox_of(&tx, 1), 1);

        let label = redo(&stack, &tx).expect("redo");
        assert_eq!(label.as_deref(), Some("Move to Archive"));
        assert_eq!(mailbox_of(&tx, 1), 2);
    }

    #[test]
    fn a_new_action_discards_the_redo_branch() {
        // Keeping it would let the user redo their way into a state that never existed.
        let mut conn = store();
        let stack = Stack::new();

        let tx = conn.transaction().expect("tx");

        let step = capture(&tx, "Move to Archive", &[1], &[Field::Mailbox]).expect("capture");
        crate::db::write::move_to(&tx, &[1], 2).expect("move");
        stack.record(step);
        undo(&stack, &tx).expect("undo");

        assert!(stack.available().redo.is_some());

        let step = capture(&tx, "Mark as Read", &[1], &[Field::Seen]).expect("capture");
        tx.execute("UPDATE message SET flag_seen = 1 WHERE id = 1", [])
            .expect("update");
        stack.record(step);

        assert!(
            stack.available().redo.is_none(),
            "redo survived a new action"
        );
    }

    #[test]
    fn undoing_mark_as_read_puts_the_unread_count_back() {
        // M20. The sidebar shows the count cached on the mailbox, and undo changed the message
        // without it, so the count stayed one too low until the next sync.
        let mut conn = store();
        let stack = Stack::new();
        let tx = conn.transaction().expect("tx");
        crate::db::write::recount_mailboxes(&tx, &[1]).expect("recount");
        let count = |tx: &Transaction<'_>| -> i64 {
            tx.query_row("SELECT unread_count FROM mailbox WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("read")
        };
        let before = count(&tx);

        let step = capture(&tx, "Mark as Read", &[1], &[Field::Seen]).expect("capture");
        crate::db::write::set_flags(
            &tx,
            &[1],
            crate::db::model::FlagPatch {
                seen: Some(true),
                flagged: None,
            },
        )
        .expect("read");
        stack.record(step);
        assert_eq!(count(&tx), before - 1);

        undo(&stack, &tx).expect("undo");
        assert_eq!(count(&tx), before, "undo restored the count");

        redo(&stack, &tx).expect("redo");
        assert_eq!(count(&tx), before - 1, "and redo took it away again");
    }

    #[test]
    fn undoing_nothing_is_not_an_error() {
        let mut conn = store();
        let stack = Stack::new();
        let tx = conn.transaction().expect("tx");

        assert_eq!(undo(&stack, &tx).expect("undo"), None);
        assert_eq!(redo(&stack, &tx).expect("redo"), None);
        assert!(stack.available().undo.is_none());
    }

    #[test]
    fn a_step_that_changed_nothing_is_not_offered() {
        // An enabled menu item that does nothing when pressed is worse than a greyed-out one.
        let mut conn = store();
        let stack = Stack::new();
        let tx = conn.transaction().expect("tx");

        let step = capture(&tx, "Move to Archive", &[], &[Field::Mailbox]).expect("capture");
        stack.record(step);

        assert!(stack.available().undo.is_none());
    }

    #[test]
    fn a_flag_colour_comes_back_with_the_flag() {
        // Undoing a flag must restore the colour too, or the message returns flagged in a
        // colour it never had.
        let mut conn = store();
        let stack = Stack::new();

        let tx = conn.transaction().expect("tx");
        tx.execute(
            "UPDATE message SET flag_flagged = 1, flag_color = 'blue' WHERE id = 1",
            [],
        )
        .expect("setup");

        let step = capture(&tx, "Flag Red", &[1], &[Field::Flagged]).expect("capture");
        tx.execute(
            "UPDATE message SET flag_flagged = 1, flag_color = 'red' WHERE id = 1",
            [],
        )
        .expect("update");
        stack.record(step);

        undo(&stack, &tx).expect("undo");

        let colour: Option<String> = tx
            .query_row("SELECT flag_color FROM message WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("read");

        assert_eq!(colour.as_deref(), Some("blue"));
    }

    #[test]
    fn the_stack_is_bounded() {
        // Each entry holds the prior state of every message it touched. "Select all, mark read"
        // over a large mailbox would otherwise pin a row per message for the session.
        let mut conn = store();
        let stack = Stack::new();
        let tx = conn.transaction().expect("tx");

        for index in 0..(DEPTH + 10) {
            let step =
                capture(&tx, format!("Step {index}"), &[1], &[Field::Seen]).expect("capture");
            stack.record(step);
        }

        let inner = stack.inner.lock().expect("lock");
        assert_eq!(inner.done.len(), DEPTH);
        assert_eq!(
            inner.done.front().map(|step| step.label.as_str()),
            Some("Step 10"),
            "the wrong end was dropped"
        );
    }

    #[test]
    fn a_message_deleted_between_the_action_and_the_undo_is_skipped() {
        // Failing the whole undo because one message vanished would make undo less reliable
        // rather than more.
        let mut conn = store();
        let stack = Stack::new();

        let tx = conn.transaction().expect("tx");
        let step = capture(&tx, "Move to Archive", &[1], &[Field::Mailbox]).expect("capture");
        crate::db::write::move_to(&tx, &[1], 2).expect("move");
        stack.record(step);

        tx.execute("DELETE FROM message WHERE id = 1", [])
            .expect("delete");

        assert_eq!(
            undo(&stack, &tx).expect("undo").as_deref(),
            Some("Move to Archive")
        );
    }

    /// Undoing a junk judgement takes it back out of the classifier too.
    ///
    /// The flag was restored and the training example was not, so Ctrl+Z put the message back
    /// while the filter went on believing what the user had just retracted -- and acting on it
    /// for every message that arrived afterwards. `junk_mark` says the same thing about the
    /// other direction: "their correction is only half applied and the filter keeps the belief
    /// that caused the mistake."
    #[test]
    fn undoing_a_junk_judgement_untrains_the_filter() {
        let mut conn = store();
        let stack = Stack::new();

        conn.execute(
            "UPDATE message
                SET from_all = 'spammer@example.test',
                    subject = 'Cheap pills now',
                    body_text = 'buy cheap pills today'
              WHERE id = 1",
            [],
        )
        .expect("text");

        // Summed, not counted: `untrain` decrements each token to zero and leaves the row, which
        // is what the corpus tests assert too.
        let tokens = |tx: &Transaction<'_>| -> i64 {
            tx.query_row(
                "SELECT COALESCE(SUM(count), 0) FROM junk_token",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
        };

        let tx = conn.transaction().expect("tx");
        assert_eq!(tokens(&tx), 0, "the corpus starts empty");

        // Marking as junk, in the order `junk_mark` does it: capture, train, write the flags.
        let step = capture(&tx, "Mark as Junk", &[1], &[Field::Junk]).expect("capture");
        crate::rules::junk::train(
            &tx,
            "spammer@example.test",
            "Cheap pills now",
            "buy cheap pills today",
            true,
        )
        .expect("train");
        tx.execute(
            "UPDATE message SET is_junk = 1, junk_by_user = 1 WHERE id = 1",
            [],
        )
        .expect("mark");
        stack.record(step);

        assert!(tokens(&tx) > 0, "marking as junk trains the filter");

        let label = undo(&stack, &tx).expect("undo");
        assert_eq!(label.as_deref(), Some("Mark as Junk"));

        let (is_junk, by_user): (i64, i64) = tx
            .query_row(
                "SELECT is_junk, junk_by_user FROM message WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read");

        assert_eq!(is_junk, 0, "the flag was not restored");
        assert_eq!(by_user, 0, "the judgement was not withdrawn");
        assert_eq!(
            tokens(&tx),
            0,
            "the training example survived the undo, so the filter still believes it"
        );
    }

    #[test]
    fn undoing_a_change_to_many_messages_queues_one_change_for_the_server() {
        // Undo queued one operation per message, sent one STORE at a time; undoing a Mark All as
        // Read over 5,000 messages held up the account's sync for minutes (L4). Redo too.
        let mut conn = store();
        for id in 2..=6 {
            conn.execute(
                "INSERT INTO message (id, account_id, mailbox_id, uid, date_sent, date_received,
                                      flag_seen)
                 VALUES (?1, 1, 1, ?1, 0, 0, 0)",
                params![id],
            )
            .expect("message");
        }
        let ids = [1, 2, 3, 4, 5, 6];
        let stack = Stack::new();
        let tx = conn.transaction().expect("tx");

        let step = capture(&tx, "Mark All as Read", &ids, &[Field::Seen]).expect("capture");
        crate::db::write::set_flags(
            &tx,
            &ids,
            crate::db::model::FlagPatch {
                seen: Some(true),
                flagged: None,
            },
        )
        .expect("read");
        stack.record(step);

        let queued = |tx: &Transaction<'_>| -> i64 {
            tx.query_row("SELECT COUNT(*) FROM pending_op", [], |row| row.get(0))
                .expect("count")
        };

        undo(&stack, &tx).expect("undo");
        assert_eq!(queued(&tx), 1, "one operation per message");

        redo(&stack, &tx).expect("redo");
        assert_eq!(queued(&tx), 2);
    }

    #[test]
    fn undoing_agreement_with_the_filter_untrains_it_too() {
        // The filter had put it in Junk and the user said so too. `junk_mark` trains on that,
        // since the user's word is a label and the filter's verdict is not; undo saw the junk
        // flag unchanged and left the example in (L17). Redo has to put it back.
        let mut conn = store();
        let stack = Stack::new();
        conn.execute(
            "UPDATE message
                SET from_all = 'spammer@example.test', subject = 'Cheap pills now',
                    body_text = 'buy cheap pills today', is_junk = 1, junk_by_user = 0
              WHERE id = 1",
            [],
        )
        .expect("the filter's verdict");

        let corpus = |tx: &Transaction<'_>| -> (i64, i64) {
            let tokens = tx
                .query_row(
                    "SELECT COALESCE(SUM(count), 0) FROM junk_token",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            let messages = tx
                .query_row(
                    "SELECT COALESCE(SUM(messages), 0) FROM junk_corpus",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            (tokens, messages)
        };

        let tx = conn.transaction().expect("tx");

        // As `junk_mark` does it for a message the user had not judged before.
        let step = capture(&tx, "Mark as Junk", &[1], &[Field::Junk]).expect("capture");
        crate::rules::junk::train(
            &tx,
            "spammer@example.test",
            "Cheap pills now",
            "buy cheap pills today",
            true,
        )
        .expect("train");
        tx.execute("UPDATE message SET junk_by_user = 1 WHERE id = 1", [])
            .expect("mark");
        stack.record(step);
        let trained = corpus(&tx);
        assert_eq!(trained.1, 1);

        undo(&stack, &tx).expect("undo");
        assert_eq!(corpus(&tx), (0, 0), "the example outlived its undo");

        redo(&stack, &tx).expect("redo");
        assert_eq!(corpus(&tx), trained, "redo did not put it back");
    }
}
