//! How long a cached message source lives: until nothing needs it, and then no longer.
//!
//! A downloaded message is kept twice — parsed into its row, and as its raw source in
//! `bodies/<account>/<message>.eml`, which Reply quotes, Redirect resends and attachments are
//! decoded from. Until 1.0.1 nothing deleted the second copy. Removing an account purged its
//! rows, its search entries and its password and left every message it had downloaded on disk,
//! and Erase Deleted Items did the same to each message it erased. docs/04's _remove with purge_
//! meant all of it.
//!
//! ## How a source is let go
//!
//! - **A row that stops pointing at its file queues it.** Migration 0015's triggers put the old
//!   `raw_path` in `discarded_source` whenever a message row is deleted or its path replaced,
//!   from any of the places that delete messages, and from any added later.
//! - **The upkeep tick deletes what is queued**, so a discarded source is gone within a minute.
//! - **Removing an account sets its folder aside at once**, by renaming it, and deletes it after.
//! - **Once a launch, a sweep deletes what the queue never heard of**: sources left by versions
//!   before it existed, and a body whose row was deleted while it was being fetched — the engine
//!   writes the file before it records the path, so no trigger ever saw that one.
//!
//! ## Never a file a live row needs
//!
//! Ids are reused. `message.id` is a plain `INTEGER PRIMARY KEY`, so the message after the newest
//! is deleted gets its id, and its source the same file name. So a queued file is deleted only if
//! no row points at it again, and nothing here deletes a file written less than [`GRACE`] ago: a
//! file that new may belong to a row that is about to record it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusqlite::{params, Connection, Transaction};

use super::bodies;
use crate::db::{Db, DbError};

/// How recently written a source must not be, to be deleted. Ten minutes is far longer than any
/// fetch takes between writing a file and recording it, and nothing waits on the delete.
pub const GRACE: Duration = Duration::from_secs(10 * 60);

/// Queue rows looked at per read, so one purge never holds a connection for long.
const BATCH: i64 = 500;

/// Batches per purge. Twenty thousand files a minute drains even an Erase Deleted Items on a
/// Bin of years of mail in a few ticks, without one tick doing it all.
const BATCHES_PER_PURGE: usize = 40;

/// What a removed account's folder is renamed to while it is deleted: `<id>.discarded-<nanos>`.
const SET_ASIDE: &str = ".discarded-";

/// What one purge did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Purged {
    /// Files deleted, or found already gone.
    pub deleted: usize,
    /// Paths a live row points at again — a reused id whose body has been fetched since.
    pub claimed: usize,
    /// Files written too recently to touch, left queued for a later tick.
    pub waiting: usize,
}

/// What one sweep did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Swept {
    pub files: usize,
    pub folders: usize,
}

/// The decision about one queued path. All but `Waiting` take it off the queue.
enum Verdict {
    /// Deleted, or found already gone.
    Deleted,
    /// A live row points at it again.
    Claimed,
    /// Written too recently to touch.
    Waiting,
    /// Not deleted and not worth retrying every minute: outside the cache, or refused by
    /// Windows. The next launch's sweep looks again at anything in the cache.
    Dropped,
}

/// Deletes the queued sources nothing points at any more.
pub async fn purge_discarded(db: &Db, grace: Duration) -> Result<Purged, DbError> {
    let folder = bodies::sources_folder(db.folder());
    let mut total = Purged::default();
    let mut after = String::new();

    for _ in 0..BATCHES_PER_PURGE {
        let folder = folder.clone();
        let cursor = after.clone();

        // Decided and done on a reader: the files are deleted here, and only the queue rows go
        // through the writer, so a large purge never holds the write lock while it waits on disk.
        let (settled, last, purged, full) = db
            .read(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT path FROM discarded_source WHERE path > ?1 ORDER BY path LIMIT ?2",
                )?;
                let paths = statement
                    .query_map(params![cursor, BATCH], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;

                let full = paths.len() as i64 == BATCH;
                let last = paths.last().cloned().unwrap_or_default();
                let mut purged = Purged::default();
                let mut settled = Vec::new();

                for path in paths {
                    match verdict(conn, &folder, &path, grace)? {
                        Verdict::Deleted => {
                            purged.deleted += 1;
                            settled.push(path);
                        }
                        Verdict::Claimed => {
                            purged.claimed += 1;
                            settled.push(path);
                        }
                        Verdict::Dropped => settled.push(path),
                        Verdict::Waiting => purged.waiting += 1,
                    }
                }

                Ok((settled, last, purged, full))
            })
            .await?;

        total.deleted += purged.deleted;
        total.claimed += purged.claimed;
        total.waiting += purged.waiting;

        if !settled.is_empty() {
            db.write(move |tx| {
                for path in &settled {
                    tx.execute(
                        "DELETE FROM discarded_source WHERE path = ?1",
                        params![path],
                    )?;
                }
                Ok(())
            })
            .await?;
        }

        if !full {
            break;
        }
        after = last;
    }

    Ok(total)
}

