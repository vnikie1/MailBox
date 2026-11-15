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
    Migration {
        version: 8,
        name: "draft_conflict",
        sql: include_str!("../../migrations/0008_draft_conflict.sql"),
    },
    Migration {
        version: 9,
        name: "search_history",
        sql: include_str!("../../migrations/0009_search_history.sql"),
    },
    Migration {
        version: 10,
        name: "gmail_thread_id",
        sql: include_str!("../../migrations/0010_gmail_thread_id.sql"),
    },
    Migration {
        version: 11,
        name: "backfill_contacts",
        sql: include_str!("../../migrations/0011_backfill_contacts.sql"),
    },
    Migration {
        version: 12,
        name: "moved_message_origin",
        sql: include_str!("../../migrations/0012_moved_message_origin.sql"),
    },
    Migration {
        version: 13,
        name: "mailbox_management",
        sql: include_str!("../../migrations/0013_mailbox_management.sql"),
    },
    Migration {
        version: 14,
        name: "favourites_roles_rebuild",
        sql: include_str!("../../migrations/0014_favourites_roles_rebuild.sql"),
    },
    Migration {
        version: 15,
        name: "discarded_sources",
        sql: include_str!("../../migrations/0015_discarded_sources.sql"),
    },
    Migration {
        version: 16,
        name: "draft_lifecycle",
        sql: include_str!("../../migrations/0016_draft_lifecycle.sql"),
    },
    Migration {
        version: 17,
        name: "mailbox_server_exists",
        sql: include_str!("../../migrations/0017_mailbox_server_exists.sql"),
    },
    Migration {
        version: 18,
        name: "pending_op_uid_validity",
        sql: include_str!("../../migrations/0018_pending_op_uid_validity.sql"),
    },
    Migration {
        version: 19,
        name: "fts_update_only_on_text",
        sql: include_str!("../../migrations/0019_fts_update_only_on_text.sql"),
    },
    Migration {
        version: 20,
        name: "imported_inbox_is_a_folder",
        sql: include_str!("../../migrations/0020_imported_inbox_is_a_folder.sql"),
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

    /// A store at `version`, as an install that stopped there has it.
    fn at_version(version: i64) -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        conn.pragma_update(None, "foreign_keys", "ON").expect("fk");
        conn.execute_batch(
            "CREATE TABLE schema_migration (
               version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at INTEGER NOT NULL)",
        )
        .expect("bookkeeping");

        for migration in MIGRATIONS.iter().filter(|m| m.version <= version) {
            conn.execute_batch(migration.sql).expect("older migration");
            conn.execute(
                "INSERT INTO schema_migration (version, name, applied_at) VALUES (?1, ?2, 0)",
                (migration.version, migration.name),
            )
            .expect("record");
        }

        conn
    }

    #[test]
    fn an_imported_inbox_loses_the_inbox_role_and_a_real_one_keeps_it() {
        // L40. The badge and the Inboxes row read the role, and an archive's old unread mail
        // joined both.
        let mut conn = at_version(19);
        conn.execute_batch(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref) VALUES
                 (1, 'Work', 'me@work.test', 'imap', 'password', 'ref'),
                 (2, 'On My PC', 'local@localhost', 'local', 'none', 'local:none');
             INSERT INTO mailbox (id, account_id, remote_path, display_name, role) VALUES
                 (1, 1, 'INBOX', 'Inbox', 'inbox'),
                 (2, 2, 'Outlook/Inbox', 'Inbox', 'inbox'),
                 (3, 2, 'Outlook/Sent Items', 'Sent Items', 'sent');",
        )
        .expect("a version 19 store");

        run(&mut conn).expect("migrate");

        let roles: Vec<Option<String>> = conn
            .prepare("SELECT role FROM mailbox ORDER BY id")
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");

        assert_eq!(
            roles,
            vec![Some("inbox".to_string()), None, Some("sent".to_string())]
        );
    }

    #[test]
    fn favourites_chosen_before_0014_keep_their_order_after_the_built_in_rows() {
        let mut conn = at_version(13);

        conn.execute_batch(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'A', 'a@example.test', 'imap', 'password', 'ref');
             INSERT INTO mailbox (id, account_id, remote_path, display_name, favourite_order)
             VALUES (1, 1, 'INBOX', 'Inbox', NULL),
                    (2, 1, 'Clients', 'Clients', 7),
                    (3, 1, 'Receipts', 'Receipts', 3),
                    (4, 1, 'Travel', 'Travel', NULL);",
        )
        .expect("a version 13 store");

        run(&mut conn).expect("migrate");

        let order: Vec<(i64, Option<String>, Option<i64>)> = conn
            .prepare("SELECT position, builtin, mailbox_id FROM favourite ORDER BY position")
            .expect("prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");

        assert_eq!(
            order,
            vec![
                (1, Some("allInboxes".to_string()), None),
                (2, Some("vips".to_string()), None),
                (3, Some("flagged".to_string()), None),
                (4, Some("allDrafts".to_string()), None),
                (5, Some("allSent".to_string()), None),
                (6, None, Some(3)),
                (7, None, Some(2)),
            ]
        );

        // The column is gone, so nothing can go on reading an order that is no longer kept.
        assert!(conn.prepare("SELECT favourite_order FROM mailbox").is_err());

        // And a deleted mailbox takes its place in Favourites with it, as the column did.
        conn.execute("DELETE FROM mailbox WHERE id = 3", [])
            .expect("delete");
        let left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM favourite WHERE mailbox_id IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(left, 1);
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

    #[test]
    fn changing_what_is_not_indexed_leaves_the_index_alone() {
        // M19. The update trigger re-indexed a message's whole text on every UPDATE, so marking
        // one read rewrote its body in the index, and so did every row a threading pass touched.
        let conn = fresh();
        conn.execute_batch(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'A', 'a@example.test', 'imap', 'password', 'ref');
             INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (1, 1, 'INBOX', 'Inbox', 'inbox');
             INSERT INTO thread (id, account_id, subject_base, last_date, message_count)
             VALUES (1, 1, 'Quarterly figures', 0, 1);
             INSERT INTO message
             (id, account_id, mailbox_id, uid, subject, date_sent, date_received,
              body_text, from_all, to_all, attachment_names)
             VALUES (1, 1, 1, 1, 'Quarterly figures', 100, 100, 'the numbers are attached',
                     'ada@example.test', 'me@example.test', '');",
        )
        .expect("seed");

        // What the index has stored, byte for byte.
        let stored = |conn: &Connection| -> Vec<(i64, Vec<u8>)> {
            let mut statement = conn
                .prepare("SELECT id, block FROM message_fts_data ORDER BY id")
                .expect("prepare");
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .expect("query")
                .collect::<rusqlite::Result<_>>()
                .expect("rows")
        };
        let before = stored(&conn);

        conn.execute("UPDATE message SET flag_seen = 1 WHERE id = 1", [])
            .expect("read");
        conn.execute("UPDATE message SET thread_id = 1 WHERE id = 1", [])
            .expect("thread");
        // Setting an indexed column to the value it already has is no change either.
        conn.execute(
            "UPDATE message SET subject = 'Quarterly figures', flag_flagged = 1 WHERE id = 1",
            [],
        )
        .expect("flag");

        assert_eq!(stored(&conn), before, "the index was rewritten");
    }
}
