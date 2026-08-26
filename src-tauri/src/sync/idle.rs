//! Waiting for the server to speak first. RFC 2177, docs/03 §5.
//!
//! Without this the app only learns about new mail when something asks it to look. Polling
//! every minute is both too slow to feel live and too chatty to be polite; IDLE is the server
//! telling us the moment something changes, on a connection that is otherwise silent.
//!
//! **On its own connection, always.** An IDLE'ing connection cannot be used for anything else
//! — the mailbox is held open and the client is mid-command — so sharing it with the sync
//! would mean tearing the idle down and rebuilding it around every fetch. docs/03 §5 budgets
//! 2–4 connections per account precisely so this one can sit still.
//!
//! Three things here are less obvious than they look, and each is commented where it happens:
//! the 29-minute re-issue, the debounce that stops our own writes waking us, and the fallback
//! to polling on a server with no IDLE at all.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

use crate::db::Db;

use super::backoff::Backoff;
use super::engine::{credential_for, SyncEngine};
use super::session::{self, SyncError};

/// How long to hold one IDLE before re-issuing it.
///
/// RFC 2177 §3 is explicit: a server may treat an idling client as inactive and log it off, so
/// clients are advised to re-issue at least every 29 minutes. Sitting there for longer does not
/// get more notifications, it gets disconnected — and a disconnected watcher is indistinguishable
/// from a quiet mailbox, which is the failure nobody notices.
const IDLE_REISSUE: Duration = Duration::from_secs(29 * 60);

/// How long to wait after a notification before syncing.
///
/// The server tells us about *our own* writes too. A drain that stores twenty flags produces a
/// burst of notifications, each of which would otherwise start a sync, which would drain and
/// notify again. Coalescing the burst is what stops that becoming a loop.
const DEBOUNCE: Duration = Duration::from_secs(2);

/// The least time between two IDLE-triggered syncs.
///
/// The debounce handles a burst; this handles a mailbox that is genuinely busy. Without it a
/// mailing list arriving in bulk could start a sync per message.
const MIN_INTERVAL: Duration = Duration::from_secs(10);

/// How often to look when the server has no IDLE.
///
/// Slow enough to be polite on a protocol that charges a full round trip per look, fast enough
/// that mail is not visibly stale. docs/03 §5 names polling as the fallback, not the default.
const POLL_INTERVAL: Duration = Duration::from_secs(120);

/// What the UI is told when the server reports a change.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub account_id: i64,
    /// True when this came from a real IDLE notification rather than the polling fallback.
    pub live: bool,
}

/// A running watcher. Dropping it does not stop the task; `stop` does.
pub struct Watcher {
    stop: Arc<Notify>,
}

impl Watcher {
    /// Asks the watcher to finish after its current wait.
    pub fn stop(&self) {
        self.stop.notify_waiters();
    }
}

/// Starts watching one account for server-side changes.
///
/// Returns immediately; the work happens on a spawned task. Errors are handled inside — a
/// watcher that gave up on the first dropped connection would be worse than no watcher, because
/// the app would look live and not be.
pub fn watch(app: AppHandle, db: Db, engine: SyncEngine, account_id: i64) -> Watcher {
    let stop = Arc::new(Notify::new());
    let signal = Arc::clone(&stop);

    tokio::spawn(async move {
        let mut backoff = Backoff::new();

        // Lives across reconnects, and has to. A notification ends the connection (the sync
        // wants the slot), so a `last_sync` scoped to one connection would be discarded every
        // time it was set — leaving `MIN_INTERVAL` enforced on paper and never in practice.
        let mut last_sync: Option<std::time::Instant> = None;

        loop {
            let outcome = run(&app, &db, &engine, account_id, &signal, &mut last_sync).await;

            match outcome {
                // Asked to stop.
                Ok(Stopped::Requested) => {
                    tracing::debug!(account_id, "idle watcher stopped");
                    return;
                }

                // The connection ended for an ordinary reason. Reconnect without treating it
                // as a failure — an idle connection being closed after half an hour is normal.
                Ok(Stopped::ConnectionEnded) => {
                    backoff.reset();
                }

                Err(error) if !error.is_retryable() => {
                    // A rejected credential or an unconfigured account will not fix itself,
                    // and the sync path already tells the user about it. Stop quietly rather
                    // than reconnecting in a loop against a server that is saying no.
                    tracing::debug!(account_id, %error, "idle watcher giving up: not retryable");
                    return;
                }

                Err(error) => {
                    let delay = backoff.next_delay();
                    tracing::debug!(
                        account_id,
                        %error,
                        retry_in_ms = delay.as_millis() as u64,
                        "idle watcher failed; backing off"
                    );

                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = signal.notified() => return,
                    }
                }
            }
        }
    });

    Watcher { stop }
}

