//! The mail command surface. docs/03-architecture.md §4.
//!
//! Only the commands with real behaviour behind them are here. `compose_*`, `rule_*` and
//! the rest of §4 arrive with the phases that implement them — standing rule 18 forbids a
//! command that exists to return a plausible shape and do nothing, and a `compose_send`
//! with no SMTP behind it is exactly that.
//!
//! Every mutation emits an event when it commits. The UI subscribes and invalidates query
//! keys; it never polls (standing rule 14).

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use ts_rs::TS;

use crate::db::{
    model::{
        AccountRow, FlagPatch, ListQuery, MailboxRow, MessageFull, MessageRow, Page, SearchQuery,
    },
    query, write, Db, DbError,
};
use crate::sync::ops;

/// What a failed command looks like to the UI.
///
/// `code` is for the UI to branch on, `message` for a human. The underlying SQL error is
/// logged, never returned: standing rule 12 keeps secrets out of error messages, and a
/// database error can quote the row it was working on.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
}

impl From<DbError> for AppError {
    fn from(error: DbError) -> Self {
        tracing::error!(%error, "command failed");

        let code = match error {
            DbError::Migration { .. } => "migration",
            DbError::WriterGone => "writerGone",
            _ => "database",
        };

        Self {
            code: code.into(),
            // Deliberately generic. The detail is in the log, where it is not attached to
            // whatever the user was reading.
            message: "The mail store could not complete that request.".into(),
        }
    }
}

type Response<T> = Result<T, AppError>;

/// Payload for `mailbox:changed`. docs/03 §4.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MailboxChanged {
    mailbox_id: i64,
    unread: i64,
    total: i64,
}

/// Tells the UI which mailboxes moved, so it can invalidate exactly those query keys.
///
/// Emitted after the transaction has committed, never before: an event that arrives while
/// the write could still roll back would have the UI showing a change that then unhappens.
fn announce(app: &AppHandle, db: &Db, mailbox_ids: Vec<i64>, message_ids: &[i64]) {
    let _ = app.emit("messages:updated", message_ids);

    let handle = app.clone();
    let db = db.clone();

    // Counts are read back rather than computed here, so the event carries what the
    // database actually holds rather than what the caller believed it would hold.
    tauri::async_runtime::spawn(async move {
        let counts = db
            .read(move |conn| query::mailbox_counts(conn, &mailbox_ids))
            .await;

        if let Ok(counts) = counts {
            for entry in counts {
                let _ = handle.emit(
                    "mailbox:changed",
                    MailboxChanged {
                        mailbox_id: entry.mailbox_id,
                        unread: entry.unread,
                        total: entry.total,
                    },
                );
            }
        }
    });
}

#[tauri::command]
pub async fn accounts_list(db: State<'_, Db>) -> Response<Vec<AccountRow>> {
    Ok(db.read(query::accounts_list).await?)
}

#[tauri::command]
pub async fn mailboxes_tree(
    db: State<'_, Db>,
    account_id: Option<i64>,
) -> Response<Vec<MailboxRow>> {
    Ok(db
        .read(move |conn| query::mailboxes_tree(conn, account_id))
        .await?)
}

#[tauri::command]
pub async fn messages_page(db: State<'_, Db>, query: ListQuery) -> Response<Page<MessageRow>> {
    Ok(db
        .read(move |conn| self::query::messages_page(conn, &query))
        .await?)
}

#[tauri::command]
pub async fn message_get(db: State<'_, Db>, id: i64) -> Response<Option<MessageFull>> {
    Ok(db.read(move |conn| query::message_get(conn, id)).await?)
}

#[tauri::command]
pub async fn thread_get(db: State<'_, Db>, thread_id: i64) -> Response<Vec<MessageFull>> {
    Ok(db
        .read(move |conn| query::thread_get(conn, thread_id))
        .await?)
}

#[tauri::command]
pub async fn search(db: State<'_, Db>, query: SearchQuery) -> Response<Vec<MessageRow>> {
    Ok(db
        .read(move |conn| self::query::search(conn, &query))
        .await?)
}

