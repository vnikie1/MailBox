//! The mailbox context menu's commands. docs/01 §3.
//!
//! New Mailbox, Rename Mailbox, Delete Mailbox, Erase Deleted Items, Erase Junk Mail, Rebuild,
//! Use This Mailbox As, Add to Favourites, and the order of Favourites. Each is a local change
//! (`sync::folders`), so each returns as soon as the sidebar can show the result — standing rule
//! 10. The ones the server has to hear about queue an operation and then ask the account to sync,
//! which is what gets the change there while the user is still looking at it.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use ts_rs::TS;

use crate::db::model::FavouriteRow;
use crate::db::{query, Db};
use crate::sync::engine::SyncEngine;
use crate::sync::folders::{self, FolderError};
use crate::sync::mailboxes::Role;

use super::mail::{announce, AppError, MailboxChanged};

type Response<T> = Result<T, AppError>;

impl From<FolderError> for AppError {
    fn from(error: FolderError) -> Self {
        match error {
            // Written for the user by `sync::folders`, and carrying nothing but the name they
            // typed — which is theirs to see.
            FolderError::Invalid(message) => AppError {
                code: "invalid".into(),
                message,
            },
            FolderError::Gone => AppError {
                code: "gone".into(),
                message: "That mailbox no longer exists.".into(),
            },
            FolderError::Db(error) => error.into(),
        }
    }
}

/// Which mailbox an erase empties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum EraseTarget {
    /// Erase Deleted Items: the account's Bin.
    Trash,
    /// Erase Junk Mail.
    Junk,
}

impl EraseTarget {
    fn role(self) -> Role {
        match self {
            EraseTarget::Trash => Role::Trash,
            EraseTarget::Junk => Role::Junk,
        }
    }
}

/// What Use This Mailbox As can make a mailbox. The same words `MailboxRow::role` uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum MailboxUse {
    Drafts,
    Sent,
    Junk,
    Trash,
    Archive,
}

impl MailboxUse {
    fn role(self) -> Role {
        match self {
            MailboxUse::Drafts => Role::Drafts,
            MailboxUse::Sent => Role::Sent,
            MailboxUse::Junk => Role::Junk,
            MailboxUse::Trash => Role::Trash,
            MailboxUse::Archive => Role::Archive,
        }
    }
}

/// Tells the window the tree changed, then gets the change to the server.
///
/// The sync is started only for an account that syncs at all. One switched off in Settings
/// keeps the operation queued, exactly as it keeps a moved message's, until it is switched back
/// on — and an imported archive has no server and queued nothing.
fn settle(app: &AppHandle, db: &Db, engine: &SyncEngine, account_id: i64) {
    settle_with(app, db, engine, account_id, None);
}

/// `settle`, syncing only `mailboxes` when there are some — see `SyncEngine::sync_mailboxes`.
fn settle_with(
    app: &AppHandle,
    db: &Db,
    engine: &SyncEngine,
    account_id: i64,
    mailboxes: Option<Vec<i64>>,
) {
    let _ = app.emit("mailboxes:changed", account_id);

    let app = app.clone();
    let db = db.clone();
    let engine = engine.clone();

    tauri::async_runtime::spawn(async move {
        let account = db
            .read(move |conn| crate::accounts::store::get(conn, account_id))
            .await;

        let syncs =
            matches!(&account, Ok(Some(detail)) if detail.sync_enabled && detail.imap.is_some());
        if !syncs {
            return;
        }

        let synced = match mailboxes {
            Some(ids) => engine.sync_mailboxes(&app, &db, account_id, &ids).await,
            None => engine.sync_account(&app, &db, account_id).await,
        };

        if let Err(error) = synced {
            tracing::warn!(account_id, %error, "sync after a mailbox change failed");
        }
    });
}

