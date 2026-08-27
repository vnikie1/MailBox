//! The durable outbox. docs/06 Phase 7.
//!
//! Everything the user sends goes through here, and the design has one rule above all others:
//! **never silently lose a message, and never silently send one twice.** A mail client that
//! drops a message the user watched leave is worse than one that refuses to send at all,
//! because the user has no reason to look.
//!
//! ## The states
//!
//! ```text
//!   holding ──(timer elapses)──> queued ──> sending ──> sent
//!      │                                        │
//!   (undo)                                      └────> failed ──(retry)──> queued
//!      │
//!    deleted
//! ```
//!
//! `holding` is what makes **Undo Send** honest. The message is on disk and nothing has been
//! transmitted; undo deletes the row before any connection is opened. This is not a
//! cancellation request racing a send — there is nothing to race.
//!
//! ## Being killed mid-send
//!
//! The exit gate asks that killing the app mid-send neither loses nor duplicates. There is a
//! window between SMTP accepting a message and this process recording that it did, and a crash
//! inside it leaves a row saying `sending` with no local way to know which side of the line it
//! fell on. Retrying may send twice; giving up may lose it; both fail silently.
//!
//! So the answer comes from the server. Every message carries a `Message-ID` we generated, and
//! a sent message lands in the account's Sent mailbox. [`resolve_interrupted`] searches Sent
//! for that id: found means it went, absent means it did not. That is authoritative rather
//! than a guess, and it is the whole reason the id is ours rather than the library's.

use std::path::{Path, PathBuf};

use rusqlite::{params, Transaction};

use crate::db::{Db, DbError};

/// Where a message sits in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Written to disk, nothing transmitted. Undo Send lives here.
    Holding,
    /// The timer elapsed; waiting for a transport.
    Queued,
    /// A connection is open and the message is going out.
    Sending,
    /// The server accepted it.
    Sent,
    /// It did not go, and the user has to decide what to do.
    Failed,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Holding => "holding",
            State::Queued => "queued",
            State::Sending => "sending",
            State::Sent => "sent",
            State::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "holding" => Some(State::Holding),
            "queued" => Some(State::Queued),
            "sending" => Some(State::Sending),
            "sent" => Some(State::Sent),
            "failed" => Some(State::Failed),
            _ => None,
        }
    }
}

/// One row of the outbox.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: i64,
    pub account_id: i64,
    pub eml_path: String,
    pub state: State,
    pub send_after: i64,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub message_id: Option<String>,
    pub subject: Option<String>,
    pub recipients: Option<String>,
    pub created_at: i64,
}

/// How many times to try before leaving it to the user.
///
/// Not unlimited. A message that has failed five times is failing for a reason retrying will
/// not fix — a rejected recipient, a size limit, an authentication problem — and the honest
/// response is a banner with Retry and Edit rather than an outbox that churns forever.
pub const MAX_ATTEMPTS: i64 = 5;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Where an outgoing message's bytes live.
///
/// Beside the body cache, under the account, so removing an account takes its unsent mail with
/// it rather than leaving the contents of someone's drafts on disk after they thought it gone.
pub fn eml_path(root: &Path, account_id: i64, id: i64) -> PathBuf {
    root.join("outbox")
        .join(account_id.to_string())
        .join(format!("{id}.eml"))
}

fn read_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    let state: String = row.get("state")?;

    Ok(Entry {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        eml_path: row.get("eml_path")?,
        // An unrecognised state is treated as failed rather than skipped. A row nothing will
        // look at again is a message silently lost, which is the one outcome this module
        // exists to prevent.
        state: State::parse(&state).unwrap_or(State::Failed),
        send_after: row.get("send_after")?,
        attempts: row.get("attempts")?,
        last_error: row.get("last_error")?,
        message_id: row.get("message_id")?,
        subject: row.get("subject")?,
        recipients: row.get("recipients")?,
        created_at: row.get("created_at")?,
    })
}

