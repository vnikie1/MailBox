//! Bringing mail in from another client. docs/06 Phase 11.
//!
//! ## Where imported mail goes, and why not into an existing account
//!
//! Into a **local account** of its own — the equivalent of Mail's "On My Mac" — with syncing
//! turned off. Never into an account that syncs, and the reason is not tidiness.
//!
//! A synced mailbox's rows are keyed by `(mailbox_id, uid)`, and the UID belongs to the
//! server. `sync::persist::remove_missing` deletes local rows the server no longer lists, which
//! is how a message deleted in webmail disappears here. Imported messages have no server and
//! no real UID, so the next sync of that mailbox would find rows the server has never heard of
//! and delete every one of them. The user's twenty-year archive would import successfully and
//! vanish within the minute.
//!
//! A local account has `sync_enabled = 0`, which both `sync_all` and the IDLE watcher honour,
//! so nothing ever reconciles it against a server that does not exist.
//!
//! ## UIDs
//!
//! Assigned sequentially per mailbox as messages arrive. They mean nothing beyond being unique,
//! which is all the schema asks of them — but re-importing the same file therefore *duplicates*
//! rather than updating, because there is no stable server-side identity to key on. The UI says
//! so before it starts. Deduplicating on `Message-ID` was considered and rejected: it is absent
//! from a surprising amount of old mail, and forged in some of the rest, so it would silently
//! drop messages that are not duplicates at all.

use std::path::Path;

use rusqlite::{params, Transaction};

use crate::db::DbError;
use crate::sync::bodies;
use crate::sync::envelope;
use crate::sync::fetch::{Fetched, Flags};
use crate::sync::persist;

use super::thunderbird::MozillaFlags;

/// The provider id a local account carries. Not a server; there is nothing to connect to.
pub const LOCAL_PROVIDER: &str = "local";

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Imported {
    pub folders: usize,
    pub messages: usize,
    /// Messages the mbox reader refused as oversized.
    pub skipped: usize,
}

/// Finds the local account, creating it if this is the first import.
///
/// One per machine rather than one per import, so a second import lands beside the first
/// instead of scattering the user's archives across accounts that all look the same.
///
/// The `cred_ref` is a sentinel. The column is `NOT NULL` because every *server* account needs
/// a Credential Manager key, and standing rule 12 keeps the key here and the secret there — a
/// local account has neither, and this name is deliberately one that refers to no credential.
pub fn local_account(tx: &Transaction<'_>, display_name: &str) -> Result<i64, DbError> {
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM account WHERE provider = ?1 ORDER BY id LIMIT 1",
            params![LOCAL_PROVIDER],
            |row| row.get(0),
        )
        .ok();

    if let Some(id) = existing {
        return Ok(id);
    }

    tx.execute(
        "INSERT INTO account (
             display_name, email, provider, auth_kind, cred_ref, sort_order, sync_enabled
         ) VALUES (
             ?1, ?2, ?3, 'none', 'local:none',
             (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM account), 0
         )",
        params![display_name, "local@localhost", LOCAL_PROVIDER],
    )?;

    Ok(tx.last_insert_rowid())
}

/// Finds or creates a mailbox under an account, building any parent folders on the way.
///
/// `path` is `/`-separated. `Local Folders/Work/Projects` becomes three rows, the first two of
/// which may already exist from an earlier folder in the same import.
pub fn mailbox_for(tx: &Transaction<'_>, account_id: i64, path: &str) -> Result<i64, DbError> {
    let mut parent: Option<i64> = None;
    let mut walked = String::new();

    for segment in path.split('/').filter(|part| !part.is_empty()) {
        if !walked.is_empty() {
            walked.push('/');
        }
        walked.push_str(segment);

        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM mailbox WHERE account_id = ?1 AND remote_path = ?2",
                params![account_id, walked],
                |row| row.get(0),
            )
            .ok();

        parent = Some(match existing {
            Some(id) => id,
            None => {
                tx.execute(
                    "INSERT INTO mailbox (
                         account_id, remote_path, display_name, parent_id, role, sort_order
                     ) VALUES (
                         ?1, ?2, ?3, ?4, ?5,
                         (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM mailbox WHERE account_id = ?1)
                     )",
                    params![account_id, walked, segment, parent, role_for(segment)],
                )?;
                tx.last_insert_rowid()
            }
        });
    }

    parent.ok_or_else(|| DbError::Sqlite(rusqlite::Error::InvalidQuery))
}

