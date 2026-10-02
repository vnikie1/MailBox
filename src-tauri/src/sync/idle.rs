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
//! Four things here are less obvious than they look, and each is commented where it happens:
//! the 29-minute re-issue, the debounce that stops our own writes waking us, the fallback to
//! polling on a server with no IDLE at all, and the watcher that *pauses* rather than dies when
//! the server refuses the sign-in.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

use crate::db::Db;

use super::events::{payload, Events};

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

/// How often every account is re-synced regardless of what IDLE is doing.
///
/// **IDLE watches INBOX and only INBOX.** Watching every mailbox would need a connection each,
/// and the note below `select("INBOX")` used to excuse that by saying the rest were "covered by
/// the periodic sync" — there was no periodic sync. Nothing polled an IDLE-capable server, so
/// on Gmail or Yahoo a message filed straight into a label by a server-side rule, a flag set on
/// a phone, or anything at all outside the inbox was invisible until the inbox happened to
/// change, the user pressed Get Mail, or the app was restarted.
///
/// It is also the only thing that recovers a *silently* dead connection inside the re-issue
/// window. A socket killed by a sleeping laptop or a NAT timeout does not report itself: the
/// watcher sits in a wait that will never be woken, and nothing notices until `IDLE_REISSUE`
/// comes round twenty-nine minutes later.
///
/// Five minutes is what Mail offers as its shortest automatic check, and `sync_account` locks
/// per account and returns quickly when there is nothing to do, so a pass that finds nothing
/// costs one round trip per mailbox and no writes.
const REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);

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
    /// Wakes a watcher that has paused on a refused sign-in. See [`Watcher::resume`].
    resume: tokio::sync::watch::Sender<()>,
}

impl Watcher {
    /// Asks the watcher to finish after its current wait.
    pub fn stop(&self) {
        self.stop.notify_waiters();
    }

    /// Tells a watcher that has paused on a refused sign-in to try again.
    ///
    /// A watcher that is not paused ignores this, so calling it whenever an account *might* have
    /// been fixed is safe. Without it, a watcher that met an expired Google sign-in stayed down
    /// for the rest of the session: the user signed in again, mail synced once, and nothing
    /// arrived by IDLE until the app was restarted.
    ///
    /// A `watch` channel rather than a `Notify`, because both of `Notify`'s modes lose
    /// something here. `notify_waiters` drops a resume that arrives while the watcher is still
    /// mid-attempt — so a sign-in finishing during the refused token request that is about to
    /// pause it would be missed. `notify_one` keeps a permit for later, but keeps it for ever —
    /// so a resume sent while the account was healthy would fire the moment it first failed,
    /// spending a second attempt on a credential the server had just refused. The channel's
    /// version answers the actual question: has anything been resumed *since the attempt that
    /// failed began*?
    pub fn resume(&self) {
        self.resume.send_replace(());
    }
}

