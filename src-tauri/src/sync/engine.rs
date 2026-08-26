//! The per-account sync supervisor. docs/03 §5, docs/06 Phase 5.
//!
//! One task per account. It connects, discovers the mailbox tree, renders the newest page of
//! the Inbox as fast as it can, then backfills the rest at low priority — the order docs/03
//! §5 specifies, and the order that makes the app *usable in under ten seconds* rather than
//! merely finished in five minutes.
//!
//! Failures are expected rather than exceptional: a laptop lid closes, a train enters a
//! tunnel, a provider has a bad minute. Every one of them goes through the same jittered
//! backoff, and only a credential the server has actually rejected stops the loop.
//!
//! No `unwrap()` in this module, per docs/06 Phase 5.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::accounts::credentials::{self, Kind};
use crate::accounts::provider::{AuthKind, Provider};
use crate::accounts::store::AccountDetail;
use crate::accounts::{self};
use crate::db::Db;

use super::backoff::Backoff;
use super::bodies;
use super::fetch::{self, BACKFILL_BATCH, FIRST_PAGE};
use super::mailboxes;
use super::ops;
use super::persist;
use super::session::{self, Caps, Credential, ImapSession, SyncError};

/// How many messages to re-thread after a batch.
///
/// Threading is not local — a message can bridge two conversations from years apart — but
/// re-threading an entire 100,000-message account after every 500-message batch would hold
/// the writer for seconds at a time. The most recent 5,000 covers every merge that matters
/// in practice, and a full pass runs once at the end of the initial sync.
const RETHREAD_WINDOW: usize = 5_000;

/// What the UI is told while a sync runs. docs/03 §4's `sync:progress`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub account_id: i64,
    pub mailbox: String,
    /// Messages written so far in this pass.
    pub written: usize,
    /// True once the first page is on screen and only backfill remains.
    pub usable: bool,
    pub done: bool,
}

/// A per-account error the UI can act on. docs/06 Phase 5 §9 — *a retry-at time, not a
/// spinner that never ends.*
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountError {
    pub account_id: i64,
    pub message: String,
    /// Seconds until the next attempt, so the UI can say "retrying in 30s" rather than spin.
    pub retry_in_seconds: u64,
    /// True when no retry will help and the user has to sign in again.
    pub needs_reauth: bool,
}

/// Handle to the running engine, managed by Tauri.
///
/// The lock is **per account**, not global. The first version held one mutex across every
/// account, and running it showed why that is wrong: three unconfigured demo accounts each
/// backed off through five attempts before the real account was reached, so a working
/// mailbox waited ninety seconds behind three broken ones. One account's bad day must not
/// be another account's.
#[derive(Clone)]
pub struct SyncEngine {
    locks: Arc<Mutex<HashMap<i64, Arc<Mutex<()>>>>>,
}