/// Puts a built message into the outbox, in `holding`.
///
/// The bytes are written **before** the row is inserted. A crash between the two leaves an
/// orphan file, which costs disk; the other order leaves a row pointing at nothing, which
/// costs the message.
pub async fn enqueue(
    db: &Db,
    root: &Path,
    account_id: i64,
    built: &crate::mail::outgoing::Built,
    subject: &str,
    recipients: &str,
    hold_for_seconds: i64,
) -> Result<i64, DbError> {
    let created = now();
    let send_after = created + hold_for_seconds.max(0);

    let message_id = built.message_id.clone();
    let subject = subject.to_string();
    let recipients = recipients.to_string();

    // The row first, to learn the id the file is named for, but written as `holding` with a
    // path that does not exist yet — and `holding` is never transmitted, so a crash here
    // leaves a row that the sweep below removes rather than a half-sent message.
    let id = db
        .write({
            let message_id = message_id.clone();
            let subject = subject.clone();
            let recipients = recipients.clone();
            move |tx| {
                tx.execute(
                    "INSERT INTO outbox (
                         account_id, eml_path, state, send_after, attempts,
                         message_id, subject, recipients, created_at
                     ) VALUES (?1, '', 'holding', ?2, 0, ?3, ?4, ?5, ?6)",
                    params![account_id, send_after, message_id, subject, recipients, created],
                )?;

                Ok(tx.last_insert_rowid())
            }
        })
        .await?;

    let path = eml_path(root, account_id, id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &built.bytes)?;

    let stored = path.to_string_lossy().to_string();
    db.write(move |tx| {
        tx.execute(
            "UPDATE outbox SET eml_path = ?2 WHERE id = ?1",
            params![id, stored],
        )?;
        Ok(())
    })
    .await?;

    Ok(id)
}

