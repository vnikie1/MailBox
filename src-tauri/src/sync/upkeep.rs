//! The periodic tick: waking reminders, detecting follow-ups, scoring junk. docs/06 Phase 8.
//!
//! Phase 8 added three pieces of state that only mean something if *something reads them* on a
//! schedule. A `snooze_until` nothing wakes is a column, not a reminder. This is that caller.
//!
//! ## Why a tick and not a timer per message
//!
//! A timer per snoozed message is exact and unaffordable: a user who snoozes forty things has
//! forty tasks, each holding a wakeup, and the machine cannot sleep while any of them is
//! pending. One tick with a coarse interval costs one wakeup and is late by at most that
//! interval, which for "remind me tomorrow morning" is not a difference anyone can perceive.
//!
//! ## Why it does not run on a fixed wall-clock schedule
//!
//! Laptops sleep. A tick that assumed it ran every minute would, after a lid opens, believe one
//! minute has passed when it was nine hours — the same class of bug that killed IDLE in Phase 5,
//! where a clock jump was read as a transient failure. Everything here asks the database *what
//! is due now* rather than tracking what it thinks it has already done, so a missed tick catches
//! up on the next one and a duplicated tick is a no-op.

use std::sync::Arc;
use std::time::Duration;

use crate::db::{Db, DbError};
use crate::rules::junk::Classifier;
use crate::rules::vip;

use super::events::Events;

/// How often the tick runs.
///
/// A minute is well under the precision anyone expects from "remind me tonight", and coarse
/// enough that the cost is a single wakeup rather than a reason the machine stays awake.
pub const INTERVAL: Duration = Duration::from_secs(60);

/// How often follow-up detection runs, in ticks.
///
/// Much rarer than reminders: the query walks sent mail looking for unanswered threads, the
/// answer changes on the scale of days, and running it every minute would be a table scan a
/// thousand times over for a result that cannot have moved.
const FOLLOW_UP_EVERY: u64 = 30;

/// What one tick did. Returned rather than logged so the caller can decide what is worth
/// telling the user about — a reminder coming due is a notification, a follow-up mark is not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ticked {
    /// Messages whose reminder just came due.
    pub woken: Vec<i64>,
    pub followed_up: usize,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// One pass. Safe to call at any interval, including twice in a row.
pub async fn tick(db: &Db, count: u64) -> Result<Ticked, DbError> {
    let stamp = now();
    let follow_up_due = count % FOLLOW_UP_EVERY == 0;

    db.write(move |tx| {
        // Clears the reminders and returns exactly which messages it cleared, in one
        // transaction. Reading them first and clearing them second would race the next tick and
        // either notify twice or not at all.
        let woken = vip::wake_due(tx, stamp)?;

        let followed_up = if follow_up_due {
            vip::detect_follow_ups(tx, stamp)?
        } else {
            0
        };

        Ok(Ticked { woken, followed_up })
    })
    .await
}