impl Default for SyncEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncEngine {
    pub fn new() -> Self {
        Self {
            locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The lock for one account, created on first use.
    ///
    /// Serialising per account is not just tidiness: two concurrent passes over the same
    /// mailbox would both fetch the same UID range and both write it, and in development
    /// React's StrictMode double-invokes effects, so the launch sync really does fire twice.
    async fn lock_for(&self, account_id: i64) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().await;
        Arc::clone(
            locks
                .entry(account_id)
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    /// Runs one full pass for an account: connect, discover, first page, backfill.
    ///
    /// Serialised across accounts by the mutex. docs/03 §5 allows 2–4 connections *per
    /// account*, but running every account's initial sync at once on a cold start is the
    /// connection storm the soak test looks for, and the first page of the account the user
    /// is looking at matters more than parallelism.
    pub async fn sync_account(
        &self,
        app: &AppHandle,
        db: &Db,
        account_id: i64,
    ) -> Result<(), SyncError> {
        let lock = self.lock_for(account_id).await;
        let _guard = lock.lock().await;

        let mut backoff = Backoff::new();

        loop {
            match run_once(app, db, account_id).await {
                Ok(()) => {
                    backoff.reset();
                    return Ok(());
                }

                Err(error) if !error.is_retryable() => {
                    tracing::warn!(account_id, %error, "sync stopped: not retryable");

                    let _ = app.emit(
                        "account:error",
                        AccountError {
                            account_id,
                            message: describe(&error),
                            retry_in_seconds: 0,
                            needs_reauth: matches!(error, SyncError::Rejected { .. }),
                        },
                    );

                    return Err(error);
                }

                Err(error) => {
                    let delay = backoff.next_delay();

                    tracing::warn!(
                        account_id,
                        %error,
                        retry_in_ms = delay.as_millis() as u64,
                        "sync failed; backing off"
                    );

                    // One blip is a lid closing. Two is a real problem, and only then does
                    // the user hear about it — a client that cries wolf on every sleep gets
                    // ignored when it finally matters.
                    if backoff.should_surface_error() {
                        let _ = app.emit(
                            "account:error",
                            AccountError {
                                account_id,
                                message: describe(&error),
                                retry_in_seconds: delay.as_secs(),
                                needs_reauth: false,
                            },
                        );
                    }

                    // Five attempts, then leave it to the next explicit sync. Retrying
                    // forever inside one call would hold the mutex against every other
                    // account.
                    if backoff.attempts() >= 5 {
                        return Err(error);
                    }

                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
}

/// Turns a sync failure into a sentence for the UI.
///
/// Never the underlying error's `Display`: those carry hostnames and protocol text, and
/// docs/03 §4 keeps that in the log where it is not attached to whatever the user was
/// reading.
fn describe(error: &SyncError) -> String {
    match error {
        SyncError::Rejected { .. } => {
            "The saved sign-in for this account was refused. Signing in again will fix it."
                .to_string()
        }
        SyncError::Unreachable { .. } | SyncError::Timeout { .. } | SyncError::Io(_) => {
            "Could not reach the mail server.".to_string()
        }
        SyncError::Tls { .. } => {
            "The mail server's security certificate was not accepted.".to_string()
        }
        SyncError::Insecure { .. } => {
            "This account is set to an unencrypted port. Halcyon needs IMAP over TLS on 993."
                .to_string()
        }
        SyncError::NotConfigured { .. } => {
            "This account has no incoming mail server set. Open Settings to add one.".to_string()
        }
        SyncError::MissingClientSecret { .. } => {
            "Google needs the client secret for your sign-in application before it will \n             refresh this account. Paste it into Settings — signing in again will not help."
                .to_string()
        }
        SyncError::UidValidityChanged { .. } => {
            "The server reorganised a mailbox; Halcyon is downloading it again.".to_string()
        }
        _ => "Mail could not be synchronised.".to_string(),
    }
}

/// Loads the account's credential, refreshing an OAuth token if it is close to expiring.
pub(crate) async fn credential_for(
    db: &Db,
    account: &AccountDetail,
) -> Result<Credential, SyncError> {
    let reference = credentials::reference_for(&account.email);

    match account.auth_kind {
        AuthKind::Password => {
            let secret =
                credentials::load(&reference, Kind::Password).map_err(|_| SyncError::Rejected {
                    host: account.email.clone(),
                    detail: "no password stored".into(),
                })?;

            Ok(Credential::Password(secret))
        }

        AuthKind::OAuth2 => {
            let provider = Provider::from_id(&account.provider).unwrap_or(Provider::Other);

            let client = {
                let reference = reference.clone();
                let _ = &reference;
                db.read(move |conn| accounts::client_config(conn, provider))
                    .await?
                    .ok_or_else(|| SyncError::Rejected {
                        host: account.email.clone(),
                        detail: "no oauth client configured".into(),
                    })?
            };

            // Google will not refresh a desktop client's token without the secret it issued,
            // and its refusal is `invalid_request` — indistinguishable from a rejected
            // sign-in unless we check first. Telling the user to sign in again here sends
            // them through a browser round trip that cannot possibly help.
            if provider.requires_client_secret() && client.client_secret.is_none() {
                return Err(SyncError::MissingClientSecret {
                    provider: provider.id().to_string(),
                });
            }

            let expiry = {
                let reference = reference.clone();
                db.read(move |conn| Ok(accounts::read_expiry(conn, &reference)))
                    .await?
            };

            let (token, refreshed) = accounts::access_token(expiry, provider, &client, &reference)
                .await
                .map_err(|error| SyncError::Rejected {
                    host: account.email.clone(),
                    detail: error.to_string(),
                })?;

            if let Some(expires_at) = refreshed {
                let reference = reference.clone();
                let _ = db
                    .write(move |tx| accounts::write_expiry(tx, &reference, expires_at))
                    .await;
            }

            Ok(Credential::OAuth(token))
        }
    }
}

/// One attempt at a full pass.
async fn run_once(app: &AppHandle, db: &Db, account_id: i64) -> Result<(), SyncError> {
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

    // Logged *before* the connect, not after. The first version logged it afterwards, so a
    // handshake that hung produced no line at all and the account looked as though it had
    // never been attempted — which sent the diagnosis to the wrong place entirely.
    tracing::info!(account_id, email = %account.email, "sync starting");

    let credential = credential_for(db, &account).await?;
    let (mut session, caps) = session::connect(&imap, &account.email, &credential).await?;

    tracing::info!(account_id, "connected");

    // ---- 0. push what the user changed while we were away --------------------------------
    // Before anything is fetched, and that order is load-bearing. Pulling first would
    // overwrite the local change with the stale value the server still holds, and the queued
    // operation would then push a value the user had already watched revert.
    ops::drain(db, &mut session, account_id, caps.move_command).await?;

    // ---- 1. the mailbox tree -----------------------------------------------------------
    let discovered = mailboxes::discover(&mut session).await?;
    tracing::info!(
        account_id,
        mailboxes = discovered.len(),
        "discovered mailboxes"
    );

    let selectable: Vec<mailboxes::Discovered> = discovered
        .iter()
        .filter(|m| m.selectable)
        .cloned()
        .collect();

    let ids = {
        let to_persist = selectable.clone();
        db.write(move |tx| mailboxes::persist(tx, account_id, &to_persist))
            .await?
    };

    let _ = app.emit("mailboxes:changed", account_id);

    // ---- 2. the Inbox, newest page first ------------------------------------------------
    // docs/03 §5 orders this deliberately: the Inbox is what the user is looking at, and its
    // newest page is what they see. Everything else waits.
    let inbox = selectable
        .iter()
        .zip(ids.iter())
        .find(|(mailbox, _)| mailbox.role == Some(mailboxes::Role::Inbox))
        .map(|(mailbox, (id, _))| (mailbox.remote_path.clone(), *id));

    // Counted so "sync finished" can say what it did. A run that logs only its own start and
    // end is indistinguishable from a run that hung — which is exactly how a 46-mailbox
    // account read for the several minutes it was working correctly.
    let mut inserted_total = 0usize;
    let mut synced = 0usize;
    let mut failed = 0usize;

    if let Some((path, mailbox_id)) = inbox {
        inserted_total += sync_mailbox(
            app,
            db,
            &mut session,
            caps,
            account_id,
            mailbox_id,
            &path,
            true,
        )
        .await?;
        synced += 1;
    }

    // ---- 3. everything else, newest 200 each -------------------------------------------
    for (mailbox, (mailbox_id, _)) in selectable.iter().zip(ids.iter()) {
        if mailbox.role == Some(mailboxes::Role::Inbox) {
            continue;
        }

        // Gmail's All Mail contains every message in the account a second time. Syncing it
        // alongside the labels would double the entire mailbox — docs/03 §5 names this.
        if mailbox.role == Some(mailboxes::Role::All) {
            tracing::debug!(path = %mailbox.remote_path, "skipping All Mail; its messages are in their labels");
            continue;
        }

        match sync_mailbox(
            app,
            db,
            &mut session,
            caps,
            account_id,
            *mailbox_id,
            &mailbox.remote_path,
            false,
        )
        .await
        {
            Ok(inserted) => {
                inserted_total += inserted;
                synced += 1;
            }

            // One unreadable mailbox must not abort the account. A shared folder whose
            // permissions changed is common, and losing the Inbox because of it is not.
            Err(error) => {
                failed += 1;
                tracing::warn!(path = %mailbox.remote_path, %error, "mailbox sync failed; continuing");
            }
        }
    }

    let _ = session.logout().await;

    let _ = app.emit(
        "sync:progress",
        Progress {
            account_id,
            mailbox: String::new(),
            written: 0,
            usable: true,
            done: true,
        },
    );

    tracing::info!(
        account_id,
        mailboxes = synced,
        failed,
        inserted = inserted_total,
        "sync finished"
    );
    Ok(())
}

/// Syncs one mailbox: newest page, then backfill if this is the Inbox.
#[allow(clippy::too_many_arguments)]
async fn sync_mailbox(
    app: &AppHandle,
    db: &Db,
    session: &mut ImapSession,
    caps: Caps,
    account_id: i64,
    mailbox_id: i64,
    path: &str,
    backfill: bool,
) -> Result<usize, SyncError> {
    let stored_state = db
        .read(move |conn| Ok(StoredState::read(conn, mailbox_id)))
        .await?;

    let StoredState {
        uid_validity: stored,
        backfill_uid: backfilled_to,
        uid_next: stored_uid_next,
        highest_modseq: stored_modseq,
    } = stored_state;

    let selected = match fetch::select(session, path, stored, caps).await {
        Ok(selected) => selected,

        Err(SyncError::UidValidityChanged {
            mailbox,
            stored,
            found,
        }) => {
            // docs/03 §5: *drop and re-sync that mailbox. Do not try to be clever.* Every
            // UID we hold for it now refers to a different message, so keeping any of them
            // would silently attach the wrong flags to the wrong mail.
            tracing::warn!(%mailbox, stored, found, "UIDVALIDITY changed; dropping the mailbox");

            db.write(move |tx| persist::drop_mailbox_contents(tx, mailbox_id))
                .await?;

            fetch::select(session, path, None, caps).await?
        }

        Err(error) => return Err(error),
    };

    if selected.uid_next == 0 && selected.exists == 0 {
        tracing::debug!(path, "mailbox is empty");
        return Ok(0);
    }

    tracing::debug!(
        path,
        exists = selected.exists,
        uid_next = selected.uid_next,
        backfill,
        "syncing mailbox"
    );

    // ---- the incremental path ------------------------------------------------------------
    // RFC 7162. When the server keeps modification sequences and we have seen this mailbox
    // before, it can tell us exactly what changed instead of us re-reading the newest page and
    // hoping. Two things follow, and the second is the one that matters:
    //
    //   * a mailbox whose MODSEQ has not moved needs no work at all — no fetch, nothing;
    //   * a flag changed on a phone is reported *wherever it is in the mailbox*, not only in
    //     the part we happen to re-read. Reconciling by re-fetching the newest page is why
    //     most clients silently miss a message read on another device last month.
    //
    // Falls through to the full path whenever anything is missing: no CONDSTORE, or a mailbox
    // this install has not stored state for yet.
    if let (true, Some(stored_modseq), Some(stored_uid_next), Some(server_modseq)) = (
        caps.has_modseq(),
        stored_modseq,
        stored_uid_next,
        selected.highest_modseq,
    ) {
        if server_modseq >= stored_modseq {
            return incremental(
                app,
                db,
                session,
                caps,
                account_id,
                mailbox_id,
                path,
                &selected,
                stored_modseq,
                stored_uid_next,
            )
            .await;
        }

        // A MODSEQ that went *backwards* means the server has lost or reset its modification
        // sequences — RFC 7162 §3.1.2.2 allows this after a restore from backup. Everything we
        // would ask "what changed since" is now meaningless, so fall through to the full path.
        tracing::warn!(
            path,
            stored = stored_modseq,
            found = server_modseq,
            "MODSEQ went backwards; falling back to a full pass"
        );
    }

    let first_page = if backfill { FIRST_PAGE } else { 200 };
    let range = fetch::newest_range(selected.uid_next, first_page);

    // Timed separately because "the mailbox took thirty seconds" says nothing about whether
    // the server, the network or our own writer is responsible — and the answer decided what
    // to fix. Cheap enough to leave in: two clock reads per mailbox.
    let fetch_started = std::time::Instant::now();
    let batch = fetch::envelopes(session, &range, caps).await?;
    let fetch_ms = fetch_started.elapsed().as_millis() as u64;

    let write_started = std::time::Instant::now();
    let (written, batch_ms, count_ms, thread_ms) = {
        let batch = batch.clone();
        db.write(move |tx| {
            let started = std::time::Instant::now();
            let written = persist::write_batch(tx, account_id, mailbox_id, &batch)?;
            let batch_ms = started.elapsed().as_millis() as u64;

            let started = std::time::Instant::now();
            persist::recount(tx, mailbox_id)?;
            persist::record_mailbox_state(
                tx,
                mailbox_id,
                selected.uid_validity,
                selected.uid_next,
                selected.highest_modseq,
            )?;
            let count_ms = started.elapsed().as_millis() as u64;

            let started = std::time::Instant::now();
            persist::rethread(tx, account_id, RETHREAD_WINDOW)?;
            let thread_ms = started.elapsed().as_millis() as u64;

            Ok((written, batch_ms, count_ms, thread_ms))
        })
        .await?
    };
    let write_ms = write_started.elapsed().as_millis() as u64;

    let _ = app.emit(
        "sync:progress",
        Progress {
            account_id,
            mailbox: path.to_string(),
            written: written.inserted,
            usable: true,
            done: false,
        },
    );

    let _ = app.emit("messages:added", mailbox_id);

    tracing::debug!(
        path,
        inserted = written.inserted,
        updated = written.updated,
        fetch_ms,
        write_ms,
        batch_ms,
        count_ms,
        thread_ms,
        "newest page stored"
    );

    let mut total = written.inserted;

    if !backfill {
        return Ok(total);
    }

    // ---- backfill ------------------------------------------------------------------------
    // Batches of 500 walking backwards, lowest priority. The pause between batches is not
    // politeness: it is what stops a backfill from saturating the connection the user's
    // interactions share, which docs/03 §5 calls "pausing on user interaction".
    //
    // Two things here were wrong and are worth stating, because both were invisible until the
    // per-mailbox logging above went in and both looked like a hang rather than a bug.
    //
    // First, the walk restarts nowhere: it resumes from `backfill_uid`. It used to begin again
    // below the newest page every time, and since it only ended at UID 1 it effectively never
    // ended.
    //
    // Second — and much worse — it walked the *numeric UID range* in windows of 500 rather
    // than the UIDs that exist. On a mailbox archived from for years those are not remotely
    // the same size: the real Gmail Inbox this was found on holds 214 messages with `uid_next`
    // at 106,287, so the range walk needed 213 round trips of about twenty seconds each, some
    // seventy minutes, to fetch 214 messages — inserting nothing on almost every one. At the
    // 50k-message mailbox docs/04's exit gate asks for, it does not finish at all.
    //
    // `UID SEARCH ALL` costs one round trip and makes the whole thing proportional to the
    // number of messages instead.
    if backfilled_to == Some(1) {
        tracing::debug!(path, "backfill already complete");
        return Ok(total);
    }

    let uids = fetch::all_uids(session).await?;

    tracing::debug!(path, known = uids.len(), "backfill: uid list fetched");

    // Resume where the last run stopped; on a mailbox never backfilled, start below the page
    // just fetched.
    let mut cursor = match backfilled_to {
        Some(uid) => uid.min(written.lowest_uid.max(1)),
        None => written.lowest_uid,
    };

    // A checkpoint the loop can record without repeating itself, and the thing that makes an
    // interrupted backfill resumable rather than merely restartable.
    async fn checkpoint(db: &Db, mailbox_id: i64, uid: u32) -> Result<(), SyncError> {
        db.write(move |tx| persist::record_backfill_progress(tx, mailbox_id, uid))
            .await?;
        Ok(())
    }

    while let Some((set, lowest)) = fetch::backfill_window(&uids, cursor, BACKFILL_BATCH) {
        let batch = fetch::envelopes(session, &set, caps).await?;

        if batch.is_empty() {
            // The server listed these UIDs a moment ago and now returns nothing for them,
            // which happens when they are expunged between the search and the fetch. Step
            // past rather than stopping, or everything older never arrives.
            cursor = lowest;
            checkpoint(db, mailbox_id, cursor).await?;
            continue;
        }

        let batch_for_write = batch.clone();
        let written = db
            .write(move |tx| {
                let written = persist::write_batch(tx, account_id, mailbox_id, &batch_for_write)?;
                persist::recount(tx, mailbox_id)?;
                persist::rethread(tx, account_id, RETHREAD_WINDOW)?;
                Ok(written)
            })
            .await?;

        let _ = app.emit(
            "sync:progress",
            Progress {
                account_id,
                mailbox: path.to_string(),
                written: written.inserted,
                usable: true,
                done: false,
            },
        );
        let _ = app.emit("messages:added", mailbox_id);

        total += written.inserted;

        // Step to the bottom of the window that was asked for, not to what came back. They
        // differ when a message is expunged mid-walk, and trusting the response would leave
        // the cursor above UIDs already covered — walking them again on the next pass.
        cursor = lowest;
        checkpoint(db, mailbox_id, cursor).await?;

        tracing::debug!(
            path,
            cursor,
            inserted = written.inserted,
            total,
            "backfill batch stored"
        );

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    // Reaching the bottom is what "complete" means, and recording it is what stops the next
    // sync walking the whole mailbox again.
    checkpoint(db, mailbox_id, 1).await?;
    tracing::debug!(path, total, "backfill complete");

    Ok(total)
}

/// What the last sync recorded about a mailbox.
///
/// All four are optional and independently so: a mailbox may have been seen but never
/// backfilled, or seen on a server that had no CONDSTORE. Every field being absent is the
/// ordinary state of a mailbox this install has not touched yet, not an error.
#[derive(Debug, Clone, Copy, Default)]
struct StoredState {
    uid_validity: Option<u32>,
    backfill_uid: Option<u32>,
    uid_next: Option<u32>,
    highest_modseq: Option<u64>,
}

impl StoredState {
    fn read(conn: &rusqlite::Connection, mailbox_id: i64) -> Self {
        let row = conn.query_row(
            "SELECT uid_validity, backfill_uid, uid_next, highest_modseq
               FROM mailbox WHERE id = ?1",
            rusqlite::params![mailbox_id],
            |row| {
                Ok(Self {
                    uid_validity: row.get::<_, Option<i64>>(0)?.map(|v| v as u32),
                    backfill_uid: row.get::<_, Option<i64>>(1)?.map(|v| v as u32),
                    uid_next: row.get::<_, Option<i64>>(2)?.map(|v| v as u32),
                    highest_modseq: row.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                })
            },
        );

        row.unwrap_or_default()
    }
}

/// The CONDSTORE path: fetch what arrived, reconcile what changed, and nothing else.
///
/// Split out rather than nested in `sync_mailbox` because the two paths share almost nothing:
/// this one never reads an envelope it already holds, and never walks a page.
#[allow(clippy::too_many_arguments)]
async fn incremental(
    app: &AppHandle,
    db: &Db,
    session: &mut ImapSession,
    caps: Caps,
    account_id: i64,
    mailbox_id: i64,
    path: &str,
    selected: &fetch::Selected,
    stored_modseq: u64,
    stored_uid_next: u32,
) -> Result<usize, SyncError> {
    let server_modseq = selected.highest_modseq.unwrap_or(stored_modseq);

    // Nothing has happened here since the last look. This is the common case across 45 of an
    // account's 46 mailboxes, and skipping it is the difference between a sync that costs one
    // round trip per mailbox and one that costs a fetch per mailbox.
    if server_modseq == stored_modseq && selected.uid_next == stored_uid_next {
        tracing::debug!(
            path,
            modseq = server_modseq,
            "unchanged since the last sync"
        );
        return Ok(0);
    }

    // ---- what arrived ---------------------------------------------------------------------
    let mut inserted = 0usize;

    if let Some(range) = fetch::arrivals_range(stored_uid_next, selected.uid_next) {
        let batch = fetch::envelopes(session, &range, caps).await?;

        if !batch.is_empty() {
            let written = {
                let batch = batch.clone();
                db.write(move |tx| {
                    let written = persist::write_batch(tx, account_id, mailbox_id, &batch)?;
                    persist::rethread(tx, account_id, RETHREAD_WINDOW)?;
                    Ok(written)
                })
                .await?
            };

            inserted = written.inserted;
            tracing::debug!(path, range, inserted, "incremental: new messages stored");
        }
    }

    // ---- what changed ---------------------------------------------------------------------
    // Asked for even when nothing arrived: a flag changing is a change, and it is the half of
    // "stays correct" that a UID-range sync cannot see at all.
    let changes = fetch::flags_changed_since(session, stored_modseq).await?;
    let changed = changes.len();

    if changed > 0 {
        db.write(move |tx| persist::apply_flag_changes(tx, mailbox_id, &changes))
            .await?;
        tracing::debug!(path, changed, "incremental: flags reconciled");
    }

    // Counts and state last, and in the same order as the full path: the badge is a cache of
    // rows that have now all been written.
    let uid_validity = selected.uid_validity;
    let uid_next = selected.uid_next;

    db.write(move |tx| {
        persist::recount(tx, mailbox_id)?;
        persist::record_mailbox_state(tx, mailbox_id, uid_validity, uid_next, Some(server_modseq))?;
        Ok(())
    })
    .await?;

    if inserted > 0 || changed > 0 {
        let _ = app.emit(
            "sync:progress",
            Progress {
                account_id,
                mailbox: path.to_string(),
                written: inserted,
                usable: true,
                done: false,
            },
        );
        let _ = app.emit("messages:added", mailbox_id);
    }

    Ok(inserted)
}

/// Fetches one message's body, caches the `.eml`, and stores what was parsed out of it.
///
/// docs/06 Phase 5 §3. Opens its own connection rather than borrowing the sync session: the
/// user is waiting for this one, and queueing it behind a backfill that has three hundred
/// batches left would make opening a message take minutes.
pub async fn fetch_body(
    app: &AppHandle,
    db: &Db,
    account_id: i64,
    message_ids: Vec<i64>,
) -> Result<usize, SyncError> {
    if message_ids.is_empty() {
        return Ok(0);
    }

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

    // Only the ones we do not already hold. The prefetch calls this with the next three rows
    // every time the selection moves, and re-fetching a cached body would turn an arrow-key
    // press into three round trips.
    let wanted = {
        let ids = message_ids.clone();
        db.read(move |conn| {
            let mut out = Vec::new();

            for id in ids {
                let row: Option<(i64, String, String)> = conn
                    .query_row(
                        "SELECT m.uid, m.body_state, b.remote_path
                           FROM message m
                           JOIN mailbox b ON b.id = m.mailbox_id
                          WHERE m.id = ?1",
                        rusqlite::params![id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .ok();

                if let Some((uid, state, path)) = row {
                    if state != "full" {
                        out.push((id, uid as u32, path));
                    }
                }
            }

            Ok(out)
        })
        .await?
    };

    if wanted.is_empty() {
        return Ok(0);
    }

    let credential = credential_for(db, &account).await?;
    let (mut session, caps) = session::connect(&imap, &account.email, &credential).await?;

    let root = crate::db::default_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();

    let mut stored = 0usize;
    let mut current_mailbox = String::new();

    for (message_id, uid, path) in wanted {
        // SELECT only when the mailbox changes. A thread whose messages are all in the Inbox
        // is the common case, and re-selecting per message would triple the round trips.
        if path != current_mailbox {
            if fetch::select(&mut session, &path, None, caps)
                .await
                .is_err()
            {
                continue;
            }
            current_mailbox = path.clone();
        }

        let raw = match bodies::fetch(&mut session, uid).await {
            Ok(raw) if !raw.is_empty() => raw,
            Ok(_) => continue,
            Err(error) => {
                // One unreadable message must not abandon the rest of the batch — a body
                // over the size cap is the usual reason, and the next row is fine.
                tracing::debug!(message_id, %error, "body fetch failed; continuing");
                continue;
            }
        };

        let parsed = bodies::parse(&raw);
        let cached = bodies::write_cache(&root, account_id, message_id, &raw).ok();

        db.write(move |tx| bodies::persist(tx, message_id, &parsed, cached.as_deref()))
            .await?;

        stored += 1;
    }

    let _ = session.logout().await;

    if stored > 0 {
        let _ = app.emit("messages:updated", message_ids);
    }

    Ok(stored)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_failure_produces_a_sentence_rather_than_protocol_text() {
        // These reach a banner in the UI. A hostname and an IMAP response code in front of
        // someone reading their mail is noise they cannot act on.
        let cases = [
            SyncError::Rejected {
                host: "imap.gmail.com".into(),
                detail: "NO [AUTHENTICATIONFAILED]".into(),
            },
            SyncError::Timeout {
                host: "imap.gmail.com".into(),
            },
            SyncError::Insecure {
                host: "imap.gmail.com".into(),
                port: 143,
            },
            SyncError::UidValidityChanged {
                mailbox: "INBOX".into(),
                stored: 1,
                found: 2,
            },
            SyncError::ShuttingDown,
        ];

        for error in cases {
            let described = describe(&error);

            assert!(!described.is_empty());
            assert!(described.ends_with('.'), "{described}");
            assert!(!described.contains("imap.gmail.com"), "{described}");
            assert!(!described.contains("AUTHENTICATIONFAILED"), "{described}");
        }
    }

    #[test]
    fn a_misconfigured_account_is_never_retried() {
        // Found by running the engine: three demo accounts with no IMAP host each backed off
        // through five attempts — about thirty seconds apiece — before the account that
        // actually worked was reached. Waiting does not add a hostname to a row.
        assert!(!SyncError::NotConfigured {
            email: "ada@example.test".into()
        }
        .is_retryable());

        assert!(!SyncError::Insecure {
            host: "imap.example.test".into(),
            port: 143
        }
        .is_retryable());

        // Weather, by contrast, is always worth another go.
        assert!(SyncError::Timeout {
            host: "imap.example.test".into()
        }
        .is_retryable());
    }

    #[test]
    fn a_missing_client_secret_is_not_reported_as_a_rejected_sign_in() {
        // Google refuses a refresh without the secret and calls it `invalid_request`, which
        // is indistinguishable from a bad credential. The obvious remedy — sign in again —
        // is a browser round trip that cannot possibly fix it, so the message says so.
        let described = describe(&SyncError::MissingClientSecret {
            provider: "google".into(),
        });

        assert!(described.contains("client secret"), "{described}");
        assert!(described.contains("Settings"), "{described}");
        assert!(
            described.contains("signing in again will not help"),
            "it must rule out the wrong remedy: {described}"
        );

        assert!(!SyncError::MissingClientSecret {
            provider: "google".into()
        }
        .is_retryable());
    }

    #[test]
    fn an_unconfigured_account_says_what_to_do_about_it() {
        let described = describe(&SyncError::NotConfigured {
            email: "ada@example.test".into(),
        });

        assert!(described.contains("Settings"), "{described}");
        assert!(!described.contains("ada@example.test"), "{described}");
    }

    #[test]
    fn a_refused_sign_in_tells_the_user_what_will_fix_it() {
        // The one error with a specific remedy: everything else is "try later", this one
        // needs the user to do something.
        let described = describe(&SyncError::Rejected {
            host: "imap.gmail.com".into(),
            detail: "invalid_grant".into(),
        });

        assert!(described.contains("Signing in again"), "{described}");
    }
}