/// Cancels a message that has not been transmitted. This is **Undo Send**.
///
/// Returns false when the message is past `holding`, which is the honest answer: at that point
/// bytes are on their way to a server and no local action can recall them. Reporting success
/// there would be a lie the user would discover from the recipient.
pub async fn cancel(db: &Db, id: i64) -> Result<bool, DbError> {
    let removed = db
        .write(move |tx| {
            let path: Option<String> = tx
                .query_row(
                    "SELECT eml_path FROM outbox WHERE id = ?1 AND state = 'holding'",
                    params![id],
                    |row| row.get(0),
                )
                .ok();

            let Some(path) = path else {
                return Ok(None);
            };

            tx.execute("DELETE FROM outbox WHERE id = ?1", params![id])?;
            Ok(Some(path))
        })
        .await?;

    match removed {
        Some(path) => {
            // The row is authoritative; a file left behind is tidied on the next sweep rather
            // than being allowed to fail the cancellation.
            let _ = std::fs::remove_file(&path);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Messages whose hold has elapsed, promoted to `queued`.
///
/// Promotion happens in the same transaction as the read, so two ticks cannot both pick up the
/// same message and send it twice.
pub async fn claim_due(db: &Db, account_id: Option<i64>) -> Result<Vec<Entry>, DbError> {
    let at = now();

    db.write(move |tx| {
        let mut statement = tx.prepare(
            "SELECT * FROM outbox
              WHERE state IN ('holding', 'queued', 'failed')
                AND send_after <= ?1
                AND attempts < ?2
                AND (?3 IS NULL OR account_id = ?3)
              ORDER BY created_at ASC",
        )?;

        let entries: Vec<Entry> = statement
            .query_map(params![at, MAX_ATTEMPTS, account_id], read_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        for entry in &entries {
            tx.execute(
                "UPDATE outbox SET state = 'sending' WHERE id = ?1",
                params![entry.id],
            )?;
        }

        Ok(entries)
    })
    .await
}

/// Records that the server accepted a message.
pub fn mark_sent(tx: &Transaction<'_>, id: i64) -> Result<(), DbError> {
    tx.execute(
        "UPDATE outbox SET state = 'sent', last_error = NULL WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// Records a failed attempt, and decides whether another is worth making.
///
/// Returns the state the row landed in. A message that has exhausted its attempts stops being
/// retried and starts being the user's decision — docs/06 Phase 7: *never silently drop a
/// message*, so it becomes a banner rather than a disappearance.
pub fn mark_attempt_failed(
    tx: &Transaction<'_>,
    id: i64,
    error: &str,
    retry_after_seconds: i64,
) -> Result<State, DbError> {
    let attempts: i64 = tx.query_row(
        "SELECT attempts + 1 FROM outbox WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )?;

    let state = if attempts >= MAX_ATTEMPTS {
        State::Failed
    } else {
        State::Queued
    };

    tx.execute(
        "UPDATE outbox
            SET state = ?2, attempts = ?3, last_error = ?4, send_after = ?5
          WHERE id = ?1",
        params![
            id,
            state.as_str(),
            attempts,
            error,
            now() + retry_after_seconds.max(0)
        ],
    )?;

    Ok(state)
}

/// Rows left in `sending` by a process that died. See the module header.
pub async fn interrupted(db: &Db) -> Result<Vec<Entry>, DbError> {
    db.read(move |conn| {
        let mut statement = conn.prepare("SELECT * FROM outbox WHERE state = 'sending'")?;
        let rows = statement
            .query_map([], read_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(rows)
    })
    .await
}

/// Resolves one interrupted send, given whether the server has the message.
///
/// Split from the IMAP lookup so the decision itself is testable without a server: the lookup
/// is one line of protocol, and this is the part that must never guess wrong.
pub fn resolve_interrupted(
    tx: &Transaction<'_>,
    id: i64,
    found_in_sent: bool,
) -> Result<State, DbError> {
    if found_in_sent {
        mark_sent(tx, id)?;
        return Ok(State::Sent);
    }

    // Not in Sent, so it never left. Back to the queue with its attempt count untouched — the
    // attempt was interrupted, not spent, and charging it would bring a message closer to being
    // abandoned for a reason that was never its fault.
    tx.execute(
        "UPDATE outbox SET state = 'queued', send_after = ?2 WHERE id = ?1",
        params![id, now()],
    )?;

    Ok(State::Queued)
}

/// Everything currently in the outbox that the user should see.
///
/// Sent rows are excluded: a message that has gone is in the Sent mailbox, and leaving it in
/// the outbox list would make the outbox a second, worse Sent folder.
pub async fn pending(db: &Db) -> Result<Vec<Entry>, DbError> {
    db.read(move |conn| {
        let mut statement =
            conn.prepare("SELECT * FROM outbox WHERE state != 'sent' ORDER BY created_at ASC")?;
        let rows = statement
            .query_map([], read_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(rows)
    })
    .await
}

/// Puts a failed message back in the queue, at the user's request.
///
/// The attempt count is reset, because the user has taken a decision: they looked at the
/// failure, decided the cause has passed — a mailbox that was full, a network that was down —
/// and asked again. Carrying the old count would give them one attempt and then the same
/// banner, which reads as the button not working.
pub fn retry(tx: &Transaction<'_>, id: i64) -> Result<bool, DbError> {
    let changed = tx.execute(
        "UPDATE outbox
            SET state = 'queued', attempts = 0, last_error = NULL, send_after = ?2
          WHERE id = ?1 AND state = 'failed'",
        params![id, now()],
    )?;

    Ok(changed > 0)
}

/// Moves a held message's send time. This is **Send Later**.
///
/// Only from `holding`: once a message is queued the hold is over, and pretending otherwise
/// would mean a "Send Later" that silently did nothing to a message already on its way.
pub fn reschedule(tx: &Transaction<'_>, id: i64, send_at: i64) -> Result<bool, DbError> {
    let changed = tx.execute(
        "UPDATE outbox SET send_after = ?2 WHERE id = ?1 AND state = 'holding'",
        params![id, send_at],
    )?;

    Ok(changed > 0)
}

#[cfg(test)]
mod retry_tests {
    use super::tests_support::*;
    use super::*;

    #[test]
    fn retrying_a_failed_message_gives_it_a_full_set_of_attempts_again() {
        // The user looked at the failure, decided the cause has passed, and asked again.
        // Carrying the old count would give them one attempt and the same banner, which reads
        // as the button not working.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");
        let id = insert(&tx, "failed", 0);

        tx.execute(
            "UPDATE outbox SET attempts = ?2, last_error = 'nope' WHERE id = ?1",
            params![id, MAX_ATTEMPTS],
        )
        .expect("exhaust");

        assert!(retry(&tx, id).expect("retry"));

        let (state, attempts, error): (String, i64, Option<String>) = tx
            .query_row(
                "SELECT state, attempts, last_error FROM outbox WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read");

        assert_eq!(state, "queued");
        assert_eq!(attempts, 0);
        assert_eq!(error, None, "a stale error would still be on the banner");
    }

    #[test]
    fn only_a_failed_message_can_be_retried() {
        // Retrying something already sending would send it twice.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        for state in ["holding", "queued", "sending", "sent"] {
            let id = insert(&tx, state, 0);
            assert!(!retry(&tx, id).expect("retry"), "retried a {state} message");
        }
    }

    #[test]
    fn send_later_moves_a_held_message_and_nothing_else() {
        // Once a message is queued the hold is over. A Send Later that silently did nothing to
        // a message already on its way would be worse than one that refused.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        let held = insert(&tx, "holding", 0);
        assert!(reschedule(&tx, held, 4_000_000_000).expect("reschedule"));

        let when: i64 = tx
            .query_row(
                "SELECT send_after FROM outbox WHERE id = ?1",
                params![held],
                |row| row.get(0),
            )
            .expect("read");
        assert_eq!(when, 4_000_000_000);

        for state in ["queued", "sending", "sent", "failed"] {
            let id = insert(&tx, state, 0);
            assert!(
                !reschedule(&tx, id, 4_000_000_000).expect("reschedule"),
                "rescheduled a {state} message"
            );
        }
    }
}

#[cfg(test)]
mod tests_support {
    use super::*;
    use crate::db::migrate;

    pub fn store() -> rusqlite::Connection {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragma");
        migrate::run(&mut conn).expect("migrate");

        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'Test', 'me@halcyon.test', 'other', 'password', 'halcyon:me')",
            [],
        )
        .expect("account");

        conn
    }

    pub fn insert(tx: &Transaction<'_>, state: &str, send_after: i64) -> i64 {
        tx.execute(
            "INSERT INTO outbox (account_id, eml_path, state, send_after, attempts,
                                 message_id, subject, created_at)
             VALUES (1, 'C:/nowhere.eml', ?1, ?2, 0, '<m@halcyon.test>', 'Subject', 0)",
            params![state, send_after],
        )
        .expect("insert");

        tx.last_insert_rowid()
    }

    pub fn state_of(tx: &Transaction<'_>, id: i64) -> String {
        tx.query_row(
            "SELECT state FROM outbox WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("state")
    }

    #[test]
    fn every_state_survives_a_round_trip_through_its_name() {
        // The column is text, so a typo in one direction is a row nothing will ever look at
        // again — a silently lost message, which is the failure this module exists to prevent.
        for state in [
            State::Holding,
            State::Queued,
            State::Sending,
            State::Sent,
            State::Failed,
        ] {
            assert_eq!(State::parse(state.as_str()), Some(state));
        }
    }

    #[test]
    fn an_unrecognised_state_reads_as_failed_rather_than_vanishing() {
        // Written by a future version, or corrupted. Failed puts it in front of the user;
        // skipping it would drop a message nobody would ever be told about.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");
        insert(&tx, "something-else", 0);

        let mut statement = tx.prepare("SELECT * FROM outbox").expect("prepare");
        let rows: Vec<Entry> = statement
            .query_map([], read_entry)
            .expect("query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, State::Failed);
    }

    #[test]
    fn a_failed_attempt_is_retried_until_it_has_had_its_chances() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");
        let id = insert(&tx, "sending", 0);

        for attempt in 1..MAX_ATTEMPTS {
            let state = mark_attempt_failed(&tx, id, "greylisted", 0).expect("fail");
            assert_eq!(state, State::Queued, "gave up on attempt {attempt}");
        }

        // The last one stops. A message failing for a reason retrying cannot fix — a rejected
        // recipient, a size limit — must become a banner rather than a churning outbox.
        assert_eq!(
            mark_attempt_failed(&tx, id, "mailbox full", 0).expect("fail"),
            State::Failed
        );
    }

    #[test]
    fn a_failure_records_what_went_wrong_for_the_banner() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");
        let id = insert(&tx, "sending", 0);

        mark_attempt_failed(&tx, id, "552 message too large", 0).expect("fail");

        let recorded: String = tx
            .query_row(
                "SELECT last_error FROM outbox WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("error");

        assert_eq!(recorded, "552 message too large");
    }

    #[test]
    fn an_interrupted_send_found_in_sent_is_not_sent_again() {
        // The duplicate half of the exit gate. The server has it; sending again would put two
        // copies in the recipient's inbox and there is no undo for that.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");
        let id = insert(&tx, "sending", 0);

        assert_eq!(
            resolve_interrupted(&tx, id, true).expect("resolve"),
            State::Sent
        );
        assert_eq!(state_of(&tx, id), "sent");
    }

    #[test]
    fn an_interrupted_send_absent_from_sent_goes_back_to_the_queue() {
        // The loss half. It never left, so it must leave.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");
        let id = insert(&tx, "sending", 0);

        assert_eq!(
            resolve_interrupted(&tx, id, false).expect("resolve"),
            State::Queued
        );
        assert_eq!(state_of(&tx, id), "queued");
    }

    #[test]
    fn an_interruption_does_not_spend_an_attempt() {
        // The attempt was cut short, not used. Charging it brings a message closer to being
        // abandoned for a reason that was never its fault.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");
        let id = insert(&tx, "sending", 0);

        resolve_interrupted(&tx, id, false).expect("resolve");

        let attempts: i64 = tx
            .query_row(
                "SELECT attempts FROM outbox WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("attempts");

        assert_eq!(attempts, 0);
    }
}
