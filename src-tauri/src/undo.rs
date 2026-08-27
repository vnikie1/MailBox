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

                let Some(current) = current else { continue };

                inverse.push(Prior::Mailbox {
                    id: *id,
                    mailbox_id: current,
                });

                crate::db::write::move_to(tx, &[*id], *mailbox_id)?;
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

                tx.execute(
                    "UPDATE message SET flag_seen = ?2 WHERE id = ?1",
                    params![id, i64::from(*seen)],
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

                tx.execute(
                    "UPDATE message SET flag_flagged = ?2, flag_color = ?3 WHERE id = ?1",
                    params![id, i64::from(*flagged), color],
                )?;
            }
            Prior::Junk { id, junk, by_user } => {
                let current: Option<(i64, i64)> = tx
                    .query_row(
                        "SELECT is_junk, junk_by_user FROM message WHERE id = ?1",
                        params![id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .ok();

                let Some((current_junk, current_by_user)) = current else {
                    continue;
                };

                inverse.push(Prior::Junk {
                    id: *id,
                    junk: current_junk != 0,
                    by_user: current_by_user != 0,
                });

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
}
