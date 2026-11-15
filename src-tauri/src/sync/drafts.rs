//! What happens to a draft after it is saved: where its copy lives, and letting it go.
//!
//! A draft exists in three places — its row in `draft`, its `.eml` under `drafts/`, and copies in
//! the account's Drafts mailbox, on the server and as rows synced from there. Nothing removed any
//! of them when the message was sent or thrown away (H4, 2026-11-14 audit), so a sent message
//! stayed in Drafts on every device and could be sent again from any of them. [`discard`] is the
//! one place that removes all three, for every path that ends a draft.

use std::path::{Path, PathBuf};

use rusqlite::{params, Transaction};

use crate::db::DbError;

use super::ops::{enqueue, Op};

/// Where a draft's bytes are kept between saves.
pub fn eml_path(root: &Path, draft_id: i64) -> PathBuf {
    root.join("drafts").join(format!("{draft_id}.eml"))
}

/// A `Message-ID` as drafts are keyed: in angle brackets, as the builder writes it.
pub fn bracketed(message_id: &str) -> String {
    format!("<{}>", bare(message_id))
}

/// A `Message-ID` as messages are keyed: without them, as the envelope stores it.
pub fn bare(message_id: &str) -> &str {
    message_id
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
}

/// Discards a draft everywhere, and returns the files to delete once the transaction commits.
///
/// * Its row goes, so nothing saves it again.
/// * Its synced copies in Drafts go from this store, so it leaves the list now rather than at
///   the next sync.
/// * Every server copy is queued for deletion, in each account that has one — the draft's own
///   account, and any whose Drafts holds a copy with its id. A save of it still queued is dropped.
///
/// A draft that never existed is not an error: discarding nothing is done.
pub fn discard(
    tx: &Transaction<'_>,
    root: &Path,
    message_id: &str,
) -> Result<Vec<PathBuf>, DbError> {
    let bare_id = bare(message_id).to_string();
    if bare_id.is_empty() {
        return Ok(Vec::new());
    }
    let keyed = bracketed(&bare_id);

    let rows: Vec<(i64, i64)> = {
        let mut statement =
            tx.prepare("SELECT id, account_id FROM draft WHERE message_id IN (?1, ?2)")?;
        let rows = statement
            .query_map(params![keyed, bare_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    let synced: Vec<(i64, i64)> = {
        let mut statement = tx.prepare(
            "SELECT message.id, message.account_id
               FROM message JOIN mailbox ON mailbox.id = message.mailbox_id
              WHERE mailbox.role = 'drafts' AND message.message_id = ?1",
        )?;
        let rows = statement
            .query_map(params![bare_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    let mut accounts: Vec<i64> = rows
        .iter()
        .chain(synced.iter())
        .map(|(_, account)| *account)
        .collect();
    accounts.sort_unstable();
    accounts.dedup();

    for (id, _) in &rows {
        tx.execute("DELETE FROM draft WHERE id = ?1", params![id])?;
    }

    let synced_ids: Vec<i64> = synced.iter().map(|(id, _)| *id).collect();
    crate::db::write::delete(tx, &synced_ids, true, None)?;

    for account in accounts {
        let drafts: Option<String> = tx
            .query_row(
                "SELECT remote_path FROM mailbox WHERE account_id = ?1 AND role = 'drafts'",
                params![account],
                |row| row.get(0),
            )
            .ok();

        // No Drafts mailbox known: nothing was ever appended anywhere this could name.
        if let Some(mailbox) = drafts {
            enqueue(
                tx,
                account,
                &Op::DeleteDraft {
                    mailbox,
                    message_id: keyed.clone(),
                },
            )?;
        }
    }

    Ok(rows.iter().map(|(id, _)| eml_path(root, *id)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> rusqlite::Connection {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open");
        crate::db::migrate::run(&mut conn).expect("migrate");

        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'T', 'me@t.test', 'other', 'password', 'halcyon:me')",
            [],
        )
        .expect("account");
        for (id, path, role) in [(1, "INBOX", "inbox"), (2, "Drafts", "drafts")] {
            conn.execute(
                "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
                 VALUES (?1, 1, ?2, ?2, ?3)",
                params![id, path, role],
            )
            .expect("mailbox");
        }
        conn
    }

    /// H4 in the 2026-11-14 audit: a sent or deleted message's draft stayed, here and on the
    /// server, so it kept reappearing in Drafts on every device and could be sent again.
    #[test]
    fn a_draft_is_let_go_of_everywhere() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        tx.execute(
            "INSERT INTO draft (id, account_id, message_id, updated_at)
             VALUES (5, 1, '<plan@t.test>', 0)",
            [],
        )
        .expect("draft");
        // Its synced copy in Drafts, and an unrelated message in the Inbox with the same id.
        for (id, mailbox) in [(10, 2), (11, 1)] {
            tx.execute(
                "INSERT INTO message (
                     id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
                     from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged,
                     is_junk, message_id
                 ) VALUES (?1, 1, ?2, ?1, 'Plan', 0, 0, 10, '', '', '', 0, 1, 0, 0, 'plan@t.test')",
                params![id, mailbox],
            )
            .expect("message");
        }
        // A save still on its way to the server.
        enqueue(
            &tx,
            1,
            &Op::AppendDraft {
                mailbox: "Drafts".into(),
                eml_path: "x".into(),
                replaces: None,
                message_id: "<plan@t.test>".into(),
            },
        )
        .expect("queue");

        let root = Path::new("C:/store");
        let files = discard(&tx, root, "plan@t.test").expect("discard");

        assert_eq!(files, vec![eml_path(root, 5)]);

        let drafts: i64 = tx
            .query_row("SELECT COUNT(*) FROM draft", [], |row| row.get(0))
            .expect("count");
        assert_eq!(drafts, 0);

        let left: Vec<i64> = {
            let mut statement = tx
                .prepare("SELECT id FROM message ORDER BY id")
                .expect("sql");
            let ids = statement
                .query_map([], |row| row.get(0))
                .expect("query")
                .collect::<rusqlite::Result<Vec<_>>>()
                .expect("ids");
            ids
        };
        assert_eq!(left, vec![11], "only the Drafts copy goes");

        let queued: Vec<Op> = crate::sync::ops::queued(&tx, 1)
            .expect("queue")
            .into_iter()
            .map(|(_, op)| op)
            .collect();
        assert_eq!(
            queued,
            vec![Op::DeleteDraft {
                mailbox: "Drafts".into(),
                message_id: "<plan@t.test>".into(),
            }],
            "the pending save is dropped and the server copies are deleted"
        );
    }

    /// H5 in the 2026-11-14 audit: every autosave queued its own append, each carrying the
    /// `replaces` known when it was queued — none, until the first had run. Three saves between
    /// pushes appended three copies, and the extra two looked like another device's edits.
    #[test]
    fn saves_queued_between_pushes_become_one() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        let save = |path: &str| Op::AppendDraft {
            mailbox: "Drafts".into(),
            eml_path: path.into(),
            replaces: None,
            message_id: "<plan@t.test>".into(),
        };

        enqueue(&tx, 1, &save("first")).expect("queue");
        enqueue(&tx, 1, &save("second")).expect("queue");
        // Another draft's save is its own and stays.
        enqueue(
            &tx,
            1,
            &Op::AppendDraft {
                mailbox: "Drafts".into(),
                eml_path: "other".into(),
                replaces: None,
                message_id: "<other@t.test>".into(),
            },
        )
        .expect("queue");
        enqueue(&tx, 1, &save("third")).expect("queue");

        let paths: Vec<String> = crate::sync::ops::queued(&tx, 1)
            .expect("queue")
            .into_iter()
            .filter_map(|(_, op)| match op {
                Op::AppendDraft { eml_path, .. } => Some(eml_path),
                _ => None,
            })
            .collect();

        assert_eq!(paths, vec!["other", "third"]);
    }

    #[test]
    fn discarding_a_draft_that_does_not_exist_is_nothing() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        let files = discard(&tx, Path::new("C:/store"), "<never@t.test>").expect("discard");

        assert!(files.is_empty());
        assert!(crate::sync::ops::queued(&tx, 1).expect("queue").is_empty());
    }
}