fn verdict(
    conn: &Connection,
    folder: &Path,
    path: &str,
    grace: Duration,
) -> Result<Verdict, DbError> {
    let file = Path::new(path);

    // Only ever a file in this store's own cache. A path anywhere else — a store copied from
    // another folder still names the old one — is dropped from the queue, never deleted.
    let Some(id) = source_id(folder, file) else {
        tracing::warn!("a discarded source outside the cache was left alone");
        return Ok(Verdict::Dropped);
    };

    let claimed: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM message WHERE id = ?1 AND raw_path = ?2)",
        params![id, path],
        |row| row.get(0),
    )?;
    if claimed {
        return Ok(Verdict::Claimed);
    }

    match std::fs::metadata(file) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Verdict::Deleted),
        Err(error) => {
            tracing::warn!(%error, "could not read a discarded source; the next launch's sweep will retry");
            return Ok(Verdict::Dropped);
        }
        Ok(metadata) if is_fresh(&metadata, grace) => return Ok(Verdict::Waiting),
        Ok(_) => {}
    }

    match std::fs::remove_file(file) {
        Ok(()) => Ok(Verdict::Deleted),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Verdict::Deleted),
        // Not retried every minute: a file Windows will not let go of — open in another program —
        // would otherwise log forever. The next launch's sweep tries again.
        Err(error) => {
            tracing::warn!(%error, "could not delete a discarded source; the next launch's sweep will retry");
            Ok(Verdict::Dropped)
        }
    }
}