/// Whether any of these messages is unflagged, which decides a flag toggle's direction.
///
/// Mirrors [`any_unread`], including the rule: a mixed selection becomes flagged.
pub fn any_unflagged(conn: &rusqlite::Connection, ids: &[i64]) -> Result<bool, DbError> {
    if ids.is_empty() {
        return Ok(false);
    }

    let list = (0..ids.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!("SELECT COUNT(*) FROM message WHERE id IN ({list}) AND flag_flagged = 0");
    let params: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
    let unflagged: i64 = conn.query_row(&sql, params.as_slice(), |row| row.get(0))?;

    Ok(unflagged > 0)
}

/// Whether any of these messages is unread, which is what decides a toggle's direction.
///
/// Separate from the command so the rule can be tested against real rows without a Tauri
/// `State`. The rule itself is the interesting part: **a mixed selection becomes read.**
pub fn any_unread(conn: &rusqlite::Connection, ids: &[i64]) -> Result<bool, DbError> {
    if ids.is_empty() {
        return Ok(false);
    }

    let list = (0..ids.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!("SELECT COUNT(*) FROM message WHERE id IN ({list}) AND flag_seen = 0");
    let params: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
    let unread: i64 = conn.query_row(&sql, params.as_slice(), |row| row.get(0))?;

    Ok(unread > 0)
}

/// Toggles the flag across a selection. Ctrl+L, docs/01 §14.
///
/// The same shape as `msg_toggle_read` and for the same reason: the direction is decided from
/// the stored rows rather than passed in, so the window keeps no second copy of state the
/// database already holds. A mixed selection becomes flagged.
/// Recorded on the undo stack, for the same reason archive is — see `msg_archive`.
///
/// It was not, until an unflagged message was flagged from the toolbar and Ctrl+Z did nothing.
/// The note above `msg_archive` says every command in this file used to miss the undo stack and
/// that archive, move and delete were fixed; the three flag commands were not, so exactly the
/// actions somebody performs dozens of times an hour were the ones that could not be taken back.
///
/// The Phase 8 gate did not catch it because it calls `undo::capture` directly rather than
/// through these commands: it proves the undo machinery works, not that anything uses it.
#[tauri::command]
pub async fn msg_toggle_flag(
    app: AppHandle,
    db: State<'_, Db>,
    stack: State<'_, std::sync::Arc<crate::undo::Stack>>,
    ids: Vec<i64>,
) -> Response<bool> {
    if ids.is_empty() {
        return Ok(false);
    }

    let asked = ids.clone();
    let now_flagged = db.read(move |conn| any_unflagged(conn, &asked)).await?;

    let patch = FlagPatch {
        seen: None,
        flagged: Some(now_flagged),
    };

    let affected = ids.clone();

    let (_, mailboxes, step) = db
        .write(move |tx| {
            // Captured before the write, from the same rows, so undo restores what was actually
            // there rather than re-running the toggle against whatever the state is later.
            let step = crate::undo::capture(
                tx,
                if now_flagged { "Flag" } else { "Clear Flag" },
                &affected,
                &[crate::undo::Field::Flagged],
            )?;

            for group in ops::locate(tx, &affected)? {
                ops::enqueue(
                    tx,
                    group.account_id,
                    &ops::Op::Flag {
                        mailbox: group.mailbox,
                        uids: group.uids,
                        seen: None,
                        flagged: Some(now_flagged),
                    },
                )?;
            }

            let changed = write::set_flags(tx, &affected, patch)?;
            let mailboxes = write::mailboxes_of(tx, &affected)?;
            Ok((changed, mailboxes, step))
        })
        .await?;

    stack.record(step);
    announce(&app, &db, mailboxes, &ids);
    Ok(now_flagged)
}

/// Archives a selection into **each message's own account** Archive. Ctrl+Shift+A.
///
/// Resolved per message rather than once, and that is the whole reason this is a command rather
/// than a move the window works out for itself. With several accounts on screen at the same
/// time — "All Inboxes" is the default view — a selection routinely spans two of them, and
/// there is no single Archive to move it to. Picking one would file somebody's mail into a
/// different account, which is both wrong and tedious to undo.
///
/// Where a message should go when it is archived.
///
/// 'archive' first, 'all' second.
///
/// Gmail has no Archive mailbox: it has All Mail, and archiving *is* removing the INBOX label,
/// which over IMAP is a move from INBOX to [Gmail]/All Mail. Looking only for role = 'archive'
/// therefore found nothing on the commonest provider there is, and the caller skipped the
/// message in silence — so the toolbar button, Ctrl+Shift+A and the toast's Archive action were
/// all dead against a real Gmail account. They worked only against seeded fixtures, which are
/// the only place a mailbox with role 'archive' has ever existed in this project.
///
/// Found by running the Phase 10 exit gate's triage session against live mail.
///
/// The preference order matters: a server with a genuine Archive folder must use it. All Mail
/// is the fallback, not the default — see `sync::mailboxes` on why `Role::All` is deliberately
/// not `Role::Archive`.
fn archive_mailbox_for(conn: &rusqlite::Connection, message_id: i64) -> Option<i64> {
    conn.query_row(
        "SELECT archive.id
           FROM message
           JOIN mailbox archive ON archive.account_id = message.account_id
          WHERE message.id = ?1 AND archive.role IN ('archive', 'all')
          ORDER BY CASE archive.role WHEN 'archive' THEN 0 ELSE 1 END
          LIMIT 1",
        rusqlite::params![message_id],
        |row| row.get(0),
    )
    .ok()
}

/// Returns how many were moved. A message whose account has no Archive mailbox is skipped
/// rather than failing the whole command, because the alternative is one misconfigured account
/// stopping the user archiving anything at all.
///
/// Recorded on the undo stack. It was not, until the Phase 10 exit gate archived a real message
/// and Ctrl+Z did nothing: every command in `ipc::organise` captured an undo step and none of
/// the ones in this file did, so archive, the most-used destructive-feeling action in the app,
/// was the one that could not be taken back. The toast's Archive button was documented as "easy
/// to undo", which was simply untrue.
#[tauri::command]
pub async fn msg_archive(
    app: AppHandle,
    db: State<'_, Db>,
    stack: State<'_, std::sync::Arc<crate::undo::Stack>>,
    ids: Vec<i64>,
) -> Response<usize> {
    if ids.is_empty() {
        return Ok(0);
    }

    let affected = ids.clone();

    let (moved, mailboxes, step) = db
        .write(move |tx| {
            // Captured before anything moves, and over Mailbox alone: archiving changes where a
            // message is and nothing else about it.
            let step =
                crate::undo::capture(tx, "Archive", &affected, &[crate::undo::Field::Mailbox])?;

            let mut moved = 0usize;

            for id in &affected {
                let archive = archive_mailbox_for(tx, *id);

                let Some(archive) = archive else {
                    continue;
                };

                // Already there. Moving a message to the mailbox it is in would queue a
                // pointless server round trip and, on some servers, change its UID.
                let current: i64 = tx.query_row(
                    "SELECT mailbox_id FROM message WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get(0),
                )?;

                if current == archive {
                    continue;
                }

                for group in ops::locate(tx, &[*id])? {
                    let destination: Option<String> = tx
                        .query_row(
                            "SELECT remote_path FROM mailbox WHERE id = ?1",
                            rusqlite::params![archive],
                            |row| row.get(0),
                        )
                        .ok();

                    if let Some(to) = destination {
                        ops::enqueue(
                            tx,
                            group.account_id,
                            &ops::Op::Move {
                                from: group.mailbox,
                                to,
                                uids: group.uids,
                            },
                        )?;
                    }
                }

                moved += write::move_to(tx, &[*id], archive)?;
            }

            let mailboxes = write::mailboxes_of(tx, &affected)?;
            Ok((moved, mailboxes, step))
        })
        .await?;

    // Recorded even when nothing moved. An empty step undoes to the same state, and the
    // alternative — deciding here whether it was worth keeping — puts the rule in two places.
    stack.record(step);

    announce(&app, &db, mailboxes, &ids);
    Ok(moved)
}

/// Toggles read and unread across a selection. Ctrl+U, docs/01 §14.
///
/// The *core* decides which way it goes, not the UI. The alternative is the window keeping its
/// own idea of what is read and sending an explicit `seen: true|false`, which is wrong twice
/// over: it is a second copy of state the database already holds, and it is stale the moment
/// another client marks something read while the user is looking at it.
///
/// A mixed selection becomes **read**. Someone who selects forty messages and presses Ctrl+U
/// means "clear these"; marking the already-read half unread instead would be a mess to undo.
/// Recorded on the undo stack — see `msg_toggle_flag`.
#[tauri::command]
pub async fn msg_toggle_read(
    app: AppHandle,
    db: State<'_, Db>,
    stack: State<'_, std::sync::Arc<crate::undo::Stack>>,
    ids: Vec<i64>,
) -> Response<bool> {
    if ids.is_empty() {
        return Ok(false);
    }

    let asked = ids.clone();

    let now_seen = db.read(move |conn| any_unread(conn, &asked)).await?;

    let patch = FlagPatch {
        seen: Some(now_seen),
        flagged: None,
    };

    let affected = ids.clone();

    let (_, mailboxes, step) = db
        .write(move |tx| {
            let step = crate::undo::capture(
                tx,
                if now_seen { "Mark Read" } else { "Mark Unread" },
                &affected,
                &[crate::undo::Field::Seen],
            )?;

            // Queued inside the same transaction as the local write, so closing the lid between
            // them cannot lose the change. Same contract as `msg_set_flags`.
            for group in ops::locate(tx, &affected)? {
                ops::enqueue(
                    tx,
                    group.account_id,
                    &ops::Op::Flag {
                        mailbox: group.mailbox,
                        uids: group.uids,
                        seen: Some(now_seen),
                        flagged: None,
                    },
                )?;
            }

            let changed = write::set_flags(tx, &affected, patch)?;
            let mailboxes = write::mailboxes_of(tx, &affected)?;
            Ok((changed, mailboxes, step))
        })
        .await?;

    stack.record(step);
    announce(&app, &db, mailboxes, &ids);
    Ok(now_seen)
}

/// Recorded on the undo stack — see `msg_toggle_flag`.
#[tauri::command]
pub async fn msg_set_flags(
    app: AppHandle,
    db: State<'_, Db>,
    stack: State<'_, std::sync::Arc<crate::undo::Stack>>,
    ids: Vec<i64>,
    patch: FlagPatch,
) -> Response<usize> {
    let affected = ids.clone();

    // Which fields this patch actually touches. Capturing a field the caller did not change
    // would make undo restore a value nothing had altered.
    let mut fields = Vec::new();
    if patch.seen.is_some() {
        fields.push(crate::undo::Field::Seen);
    }
    if patch.flagged.is_some() {
        fields.push(crate::undo::Field::Flagged);
    }

    let (changed, mailboxes, step) = db
        .write(move |tx| {
            let step = crate::undo::capture(tx, "Change Flags", &affected, &fields)?;

            // Located before the write and queued inside the same transaction: the local
            // change and the obligation to tell the server are one unit, so closing the lid
            // between them cannot lose the change. See `sync::ops`.
            for group in ops::locate(tx, &affected)? {
                ops::enqueue(
                    tx,
                    group.account_id,
                    &ops::Op::Flag {
                        mailbox: group.mailbox,
                        uids: group.uids,
                        seen: patch.seen,
                        flagged: patch.flagged,
                    },
                )?;
            }

            let changed = write::set_flags(tx, &affected, patch)?;
            let mailboxes = write::mailboxes_of(tx, &affected)?;
            Ok((changed, mailboxes, step))
        })
        .await?;

    stack.record(step);
    announce(&app, &db, mailboxes, &ids);
    Ok(changed)
}

/// Recorded on the undo stack, for the same reason archive is — see `msg_archive`. Moving mail
/// to the wrong folder is the mistake undo exists for.
#[tauri::command]
pub async fn msg_move(
    app: AppHandle,
    db: State<'_, Db>,
    stack: State<'_, std::sync::Arc<crate::undo::Stack>>,
    ids: Vec<i64>,
    mailbox_id: i64,
) -> Response<usize> {
    let affected = ids.clone();

    let (changed, mut mailboxes, step) = db
        .write(move |tx| {
            let step = crate::undo::capture(tx, "Move", &affected, &[crate::undo::Field::Mailbox])?;

            // Source mailboxes have to be read before the move, or the event only names
            // the destination and the source badge never updates.
            let mut mailboxes = write::mailboxes_of(tx, &affected)?;

            // Same reason, and the stronger one: after the move every row names the
            // destination, so a queued operation built afterwards would ask the server to
            // move messages out of the mailbox they had already been moved into.
            if let Some(destination) = ops::mailbox_path(tx, mailbox_id)? {
                for group in ops::locate(tx, &affected)? {
                    if group.mailbox == destination {
                        continue;
                    }

                    ops::enqueue(
                        tx,
                        group.account_id,
                        &ops::Op::Move {
                            from: group.mailbox,
                            to: destination.clone(),
                            uids: group.uids,
                        },
                    )?;
                }
            }

            let changed = write::move_to(tx, &affected, mailbox_id)?;
            if !mailboxes.contains(&mailbox_id) {
                mailboxes.push(mailbox_id);
            }
            Ok((changed, mailboxes, step))
        })
        .await?;

    stack.record(step);

    mailboxes.sort_unstable();
    mailboxes.dedup();

    announce(&app, &db, mailboxes, &ids);
    Ok(changed)
}

#[tauri::command]
pub async fn msg_delete(
    app: AppHandle,
    db: State<'_, Db>,
    stack: State<'_, std::sync::Arc<crate::undo::Stack>>,
    ids: Vec<i64>,
    permanent: bool,
) -> Response<usize> {
    let affected = ids.clone();

    let (changed, mailboxes, step) = db
        .write(move |tx| {
            // Only a move to Trash is recorded. A permanent delete has nothing to restore —
            // the row is gone and the server has been told to expunge it — and an undo entry
            // that silently fails to bring mail back is worse than no entry at all, because
            // the user stops checking.
            let step = if permanent {
                None
            } else {
                Some(crate::undo::capture(
                    tx,
                    "Delete",
                    &affected,
                    &[crate::undo::Field::Mailbox],
                )?)
            };

            let mut mailboxes = write::mailboxes_of(tx, &affected)?;

            // Which mailbox is Trash depends on the account the messages are in, so it is
            // resolved here rather than passed in — a UI that had to know would get it
            // wrong the moment a selection spans two accounts.
            let trash = trash_for(tx, &affected)?;
            if let Some(trash_id) = trash {
                if !mailboxes.contains(&trash_id) {
                    mailboxes.push(trash_id);
                }
            }

            // A non-permanent delete is a move to Trash, and is queued as one — the server
            // must not be told to expunge mail the user expects to be recoverable.
            let destination = match trash {
                Some(trash_id) if !permanent => ops::mailbox_path(tx, trash_id)?,
                _ => None,
            };

            for group in ops::locate(tx, &affected)? {
                let op = match &destination {
                    Some(to) if group.mailbox != *to => ops::Op::Move {
                        from: group.mailbox,
                        to: to.clone(),
                        uids: group.uids,
                    },
                    Some(_) => continue,
                    None if permanent => ops::Op::Delete {
                        mailbox: group.mailbox,
                        uids: group.uids,
                    },
                    // No Trash and not permanent: `write::delete` refuses too, so there is
                    // nothing local to mirror.
                    None => continue,
                };

                ops::enqueue(tx, group.account_id, &op)?;
            }

            let changed = write::delete(tx, &affected, permanent, trash)?;
            Ok((changed, mailboxes, step))
        })
        .await?;

    if let Some(step) = step {
        stack.record(step);
    }

    announce(&app, &db, mailboxes, &ids);
    Ok(changed)
}

/// The Trash mailbox of the account these messages belong to.
///
/// Returns `None` when the selection spans more than one account, because there is no
/// single answer — the caller then refuses rather than moving mail somewhere arbitrary.
/// Phase 5 splits such a selection per account; until there is a sync engine to do that
/// against, refusing is the honest behaviour.
fn trash_for(tx: &rusqlite::Transaction<'_>, ids: &[i64]) -> Result<Option<i64>, DbError> {
    if ids.is_empty() {
        return Ok(None);
    }

    let placeholders = (0..ids.len())
        .map(|i| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!("SELECT DISTINCT account_id FROM message WHERE id IN ({placeholders})");
    let params: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

    let accounts: Vec<i64> = tx
        .prepare(&sql)?
        .query_map(params.as_slice(), |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    if accounts.len() != 1 {
        return Ok(None);
    }

    let account_id = accounts[0];
    let trash = tx
        .query_row(
            "SELECT id FROM mailbox WHERE account_id = ?1 AND role = 'trash' LIMIT 1",
            [account_id],
            |row| row.get(0),
        )
        .ok();

    Ok(trash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn store() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open");
        crate::db::migrate::run(&mut conn).expect("migrate");

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

        conn
    }

    fn add(conn: &Connection, id: i64, seen: bool) {
        conn.execute(
            "INSERT INTO message (
                 id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
                 from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged, is_junk
             ) VALUES (?1, 1, 1, ?1, 'S', 0, 0, 10, 'a@b.test', '', '', 0, ?2, 0, 0)",
            rusqlite::params![id, i64::from(seen)],
        )
        .expect("message");
    }

    /// Adds a second mailbox with a given role, for the archive-target tests.
    fn add_mailbox(conn: &Connection, id: i64, path: &str, role: &str) {
        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (?1, 1, ?2, ?2, ?3)",
            rusqlite::params![id, path, role],
        )
        .expect("mailbox");
    }

    #[test]
    fn a_real_archive_mailbox_is_the_target() {
        let conn = store();
        add(&conn, 1, true);
        add_mailbox(&conn, 2, "Archive", "archive");

        assert_eq!(archive_mailbox_for(&conn, 1), Some(2));
    }

    #[test]
    fn gmail_archives_into_all_mail() {
        // The bug this function exists for. Gmail has no Archive mailbox, so archiving found
        // no target and the message was skipped without a word — on the commonest provider
        // there is. Every archive path in the app goes through here.
        let conn = store();
        add(&conn, 1, true);
        add_mailbox(&conn, 2, "[Gmail]/All Mail", "all");

        assert_eq!(archive_mailbox_for(&conn, 1), Some(2));
    }

    #[test]
    fn a_real_archive_mailbox_wins_over_all_mail() {
        // Order, not availability. A server offering both must use the one that means archive;
        // All Mail holds every message including the ones still in the inbox, so preferring it
        // would archive into a folder the message is arguably already in.
        let conn = store();
        add(&conn, 1, true);
        add_mailbox(&conn, 2, "[Gmail]/All Mail", "all");
        add_mailbox(&conn, 3, "Archive", "archive");

        assert_eq!(archive_mailbox_for(&conn, 1), Some(3));
    }

    #[test]
    fn an_account_with_neither_has_no_target() {
        // Still skipped, and that is right: inventing a destination for someone's mail is worse
        // than doing nothing. What changed is that Gmail is no longer in this case.
        let conn = store();
        add(&conn, 1, true);

        assert_eq!(archive_mailbox_for(&conn, 1), None);
    }

    #[test]
    fn another_accounts_archive_is_not_a_target() {
        // The JOIN is on account_id. Without it, a two-account setup could archive one
        // account's mail into another account's folder, which is not a move the server would
        // even accept.
        let conn = store();
        add(&conn, 1, true);

        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (2, 'Other', 'other@t.test', 'other', 'password', 'halcyon:other')",
            [],
        )
        .expect("account");
        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (9, 2, 'Archive', 'Archive', 'archive')",
            [],
        )
        .expect("mailbox");

        assert_eq!(archive_mailbox_for(&conn, 1), None);
    }

    #[test]
    fn an_unread_message_toggles_to_read() {
        let conn = store();
        add(&conn, 1, false);

        assert!(any_unread(&conn, &[1]).expect("query"));
    }

    #[test]
    fn a_read_message_toggles_to_unread() {
        let conn = store();
        add(&conn, 1, true);

        assert!(!any_unread(&conn, &[1]).expect("query"));
    }

    #[test]
    fn a_mixed_selection_becomes_read() {
        // Someone who selects forty messages and presses Ctrl+U means "clear these". Marking
        // the already-read half unread instead would be a mess to undo, one message at a time.
        let conn = store();
        add(&conn, 1, true);
        add(&conn, 2, false);
        add(&conn, 3, true);

        assert!(
            any_unread(&conn, &[1, 2, 3]).expect("query"),
            "a mixed selection should have been marked read"
        );
    }

    #[test]
    fn an_empty_selection_changes_nothing() {
        let conn = store();
        assert!(!any_unread(&conn, &[]).expect("query"));
    }

    #[test]
    fn a_message_that_no_longer_exists_is_not_counted_as_unread() {
        // Deleted between the keystroke and the query. Counting it as unread would mark the
        // rest of the selection read on the strength of a row that is gone.
        let conn = store();
        add(&conn, 1, true);

        assert!(!any_unread(&conn, &[1, 999]).expect("query"));
    }
}
