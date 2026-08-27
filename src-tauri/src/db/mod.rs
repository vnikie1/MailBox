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

    Ok(())
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
        })
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
}