enum Stopped {
    Requested,
    ConnectionEnded,
}

/// Every watcher this process is running, one per account at most.
///
/// Managed by Tauri so it outlives any single command. `reconcile` is idempotent, which is
/// what lets the UI call it whenever accounts change without tracking what is already running:
/// starting a second watcher for an account would double every notification and hold a
/// connection the account's budget does not have.
#[derive(Clone, Default)]
pub struct Watchers {
    running: Arc<std::sync::Mutex<std::collections::HashMap<i64, Watcher>>>,
}

impl Watchers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts watchers for accounts that should have one and stops the rest.
    pub async fn reconcile(&self, app: &AppHandle, db: &Db, engine: &SyncEngine) {
        let accounts = match db.read(crate::accounts::store::list).await {
            Ok(accounts) => accounts,
            Err(error) => {
                tracing::warn!(%error, "could not read accounts to reconcile idle watchers");
                return;
            }
        };

        let wanted: Vec<i64> = accounts
            .iter()
            .filter(|account| account.sync_enabled)
            .map(|account| account.id)
            .collect();

        let to_start: Vec<i64> = {
            let Ok(mut running) = self.running.lock() else {
                tracing::warn!("idle watcher registry is poisoned; leaving it alone");
                return;
            };

            // An account that was removed or had syncing turned off. Stopping it releases the
            // connection; leaving it would keep an idle open against an account the user has
            // told us to leave alone.
            running.retain(|account_id, watcher| {
                let keep = wanted.contains(account_id);
                if !keep {
                    watcher.stop();
                }
                keep
            });

            wanted
                .into_iter()
                .filter(|account_id| !running.contains_key(account_id))
                .collect()
        };

        for account_id in to_start {
            let watcher = watch(app.clone(), db.clone(), engine.clone(), account_id);

            if let Ok(mut running) = self.running.lock() {
                // Checked again under the lock: two reconciles racing would otherwise both
                // see "not running" and start two watchers for the same account.
                if let Some(previous) = running.insert(account_id, watcher) {
                    previous.stop();
                }
            }
        }
    }

    /// Stops every watcher. Called on shutdown.
    pub fn stop_all(&self) {
        if let Ok(mut running) = self.running.lock() {
            for (_, watcher) in running.drain() {
                watcher.stop();
            }
        }
    }
}

