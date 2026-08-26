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

#[tauri::command]
pub async fn msg_set_flags(
    app: AppHandle,
    db: State<'_, Db>,
    ids: Vec<i64>,
    patch: FlagPatch,
) -> Response<usize> {
    let affected = ids.clone();

    let (changed, mailboxes) = db
        .write(move |tx| {
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
            Ok((changed, mailboxes))
        })
        .await?;

    announce(&app, &db, mailboxes, &ids);
    Ok(changed)
}

#[tauri::command]
pub async fn msg_move(
    app: AppHandle,
    db: State<'_, Db>,
    ids: Vec<i64>,
    mailbox_id: i64,
) -> Response<usize> {
    let affected = ids.clone();

    let (changed, mut mailboxes) = db
        .write(move |tx| {
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
            Ok((changed, mailboxes))
        })
        .await?;

    mailboxes.sort_unstable();
    mailboxes.dedup();

    announce(&app, &db, mailboxes, &ids);
    Ok(changed)
}

#[tauri::command]
pub async fn msg_delete(
    app: AppHandle,
    db: State<'_, Db>,
    ids: Vec<i64>,
    permanent: bool,
) -> Response<usize> {
    let affected = ids.clone();

    let (changed, mailboxes) = db
        .write(move |tx| {
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
            Ok((changed, mailboxes))
        })
        .await?;

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