/// The message id a cache file is named for, if `file` is `<folder>/<account>/<message>.eml`.
fn source_id(folder: &Path, file: &Path) -> Option<i64> {
    let account = file.parent()?;
    if account.parent()? != folder || !is_number(account.file_name()?.to_str()?) {
        return None;
    }

    let stem = file.file_name()?.to_str()?.strip_suffix(".eml")?;
    if !is_number(stem) {
        return None;
    }
    stem.parse().ok()
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Written within `grace` — or dated in the future, which a clock that moved can do and which
/// is no reason to delete anything.
fn is_fresh(metadata: &std::fs::Metadata, grace: Duration) -> bool {
    let Ok(modified) = metadata.modified() else {
        return true;
    };

    match SystemTime::now().duration_since(modified) {
        Ok(age) => age < grace,
        Err(_) => true,
    }
}

/// Moves a removed account's sources out of the way, for the caller to delete.
///
/// Renamed first, which is instant, and deleted after, which for a large account is not. An
/// account added straight back — the same address signing in again — can get the same id, and
/// must start with an empty folder rather than one being deleted underneath it.
pub fn set_aside_account(root: &Path, account_id: i64) -> std::io::Result<Option<PathBuf>> {
    let folder = bodies::account_folder(root, account_id);
    if !folder.exists() {
        return Ok(None);
    }

    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let aside = bodies::sources_folder(root).join(format!("{account_id}{SET_ASIDE}{nanos}"));

    std::fs::rename(&folder, &aside)?;
    Ok(Some(aside))
}

/// Forgets the queued paths under `folder`, once the folder itself has been set aside.
///
/// Removing an account queues every source it had, through the trigger; with the folder renamed
/// there is nothing at any of those paths to delete. A prefix match by length rather than
/// `LIKE`, in which `_` and `%` would be wildcards and a user name can contain either.
pub fn forget_queued_under(tx: &Transaction<'_>, folder: &Path) -> Result<usize, DbError> {
    let mut prefix = folder.to_string_lossy().to_string();
    prefix.push(std::path::MAIN_SEPARATOR);

    Ok(tx.execute(
        "DELETE FROM discarded_source WHERE substr(path, 1, length(?1)) = ?1",
        params![prefix],
    )?)
}

/// Deletes the sources the queue never heard of. Once a launch.
pub async fn sweep(db: &Db, grace: Duration) -> Result<Swept, DbError> {
    let root = db.folder().to_path_buf();
    db.read(move |conn| sweep_folder(conn, &root, grace)).await
}

fn sweep_folder(conn: &Connection, root: &Path, grace: Duration) -> Result<Swept, DbError> {
    let folder = bodies::sources_folder(root);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Ok(Swept::default());
    };

    // Which account each message whose source is recorded belongs to. Compared by account and
    // id, not by the whole path, so a store moved to another folder keeps its sources.
    let mut recorded: HashMap<i64, i64> = HashMap::new();
    {
        let mut statement =
            conn.prepare("SELECT id, account_id FROM message WHERE raw_path IS NOT NULL")?;
        let rows =
            statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
        for row in rows {
            let (id, account) = row?;
            recorded.insert(id, account);
        }
    }

    let accounts: std::collections::HashSet<i64> = {
        let mut statement = conn.prepare("SELECT id FROM account")?;
        let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };

    let mut swept = Swept::default();

    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();

        // A removed account's folder, set aside and not finished deleting: the app closed first.
        if name
            .split_once(SET_ASIDE)
            .is_some_and(|(account, stamp)| is_number(account) && is_number(stamp))
        {
            if std::fs::remove_dir_all(&path).is_ok() {
                swept.folders += 1;
            }
            continue;
        }

        // Anything else that is not an account's folder is not ours to judge.
        if !is_number(&name) {
            continue;
        }
        let Ok(account) = name.parse::<i64>() else {
            continue;
        };

        let Ok(files) = std::fs::read_dir(&path) else {
            continue;
        };
        for file in files.flatten() {
            let file_name = file.file_name().to_string_lossy().to_string();
            let Some(id) = file_name
                .strip_suffix(".eml")
                .filter(|stem| is_number(stem))
                .and_then(|stem| stem.parse::<i64>().ok())
            else {
                continue;
            };

            if recorded.get(&id) == Some(&account) {
                continue;
            }

            let file_path = file.path();
            match file.metadata() {
                Ok(metadata) if !is_fresh(&metadata, grace) => {}
                _ => continue,
            }

            match std::fs::remove_file(&file_path) {
                Ok(()) => swept.files += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => tracing::warn!(%error, "could not delete a leftover source"),
            }
        }

        // The folder of an account that no longer exists goes once it is empty. An existing
        // account's is left even when empty, because the engine may be about to write into it.
        if !accounts.contains(&account) && std::fs::remove_dir(&path).is_ok() {
            swept.folders += 1;
        }
    }

    Ok(swept)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (Db, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = Db::open(&dir.path().join("halcyon.db")).expect("open");
        (db, dir)
    }

    fn account(conn: &Connection, id: i64) {
        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (?1, 'T', ?2, 'other', 'password', ?3)",
            params![id, format!("me{id}@t.test"), format!("halcyon:me{id}")],
        )
        .expect("account");
        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (?1, ?1, 'INBOX', 'Inbox', 'inbox')",
            params![id],
        )
        .expect("mailbox");
    }

    /// A message with a cached source, written where the engine would write it.
    fn message(conn: &Connection, root: &Path, account: i64, id: i64) -> PathBuf {
        let path =
            bodies::write_cache(root, account, id, b"Subject: hi\r\n\r\nbody").expect("write");
        conn.execute(
            "INSERT INTO message (id, account_id, mailbox_id, uid, date_sent, date_received,
                                  body_state, raw_path)
             VALUES (?1, ?2, ?2, ?1, 0, 0, 'full', ?3)",
            params![id, account, path.to_string_lossy()],
        )
        .expect("message");
        path
    }

    fn queued(conn: &Connection) -> Vec<String> {
        let mut statement = conn
            .prepare("SELECT path FROM discarded_source ORDER BY path")
            .expect("prepare");
        statement
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("rows")
    }

    #[test]
    fn deleting_a_message_queues_its_source_and_nothing_else_does() {
        let mut conn = Connection::open_in_memory().expect("open");
        crate::db::migrate::run(&mut conn).expect("migrate");
        let root = tempfile::tempdir().expect("temp dir");
        account(&conn, 1);

        let kept = message(&conn, root.path(), 1, 5);
        let deleted = message(&conn, root.path(), 1, 6);
        conn.execute(
            "INSERT INTO message (id, account_id, mailbox_id, uid, date_sent, date_received)
             VALUES (7, 1, 1, 7, 0, 0)",
            [],
        )
        .expect("headers only");

        // A body fetched again writes the same path: nothing is let go.
        conn.execute(
            "UPDATE message SET raw_path = ?1 WHERE id = 5",
            params![kept.to_string_lossy()],
        )
        .expect("refetch");
        conn.execute("DELETE FROM message WHERE id IN (6, 7)", [])
            .expect("delete");

        assert_eq!(queued(&conn), vec![deleted.to_string_lossy().to_string()]);

        // A row that stops pointing at its file lets it go.
        conn.execute("UPDATE message SET raw_path = NULL WHERE id = 5", [])
            .expect("forget");
        assert_eq!(queued(&conn).len(), 2);
    }

    #[tokio::test]
    async fn a_purge_deletes_what_nothing_points_at() {
        let (db, _dir) = store();
        let root = db.folder().to_path_buf();

        let (kept, gone) = db
            .write(move |tx| {
                account(tx, 1);
                let kept = message(tx, &root, 1, 5);
                let gone = message(tx, &root, 1, 6);
                tx.execute("DELETE FROM message WHERE id = 6", [])?;
                Ok((kept, gone))
            })
            .await
            .expect("seed");

        let purged = purge_discarded(&db, Duration::ZERO).await.expect("purge");

        assert_eq!(purged.deleted, 1);
        assert!(
            !gone.exists(),
            "the deleted message's source is still on disk"
        );
        assert!(kept.exists(), "a live message's source went with it");
        assert!(db
            .read(|conn| Ok(queued(conn)))
            .await
            .expect("read")
            .is_empty());
    }

    #[tokio::test]
    async fn a_reused_id_keeps_the_file_it_has_written_since() {
        // The id after the newest is handed out again. If the new message's body has been
        // fetched by the time the purge runs, the file at that path is the new message's.
        let (db, _dir) = store();
        let root = db.folder().to_path_buf();

        let path = db
            .write(move |tx| {
                account(tx, 1);
                message(tx, &root, 1, 9);
                tx.execute("DELETE FROM message WHERE id = 9", [])?;
                // The next message gets id 9, and its body is written to the same file.
                Ok(message(tx, &root, 1, 9))
            })
            .await
            .expect("seed");

        let purged = purge_discarded(&db, Duration::ZERO).await.expect("purge");

        assert_eq!(purged.claimed, 1);
        assert!(path.exists(), "the purge deleted the new message's source");
        assert!(db
            .read(|conn| Ok(queued(conn)))
            .await
            .expect("read")
            .is_empty());
    }

    #[tokio::test]
    async fn a_file_written_moments_ago_waits() {
        let (db, _dir) = store();
        let root = db.folder().to_path_buf();

        let path = db
            .write(move |tx| {
                account(tx, 1);
                let path = message(tx, &root, 1, 5);
                tx.execute("DELETE FROM message WHERE id = 5", [])?;
                Ok(path)
            })
            .await
            .expect("seed");

        let purged = purge_discarded(&db, Duration::from_secs(3600))
            .await
            .expect("purge");

        assert_eq!(purged.waiting, 1);
        assert!(path.exists());
        assert_eq!(
            db.read(|conn| Ok(queued(conn))).await.expect("read").len(),
            1
        );
    }

    #[tokio::test]
    async fn a_path_outside_the_cache_is_never_deleted() {
        let (db, _dir) = store();
        let elsewhere = tempfile::tempdir().expect("temp dir");
        let foreign = elsewhere.path().join("5.eml");
        std::fs::write(&foreign, b"not ours").expect("write");

        let path = foreign.to_string_lossy().to_string();
        db.write(move |tx| {
            tx.execute(
                "INSERT INTO discarded_source (path) VALUES (?1)",
                params![path],
            )?;
            Ok(())
        })
        .await
        .expect("queue");

        purge_discarded(&db, Duration::ZERO).await.expect("purge");

        assert!(
            foreign.exists(),
            "a file outside the store's cache was deleted"
        );
        assert!(db
            .read(|conn| Ok(queued(conn)))
            .await
            .expect("read")
            .is_empty());
    }

    #[tokio::test]
    async fn a_removed_account_is_set_aside_and_its_queue_forgotten() {
        let (db, _dir) = store();
        let root = db.folder().to_path_buf();

        let seeded = root.clone();
        db.write(move |tx| {
            account(tx, 3);
            message(tx, &seeded, 3, 1);
            message(tx, &seeded, 3, 2);
            crate::accounts::store::remove(tx, 3)?;
            Ok(())
        })
        .await
        .expect("seed");
        assert_eq!(
            db.read(|conn| Ok(queued(conn))).await.expect("read").len(),
            2
        );

        let aside = set_aside_account(&root, 3)
            .expect("set aside")
            .expect("a folder");
        let folder = bodies::account_folder(&root, 3);
        let forgotten = db
            .write(move |tx| forget_queued_under(tx, &folder))
            .await
            .expect("forget");

        assert_eq!(forgotten, 2);
        assert!(!bodies::account_folder(&root, 3).exists());
        assert!(aside.join("1.eml").exists(), "set aside, not yet deleted");

        // The account comes straight back with the same id: its folder starts empty.
        bodies::write_cache(&root, 3, 1, b"new").expect("write");
        assert_eq!(
            std::fs::read(bodies::cache_path(&root, 3, 1)).expect("read"),
            b"new"
        );
    }

    #[test]
    fn the_queue_prefix_is_matched_exactly() {
        let mut conn = Connection::open_in_memory().expect("open");
        crate::db::migrate::run(&mut conn).expect("migrate");
        let folder = Path::new(r"C:\Users\a_b\bodies\1");
        for path in [
            r"C:\Users\a_b\bodies\1\5.eml",
            r"C:\Users\a_b\bodies\10\5.eml",
            r"C:\Users\aXb\bodies\1\6.eml",
        ] {
            conn.execute(
                "INSERT INTO discarded_source (path) VALUES (?1)",
                params![path],
            )
            .expect("queue");
        }

        let tx = conn.transaction().expect("tx");
        assert_eq!(forget_queued_under(&tx, folder).expect("forget"), 1);
        tx.commit().expect("commit");

        assert_eq!(
            queued(&conn),
            vec![
                r"C:\Users\aXb\bodies\1\6.eml".to_string(),
                r"C:\Users\a_b\bodies\10\5.eml".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn the_sweep_deletes_leftovers_and_keeps_what_rows_need() {
        let (db, _dir) = store();
        let root = db.folder().to_path_buf();
        let folder = bodies::sources_folder(&root);

        let seeded = root.clone();
        let kept = db
            .write(move |tx| {
                account(tx, 1);
                let kept = message(tx, &seeded, 1, 5);
                // A row whose id was reused and whose body has not been fetched again.
                tx.execute(
                    "INSERT INTO message (id, account_id, mailbox_id, uid, date_sent, date_received)
                     VALUES (7, 1, 1, 7, 0, 0)",
                    [],
                )?;
                Ok(kept)
            })
            .await
            .expect("seed");

        // Left by a version before the queue: an erased message, a stale file under a reused
        // id, a removed account's whole folder, and one set aside by a launch that closed first.
        let erased = bodies::write_cache(&root, 1, 6, b"erased").expect("write");
        let stale = bodies::write_cache(&root, 1, 7, b"the old message 7").expect("write");
        let removed = bodies::write_cache(&root, 9, 1, b"removed account").expect("write");
        let aside = folder.join("4.discarded-123");
        std::fs::create_dir_all(&aside).expect("mkdir");
        std::fs::write(aside.join("1.eml"), b"set aside").expect("write");

        // Not ours to judge.
        std::fs::write(folder.join("notes.txt"), b"x").expect("write");
        std::fs::write(bodies::account_folder(&root, 1).join("readme.txt"), b"x").expect("write");

        let swept = sweep(&db, Duration::ZERO).await.expect("sweep");

        assert!(kept.exists(), "a live message's source was swept");
        assert!(!erased.exists() && !stale.exists() && !removed.exists());
        assert!(!bodies::account_folder(&root, 9).exists());
        assert!(!aside.exists());
        assert!(folder.join("notes.txt").exists());
        assert!(bodies::account_folder(&root, 1).join("readme.txt").exists());
        assert_eq!(
            swept,
            Swept {
                files: 3,
                folders: 2
            }
        );
    }

    #[tokio::test]
    async fn the_sweep_spares_what_was_written_moments_ago() {
        let (db, _dir) = store();
        let root = db.folder().to_path_buf();
        let fresh = bodies::write_cache(&root, 1, 6, b"just fetched").expect("write");

        let swept = sweep(&db, Duration::from_secs(3600)).await.expect("sweep");

        assert!(fresh.exists());
        assert_eq!(swept.files, 0);
    }
}
