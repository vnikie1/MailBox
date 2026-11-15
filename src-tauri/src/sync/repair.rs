//! One-off repairs of data an earlier version stored wrongly.
//!
//! Each repair runs once per install, in the background after launch, and records that it has
//! run in `setting`. A repair that ran on every launch would cost a scan of the whole store for
//! the life of the app to fix something that happened once.

use std::io::Read;
use std::path::Path;

use rusqlite::params;

use crate::db::{Db, DbError};

use super::envelope;

/// Recorded once [`cross_account_bodies`] has run to the end.
const CROSS_ACCOUNT_BODIES: &str = "repair.crossAccountBodies";

/// How much of a cached source is read to find its Message-ID.
///
/// Headers, not the message. A header block longer than this is not judged at all.
const HEADER_BYTES: u64 = 64 * 1024;

/// Every repair, in order. Errors are logged rather than returned: a repair that fails is
/// retried on the next launch, because it only records itself done when it finishes.
pub async fn run(db: Db) {
    match cross_account_bodies(&db).await {
        Ok(0) => {}
        Ok(cleared) => {
            tracing::info!(
                cleared,
                "repair: cleared bodies stored under the wrong message"
            );
        }
        Err(error) => tracing::warn!(%error, "repair: cross-account bodies did not finish"),
    }
}

/// Clears every downloaded body whose source is a different message from its row.
///
/// ## What put them there
///
/// Up to this version the reader's prefetch sent the next three rows' ids under the *selected*
/// row's account, and the core fetched each by its own UID and folder path from that account's
/// server. In All Inboxes, a smart mailbox or a search, rows from two accounts sit side by side,
/// so a row could be given whatever message the other account's server held at the same UID in
/// a folder of the same name — usually something, because `INBOX` exists everywhere and UIDs are
/// small integers. That message was stored as the row's body and its source cached as the row's
/// `.eml`.
///
/// ## How they are found
///
/// The row's Message-ID came from its own server's envelope; the cached source's comes from the
/// source. A source from the wrong message has the wrong one. Cleared rows are set back to "not
/// downloaded", so the right body is fetched the next time the message is shown, and the wrong
/// source file is queued for deletion like any other the store has let go of.
///
/// Only stores with two or more accounts can have been affected; any other is marked done
/// without a scan. Rows without a Message-ID are left alone, because there is nothing to compare.
pub async fn cross_account_bodies(db: &Db) -> Result<usize, DbError> {
    let candidates = db
        .read(|conn| {
            let done = conn
                .query_row(
                    "SELECT 1 FROM setting WHERE key = ?1",
                    params![CROSS_ACCOUNT_BODIES],
                    |_| Ok(()),
                )
                .is_ok();

            if done {
                return Ok(None);
            }

            let accounts: i64 =
                conn.query_row("SELECT COUNT(*) FROM account", [], |row| row.get(0))?;

            if accounts < 2 {
                return Ok(Some(Vec::new()));
            }

            let mut statement = conn.prepare(
                "SELECT id, message_id, raw_path FROM message
                  WHERE body_state = 'full'
                    AND raw_path IS NOT NULL
                    AND message_id IS NOT NULL",
            )?;

            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            Ok(Some(rows))
        })
        .await?;

    let Some(candidates) = candidates else {
        return Ok(0);
    };

    // The files are read off the async runtime: tens of thousands of small reads would otherwise
    // hold a worker for as long as the disk takes.
    let Ok(mismatched) = tokio::task::spawn_blocking(move || {
        candidates
            .into_iter()
            .filter(|(_, expected, path)| source_is_another_message(Path::new(path), expected))
            .map(|(id, ..)| id)
            .collect::<Vec<_>>()
    })
    .await
    else {
        // Not recorded as done, so the next launch tries again.
        return Ok(0);
    };

    db.write(move |tx| {
        for id in &mismatched {
            tx.execute(
                "UPDATE message
                    SET body_text = NULL,
                        body_html = NULL,
                        preview = NULL,
                        body_state = 'none',
                        raw_path = NULL,
                        has_attachment = 0,
                        attachment_names = ''
                  WHERE id = ?1",
                params![id],
            )?;
            tx.execute("DELETE FROM attachment WHERE message_id = ?1", params![id])?;
        }

        tx.execute(
            "INSERT INTO setting (key, value) VALUES (?1, '1')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![CROSS_ACCOUNT_BODIES],
        )?;

        Ok(mismatched.len())
    })
    .await
}

