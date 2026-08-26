//! Account rows, and the rules about what may be written into them.
//!
//! The one invariant worth stating: **an `account` row never holds a secret.** `cred_ref` is
//! a key into `credentials`, and `insert` is the only way to create a row, so there is no
//! path that writes a password into SQLite by accident. Standing rule 12, enforced by the
//! shape of the API rather than by remembering.

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::DbError;

use super::credentials;
use super::provider::{AuthKind, Provider, Security, ServerSettings};

/// Everything needed to create an account, with the secret kept separate on purpose — it
/// goes to the Credential Manager, and this struct goes to SQLite.
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub display_name: String,
    pub email: String,
    pub provider: Provider,
    pub imap: ServerSettings,
    pub smtp: ServerSettings,
    pub auth_kind: AuthKind,
    pub color: Option<String>,
}

/// An account as the settings UI sees it. No secret, and no field that could carry one.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AccountDetail {
    #[ts(type = "number")]
    pub id: i64,
    pub display_name: String,
    pub email: String,
    pub provider: String,
    pub auth_kind: AuthKind,
    pub imap: Option<ServerSettings>,
    pub smtp: Option<ServerSettings>,
    pub color: Option<String>,
    #[ts(type = "number")]
    pub sort_order: i64,
    pub sync_enabled: bool,
    /// False when the Credential Manager has no secret for this account — the account is
    /// present but cannot connect, and the UI shows a re-authenticate banner. docs/03 §7.
    pub has_credential: bool,
}

fn security_id(security: Security) -> &'static str {
    match security {
        Security::Tls => "tls",
        Security::StartTls => "starttls",
    }
}

fn security_from_id(id: &str) -> Security {
    match id {
        "starttls" => Security::StartTls,
        // docs/03 §13 — parse leniently. An unrecognised value from a future version or a
        // hand-edited row falls back to the stricter of the two rather than to plaintext.
        _ => Security::Tls,
    }
}

fn auth_id(kind: AuthKind) -> &'static str {
    match kind {
        AuthKind::OAuth2 => "oauth2",
        AuthKind::Password => "password",
    }
}

fn auth_from_id(id: &str) -> AuthKind {
    match id {
        "oauth2" => AuthKind::OAuth2,
        _ => AuthKind::Password,
    }
}

/// Creates the row. Returns its id.
///
/// Takes no secret, by design: the caller stores that through `credentials` and this writes
/// only the reference. The two are separate calls because they are separate stores, and
/// pretending otherwise would put a secret in a struct that is one `Serialize` away from
/// a log line.
pub fn insert(tx: &Transaction<'_>, account: &NewAccount) -> Result<i64, DbError> {
    let cred_ref = credentials::reference_for(&account.email);

    tx.execute(
        "INSERT INTO account (
             display_name, email, provider,
             imap_host, imap_port, imap_security,
             smtp_host, smtp_port, smtp_security,
             auth_kind, cred_ref, color, sort_order, sync_enabled
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
             (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM account), 1
         )",
        params![
            account.display_name,
            account.email.trim().to_lowercase(),
            account.provider.id(),
            account.imap.host,
            account.imap.port,
            security_id(account.imap.security),
            account.smtp.host,
            account.smtp.port,
            security_id(account.smtp.security),
            auth_id(account.auth_kind),
            cred_ref,
            account.color,
        ],
    )?;

    Ok(tx.last_insert_rowid())
}

/// True when an account with this address already exists.
///
/// Checked before a sign-in rather than after: sending the user through a browser consent
/// screen and *then* saying "already added" wastes the one thing that flow costs them.
pub fn exists_for_email(conn: &Connection, email: &str) -> Result<bool, DbError> {
    let normalised = email.trim().to_lowercase();

    let found: Option<i64> = conn
        .query_row(
            "SELECT id FROM account WHERE email = ?1",
            params![normalised],
            |row| row.get(0),
        )
        .optional()?;

    Ok(found.is_some())
}

fn detail_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AccountDetail> {
    let imap_host: Option<String> = row.get("imap_host")?;
    let imap_port: Option<u16> = row.get("imap_port")?;
    let imap_security: Option<String> = row.get("imap_security")?;
    let smtp_host: Option<String> = row.get("smtp_host")?;
    let smtp_port: Option<u16> = row.get("smtp_port")?;
    let smtp_security: Option<String> = row.get("smtp_security")?;
    let auth_kind: String = row.get("auth_kind")?;

    let servers = |host: Option<String>, port: Option<u16>, security: Option<String>| {
        Some(ServerSettings {
            host: host?,
            port: port?,
            security: security_from_id(security.as_deref().unwrap_or("tls")),
        })
    };

    Ok(AccountDetail {
        id: row.get("id")?,
        display_name: row.get("display_name")?,
        email: row.get("email")?,
        provider: row.get("provider")?,
        auth_kind: auth_from_id(&auth_kind),
        imap: servers(imap_host, imap_port, imap_security),
        smtp: servers(smtp_host, smtp_port, smtp_security),
        color: row.get("color")?,
        sort_order: row.get("sort_order")?,
        sync_enabled: row.get::<_, i64>("sync_enabled")? != 0,
        // Filled in by the caller, which has to touch the Credential Manager to know.
        has_credential: false,
    })
}

