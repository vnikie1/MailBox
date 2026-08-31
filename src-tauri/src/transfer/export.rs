//! Taking mail back out. docs/06 Phase 11.
//!
//! ## Why this exists at all
//!
//! An email client that can only take mail in is a trap. Standing rule 16 is about not sending
//! anything anywhere; this is the other half of the same promise — the mail is the user's, it
//! is on their disk, and they can leave with it. docs/07's uninstall gate says nothing should
//! be left behind unless the user asks to keep it, and "keep it" is only meaningful if there is
//! a way to read what was kept without this app.
//!
//! ## Two formats, because they answer different questions
//!
//! * **mbox**, one file per mailbox. What Thunderbird, Apple Mail and most Unix tools import.
//!   One file is easier to move and to archive, and it is the format an import expects.
//! * **an `.eml` tree**, one file per message in a directory per mailbox. What Outlook and
//!   Windows Explorer understand, and what a person can search with the tools they already
//!   have — a `.eml` opens on a double click, and this app registers for them.
//!
//! ## What is exported is what is stored
//!
//! Only messages whose raw source is cached — `body_state = 'full'`. A message whose body has
//! never been downloaded exists here as an envelope and nothing more, and writing an envelope
//! into an mbox produces a file of headers with no content, which looks like data loss and is
//! indistinguishable from it after the fact. Those are counted and reported instead.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use crate::db::DbError;
use crate::platform::files::safe_file_name;

use super::mbox;

/// What an export produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exported {
    pub mailboxes: usize,
    pub messages: usize,
    /// Messages skipped because their source was never downloaded.
    pub without_body: usize,
    pub files: Vec<String>,
}

/// One message, as the export needs it.
struct Row {
    subject: Option<String>,
    from_addr: Option<String>,
    date_sent: i64,
    raw_path: Option<String>,
}

fn rows(conn: &Connection, mailbox_id: i64) -> Result<Vec<Row>, DbError> {
    let mut statement = conn.prepare(
        "SELECT id, subject, from_addr, date_sent, raw_path
           FROM message
          WHERE mailbox_id = ?1 AND flag_deleted = 0
          ORDER BY date_received, id",
    )?;

    let found = statement
        .query_map(params![mailbox_id], |row| {
            Ok(Row {
                subject: row.get(1)?,
                from_addr: row.get(2)?,
                date_sent: row.get(3)?,
                raw_path: row.get(4)?,
            })
        })?
        .filter_map(Result::ok)
        .collect();

    Ok(found)
}