/// Whether the cached source at `path` is a message other than `expected`.
///
/// Anything that cannot be judged — a missing file, an unreadable one, a header block too long
/// to read in full — answers no. The cost of a wrong yes is one re-download; the cost of a wrong
/// no is a wrong body that stays, which is where things were already, so doubt leaves it alone.
fn source_is_another_message(path: &Path, expected: &str) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };

    let mut head = Vec::new();
    if file.take(HEADER_BYTES).read_to_end(&mut head).is_err() {
        return false;
    }

    let Ok((headers, ix_body)) = mailparse::parse_headers(&head) else {
        return false;
    };

    let found = envelope::from_headers(&headers).message_id;

    // Ran out of bytes before the headers ended and never saw a Message-ID: undecided.
    if found.is_none() && ix_body >= head.len() && head.len() as u64 == HEADER_BYTES {
        return false;
    }

    found.as_deref() != Some(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(dir: &Path, name: &str, message_id: Option<&str>) -> String {
        let path = dir.join(name);
        let id = message_id
            .map(|id| format!("Message-ID: <{id}>\r\n"))
            .unwrap_or_default();
        std::fs::write(
            &path,
            format!("From: a@example.test\r\n{id}Subject: s\r\n\r\nbody\r\n"),
        )
        .expect("write");
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_source_with_another_message_id_is_another_message() {
        let dir = tempfile::tempdir().expect("dir");

        let right = source(dir.path(), "1.eml", Some("one@example.test"));
        let wrong = source(dir.path(), "2.eml", Some("someone-else@example.test"));
        let none = source(dir.path(), "3.eml", None);

        assert!(!source_is_another_message(
            Path::new(&right),
            "one@example.test"
        ));
        assert!(source_is_another_message(
            Path::new(&wrong),
            "two@example.test"
        ));
        assert!(
            source_is_another_message(Path::new(&none), "three@example.test"),
            "a row with an id whose source has none was given some other message"
        );
    }

    /// A store with two accounts and one downloaded message in each.
    async fn two_accounts(dir: &Path) -> Db {
        let db = Db::open(&dir.join("halcyon.db")).expect("open");

        let good = source(dir, "good.eml", Some("good@example.test"));
        let swapped = source(dir, "swapped.eml", Some("not-mine@example.test"));

        db.write(move |tx| {
            for account in [1, 2] {
                tx.execute(
                    "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
                     VALUES (?1, 'T', ?2, 'other', 'password', ?2)",
                    params![account, format!("me{account}@t.test")],
                )?;
                tx.execute(
                    "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
                     VALUES (?1, ?1, 'INBOX', 'Inbox', 'inbox')",
                    params![account],
                )?;
            }

            for (id, account, message_id, path) in [
                (1, 1, "good@example.test", &good),
                (2, 2, "mine@example.test", &swapped),
            ] {
                tx.execute(
                    "INSERT INTO message (
                         id, account_id, mailbox_id, uid, subject, date_sent, date_received,
                         size, from_all, to_all, body_text, has_attachment, flag_seen,
                         flag_flagged, is_junk, message_id, body_state, raw_path, preview
                     ) VALUES (?1, ?2, ?2, 10, 'S', 0, 0, 10, 'a@b.test', '', 'stored text', 1,
                               0, 0, 0, ?3, 'full', ?4, 'stored preview')",
                    params![id, account, message_id, path],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");

        db
    }

    #[tokio::test]
    async fn the_repair_clears_only_the_body_that_belongs_to_another_message_and_runs_once() {
        let dir = tempfile::tempdir().expect("dir");
        let db = two_accounts(dir.path()).await;

        assert_eq!(cross_account_bodies(&db).await.expect("repair"), 1);

        let states: Vec<(i64, String, Option<String>, Option<String>)> = db
            .read(|conn| {
                let mut statement = conn.prepare(
                    "SELECT id, body_state, body_text, raw_path FROM message ORDER BY id",
                )?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .expect("read");

        assert_eq!(states[0].1, "full", "the right body is kept");
        assert_eq!(states[0].2.as_deref(), Some("stored text"));

        assert_eq!(states[1].1, "none", "the wrong body is to be fetched again");
        assert_eq!(states[1].2, None);
        assert_eq!(states[1].3, None);

        // Recorded as done: a second launch does not scan again.
        assert_eq!(cross_account_bodies(&db).await.expect("again"), 0);
    }

    #[tokio::test]
    async fn a_single_account_store_is_never_scanned() {
        let dir = tempfile::tempdir().expect("dir");
        let db = two_accounts(dir.path()).await;

        db.write(|tx| {
            tx.execute("DELETE FROM message WHERE account_id = 2", [])?;
            tx.execute("DELETE FROM mailbox WHERE account_id = 2", [])?;
            tx.execute("DELETE FROM account WHERE id = 2", [])?;
            // A mismatch the repair would clear, if it looked.
            tx.execute(
                "UPDATE message SET message_id = 'other@example.test' WHERE id = 1",
                [],
            )?;
            Ok(())
        })
        .await
        .expect("trim");

        assert_eq!(cross_account_bodies(&db).await.expect("repair"), 0);
    }

    #[test]
    fn what_cannot_be_judged_is_left_alone() {
        let dir = tempfile::tempdir().expect("dir");

        assert!(!source_is_another_message(
            &dir.path().join("missing.eml"),
            "x@example.test"
        ));

        // A header block longer than the read, with no Message-ID in what was read.
        let long = dir.path().join("long.eml");
        let mut raw = String::new();
        while raw.len() < HEADER_BYTES as usize + 100 {
            raw.push_str("X-Padding: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n");
        }
        std::fs::write(&long, raw).expect("write");

        assert!(!source_is_another_message(&long, "x@example.test"));
    }
}
