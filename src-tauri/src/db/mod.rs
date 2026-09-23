//! The local store. docs/03-architecture.md §3.
//!
//! Two access paths, deliberately asymmetric:
//!
//! * **Writes go through a single actor task** owning one connection, serialised, each job
//!   wrapped in a transaction. SQLite allows exactly one writer at a time; serialising in
//!   front of it turns `SQLITE_BUSY` from a runtime error everyone has to handle into a
//!   queue nobody has to think about.
//! * **Reads use a connection pool** on blocking threads. WAL means readers never block the
//!   writer and the writer never blocks readers, so the list can page while a sync writes.
//!
//! Every statement in this module and its children is parameterised. There is no string
//! interpolation into SQL anywhere, which docs/06 makes a hard constraint — mail bodies and
//! search terms are attacker-controlled text and this is a store that will hold someone's
//! entire private correspondence.

pub mod migrate;

pub mod model;
pub mod query;
#[cfg(test)]
mod tests_queries;
pub mod write;

use std::path::{Path, PathBuf};

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, Transaction};
use tokio::sync::{mpsc, oneshot};

/// How many readers may be in flight. The list, the reader pane and a background count can
/// all be running at once; beyond a handful the disk is the limit, not the pool.
const READER_POOL_SIZE: u32 = 4;