/// Runs one folder change in one transaction.
///
/// `sync::folders` refuses a bad name or a vanished mailbox before it writes anything, so those
/// travel out through a transaction that commits nothing. A store error can happen half way
/// through, so it is returned as the transaction's own error — and rolls everything back.
async fn change<T, F>(db: &Db, job: F) -> Response<T>
where
    F: FnOnce(&rusqlite::Transaction<'_>) -> Result<T, FolderError> + Send + 'static,
    T: Send + 'static,
{
    db.write(move |tx| match job(tx) {
        Ok(value) => Ok(Ok(value)),
        Err(FolderError::Db(error)) => Err(error),
        Err(other) => Ok(Err(other)),
    })
    .await?
    .map_err(AppError::from)
}

/// New Mailbox, at the top of the account or inside `parent_id`. Returns the new mailbox's id.
#[tauri::command]
pub async fn mailbox_create(
    app: AppHandle,
    db: State<'_, Db>,
    engine: State<'_, SyncEngine>,
    account_id: i64,
    parent_id: Option<i64>,
    name: String,
) -> Response<i64> {
    let created = change(&db, move |tx| {
        folders::create_in(tx, account_id, parent_id, &name)
    })
    .await?;

    settle(&app, &db, &engine, account_id);
    Ok(created)
}

/// Rename Mailbox.
#[tauri::command]
pub async fn mailbox_rename(
    app: AppHandle,
    db: State<'_, Db>,
    engine: State<'_, SyncEngine>,
    mailbox_id: i64,
    name: String,
) -> Response<()> {
    let account_id = change(&db, move |tx| folders::rename(tx, mailbox_id, &name)).await?;

    settle(&app, &db, &engine, account_id);
    Ok(())
}

/// Delete Mailbox. Returns how many messages went with it.
#[tauri::command]
pub async fn mailbox_delete(
    app: AppHandle,
    db: State<'_, Db>,
    engine: State<'_, SyncEngine>,
    mailbox_id: i64,
) -> Response<usize> {
    let deleted = change(&db, move |tx| folders::delete(tx, mailbox_id)).await?;

    // Emitted by hand with zero counts: `announce` reads the counts back from rows that no
    // longer exist, so it would say nothing, and a list still showing the deleted folder would
    // never be told to let go of it.
    for mailbox_id in &deleted.mailbox_ids {
        let _ = app.emit(
            "mailbox:changed",
            MailboxChanged {
                mailbox_id: *mailbox_id,
                unread: 0,
                total: 0,
            },
        );
    }

    settle(&app, &db, &engine, deleted.account_id);
    Ok(deleted.messages)
}

/// Erase Deleted Items and Erase Junk Mail. Returns how many messages were erased here.
///
/// Permanent, and not put on the undo stack: an expunge cannot be taken back, and an undo entry
/// that quietly failed to restore anything would be worse than none (see `undo`).
#[tauri::command]
pub async fn mailbox_erase(
    app: AppHandle,
    db: State<'_, Db>,
    engine: State<'_, SyncEngine>,
    account_id: i64,
    target: EraseTarget,
) -> Response<usize> {
    let erased = change(&db, move |tx| folders::erase(tx, account_id, target.role())).await?;

    announce(&app, &db, vec![erased.mailbox_id], &[]);
    settle(&app, &db, &engine, account_id);
    Ok(erased.messages)
}

/// Add to Favourites (`favourite: true`) and Remove from Favourites.
///
/// `before` is where a mailbox dragged into Favourites was dropped: the favourite it goes in
/// front of, or none for the end. The menu adds at the end.
///
/// Local only, so there is nothing to sync.
#[tauri::command]
pub async fn mailbox_set_favourite(
    app: AppHandle,
    db: State<'_, Db>,
    mailbox_id: i64,
    favourite: bool,
    before: Option<i64>,
) -> Response<()> {
    let account_id = change(&db, move |tx| match (favourite, before) {
        (true, Some(before)) => folders::add_favourite_at(tx, mailbox_id, Some(before)),
        _ => folders::set_favourite(tx, mailbox_id, favourite),
    })
    .await?;

    let _ = app.emit("mailboxes:changed", account_id);
    Ok(())
}

/// Favourites, in order: the built-in rows and the mailboxes the user added.
#[tauri::command]
pub async fn favourites_list(db: State<'_, Db>) -> Response<Vec<FavouriteRow>> {
    Ok(db.read(query::favourites_list).await?)
}

/// Moves a favourite to just before `before`, or to the end. Local only.
#[tauri::command]
pub async fn favourite_move(
    app: AppHandle,
    db: State<'_, Db>,
    favourite_id: i64,
    before: Option<i64>,
) -> Response<()> {
    change(&db, move |tx| {
        folders::move_favourite(tx, favourite_id, before)
    })
    .await?;

    // Not one account's change, so no account to name: every window refetches its tree.
    let _ = app.emit("mailboxes:changed", 0);
    Ok(())
}

/// Use This Mailbox As. Local, as in Mail: the server is not told.
#[tauri::command]
pub async fn mailbox_use_as(
    app: AppHandle,
    db: State<'_, Db>,
    mailbox_id: i64,
    usage: MailboxUse,
) -> Response<()> {
    let account_id = change(&db, move |tx| folders::use_as(tx, mailbox_id, usage.role())).await?;

    let _ = app.emit("mailboxes:changed", account_id);
    Ok(())
}

/// Rebuild. Asks for it and starts the pass that does it — over that mailbox alone, after the
/// queue — and `mailbox:rebuilt` says when it has.
#[tauri::command]
pub async fn mailbox_rebuild(
    app: AppHandle,
    db: State<'_, Db>,
    engine: State<'_, SyncEngine>,
    mailbox_id: i64,
) -> Response<()> {
    let account_id = change(&db, move |tx| folders::request_rebuild(tx, mailbox_id)).await?;

    settle_with(&app, &db, &engine, account_id, Some(vec![mailbox_id]));
    Ok(())
}