/// The display name of a mailbox, safe to use as a file name.
fn mailbox_name(conn: &Connection, mailbox_id: i64) -> String {
    let name: String = conn
        .query_row(
            "SELECT display_name FROM mailbox WHERE id = ?1",
            params![mailbox_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| format!("mailbox-{mailbox_id}"));

    // Through the same defuser an attachment name goes through. A mailbox name comes from a
    // server, which makes it no more trustworthy than a filename in a message.
    safe_file_name(&name)
}

/// Writes one mailbox as a single mbox file into `directory`.
pub fn to_mbox(
    conn: &Connection,
    mailbox_id: i64,
    directory: &Path,
    into: &mut Exported,
) -> Result<PathBuf, DbError> {
    std::fs::create_dir_all(directory).map_err(DbError::from)?;

    let name = mailbox_name(conn, mailbox_id);
    let path = unique_path(directory, &name, "mbox");

    let file = std::fs::File::create(&path).map_err(DbError::from)?;
    let mut sink = BufWriter::new(file);

    for row in rows(conn, mailbox_id)? {
        let Some(raw) = read_raw(&row) else {
            into.without_body += 1;
            continue;
        };

        mbox::write_message(
            &mut sink,
            row.from_addr.as_deref().unwrap_or("-"),
            &mbox::separator_date(row.date_sent),
            &raw,
        )
        .map_err(DbError::from)?;

        into.messages += 1;
    }

    sink.flush().map_err(DbError::from)?;

    into.mailboxes += 1;
    into.files.push(path.to_string_lossy().to_string());

    Ok(path)
}

/// Writes one mailbox as a directory of `.eml` files.
pub fn to_eml_tree(
    conn: &Connection,
    mailbox_id: i64,
    directory: &Path,
    into: &mut Exported,
) -> Result<PathBuf, DbError> {
    let name = mailbox_name(conn, mailbox_id);
    let folder = unique_path(directory, &name, "");

    std::fs::create_dir_all(&folder).map_err(DbError::from)?;

    for (index, row) in rows(conn, mailbox_id)?.into_iter().enumerate() {
        let Some(raw) = read_raw(&row) else {
            into.without_body += 1;
            continue;
        };

        // Numbered as well as named. Two messages can share a subject — a daily report does so
        // for years — and the number keeps them apart and keeps the directory in date order in
        // Explorer, which sorts by name.
        let subject = row.subject.as_deref().unwrap_or("(no subject)");
        let stem = safe_file_name(&truncate(subject, 60));
        let file = folder.join(format!("{:05}-{stem}.eml", index + 1));

        std::fs::write(&file, &raw).map_err(DbError::from)?;
        into.messages += 1;
    }

    into.mailboxes += 1;
    into.files.push(folder.to_string_lossy().to_string());

    Ok(folder)
}

/// Reads a message's cached source.
fn read_raw(row: &Row) -> Option<Vec<u8>> {
    let path = row.raw_path.as_ref()?;
    std::fs::read(path).ok()
}

/// Cuts a string to `limit` characters, on a character boundary.
fn truncate(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

/// A path that does not already exist, by appending a number.
///
/// Two mailboxes can have the same display name — every account has an Inbox — and an export of
/// both would otherwise write the second over the first and report success.
fn unique_path(directory: &Path, stem: &str, extension: &str) -> PathBuf {
    let build = |suffix: String| -> PathBuf {
        if extension.is_empty() {
            directory.join(format!("{stem}{suffix}"))
        } else {
            directory.join(format!("{stem}{suffix}.{extension}"))
        }
    };

    let first = build(String::new());
    if !first.exists() {
        return first;
    }

    for attempt in 2..1000 {
        let candidate = build(format!(" ({attempt})"));
        if !candidate.exists() {
            return candidate;
        }
    }

    first
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store with one mailbox holding `count` messages whose sources are on disk.
    fn fixture(cache: &Path, count: usize, with_bodies: bool) -> (Connection, i64) {
        let mut conn = Connection::open_in_memory().expect("open");
        crate::db::migrate::run(&mut conn).expect("migrate");

        let tx = conn.transaction().expect("tx");
        let account = super::super::import::local_account(&tx, "On My PC").expect("account");
        let mailbox = super::super::import::mailbox_for(&tx, account, "Inbox").expect("mailbox");

        for index in 0..count {
            let raw = format!(
                "From: ada@example.test\r\nSubject: message {index}\r\n\r\nFrom the top\r\n"
            );
            if with_bodies {
                super::super::import::write_message(
                    &tx,
                    cache,
                    account,
                    mailbox,
                    index as u32 + 1,
                    raw.as_bytes(),
                )
                .expect("write");
            } else {
                tx.execute(
                    "INSERT INTO message (account_id, mailbox_id, uid, subject, date_sent,
                                          date_received, body_state)
                     VALUES (?1, ?2, ?3, ?4, 0, 0, 'headers')",
                    params![
                        account,
                        mailbox,
                        index as i64 + 1,
                        format!("message {index}")
                    ],
                )
                .expect("insert");
            }
        }

        tx.commit().expect("commit");
        (conn, mailbox)
    }

    #[test]
    fn an_exported_mbox_can_be_read_back() {
        // The round trip that matters: everything this writes, this project's own importer —
        // and Thunderbird's — has to be able to read.
        let cache = std::env::temp_dir().join("halcyon-export-mbox-cache");
        let out = std::env::temp_dir().join("halcyon-export-mbox-out");
        let _ = std::fs::remove_dir_all(&cache);
        let _ = std::fs::remove_dir_all(&out);

        let (conn, mailbox) = fixture(&cache, 3, true);
        let mut counts = Exported::default();
        let path = to_mbox(&conn, mailbox, &out, &mut counts).expect("export");

        assert_eq!(counts.messages, 3);

        let file = std::fs::File::open(&path).expect("open");
        let mut found = Vec::new();
        mbox::read(std::io::BufReader::new(file), |raw| {
            found.push(String::from_utf8_lossy(raw).to_string());
            Ok(())
        })
        .expect("read back");

        assert_eq!(found.len(), 3);
        // And the body survived the escaping round trip, which is the part that silently
        // corrupts if the writer and reader disagree.
        assert!(found[0].contains("From the top"));
        assert!(!found[0].contains(">From the top"));

        let _ = std::fs::remove_dir_all(&cache);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn an_eml_tree_is_one_file_per_message() {
        let cache = std::env::temp_dir().join("halcyon-export-eml-cache");
        let out = std::env::temp_dir().join("halcyon-export-eml-out");
        let _ = std::fs::remove_dir_all(&cache);
        let _ = std::fs::remove_dir_all(&out);

        let (conn, mailbox) = fixture(&cache, 4, true);
        let mut counts = Exported::default();
        let folder = to_eml_tree(&conn, mailbox, &out, &mut counts).expect("export");

        let files: Vec<_> = std::fs::read_dir(&folder)
            .expect("read dir")
            .filter_map(Result::ok)
            .collect();

        assert_eq!(files.len(), 4);
        assert_eq!(counts.messages, 4);
        assert!(files.iter().all(|f| f.path().extension().unwrap() == "eml"));

        let _ = std::fs::remove_dir_all(&cache);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn a_message_with_no_downloaded_body_is_counted_rather_than_written_empty() {
        // Writing an envelope with no content into an mbox produces a file of headers that
        // looks exactly like data loss, and is indistinguishable from it afterwards.
        let cache = std::env::temp_dir().join("halcyon-export-nobody-cache");
        let out = std::env::temp_dir().join("halcyon-export-nobody-out");
        let _ = std::fs::remove_dir_all(&out);

        let (conn, mailbox) = fixture(&cache, 2, false);
        let mut counts = Exported::default();
        to_mbox(&conn, mailbox, &out, &mut counts).expect("export");

        assert_eq!(counts.messages, 0);
        assert_eq!(counts.without_body, 2);

        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn exporting_two_mailboxes_of_the_same_name_does_not_overwrite_one_with_the_other() {
        // Every account has an Inbox. Without this the second export silently replaces the
        // first and reports success.
        let out = std::env::temp_dir().join("halcyon-export-clash");
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).expect("dirs");
        std::fs::write(out.join("Inbox.mbox"), "already here").expect("write");

        let second = unique_path(&out, "Inbox", "mbox");
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            "Inbox (2).mbox"
        );

        let _ = std::fs::remove_dir_all(&out);
    }
}
