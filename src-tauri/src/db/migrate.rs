//! Forward-only migrations.
//!
//! Each migration is a `.sql` file compiled into the binary with `include_str!`, applied
//! once, in order, inside a transaction, and recorded in `schema_migration`. There is no
//! `down`: a rollback that has to undo a data migration correctly is a fiction, and the
//! honest recovery from a bad migration is a fix-forward migration plus a restore.
//!
//! Embedding rather than reading from disk is deliberate. A migration file that ships
//! separately from the binary that expects it is a mismatch waiting to happen, and an
//! installed app has nowhere sensible to read them from anyway.

use rusqlite::Connection;

use super::DbError;

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

/// The full history. **Append only** — editing an applied migration changes what new
/// databases get without changing existing ones, which is how two installs of the same
/// version end up with different schemas.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: include_str!("../../migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "backfill_progress",
        sql: include_str!("../../migrations/0002_backfill_progress.sql"),
    },
    Migration {
        version: 3,
        name: "thread_rollup_indexes",
        sql: include_str!("../../migrations/0003_thread_rollup_indexes.sql"),
    },
    Migration {
        version: 4,
        name: "outbox_identity",
        sql: include_str!("../../migrations/0004_outbox_identity.sql"),
    },
    Migration {
        version: 5,
        name: "signatures",
        sql: include_str!("../../migrations/0005_signatures.sql"),
    },
    Migration {
        version: 6,
        name: "drafts",
        sql: include_str!("../../migrations/0006_drafts.sql"),
    },
    Migration {
        version: 7,
        name: "vips_snooze_flags",
        sql: include_str!("../../migrations/0007_vips_snooze_flags.sql"),
    },
];

/// Applies whatever has not been applied yet. Safe to call on every start.
pub fn run(conn: &mut Connection) -> Result<(), DbError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migration (
           version    INTEGER PRIMARY KEY,
           name       TEXT NOT NULL,
           applied_at INTEGER NOT NULL
         )",
    )?;

    let applied = current_version(conn)?;

    for migration in MIGRATIONS.iter().filter(|m| m.version > applied) {
        tracing::info!(
            version = migration.version,
            name = migration.name,
            "applying migration"
        );

        let tx = conn.transaction()?;

        // The whole file plus its bookkeeping row in one transaction: a migration that
        // half-applied and still counted as done is the one failure mode with no clean
        // recovery.
        tx.execute_batch(migration.sql)
            .map_err(|source| DbError::Migration {
                version: migration.version,
                name: migration.name.to_string(),
                source,
            })?;

        tx.execute(
            "INSERT INTO schema_migration (version, name, applied_at) VALUES (?1, ?2, ?3)",
            (
                migration.version,
                migration.name,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
            ),
        )?;

        tx.commit()?;
    }

    Ok(())
}

/// The highest applied version, or 0 on a fresh database.
pub fn current_version(conn: &Connection) -> Result<i64, DbError> {
    let version = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migration",
        [],
        |row| row.get(0),
    )?;

    Ok(version)
}

/// What the code expects to be applied. Used by tests and by the seed tool.
pub fn latest_version() -> i64 {
    MIGRATIONS.last().map(|m| m.version).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open");
        conn.pragma_update(None, "foreign_keys", "ON").expect("fk");
        run(&mut conn).expect("migrate");
        conn
    }

    #[test]
    fn a_fresh_database_reaches_the_latest_version() {
        let conn = fresh();
        assert_eq!(current_version(&conn).expect("version"), latest_version());
    }

    #[test]
    fn running_twice_is_a_no_op() {
        let mut conn = Connection::open_in_memory().expect("open");
        run(&mut conn).expect("first");
        let after_first = current_version(&conn).expect("version");

        run(&mut conn).expect("second");

        assert_eq!(current_version(&conn).expect("version"), after_first);
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migration", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(
            rows,
            latest_version(),
            "each migration recorded exactly once"
        );
    }

    #[test]
    fn versions_are_unique_and_ascending() {
        // An out-of-order or duplicated version silently skips a migration on some
        // installs and not others.
        let versions: Vec<i64> = MIGRATIONS.iter().map(|m| m.version).collect();
        let mut sorted = versions.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(versions, sorted);
    }

    #[test]
    fn the_search_index_tracks_the_message_table() {
        // The triggers are the whole point of an external-content FTS table; if they are
        // wrong, search silently returns stale or missing rows rather than failing.
        let conn = fresh();

        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'A', 'a@example.test', 'imap', 'password', 'ref')",
            [],
        )
        .expect("account");
        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (1, 1, 'INBOX', 'Inbox', 'inbox')",
            [],
        )
        .expect("mailbox");

        let insert = "INSERT INTO message
             (id, account_id, mailbox_id, uid, subject, date_sent, date_received,
              body_text, from_all, to_all, attachment_names)
             VALUES (?1, 1, 1, ?1, ?2, 100, 100, ?3, 'ada@example.test', 'me@example.test', '')";

        conn.execute(insert, (1, "Quarterly figures", "the numbers are attached"))
            .expect("insert");

        let hits = |conn: &Connection, term: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM message_fts WHERE message_fts MATCH ?1",
                [term],
                |row| row.get(0),
            )
            .expect("match")
        };

        assert_eq!(hits(&conn, "quarterly"), 1, "insert should index");

        conn.execute(
            "UPDATE message SET subject = ?2 WHERE id = ?1",
            (1, "Annual figures"),
        )
        .expect("update");
        assert_eq!(
            hits(&conn, "quarterly"),
            0,
            "update should drop the old term"
        );
        assert_eq!(hits(&conn, "annual"), 1, "update should index the new term");

        conn.execute("DELETE FROM message WHERE id = ?1", [1])
            .expect("delete");
        assert_eq!(hits(&conn, "annual"), 0, "delete should unindex");
    }
}
