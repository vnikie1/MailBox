//! The sync command surface. docs/03 §4, docs/06 Phase 5.
//!
//! Deliberately small: the engine is not something the UI drives step by step. It asks for a
//! sync and then listens for `sync:progress`, `messages:added` and `account:error` — standing
//! rule 14, freshness arrives as events rather than by polling for it.

use tauri::{AppHandle, State};

use crate::db::Db;
use crate::sync::engine::SyncEngine;
use crate::sync::idle::Watchers;

use super::mail::AppError;

/// Syncs one account now.
///
/// Returns as soon as the work is *scheduled*, not when it finishes. A first sync of a large
/// mailbox takes minutes, and an IPC call that blocked for minutes would make the UI look
/// hung — the progress events are how the caller follows along.
#[tauri::command]
pub async fn sync_now(
    app: AppHandle,
    db: State<'_, Db>,
    engine: State<'_, SyncEngine>,
    account_id: i64,
) -> Result<(), AppError> {
    let engine = engine.inner().clone();
    let db = db.inner().clone();

    tauri::async_runtime::spawn(async move {
        if let Err(error) = engine.sync_account(&app, &db, account_id).await {
            tracing::warn!(account_id, %error, "sync_now finished with an error");
        }
    });

    Ok(())
}

/// Syncs every account with syncing enabled.
#[tauri::command]
pub async fn sync_all(
    app: AppHandle,
    db: State<'_, Db>,
    engine: State<'_, SyncEngine>,
) -> Result<(), AppError> {
    let accounts = db.read(crate::accounts::store::list).await?;

    let engine = engine.inner().clone();
    let db = db.inner().clone();

    // One task per account rather than a loop. The engine locks per account, so these do not
    // interfere — and running them in sequence meant a working mailbox waited behind every
    // broken one ahead of it in the list.
    for account in accounts.into_iter().filter(|a| a.sync_enabled) {
        let app = app.clone();
        let engine = engine.clone();
        let db = db.clone();

        tauri::async_runtime::spawn(async move {
            if let Err(error) = engine.sync_account(&app, &db, account.id).await {
                tracing::warn!(account_id = account.id, %error, "sync_all: account failed");
            }
        });
    }

    Ok(())
}

/// Ensures the bodies for these messages are downloaded. docs/06 Phase 5 §3.
///
/// The UI calls this with the selected message plus the next three rows — *lazy fetch on
/// selection + prefetch of the next 3*. Already-cached messages cost nothing, so the caller
/// does not have to track what it has.
///
/// Returns as soon as the work is scheduled. `messages:updated` is what says it landed, and
/// the reader is already listening for it.
#[tauri::command]
pub async fn bodies_ensure(
    app: AppHandle,
    db: State<'_, Db>,
    account_id: i64,
    message_ids: Vec<i64>,
) -> Result<(), AppError> {
    let db = db.inner().clone();

    tauri::async_runtime::spawn(async move {
        if let Err(error) =
            crate::sync::engine::fetch_body(&app, &db, account_id, message_ids).await
        {
            tracing::warn!(account_id, %error, "body fetch failed");
        }
    });

    Ok(())
}

/// Starts (or stops) the per-account IDLE watchers to match the current account list.
///
/// Idempotent, so the UI calls it on launch and again whenever accounts change rather than
/// tracking what is already running. docs/03 §5 — freshness arrives without being asked for.
#[tauri::command]
pub async fn sync_watch(
    app: AppHandle,
    db: State<'_, Db>,
    engine: State<'_, SyncEngine>,
    watchers: State<'_, Watchers>,
) -> Result<(), AppError> {
    // The handle becomes an `Events` here rather than deeper in, so the sync engine keeps no
    // dependency on Tauri at all — see `sync::events`.
    let events: std::sync::Arc<dyn crate::sync::events::Events> = std::sync::Arc::new(app);

    watchers
        .inner()
        .reconcile(&events, db.inner(), engine.inner())
        .await;

    Ok(())
}