/// Guesses a folder's role from its name, so an imported Inbox looks like an inbox.
///
/// Only the names the mail clients themselves use, matched case-insensitively. A guess wider
/// than this starts filing someone's folder called "Sent to accountant" as their Sent mailbox.
fn role_for(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "inbox" => Some("inbox"),
        "sent" | "sent messages" | "sent items" => Some("sent"),
        "drafts" => Some("drafts"),
        "trash" | "deleted items" | "deleted messages" => Some("trash"),
        "junk" | "spam" | "bulk mail" => Some("junk"),
        "archive" | "archives" => Some("archive"),
        _ => None,
    }
}

/// The next UID to hand out in a mailbox.
pub fn next_uid(tx: &Transaction<'_>, mailbox_id: i64) -> Result<u32, DbError> {
    let highest: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(uid), 0) FROM message WHERE mailbox_id = ?1",
            params![mailbox_id],
            |row| row.get(0),
        )
        .unwrap_or(0);

    Ok((highest as u32).saturating_add(1))
}

/// Writes one raw message into a mailbox, body and all.
///
/// Takes the same path a synced message takes — `persist::write_batch`, then the body cache,
/// then `bodies::persist` — rather than its own INSERT. Two writers for the same table is how
/// an imported message ends up missing from search: the FTS columns are filled by the writer,
/// and a second writer that forgot one would produce rows that exist and cannot be found.
pub fn write_message(
    tx: &Transaction<'_>,
    cache_root: &Path,
    account_id: i64,
    mailbox_id: i64,
    uid: u32,
    raw: &[u8],
    seen: Option<bool>,
) -> Result<Option<i64>, DbError> {
    let Ok(parsed) = mailparse::parse_mail(raw) else {
        // Not a message. Standing rule 13 says degrade visibly, and the visible part is the
        // count the caller reports — one file that is not mail should not stop an import of
        // fifty thousand that are.
        tracing::debug!(mailbox_id, uid, "imported item could not be parsed as MIME");
        return Ok(None);
    };

    let envelope = envelope::from_headers(&parsed);

    // Thunderbird records read state in the message itself; a PST records it in a MAPI property
    // the message does not carry. The caller supplies it when it knows better than the headers do.
    let flags = MozillaFlags::from_headers(raw);
    let seen = seen.unwrap_or(flags.seen);

    let references = parsed
        .headers
        .iter()
        .find(|header| header.get_key_ref().eq_ignore_ascii_case("References"))
        .map(|header| envelope::parse_references(&header.get_value()))
        .unwrap_or_default();

    // The Date header, since a file has no INTERNALDATE. Where it is missing or unparseable
    // this is 0, and the list sorts the message to the bottom rather than dropping it.
    let date = envelope.date_sent;

    let fetched = Fetched {
        uid,
        envelope,
        flags: Flags {
            seen,
            answered: flags.answered,
            flagged: flags.flagged,
            draft: false,
            deleted: flags.deleted,
        },
        size: raw.len() as u32,
        internal_date: date,
        modseq: None,
        gm_thrid: None,
        gm_msgid: None,
        references,
    };

    let written = persist::write_batch(tx, account_id, mailbox_id, &[fetched])?;
    let Some(message_id) = written.inserted_ids.first().copied() else {
        return Ok(None);
    };

    // The body cache, so the reader can render it and a reply can quote it exactly — the same
    // `.eml` on disk a synced message gets.
    let cached = bodies::write_cache(cache_root, account_id, message_id, raw).ok();
    let body = bodies::parse(raw);
    bodies::persist(tx, message_id, &body, cached.as_deref())?;

    Ok(Some(message_id))
}