/// Queue depth in front of the writer. Deep enough that a burst of optimistic mutations
/// never blocks the UI thread, shallow enough that a wedged writer is noticed.
const WRITE_QUEUE_DEPTH: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("connection pool: {0}")]
    Pool(#[from] r2d2::Error),

    #[error("migration {version} ({name}) failed: {source}")]
    Migration {
        version: i64,
        name: String,
        #[source]
        source: rusqlite::Error,
    },

    #[error("the database writer has stopped")]
    WriterGone,

    /// A value could not be turned into the text a column stores.
    ///
    /// Its own variant rather than a stringly-typed `Sqlite` error because the caller can act
    /// on it: this is our bug, not the database's, and retrying it in a loop cannot help.
    #[error("could not encode {what}: {detail}")]
    Encode { what: &'static str, detail: String },

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

type WriteJob = Box<dyn FnOnce(&mut Connection) + Send>;

/// A handle to the store. Cheap to clone; every clone talks to the same writer.
#[derive(Clone)]
pub struct Db {
    readers: Pool<SqliteConnectionManager>,
    writer: mpsc::Sender<WriteJob>,
    folder: std::sync::Arc<PathBuf>,
}

/// Where the mail store lives.
///
/// `%LOCALAPPDATA%`, not `%APPDATA%`: a mail store is gigabytes and roaming profiles copy
/// themselves between machines at sign-in. Tauri derives this directory from the bundle
/// identifier, and computing it the same way here means the app and the `seed` binary agree
/// without one having to ask the other.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    base.join("com.uniki.halcyon").join("halcyon.db")
}

/// PRAGMAs every connection needs, reader or writer. docs/03 §3.
fn configure(conn: &Connection) -> Result<(), rusqlite::Error> {
    // WAL is what lets readers and the writer run at once; without it the list stalls
    // every time a sync commits.
    conn.pragma_update(None, "journal_mode", "WAL")?;

    // NORMAL rather than FULL: in WAL mode this risks losing only the last transaction on
    // a power cut, not corruption, and FULL costs an fsync per commit — which a sync
    // writing thousands of messages would feel.
    conn.pragma_update(None, "synchronous", "NORMAL")?;

    conn.pragma_update(None, "foreign_keys", "ON")?;

    // Belt and braces behind the writer actor: a stray direct write should wait rather
    // than fail. Five seconds is far longer than any transaction here should take.
    conn.busy_timeout(std::time::Duration::from_secs(5))?;

    // Keeps the temp b-trees an ORDER BY or a large FTS query needs off the disk.
    conn.pragma_update(None, "temp_store", "MEMORY")?;

    // The page cache, set deliberately rather than left to the default.
    //
    // SQLite's default is -2000, which is 2MB **per connection**. With `READER_POOL_SIZE`
    // readers plus the writer that is a ceiling of six times the default, arrived at by
    // multiplying a number nobody chose by however many connections happen to exist. The
    // twelve-hour soak measured the result: memory flat for 200 minutes, then a single 7MB step
    // as the pool warmed and the caches filled, settling +21.2% above where it started. That
    // passed its 25% threshold, and passing a threshold by accident is not the same as knowing
    // what the number is.
    //
    // Negative means kibibytes rather than pages, so this is independent of `page_size`. 8MB
    // per connection is larger than the default on purpose: the mail store is the whole point
    // of the app and the queries that matter — the list, a search over 100,000 messages — are
    // exactly the ones a bigger cache helps. Five connections gives a bounded 40MB, which is
    // the number to reach for if this ever needs to come down on a smaller machine.
    conn.pragma_update(None, "cache_size", -8 * 1024)?;

    unicode_lower(conn)?;

    Ok(())
}

/// Replaces SQLite's `lower()` with one that folds the whole of Unicode.
///
/// SQLite's built-in converts **A-Z and nothing else** — documented behaviour, not a bug in it —
/// so `lower('JOSÉ')` is `'josÉ'`. Every case-insensitive comparison in this app pairs that
/// against a needle lowercased in Rust by `to_lowercase`, which folds properly, and the two
/// therefore stopped agreeing at the first accented capital.
///
/// It broke matching in two places that had no idea they were related:
///
/// * **Search.** `from:`, `to:`, `subject:` and `mailbox:` compile to `LOWER(column) LIKE ?`
///   against a folded needle, so searching for a name written with any non-ASCII capital found
///   nothing at all.
/// * **Rules and smart mailboxes.** These are evaluated twice — in SQL when the mailbox is
///   listed, and in memory when a message arrives. The in-memory half uses `to_lowercase`, and
///   its comment says it does so deliberately, "or the two halves disagree on every capital
///   letter". They agreed for ASCII and parted company on everything else, so a rule could file
///   a message on arrival that its own smart mailbox then refused to list.
///
/// Overriding the built-in rather than adding a differently-named function, because the point is
/// that every existing `LOWER(` becomes correct without each caller having to know. All of them
/// here are case-insensitive *matches*; none wants ASCII-only folding.
///
/// Marked deterministic so SQLite may still use it in an index expression.
fn unicode_lower(conn: &Connection) -> Result<(), rusqlite::Error> {
    use rusqlite::functions::FunctionFlags;

    conn.create_scalar_function(
        "lower",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |context| {
            // NULL in, NULL out, as the built-in does. Callers wrap in COALESCE where they
            // care, and changing that here would alter what those comparisons mean.
            match context.get_raw(0).as_str_or_null()? {
                Some(text) => Ok(Some(text.to_lowercase())),
                None => Ok(None),
            }
        },
    )
}

impl Db {
    /// Opens the store at `path`, creating and migrating it if needed.
    pub fn open(path: &Path) -> Result<Self, DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Migrations run on a connection of their own, before either the pool or the
        // writer exists, so nothing can read a half-migrated schema.
        let mut conn = Connection::open(path)?;
        configure(&conn)?;
        migrate::run(&mut conn)?;
        drop(conn);

        let manager = SqliteConnectionManager::file(path).with_init(|conn| configure(conn));
        let readers = Pool::builder().max_size(READER_POOL_SIZE).build(manager)?;

        let (sender, mut receiver) = mpsc::channel::<WriteJob>(WRITE_QUEUE_DEPTH);

        let mut writer_conn = Connection::open(path)?;
        configure(&writer_conn)?;

        // A dedicated OS thread rather than a tokio task: every job here is blocking
        // SQLite work, and parking a runtime worker thread on it for the life of the
        // process is what starves everything else.
        std::thread::Builder::new()
            .name("halcyon-db-writer".into())
            .spawn(move || {
                while let Some(job) = receiver.blocking_recv() {
                    job(&mut writer_conn);
                }
                tracing::debug!("database writer stopped");
            })?;

        Ok(Self {
            readers,
            writer: sender,
            folder: std::sync::Arc::new(path.parent().map(Path::to_path_buf).unwrap_or_default()),
        })
    }

    /// The folder the store is in, which is where the files that belong to it go: the cached
    /// message sources among them.
    ///
    /// Asked of the store rather than of `default_path`, because a store opened somewhere else —
    /// every rig test opens one in a temporary folder — would otherwise write its cache into the
    /// user's, under message ids that name the user's own messages there.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Runs a read on a pooled connection, off the async runtime.
    pub async fn read<T, F>(&self, job: F) -> Result<T, DbError>
    where
        F: FnOnce(&Connection) -> Result<T, DbError> + Send + 'static,
        T: Send + 'static,
    {
        let pool = self.readers.clone();

        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            job(&conn)
        })
        .await
        .map_err(|_| DbError::WriterGone)?
    }

    /// Queues a write. The closure runs inside a transaction that commits on `Ok` and
    /// rolls back on `Err`, so a half-applied mutation is not representable.
    pub async fn write<T, F>(&self, job: F) -> Result<T, DbError>
    where
        F: FnOnce(&Transaction<'_>) -> Result<T, DbError> + Send + 'static,
        T: Send + 'static,
    {
        let (reply, wait) = oneshot::channel();

        let boxed: WriteJob = Box::new(move |conn| {
            let outcome = (|| {
                let tx = conn.transaction()?;
                let value = job(&tx)?;
                tx.commit()?;
                Ok(value)
            })();

            // The receiver is gone only if the caller was cancelled; the transaction has
            // already committed or rolled back either way, so there is nothing to undo.
            let _ = reply.send(outcome);
        });

        self.writer
            .send(boxed)
            .await
            .map_err(|_| DbError::WriterGone)?;

        wait.await.map_err(|_| DbError::WriterGone)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An in-memory store is not usable here — the pool and the writer open the file
    /// separately, and `:memory:` would give each its own empty database. A temp file is
    /// the only honest way to test the real arrangement.
    pub(crate) fn temp_db() -> (Db, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = Db::open(&dir.path().join("test.db")).expect("open");
        (db, dir)
    }

    #[tokio::test]
    async fn opens_migrates_and_round_trips() {
        let (db, _dir) = temp_db();

        db.write(|tx| {
            tx.execute(
                "INSERT INTO setting (key, value) VALUES (?1, ?2)",
                ("greeting", "hello"),
            )?;
            Ok(())
        })
        .await
        .expect("write");

        let value: String = db
            .read(|conn| {
                Ok(conn.query_row(
                    "SELECT value FROM setting WHERE key = ?1",
                    ["greeting"],
                    |row| row.get(0),
                )?)
            })
            .await
            .expect("read");

        assert_eq!(value, "hello");
    }

    #[tokio::test]
    async fn a_failed_write_rolls_back_the_whole_transaction() {
        let (db, _dir) = temp_db();

        let result: Result<(), DbError> = db
            .write(|tx| {
                tx.execute(
                    "INSERT INTO setting (key, value) VALUES (?1, ?2)",
                    ("a", "1"),
                )?;
                // Same key twice: the second insert violates the primary key, and the
                // first must not survive it.
                tx.execute(
                    "INSERT INTO setting (key, value) VALUES (?1, ?2)",
                    ("a", "2"),
                )?;
                Ok(())
            })
            .await;

        assert!(result.is_err());

        let count: i64 = db
            .read(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM setting", [], |row| row.get(0))?))
            .await
            .expect("read");

        assert_eq!(count, 0, "the first insert should have rolled back");
    }

    #[tokio::test]
    async fn wal_and_foreign_keys_are_on_for_readers_too() {
        let (db, _dir) = temp_db();

        let (journal, foreign_keys): (String, i64) = db
            .read(|conn| {
                let journal =
                    conn.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))?;
                let fk = conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))?;
                Ok((journal, fk))
            })
            .await
            .expect("read");

        assert_eq!(journal.to_lowercase(), "wal");
        assert_eq!(foreign_keys, 1);
    }

    #[tokio::test]
    async fn lower_folds_more_than_the_ascii_alphabet() {
        // SQLite's built-in converts A-Z and nothing else, so `lower('JOSÉ')` came back as
        // 'josÉ'. Every case-insensitive comparison in the app pairs that against a needle
        // folded by Rust's `to_lowercase`, and the two therefore stopped agreeing at the first
        // accented capital: searching `from:José` matched nothing, and a rule could file a
        // message on arrival that its own smart mailbox then refused to list.
        let (db, _dir) = temp_db();

        let folded: Vec<String> = db
            .read(|conn| {
                let mut out = Vec::new();
                for word in ["JOSÉ", "ÅNGSTRÖM", "ΣΊΣΥΦΟΣ", "ЖУРНАЛ", "PLAIN"] {
                    out.push(
                        conn.query_row("SELECT lower(?1)", [word], |row| row.get::<_, String>(0))?,
                    );
                }
                Ok(out)
            })
            .await
            .expect("read");

        assert_eq!(
            folded,
            vec![
                "josé".to_string(),
                "ångström".to_string(),
                // Final sigma, not the medial one. Greek lowercasing is context-sensitive and
                // `to_lowercase` knows it — which is the sort of thing a hand-rolled A-Z fold
                // is never going to get right, and the reason for borrowing Rust's.
                "σίσυφος".to_string(),
                "журнал".to_string(),
                "plain".to_string(),
            ],
        );
    }

    #[tokio::test]
    async fn lower_agrees_with_the_rust_side_it_is_compared_against() {
        // The invariant that actually matters. The SQL half and the in-memory half of every
        // rule are only equivalent if they fold identically, and `predicate.rs` says so in as
        // many words: matching case-sensitively there "would make the two halves disagree on
        // every capital letter".
        let (db, _dir) = temp_db();

        for word in [
            "JOSÉ Álvarez",
            "Straße",
            "ÉCOLE",
            "İstanbul",
            "ordinary Text",
        ] {
            let sql: String = db
                .read(move |conn| {
                    Ok(conn.query_row("SELECT lower(?1)", [word], |row| row.get::<_, String>(0))?)
                })
                .await
                .expect("read");

            assert_eq!(
                sql,
                word.to_lowercase(),
                "SQL and Rust disagree on {word:?}"
            );
        }
    }

    #[tokio::test]
    async fn lower_leaves_null_alone() {
        // The built-in returns NULL for NULL, and callers wrap in COALESCE where they care.
        // Returning an empty string instead would change what those comparisons mean.
        let (db, _dir) = temp_db();

        let null: Option<String> = db
            .read(|conn| {
                Ok(conn.query_row("SELECT lower(NULL)", [], |row| {
                    row.get::<_, Option<String>>(0)
                })?)
            })
            .await
            .expect("read");

        assert_eq!(null, None);
    }

    #[tokio::test]
    async fn a_non_ascii_capital_is_found_by_a_like_comparison() {
        // The shape every search field and every rule uses, end to end.
        let (db, _dir) = temp_db();

        db.write(|tx| {
            tx.execute(
                "INSERT INTO setting (key, value) VALUES ('who', 'JOSÉ ÁLVAREZ')",
                [],
            )?;
            Ok(())
        })
        .await
        .expect("write");

        let needle = format!("%{}%", "José".to_lowercase());
        let found: i64 = db
            .read(move |conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM setting WHERE key = 'who' AND lower(value) LIKE ?1",
                    [needle],
                    |row| row.get(0),
                )?)
            })
            .await
            .expect("read");

        assert_eq!(found, 1, "a non-ASCII capital was not matched");
    }

    #[tokio::test]
    async fn the_contact_index_is_filled_from_mail_already_here() {
        // `persist::write_batch` records senders as messages arrive, which covers everything
        // from now on and nothing from before. An install with a mailbox already downloaded
        // would have an empty People group for weeks, until enough new mail arrived — and the
        // people worth suggesting are the ones already in the mailbox. Migration 0011 is the
        // one-time pass that fixes that, so this asserts it against a store seeded the way a
        // real one is: rows written before the migration ran.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.db");

        {
            // A store at the schema *before* the backfill, with mail in it.
            let mut conn = rusqlite::Connection::open(&path).expect("open");
            crate::db::migrate::run(&mut conn).expect("migrate");
            conn.execute("DELETE FROM contact", []).expect("clear");

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

            let people = [
                (1, "Ada Lovelace", "ada@example.test", 100),
                (2, "A. Lovelace", "ADA@example.test", 200),
                (3, "Grace Hopper", "grace@example.test", 150),
            ];

            for (id, name, addr, received) in people {
                conn.execute(
                    "INSERT INTO message (
                         id, account_id, mailbox_id, uid, subject, date_sent, date_received,
                         size, from_name, from_addr, from_all, to_all, body_text,
                         has_attachment, flag_seen, flag_flagged, is_junk
                     ) VALUES (?1, 1, 1, ?1, 'S', 0, ?4, 10, ?2, ?3, ?3, '', '', 0, 0, 0, 0)",
                    rusqlite::params![id, name, addr, received],
                )
                .expect("message");
            }

            // Re-run the backfill by hand, as the migration does on an existing store.
            conn.execute_batch(include_str!("../../migrations/0011_backfill_contacts.sql"))
                .expect("backfill");
        }

        let db = Db::open(&path).expect("open");

        let people: Vec<(String, Option<String>, i64)> = db
            .read(|conn| {
                Ok(conn
                    .prepare("SELECT addr, name, seen_count FROM contact ORDER BY addr")?
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
            .expect("read");

        assert_eq!(
            people.len(),
            2,
            "addresses were not folded to one contact each"
        );

        let ada = &people[0];
        assert_eq!(ada.0, "ada@example.test", "the key is not case-folded");
        assert_eq!(ada.2, 2, "both messages from Ada should be counted");
        assert_eq!(
            ada.1.as_deref(),
            Some("A. Lovelace"),
            "the most recent name is the one worth offering"
        );
    }
}