/// Starts watching one account for server-side changes.
///
/// Returns immediately; the work happens on a spawned task. Errors are handled inside — a
/// watcher that gave up on the first dropped connection would be worse than no watcher, because
/// the app would look live and not be.
pub fn watch(app: Arc<dyn Events>, db: Db, engine: SyncEngine, account_id: i64) -> Watcher {
    let stop = Arc::new(Notify::new());
    let signal = Arc::clone(&stop);
    let (resume, mut resumed) = tokio::sync::watch::channel(());

    // The safety net, on its own task because the watcher below spends its life parked in a
    // wait that only the server can end. `stop` uses `notify_waiters`, which wakes both.
    {
        let app = Arc::clone(&app);
        let db = db.clone();
        let engine = engine.clone();
        let signal = Arc::clone(&stop);
        let resume = resume.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(REFRESH_INTERVAL) => {}
                    _ = signal.notified() => return,
                }

                // The sync waits on `stop` too, and that is not belt and braces.
                // `notify_waiters` wakes the waiters registered *at that moment*, so a stop
                // arriving while this task was inside a sync would be dropped on the floor and
                // the loop would go round again — re-syncing an account the user had just
                // removed or turned off, for as long as the app ran.
                //
                // Failures are the sync's business to report, not the watcher's: it has its
                // own backoff and its own error events.
                let synced = tokio::select! {
                    result = engine.sync_account(app.as_ref(), &db, account_id) => result.is_ok(),
                    _ = signal.notified() => return,
                };

                // A pass that got through has just proved the account can sign in, however that
                // came about — a new sign-in, a password changed back, a provider that was
                // wrongly refusing for a while. A watcher paused on the refusal has no other way
                // to hear it, so this is what brings IDLE back within one interval even if
                // nothing else ever asks.
                if synced {
                    resume.send_replace(());
                }
            }
        });
    }

    tokio::spawn(async move {
        let mut backoff = Backoff::new();

        // Lives across reconnects, and has to. A notification ends the connection (the sync
        // wants the slot), so a `last_sync` scoped to one connection would be discarded every
        // time it was set — leaving `MIN_INTERVAL` enforced on paper and never in practice.
        let mut last_sync: Option<std::time::Instant> = None;

        loop {
            // Whatever asked for a resume before this attempt began is answered by the attempt,
            // so only a resume that arrives from here on can wake a pause that follows it.
            resumed.mark_unchanged();

            let outcome = run(
                app.as_ref(),
                &db,
                &engine,
                account_id,
                &signal,
                &mut last_sync,
            )
            .await;

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
                    // and the sync path already tells the user about it — so no reconnecting in
                    // a loop against a server that is saying no.
                    //
                    // But **paused, not finished**, and that is the fix. This used to `return`,
                    // which left a dead watcher in the registry: `Watchers::reconcile` skips an
                    // account it already holds, so when the user signed in again nothing ever
                    // started a new one. The log of a Gmail account whose seven-day token had
                    // expired shows it exactly — "giving up: not retryable", a successful
                    // re-sign-in half an hour later, and no IDLE traffic for that account again
                    // until the app was restarted.
                    tracing::info!(
                        account_id,
                        %error,
                        "idle watcher paused until the account can sign in again"
                    );

                    if !paused(&mut resumed, &signal).await {
                        tracing::debug!(account_id, "idle watcher stopped while paused");
                        return;
                    }

                    tracing::info!(account_id, "idle watcher resuming");
                    backoff.reset();
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

    Watcher { stop, resume }
}

/// Waits, paused, until the watcher is resumed or stopped. `true` means try again.
///
/// A resume sent since the receiver was last marked seen counts, even if it landed before this
/// was called — that is the attempt-still-in-flight case [`Watcher::resume`] describes. Every
/// sender gone counts as a stop: nothing is left that could ever resume it.
async fn paused(resumed: &mut tokio::sync::watch::Receiver<()>, stop: &Notify) -> bool {
    tokio::select! {
        changed = resumed.changed() => changed.is_ok(),
        _ = stop.notified() => false,
    }
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

    /// Wakes one account's watcher if it is paused on a refused sign-in. See [`Watcher::resume`].
    ///
    /// Called by `account_reauth` the moment a new sign-in is stored, so IDLE comes back without
    /// depending on any window being open to ask for it.
    pub fn resume(&self, account_id: i64) {
        if let Ok(running) = self.running.lock() {
            if let Some(watcher) = running.get(&account_id) {
                watcher.resume();
            }
        }
    }

    /// Starts watchers for accounts that should have one, stops the rest, and resumes any that
    /// are paused.
    ///
    /// The resume is the part that used to be missing. `account_reauth` announces
    /// `accounts:changed`, the window answers with `sync_watch`, and this is what that reaches —
    /// but an account already in the registry was simply skipped, and a watcher that had given
    /// up on a refused sign-in was still *in* the registry. So the comment promising that the
    /// event "restarts the watcher" was true of every watcher except the one that needed it.
    pub async fn reconcile(&self, app: &Arc<dyn Events>, db: &Db, engine: &SyncEngine) {
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
            //
            // And one that is kept is resumed. Every caller of this is a moment when an account
            // may have been fixed — signed in again, its client secret supplied, the network
            // back — and a watcher that is not paused ignores the resume.
            running.retain(|account_id, watcher| {
                let keep = wanted.contains(account_id);
                if keep {
                    watcher.resume();
                } else {
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
    app: &dyn Events,
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

    // The Inbox is what IDLE watches. Watching every mailbox would need a connection each, so
    // this is the one the user is looking at and the one mail arrives in. Everything else is
    // caught by `REFRESH_INTERVAL` above — which is a real timer now, where this comment used
    // to point at a "periodic sync" that did not exist.
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
                app.emit(
                    "sync:activity",
                    payload(&Activity {
                        account_id,
                        live: true,
                    }),
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
    app: &dyn Events,
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

        app.emit(
            "sync:activity",
            payload(&Activity {
                account_id,
                live: false,
            }),
        );

        let _ = engine.sync_account(app, db, account_id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_account_is_refreshed_well_inside_the_reissue_window() {
        // The safety net has to close two holes, and both are silent.
        //
        // IDLE watches INBOX alone, so without this nothing outside the inbox is ever noticed
        // — on Gmail that is every label a filter files into. And a connection killed by a
        // sleeping laptop reports nothing, so the watcher parks in a wait that will never be
        // woken; only `IDLE_REISSUE` ends that, twenty-nine minutes later. A refresh longer
        // than the re-issue would leave the second hole exactly as it was.
        assert!(
            REFRESH_INTERVAL < IDLE_REISSUE,
            "a refresh slower than the re-issue closes neither hole it exists for"
        );
        assert!(
            REFRESH_INTERVAL >= Duration::from_secs(60),
            "re-syncing every mailbox more than once a minute is a poll, not a safety net"
        );
        assert!(
            REFRESH_INTERVAL <= Duration::from_secs(10 * 60),
            "mail that takes more than ten minutes to appear reads as not arriving at all"
        );
    }

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

    /// Long enough that a pause which was going to end on its own would have.
    const STILL_WAITING: Duration = Duration::from_millis(100);

    #[tokio::test]
    async fn a_resume_that_lands_while_the_failing_attempt_is_in_flight_is_not_lost() {
        // The race `Watcher::resume` exists to win: the sign-in finishes while the watcher is
        // still waiting on the refused token request that is about to pause it. With
        // `notify_waiters` that resume reached nobody, and IDLE stayed down.
        let stop = Notify::new();
        let (resume, mut resumed) = tokio::sync::watch::channel(());

        // The attempt begins; the user finishes signing in; then the attempt fails and the
        // watcher pauses — in that order.
        resumed.mark_unchanged();
        resume.send_replace(());
        let woke = tokio::time::timeout(STILL_WAITING, paused(&mut resumed, &stop)).await;

        assert_eq!(woke.ok(), Some(true), "the resume must still count");
    }

    #[tokio::test]
    async fn a_resume_from_before_the_failing_attempt_does_not_wake_the_pause() {
        // The opposite mistake, and the reason this is not a `notify_one` permit. A resume sent
        // while the account was healthy — any `accounts:changed` sends one — must not make the
        // first refusal retry at once, spending a second request on a credential just refused.
        let stop = Notify::new();
        let (resume, mut resumed) = tokio::sync::watch::channel(());

        resume.send_replace(()); // an account was renamed, hours ago
        resumed.mark_unchanged(); // the attempt that is about to fail begins

        let woke = tokio::time::timeout(STILL_WAITING, paused(&mut resumed, &stop)).await;

        assert!(woke.is_err(), "an old resume must not end the pause");
    }

    #[tokio::test]
    async fn a_paused_watcher_resumes_when_asked_and_ends_when_stopped() {
        let stop = Arc::new(Notify::new());
        let (resume, mut resumed) = tokio::sync::watch::channel(());
        resumed.mark_unchanged();

        let waiting = tokio::spawn(async move { paused(&mut resumed, &stop).await });
        tokio::task::yield_now().await;
        resume.send_replace(());

        assert!(
            waiting.await.expect("join"),
            "a resume should wake it to try again"
        );

        // Stopped while paused: removing an account, or turning its sync off.
        let stop = Notify::new();
        let (_resume, mut resumed) = tokio::sync::watch::channel(());
        resumed.mark_unchanged();

        let waiting = paused(&mut resumed, &stop);
        tokio::pin!(waiting);
        // Polled once first, because `notify_waiters` wakes only what is already waiting. A
        // zero timeout polls the future and then reports the deadline as passed.
        assert!(
            tokio::time::timeout(Duration::ZERO, waiting.as_mut())
                .await
                .is_err(),
            "nothing has asked it to stop yet"
        );
        stop.notify_waiters();

        assert!(!waiting.await, "a stop must end the pause");
    }

    #[tokio::test]
    async fn a_paused_watcher_with_no_one_left_to_resume_it_ends() {
        // Once the registry entry and the safety net have both gone, nothing can ever send a
        // resume. Waiting on regardless would leave a task parked for the life of the process.
        let stop = Notify::new();
        let (resume, mut resumed) = tokio::sync::watch::channel(());
        resumed.mark_unchanged();
        drop(resume);

        let woke = tokio::time::timeout(STILL_WAITING, paused(&mut resumed, &stop)).await;

        assert_eq!(woke.ok(), Some(false));
    }

    /// A watcher whose resumes the test can see, registered by hand rather than started.
    fn registered(watchers: &Watchers, account_id: i64) -> tokio::sync::watch::Receiver<()> {
        let (resume, mut resumed) = tokio::sync::watch::channel(());
        resumed.mark_unchanged();

        watchers.running.lock().expect("registry").insert(
            account_id,
            Watcher {
                stop: Arc::new(Notify::new()),
                resume,
            },
        );

        resumed
    }

    #[test]
    fn resuming_one_account_leaves_the_others_alone() {
        let watchers = Watchers::new();
        let first = registered(&watchers, 1);
        let second = registered(&watchers, 2);

        watchers.resume(2);
        // An account with no watcher at all is not an error: it may have sync turned off.
        watchers.resume(99);

        assert!(!first.has_changed().expect("sender alive"));
        assert!(second.has_changed().expect("sender alive"));
    }

    struct Silent;

    impl Events for Silent {
        fn emit(&self, _event: &str, _payload: serde_json::Value) {}
    }

    #[tokio::test]
    async fn reconcile_resumes_a_watcher_it_already_holds() {
        // The bug, end to end at the registry. `account_reauth` announces `accounts:changed`,
        // the window calls `sync_watch`, and that lands here — where an account already in the
        // registry used to be skipped outright, so a watcher that had given up on an expired
        // sign-in was never replaced. Mail then stopped arriving by IDLE until a restart.
        use crate::accounts::provider::{AuthKind, Provider, Security, ServerSettings};
        use crate::accounts::store::{self, NewAccount};

        let dir = tempfile::tempdir().expect("temp dir");
        let db = Db::open(&dir.path().join("watchers.db")).expect("open store");

        let server = |host: &str, port: u16| ServerSettings {
            host: host.into(),
            port,
            security: Security::Tls,
        };
        let account = NewAccount {
            display_name: "Ada".into(),
            email: "ada@example.test".into(),
            provider: Provider::Google,
            imap: server("imap.example.test", 993),
            smtp: server("smtp.example.test", 465),
            auth_kind: AuthKind::OAuth2,
            color: None,
        };
        let account_id = db
            .write(move |tx| store::insert(tx, &account))
            .await
            .expect("insert");

        let watchers = Watchers::new();
        let resumed = registered(&watchers, account_id);

        let events: Arc<dyn Events> = Arc::new(Silent);
        watchers.reconcile(&events, &db, &SyncEngine::new()).await;

        assert!(
            resumed.has_changed().expect("sender alive"),
            "a watcher reconcile keeps must be told to try again"
        );
        assert_eq!(
            watchers.running.lock().expect("registry").len(),
            1,
            "and must not be joined by a second one for the same account"
        );
    }
}