/// Threads and recounts after an import. Run once at the end, not per message.
///
/// Threading a message needs the messages it replies to, and during an import those may not be
/// written yet — a folder is not in date order, and a reply can precede its parent in the file.
/// Running per message would thread each one against a partial mailbox and leave conversations
/// split in ways nothing later repairs.
pub fn finish(tx: &Transaction<'_>, account_id: i64, mailboxes: &[i64]) -> Result<(), DbError> {
    // Generous, because an import is the one moment the whole account is unthreaded.
    let mut remaining = persist::unthreaded_count(tx, account_id)?;
    while remaining > 0 {
        let done = persist::rethread(tx, account_id, 5_000)?;
        if done == 0 {
            break;
        }
        remaining = persist::unthreaded_count(tx, account_id)?;
    }

    for mailbox_id in mailboxes {
        persist::recount(tx, *mailbox_id)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn store() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open");
        crate::db::migrate::run(&mut conn).expect("migrate");
        conn
    }

    #[test]
    fn the_local_account_is_created_once_and_reused() {
        // A second import must land beside the first, not scatter archives across accounts
        // that all look the same.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        let first = local_account(&tx, "On My PC").expect("first");
        let second = local_account(&tx, "On My PC").expect("second");

        assert_eq!(first, second);

        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM account", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 1);
    }

    #[test]
    fn the_local_account_never_syncs() {
        // The whole safety argument. If this is ever 1, the next sync deletes every imported
        // message as one the server does not have.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");
        let id = local_account(&tx, "On My PC").expect("account");

        let enabled: i64 = tx
            .query_row(
                "SELECT sync_enabled FROM account WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("read");

        assert_eq!(enabled, 0);
    }

    #[test]
    fn the_local_account_holds_no_credential_reference() {
        // Standing rule 12. The column is NOT NULL, so it holds a sentinel that names no key
        // rather than something that looks like one.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");
        let id = local_account(&tx, "On My PC").expect("account");

        let cred: String = tx
            .query_row(
                "SELECT cred_ref FROM account WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("read");

        assert_eq!(cred, "local:none");
    }

    #[test]
    fn a_nested_path_becomes_a_tree_of_mailboxes() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");
        let account = local_account(&tx, "On My PC").expect("account");

        let leaf = mailbox_for(&tx, account, "Local Folders/Work/Projects").expect("mailbox");

        let (name, parent): (String, Option<i64>) = tx
            .query_row(
                "SELECT display_name, parent_id FROM mailbox WHERE id = ?1",
                params![leaf],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read");

        assert_eq!(name, "Projects");
        assert!(parent.is_some(), "the leaf has no parent");

        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM mailbox", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 3);
    }

    #[test]
    fn the_same_path_twice_is_the_same_mailbox() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");
        let account = local_account(&tx, "On My PC").expect("account");

        let first = mailbox_for(&tx, account, "Local Folders/Work").expect("first");
        let second = mailbox_for(&tx, account, "Local Folders/Work").expect("second");

        assert_eq!(first, second);
    }

    #[test]
    fn an_imported_inbox_is_recognised_as_one() {
        assert_eq!(role_for("Inbox"), Some("inbox"));
        assert_eq!(role_for("Sent Items"), Some("sent"));
        assert_eq!(role_for("Deleted Items"), Some("trash"));
        // And nothing wider. A folder named for what it holds is not a role.
        assert_eq!(role_for("Sent to accountant"), None);
        assert_eq!(role_for("Work"), None);
    }

    #[test]
    fn a_message_arrives_with_its_envelope_body_and_flags() {
        let mut conn = store();
        let cache = std::env::temp_dir().join("halcyon-import-one");
        let _ = std::fs::remove_dir_all(&cache);

        let tx = conn.transaction().expect("tx");
        let account = local_account(&tx, "On My PC").expect("account");
        let mailbox = mailbox_for(&tx, account, "Local Folders/Inbox").expect("mailbox");

        let raw = b"X-Mozilla-Status: 0001\r\n\
                    Message-ID: <one@example.test>\r\n\
                    From: Ada Lovelace <ada@example.test>\r\n\
                    To: vishal@example.test\r\n\
                    Subject: Re: the analytical engine\r\n\
                    Date: Thu, 27 Aug 2026 09:34:00 +0000\r\n\
                    Content-Type: text/plain\r\n\r\n\
                    It computes.\r\n";

        let id = write_message(&tx, &cache, account, mailbox, 1, raw, None)
            .expect("write")
            .expect("inserted");

        let (subject, base, from, seen, text): (String, String, String, i64, Option<String>) = tx
            .query_row(
                "SELECT subject, subject_base, from_addr, flag_seen, body_text
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
            .expect("read");

        assert_eq!(subject, "Re: the analytical engine");
        // Stripped for threading, so an imported reply threads with a synced original.
        assert_eq!(base, "the analytical engine");
        assert_eq!(from, "ada@example.test");
        assert_eq!(seen, 1, "the Thunderbird read flag was lost");
        assert!(text.unwrap_or_default().contains("It computes."));

        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn an_imported_message_is_searchable() {
        // The reason this goes through `persist::write_batch` rather than its own INSERT. The
        // FTS columns are filled by the writer; a second writer that forgot one would produce
        // messages that exist in the list and cannot be found.
        let mut conn = store();
        let cache = std::env::temp_dir().join("halcyon-import-search");
        let _ = std::fs::remove_dir_all(&cache);

        let tx = conn.transaction().expect("tx");
        let account = local_account(&tx, "On My PC").expect("account");
        let mailbox = mailbox_for(&tx, account, "Inbox").expect("mailbox");

        write_message(
            &tx,
            &cache,
            account,
            mailbox,
            1,
            b"From: ada@example.test\r\nSubject: analytical engine\r\n\r\nbody\r\n",
            None,
        )
        .expect("write");

        let hits: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM message_fts WHERE message_fts MATCH 'analytical'",
                [],
                |row| row.get(0),
            )
            .expect("search");

        assert_eq!(hits, 1);

        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn something_that_is_not_a_message_is_skipped_rather_than_fatal() {
        // One bad file must not stop an import of fifty thousand good ones.
        let mut conn = store();
        let cache = std::env::temp_dir().join("halcyon-import-junk");
        let tx = conn.transaction().expect("tx");
        let account = local_account(&tx, "On My PC").expect("account");
        let mailbox = mailbox_for(&tx, account, "Inbox").expect("mailbox");

        let written = write_message(&tx, &cache, account, mailbox, 1, &[0xff, 0xfe, 0x00], None);

        assert!(written.is_ok(), "a bad item ended the import");

        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn uids_do_not_collide_within_a_mailbox() {
        // `(mailbox_id, uid)` is unique. A second message handed the same UID would silently
        // update the first rather than insert.
        let mut conn = store();
        let cache = std::env::temp_dir().join("halcyon-import-uid");
        let tx = conn.transaction().expect("tx");
        let account = local_account(&tx, "On My PC").expect("account");
        let mailbox = mailbox_for(&tx, account, "Inbox").expect("mailbox");

        let first = next_uid(&tx, mailbox).expect("uid");
        for (offset, subject) in ["one", "two", "three"].iter().enumerate() {
            let raw = format!("From: a@x\r\nSubject: {subject}\r\n\r\nbody\r\n");
            let uid = first + offset as u32;
            write_message(&tx, &cache, account, mailbox, uid, raw.as_bytes(), None).expect("write");
        }

        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM message WHERE mailbox_id = ?1",
                params![mailbox],
                |row| row.get(0),
            )
            .expect("count");

        assert_eq!(count, 3);

        let _ = std::fs::remove_dir_all(&cache);
    }
}