/// Scores unjudged mail in one mailbox and files what the filter is sure about.
///
/// Called after a sync writes new messages, not on the tick: the answer only changes when mail
/// arrives, and re-scoring an unchanged mailbox every minute would be pure waste.
///
/// Never touches a message the user has judged. Overruling someone's own decision is the
/// fastest way to make them stop trusting a filter and turn it off for good.
pub async fn score_new_mail(db: &Db, mailbox_id: i64) -> Result<usize, DbError> {
    let classifier = db.read(Classifier::load).await?;

    // A fresh install has nothing to go on. Answering confidently from nothing would file real
    // mail as junk on day one.
    if !classifier.ready() {
        return Ok(0);
    }

    db.write(move |tx| {
        let candidates = {
            let mut statement = tx.prepare(
                "SELECT id, COALESCE(from_all, ''), COALESCE(subject, ''),
                        COALESCE(body_text, '')
                   FROM message
                  WHERE mailbox_id = ?1 AND junk_by_user = 0 AND is_junk = 0
                    AND junk_score IS NULL",
            )?;

            let rows = statement
                .query_map(rusqlite::params![mailbox_id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            rows
        };

        let mut filed = 0;

        for (id, from, subject, body) in candidates {
            let verdict = classifier.score(&from, &subject, &body);

            let Some(score) = verdict.probability() else {
                continue;
            };

            // The score is stored even below the threshold. It is what lets someone see *why* a
            // message was filed, and it is also the marker that stops the next pass rescoring
            // mail nothing has changed about.
            tx.execute(
                "UPDATE message SET junk_score = ?2, is_junk = ?3 WHERE id = ?1",
                rusqlite::params![id, score, i64::from(verdict.is_junk())],
            )?;

            if verdict.is_junk() {
                filed += 1;
            }
        }

        Ok(filed)
    })
    .await
}

/// Runs the tick forever, telling the UI when something comes due.
///
/// Spawned once at startup. It holds only a `Db` and an `Events`, so it cannot be the thing
/// keeping anything else alive.
pub fn spawn(db: Db, events: Arc<dyn Events>) {
    tokio::spawn(async move {
        let mut count: u64 = 0;

        loop {
            tokio::time::sleep(INTERVAL).await;
            count = count.wrapping_add(1);

            match tick(&db, count).await {
                Ok(ticked) => {
                    // Only when something actually happened. An event every minute would have
                    // the UI refetching a list nobody is looking at, forever.
                    if !ticked.woken.is_empty() || ticked.followed_up > 0 {
                        tracing::debug!(
                            woken = ticked.woken.len(),
                            followed_up = ticked.followed_up,
                            "upkeep tick"
                        );
                        events.emit("mailbox:changed", serde_json::Value::Null);
                    }

                    if !ticked.woken.is_empty() {
                        // A separate event so Phase 10 can raise a notification for exactly the
                        // messages that reappeared, rather than for every list change.
                        events.emit("snooze:due", serde_json::json!({ "ids": ticked.woken }));
                    }
                }
                Err(error) => {
                    // Logged and carried on. A tick that fails once — a locked database during a
                    // large sync — must not end the loop, or reminders stop working for the rest
                    // of the session with nothing on screen to say so.
                    tracing::warn!(%error, "upkeep tick failed");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn store() -> rusqlite::Connection {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open");
        crate::db::migrate::run(&mut conn).expect("migrate");

        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'T', 'me@t.test', 'other', 'password', 'halcyon:me')",
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

    fn add(conn: &rusqlite::Connection, id: i64, snooze: Option<i64>) {
        conn.execute(
            "INSERT INTO message (
                 id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
                 from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged,
                 is_junk, snooze_until
             ) VALUES (?1, 1, 1, ?1, 'S', 0, 0, 10, 'a@b.test', '', '', 0, 0, 0, 0, ?2)",
            params![id, snooze],
        )
        .expect("message");
    }

    #[test]
    fn a_reminder_that_is_due_wakes_and_one_that_is_not_stays_asleep() {
        let mut conn = store();
        add(&conn, 1, Some(1_000));
        add(&conn, 2, Some(9_000));
        add(&conn, 3, None);

        let tx = conn.transaction().expect("tx");
        let woken = vip::wake_due(&tx, 5_000).expect("wake");

        assert_eq!(woken, vec![1]);

        let still: Option<i64> = tx
            .query_row("SELECT snooze_until FROM message WHERE id = 2", [], |row| {
                row.get(0)
            })
            .expect("read");

        assert_eq!(still, Some(9_000), "a reminder that is not due was cleared");
    }

    #[test]
    fn waking_the_same_message_twice_returns_it_once() {
        // The tick can run twice in quick succession — a resumed laptop, a manual sync. A
        // reminder that notified on both would be a duplicate the user has to dismiss twice.
        let mut conn = store();
        add(&conn, 1, Some(1_000));

        let tx = conn.transaction().expect("tx");
        assert_eq!(vip::wake_due(&tx, 5_000).expect("first"), vec![1]);
        assert!(vip::wake_due(&tx, 5_000).expect("second").is_empty());
    }

    #[test]
    fn a_missed_tick_catches_up_rather_than_skipping() {
        // The laptop-sleep case. Nothing here tracks what it thinks it has already done, so a
        // reminder due nine hours ago still fires on the first tick after the lid opens.
        let mut conn = store();
        add(&conn, 1, Some(1_000));
        add(&conn, 2, Some(2_000));
        add(&conn, 3, Some(3_000));

        let tx = conn.transaction().expect("tx");
        let woken = vip::wake_due(&tx, 1_000_000).expect("wake");

        assert_eq!(woken, vec![1, 2, 3]);
    }
}