/// Every account, in the order the user arranged them.
pub fn list(conn: &Connection) -> Result<Vec<AccountDetail>, DbError> {
    let mut statement = conn.prepare(
        "SELECT id, display_name, email, provider,
                imap_host, imap_port, imap_security,
                smtp_host, smtp_port, smtp_security,
                auth_kind, color, sort_order, sync_enabled
           FROM account
          ORDER BY sort_order, id",
    )?;

    let rows = statement.query_map([], detail_from_row)?;

    let mut accounts = Vec::new();
    for account in rows {
        let mut account = account?;

        // The Credential Manager is the authority on whether this account can connect. A
        // row with no secret behind it is the state after a Windows profile reset, and the
        // UI has to be able to tell that from a working account.
        let kind = match account.auth_kind {
            AuthKind::OAuth2 => credentials::Kind::RefreshToken,
            AuthKind::Password => credentials::Kind::Password,
        };
        account.has_credential =
            credentials::exists(&credentials::reference_for(&account.email), kind);

        accounts.push(account);
    }

    Ok(accounts)
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<AccountDetail>, DbError> {
    let account = conn
        .query_row(
            "SELECT id, display_name, email, provider,
                    imap_host, imap_port, imap_security,
                    smtp_host, smtp_port, smtp_security,
                    auth_kind, color, sort_order, sync_enabled
               FROM account WHERE id = ?1",
            params![id],
            detail_from_row,
        )
        .optional()?;

    Ok(account)
}

/// The credential reference for an account, without loading the credential.
pub fn cred_ref(conn: &Connection, id: i64) -> Result<Option<String>, DbError> {
    let reference = conn
        .query_row(
            "SELECT cred_ref FROM account WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()?;

    Ok(reference)
}

/// Renames, recolours, or turns syncing off. Nothing here can write a secret.
pub fn update(
    tx: &Transaction<'_>,
    id: i64,
    display_name: Option<&str>,
    color: Option<Option<&str>>,
    sync_enabled: Option<bool>,
) -> Result<(), DbError> {
    if let Some(name) = display_name {
        tx.execute(
            "UPDATE account SET display_name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
    }

    // `Option<Option<_>>` so that "clear the colour" is expressible and distinct from "do
    // not touch the colour".
    if let Some(color) = color {
        tx.execute(
            "UPDATE account SET color = ?2 WHERE id = ?1",
            params![id, color],
        )?;
    }

    if let Some(enabled) = sync_enabled {
        tx.execute(
            "UPDATE account SET sync_enabled = ?2 WHERE id = ?1",
            params![id, i64::from(enabled)],
        )?;
    }

    Ok(())
}

/// Rewrites `sort_order` to match the order given.
///
/// Ids not in the list keep their relative order after the ones that are, so a stale UI
/// sending a partial list cannot silently drop an account off the end of the sidebar.
pub fn reorder(tx: &Transaction<'_>, ids: &[i64]) -> Result<(), DbError> {
    for (position, id) in ids.iter().enumerate() {
        tx.execute(
            "UPDATE account SET sort_order = ?2 WHERE id = ?1",
            params![id, position as i64],
        )?;
    }

    let offset = ids.len() as i64;
    let placeholders = if ids.is_empty() {
        "0".to_string()
    } else {
        ids.iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",")
    };

    tx.execute(
        &format!(
            "UPDATE account
                SET sort_order = ?1 + id
              WHERE id NOT IN ({placeholders})"
        ),
        params![offset],
    )?;

    Ok(())
}

/// Removes the account and everything under it, and returns its credential reference so the
/// caller can purge the secrets.
///
/// The reference is read *before* the delete for the same reason Phase 3's permanent delete
/// had to be fixed: after the row is gone there is nothing left to look it up from, and the
/// secrets would stay in Credential Manager forever with no account to associate them with.
///
/// Mail rows go via `ON DELETE CASCADE` from `mailbox`, except `message`, which references
/// `mailbox` and is removed explicitly so the FTS triggers fire and the search index does
/// not keep returning a removed account's mail.
pub fn remove(tx: &Transaction<'_>, id: i64) -> Result<Option<String>, DbError> {
    let reference: Option<String> = tx
        .query_row(
            "SELECT cred_ref FROM account WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()?;

    if reference.is_none() {
        return Ok(None);
    }

    // Deleted row by row rather than by cascade, so the FTS5 triggers see each DELETE.
    // A cascade would leave the search index holding the removed account's subjects and
    // sender addresses — visible in search results with no message behind them.
    tx.execute(
        "DELETE FROM message
          WHERE mailbox_id IN (SELECT id FROM mailbox WHERE account_id = ?1)",
        params![id],
    )?;

    tx.execute("DELETE FROM outbox WHERE account_id = ?1", params![id])?;
    tx.execute("DELETE FROM pending_op WHERE account_id = ?1", params![id])?;
    tx.execute("DELETE FROM mailbox WHERE account_id = ?1", params![id])?;
    tx.execute("DELETE FROM account WHERE id = ?1", params![id])?;

    Ok(reference)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate;

    fn store() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragma");
        migrate::run(&mut conn).expect("migrate");
        conn
    }

    fn sample(email: &str) -> NewAccount {
        let (imap, smtp) = Provider::ICloud.servers().expect("servers");

        NewAccount {
            display_name: "Ada Lovelace".into(),
            email: email.into(),
            provider: Provider::ICloud,
            imap,
            smtp,
            auth_kind: AuthKind::Password,
            color: Some("blue".into()),
        }
    }

    #[test]
    fn an_account_row_holds_a_reference_and_never_a_secret() {
        // The point of the whole module. If this ever fails, standing rule 12 is broken and
        // the exit-gate grep would find a password in the database file.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        insert(&tx, &sample("ada@example.test")).expect("insert");

        let cred_ref: String = tx
            .query_row("SELECT cred_ref FROM account", [], |row| row.get(0))
            .expect("cred_ref");

        assert_eq!(cred_ref, "halcyon:ada@example.test");

        // Every text column, checked — a secret smuggled into display_name or color would
        // be just as much of a leak as one in a column named for it.
        let mut statement = tx.prepare("SELECT * FROM account").expect("prepare");
        let columns = statement.column_count();
        let row_text: Vec<String> = statement
            .query_row([], |row| {
                (0..columns)
                    .map(|index| {
                        row.get::<_, rusqlite::types::Value>(index)
                            .map(|v| format!("{v:?}"))
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .expect("row");

        assert!(
            !row_text.iter().any(|value| value.contains("secret")),
            "no secret material anywhere in the row"
        );
    }

    #[test]
    fn email_is_normalised_so_the_same_address_cannot_be_added_twice() {
        let mut conn = store();

        {
            let tx = conn.transaction().expect("tx");
            insert(&tx, &sample("Ada@Example.test")).expect("insert");
            tx.commit().expect("commit");
        }

        // UNIQUE(email) only helps if both spellings normalise to the same string.
        assert!(exists_for_email(&conn, "ada@example.test").expect("exists"));
        assert!(exists_for_email(&conn, "  ADA@EXAMPLE.TEST ").expect("exists"));
        assert!(!exists_for_email(&conn, "grace@example.test").expect("exists"));

        let tx = conn.transaction().expect("tx");
        assert!(
            insert(&tx, &sample("ADA@example.test")).is_err(),
            "the second insert must be refused"
        );
    }

    #[test]
    fn servers_round_trip_including_the_security_setting() {
        // A STARTTLS account read back as TLS would fail to connect with a TLS handshake
        // error, which says nothing about the actual cause.
        let mut conn = store();

        let id = {
            let tx = conn.transaction().expect("tx");
            let id = insert(&tx, &sample("ada@example.test")).expect("insert");
            tx.commit().expect("commit");
            id
        };

        let account = get(&conn, id).expect("get").expect("present");

        assert_eq!(
            account.imap.as_ref().expect("imap").host,
            "imap.mail.me.com"
        );
        assert_eq!(account.imap.as_ref().expect("imap").port, 993);
        assert_eq!(account.imap.as_ref().expect("imap").security, Security::Tls);
        assert_eq!(account.smtp.as_ref().expect("smtp").port, 587);
        assert_eq!(
            account.smtp.as_ref().expect("smtp").security,
            Security::StartTls
        );
        assert_eq!(account.auth_kind, AuthKind::Password);
    }

    #[test]
    fn accounts_are_numbered_in_the_order_they_were_added() {
        let _guard = credentials::lock_store();
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        insert(&tx, &sample("first@example.test")).expect("insert");
        insert(&tx, &sample("second@example.test")).expect("insert");
        insert(&tx, &sample("third@example.test")).expect("insert");
        tx.commit_and_reopen();

        let accounts = list(&conn).expect("list");
        let emails: Vec<_> = accounts.iter().map(|a| a.email.as_str()).collect();

        assert_eq!(
            emails,
            [
                "first@example.test",
                "second@example.test",
                "third@example.test"
            ]
        );
    }

    #[test]
    fn reordering_keeps_accounts_the_caller_did_not_mention() {
        let _guard = credentials::lock_store();
        // A settings pane that has not refreshed could send a list missing an account added
        // in another window. Dropping it from the sidebar would look like data loss.
        let mut conn = store();

        let ids = {
            let tx = conn.transaction().expect("tx");
            let a = insert(&tx, &sample("a@example.test")).expect("insert");
            let b = insert(&tx, &sample("b@example.test")).expect("insert");
            let c = insert(&tx, &sample("c@example.test")).expect("insert");
            tx.commit().expect("commit");
            [a, b, c]
        };

        {
            let tx = conn.transaction().expect("tx");
            reorder(&tx, &[ids[2], ids[0]]).expect("reorder");
            tx.commit().expect("commit");
        }

        let order: Vec<i64> = list(&conn).expect("list").iter().map(|a| a.id).collect();

        assert_eq!(order[0], ids[2]);
        assert_eq!(order[1], ids[0]);
        assert_eq!(order[2], ids[1], "the unmentioned account is still present");
    }

    #[test]
    fn removing_an_account_returns_its_reference_before_the_row_is_gone() {
        // Phase 3's permanent-delete bug in another guise: resolve what you need *before*
        // deleting, or there is nothing left to resolve it from. Here the cost would be
        // secrets left in Credential Manager forever with no account to tie them to.
        let mut conn = store();

        let id = {
            let tx = conn.transaction().expect("tx");
            let id = insert(&tx, &sample("ada@example.test")).expect("insert");
            tx.commit().expect("commit");
            id
        };

        let tx = conn.transaction().expect("tx");
        let reference = remove(&tx, id).expect("remove");
        tx.commit().expect("commit");

        assert_eq!(reference.as_deref(), Some("halcyon:ada@example.test"));
        assert!(get(&conn, id).expect("get").is_none());

        // Removing something already gone is not an error, but it has no reference to give.
        let tx = conn.transaction().expect("tx");
        assert_eq!(remove(&tx, id).expect("remove"), None);
    }

    #[test]
    fn removing_an_account_clears_it_out_of_the_search_index() {
        // Deleting by cascade would leave the FTS rows behind, and a search would return
        // subjects from an account the user removed — with no message to open.
        let mut conn = store();

        let id = {
            let tx = conn.transaction().expect("tx");
            let id = insert(&tx, &sample("ada@example.test")).expect("insert");

            tx.execute(
                "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
                 VALUES (1, ?1, 'INBOX', 'Inbox', 'inbox')",
                params![id],
            )
            .expect("mailbox");

            tx.execute(
                "INSERT INTO message (id, account_id, mailbox_id, uid, message_id, subject,
                                      from_addr, from_all, to_all, date_sent, date_received, size)
                 VALUES (1, ?1, 1, 1, '<x@example.test>', 'Analytical Engine notes',
                         'ada@example.test', 'ada@example.test', 'grace@example.test',
                         1000, 1000, 10)",
                params![id],
            )
            .expect("message");

            tx.commit().expect("commit");
            id
        };

        let hits = |conn: &Connection| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM message_fts WHERE message_fts MATCH 'Analytical'",
                [],
                |row| row.get(0),
            )
            .expect("count")
        };

        assert_eq!(hits(&conn), 1);

        let tx = conn.transaction().expect("tx");
        remove(&tx, id).expect("remove");
        tx.commit().expect("commit");

        assert_eq!(
            hits(&conn),
            0,
            "the search index must not outlive the account"
        );
    }

    #[test]
    fn clearing_a_colour_is_distinct_from_leaving_it_alone() {
        let mut conn = store();

        let id = {
            let tx = conn.transaction().expect("tx");
            let id = insert(&tx, &sample("ada@example.test")).expect("insert");
            tx.commit().expect("commit");
            id
        };

        {
            let tx = conn.transaction().expect("tx");
            update(&tx, id, Some("Ada L."), None, None).expect("update");
            tx.commit().expect("commit");
        }

        let account = get(&conn, id).expect("get").expect("present");
        assert_eq!(account.display_name, "Ada L.");
        assert_eq!(account.color.as_deref(), Some("blue"), "colour untouched");

        {
            let tx = conn.transaction().expect("tx");
            update(&tx, id, None, Some(None), Some(false)).expect("update");
            tx.commit().expect("commit");
        }

        let account = get(&conn, id).expect("get").expect("present");
        assert_eq!(account.color, None);
        assert!(!account.sync_enabled);
    }

    /// Small helper so the ordering test reads without three nested blocks.
    trait CommitAndReopen {
        fn commit_and_reopen(self);
    }

    impl CommitAndReopen for Transaction<'_> {
        fn commit_and_reopen(self) {
            self.commit().expect("commit");
        }
    }
}