/// One connection's worth of watching: connect, select, then idle until something happens.
async fn run(
    app: &AppHandle,
    db: &Db,
    engine: &SyncEngine,
    account_id: i64,
    stop: &Notify,
    last_sync: &mut Option<std::time::Instant>,
) -> Result<Stopped, SyncError> {
    let account = db
        .read(move |conn| crate::accounts::store::get(conn, account_id))
        .await?
        .ok_or(SyncError::ShuttingDown)?;

    let imap = account
        .imap
        .clone()
        .ok_or_else(|| SyncError::NotConfigured {
            email: account.email.clone(),
        })?;

    let credential = credential_for(db, &account).await?;
    let (mut session, caps) = session::connect(&imap, &account.email, &credential).await?;

    if !caps.idle {
        // No IDLE. Close the connection rather than hold one open doing nothing, and fall
        // back to looking on a timer. Holding an idle-less connection open would consume one
        // of the account's few permitted connections for no benefit at all.
        let _ = session.logout().await;
        tracing::debug!(account_id, "server has no IDLE; falling back to polling");

        return poll(app, db, engine, account_id, stop).await;
    }

    // The Inbox is what IDLE watches. Watching every mailbox would need a connection each,
    // and the other mailboxes are covered by the periodic sync — this is about the one the
    // user is looking at.
    session.select("INBOX").await?;

    tracing::debug!(account_id, "idling on INBOX");

    loop {
        let mut handle = session.idle();
        handle.init().await?;

        let (waiter, interrupt) = handle.wait_with_timeout(IDLE_REISSUE);

        let woke = tokio::select! {
            result = waiter => Some(result),
            _ = stop.notified() => None,
        };

        // Ending the wait before taking the session back; the handle owns it until then.
        drop(interrupt);

        let Some(result) = woke else {
            let _ = handle.done().await;
            return Ok(Stopped::Requested);
        };

        session = handle.done().await?;

        match result? {
            // The re-issue timer. Nothing happened; go round and idle again.
            async_imap::extensions::idle::IdleResponse::Timeout => continue,

            async_imap::extensions::idle::IdleResponse::ManualInterrupt => {
                return Ok(Stopped::Requested)
            }

            async_imap::extensions::idle::IdleResponse::NewData(_) => {
                // Coalesce the burst. The server reports our own writes as well as other
                // people's, so a drain of twenty flags arrives as twenty notifications.
                tokio::select! {
                    _ = tokio::time::sleep(DEBOUNCE) => {}
                    _ = stop.notified() => return Ok(Stopped::Requested),
                }

                if last_sync.is_some_and(|at| at.elapsed() < MIN_INTERVAL) {
                    tracing::debug!(account_id, "idle: change seen, still inside the quiet gap");
                    continue;
                }
                *last_sync = Some(std::time::Instant::now());

                tracing::info!(account_id, "idle: the server reported a change");
                let _ = app.emit(
                    "sync:activity",
                    Activity {
                        account_id,
                        live: true,
                    },
                );

                // Dropped before syncing: the sync opens its own connection, and holding this
                // one selected on INBOX while it works wastes a slot for the whole pass.
                let _ = session.logout().await;

                // Failures are the sync's business to report, not the watcher's — it has its
                // own backoff and its own error events.
                let _ = engine.sync_account(app, db, account_id).await;

                return Ok(Stopped::ConnectionEnded);
            }
        }
    }
}

/// The fallback for a server with no IDLE: look every so often.
async fn poll(
    app: &AppHandle,
    db: &Db,
    engine: &SyncEngine,
    account_id: i64,
    stop: &Notify,
) -> Result<Stopped, SyncError> {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(POLL_INTERVAL) => {}
            _ = stop.notified() => return Ok(Stopped::Requested),
        }

        let _ = app.emit(
            "sync:activity",
            Activity {
                account_id,
                live: false,
            },
        );

        let _ = engine.sync_account(app, db, account_id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reissue_period_stays_inside_the_rfc_limit() {
        // RFC 2177 §3: re-issue at least every 29 minutes or risk being logged off. Going over
        // does not gain anything — the server stops talking to us — and the failure is silent,
        // which is what makes it worth a test rather than a comment.
        assert!(IDLE_REISSUE <= Duration::from_secs(29 * 60));
        assert!(
            IDLE_REISSUE >= Duration::from_secs(20 * 60),
            "re-issuing far more often than needed is a connection storm on a slow timer"
        );
    }

    #[test]
    fn the_debounce_is_shorter_than_the_quiet_gap() {
        // The debounce coalesces one burst; the gap rate-limits a busy mailbox. If the
        // debounce were the longer of the two it would be doing both jobs and the gap would
        // never be reached, which would quietly remove the rate limit.
        assert!(DEBOUNCE < MIN_INTERVAL);
    }

    #[test]
    fn polling_is_slower_than_the_quiet_gap_between_live_syncs() {
        // The fallback must not be more aggressive than the real thing, or a server without
        // IDLE would get more traffic from us than one with it.
        assert!(POLL_INTERVAL > MIN_INTERVAL);
    }
}
