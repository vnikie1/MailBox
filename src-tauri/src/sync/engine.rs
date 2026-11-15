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
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::accounts::credentials::{self, Kind};
use crate::accounts::provider::{AuthKind, Provider, ServerSettings};
use crate::accounts::store::AccountDetail;
use crate::accounts::{self};
use crate::db::Db;

use super::events::{payload, Events};

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

/// The ceiling on the single full threading pass at the end of a sync.
///
/// Large enough not to be a limit on any real mailbox, and finite so that a corrupt account_id
/// cannot turn the last step of a sync into an unbounded scan. The pass is affordable because
/// `ix_msg_account_recent` and `ix_msg_thread_recent` exist — before those indexes it would
/// have taken minutes rather than seconds, which is presumably why it was never written.
pub(crate) const FULL_RETHREAD: usize = 1_000_000;

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
    /// Whether a push is already waiting to go. See [`SyncEngine::push_soon`].
    push_waiting: Arc<std::sync::atomic::AtomicBool>,
    /// Accounts whose sign-in the server has refused, left alone until it changes.
    ///
    /// A refused password used to be tried again every five minutes — by the safety net, by
    /// IDLE resuming, by every manual sync — and repeated failed sign-ins are exactly how a
    /// provider locks an account (H16, 2026-11-14 audit). Held here rather than stored: after a
    /// restart the account is tried once, which is one attempt a launch, not one every five
    /// minutes. A new password or a new sign-in clears it.
    refused: Arc<std::sync::Mutex<std::collections::HashSet<i64>>>,
}

/// How long a push waits for more changes before it goes.
///
/// Changes come in bursts — Mark All as Read, a run of Delete presses, a rule over a selection —
/// and one connection for the burst is the point. Short enough that someone who marks a message
/// read and picks up their phone finds it read there.
const PUSH_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

impl Default for SyncEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncEngine {
    pub fn new() -> Self {
        Self {
            locks: Arc::new(Mutex::new(HashMap::new())),
            push_waiting: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            refused: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    fn refusals(&self) -> std::sync::MutexGuard<'_, std::collections::HashSet<i64>> {
        self.refused
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Records that the server refused this account's sign-in. See `refused`.
    pub fn mark_refused(&self, account_id: i64) {
        self.refusals().insert(account_id);
    }

    /// Lets the account be tried again: its password, or its sign-in, has changed.
    pub fn clear_refusal(&self, account_id: i64) {
        self.refusals().remove(&account_id);
    }

    /// Sends what the user has changed, shortly, without a whole sync.
    ///
    /// The queue used to be drained only at the start of a sync, and with IDLE doing its job a
    /// sync starts when the *Inbox* changes — or at the five-minute safety net. So marking a
    /// folder read, flagging a message or moving one reached the server whenever something else
    /// happened to start a sync. Driving the built app against the Dovecot rig found it: Mark All
    /// Messages as Read on a folder, and the server still had every message unread minutes later.
    /// docs/03 §3 describes a worker that drains `pending_op`; this is what wakes it.
    ///
    /// Only the queue — connect, drain, disconnect. A whole pass per flag change would re-read
    /// every folder the account has. One push at a time for the whole app, after `PUSH_DELAY`,
    /// and then one per account that has anything waiting, each under that account's lock so it
    /// cannot interleave with a sync.
    pub async fn push_soon(&self, app: Arc<dyn Events>, db: Db) {
        use std::sync::atomic::Ordering;

        if self.push_waiting.swap(true, Ordering::SeqCst) {
            return;
        }

        tokio::time::sleep(PUSH_DELAY).await;
        // Cleared before reading the queue, so a change made from here on schedules a push of its
        // own rather than being assumed to be in this one.
        self.push_waiting.store(false, Ordering::SeqCst);

        let accounts = match db.read(accounts_with_pending_ops).await {
            Ok(accounts) => accounts,
            Err(error) => {
                tracing::warn!(%error, "could not read the queue to push it");
                return;
            }
        };

        for account_id in accounts {
            let engine = self.clone();
            let app = Arc::clone(&app);
            let db = db.clone();

            tokio::spawn(async move {
                let lock = engine.lock_for(account_id).await;
                let _guard = lock.lock().await;

                // Not surfaced as an account error. The changes are safe in the queue, and the
                // next sync — which does report — sends them.
                if let Err(error) = push_once(app.as_ref(), &db, account_id).await {
                    tracing::info!(account_id, %error, "queued changes not sent yet");
                }
            });
        }
    }

    /// Sends what is queued, then syncs only these mailboxes of the account.
    ///
    /// The pass Rebuild asks for: reading one folder again has no reason to wait for a pass over
    /// every other folder of the account — which, for an account with a large Inbox, is minutes.
    /// No retries: a pass that fails leaves the rebuild asked for, and the account's next full
    /// sync does it.
    pub async fn sync_mailboxes(
        &self,
        app: &dyn Events,
        db: &Db,
        account_id: i64,
        mailbox_ids: &[i64],
    ) -> Result<(), SyncError> {
        let lock = self.lock_for(account_id).await;
        let _guard = lock.lock().await;

        run_some(app, db, account_id, mailbox_ids).await
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
        app: &dyn Events,
        db: &Db,
        account_id: i64,
    ) -> Result<(), SyncError> {
        let lock = self.lock_for(account_id).await;
        let _guard = lock.lock().await;

        // Refused before, and nothing has changed since: not asked again. The window already
        // shows the refusal and its way out, so nothing new is announced either.
        if self.refusals().contains(&account_id) {
            tracing::debug!(
                account_id,
                "sync skipped: the sign-in was refused and has not changed"
            );
            return Err(SyncError::Rejected {
                host: String::new(),
                detail: "waiting for a new password or sign-in".into(),
            });
        }

        let mut backoff = Backoff::new();

        loop {
            match run_once(app, db, account_id).await {
                Ok(()) => {
                    backoff.reset();
                    return Ok(());
                }

                Err(error) if !error.is_retryable() => {
                    tracing::warn!(account_id, %error, "sync stopped: not retryable");

                    if matches!(error, SyncError::Rejected { .. }) {
                        self.mark_refused(account_id);
                    }

                    app.emit(
                        "account:error",
                        payload(&AccountError {
                            account_id,
                            message: describe(&error),
                            retry_in_seconds: 0,
                            needs_reauth: matches!(error, SyncError::Rejected { .. }),
                        }),
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
                        app.emit(
                            "account:error",
                            payload(&AccountError {
                                account_id,
                                message: describe(&error),
                                retry_in_seconds: delay.as_secs(),
                                needs_reauth: false,
                            }),
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

/// Whether a failed token refresh means the credential is bad or the provider is having a bad
/// minute. The two need opposite handling, and only one of them should stop an account.
///
/// Split out so the decision is testable without a network or a provider, the same way
/// `outbox::resolve_interrupted` is split from the IMAP lookup it depends on.
///
/// Every one of these used to become [`SyncError::Rejected`], which is not retryable, which
/// stops the account and raises "The saved sign-in for this account was refused. Signing in
/// again will fix it." So a dropped connection that happened to coincide with the access token
/// expiring told the user their credential was bad, sent them through a sign-in that fixed
/// nothing, and stopped their mail until they did it.
///
/// `session.rs` already draws this line for the IMAP login, and its comment records what
/// conflating them cost: a soak run where the auth backend was down for ninety seconds and the
/// client stopped checking mail "for the next six and a half hours" and said nothing. A token
/// refresh is the same decision one layer up, and never got the same treatment — though
/// `OAuthError` has distinguished the cases all along.
fn oauth_failure(
    provider: Provider,
    email: &str,
    error: &accounts::oauth::OAuthError,
) -> SyncError {
    match error {
        // The provider answered, and said no — to the *application*, not to the account.
        // `invalid_client` is Google's reply to a missing or wrong client secret, and it used
        // to fall into the arm below and stop the account with "Signing in again will fix it".
        // It is the one refusal a new sign-in cannot mend, so it is sorted out first.
        accounts::oauth::OAuthError::Refused { .. }
            if accounts::oauth::indicates_client_misconfiguration(error) =>
        {
            SyncError::OauthClientUnusable {
                provider: provider.id().to_string(),
                configured: true,
            }
        }
        // The provider answered, and said no. `invalid_grant` is a revoked or expired refresh
        // token, and no amount of retrying turns that into a yes.
        //
        // **Only** `invalid_grant`. Every refusal used to land here, including a token endpoint
        // answering 500, 503 or 429 — a bad minute, not a dead grant — so the account stopped
        // with "Signing in again will fix it" and a message being sent at that moment was failed
        // for good (M27, 2026-11-14 audit). Those fall through to the arm below and are retried.
        accounts::oauth::OAuthError::Refused { .. }
            if accounts::oauth::requires_reauthentication(error) =>
        {
            SyncError::Rejected {
                host: email.to_string(),
                detail: error.to_string(),
            }
        }
        // The provider did not answer, or not usefully. The stored credential may be perfectly
        // good, so keep the account alive and try again rather than blaming the user for it.
        _ => SyncError::AuthUnavailable {
            host: email.to_string(),
            detail: error.to_string(),
        },
    }
}

/// How long ago an account last signed in through the browser, for the log.
///
/// The figure that tells a scheduled expiry from a revocation: Google ends every refresh token
/// of an OAuth application in testing exactly seven days after it is issued, so a refusal at
/// "7.0 days ago" is that and nothing else. See `accounts::write_signed_in`.
fn sign_in_age(signed_in: Option<i64>, now: i64) -> String {
    match signed_in {
        Some(at) => format!("{:.1} days ago", (now - at) as f64 / 86_400.0),
        None => "unknown".to_string(),
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
        // Deliberately not "sign in again". The credential is fine and the server is not, so
        // sending the user through a sign-in would waste their time and teach them that the
        // banner lies.
        SyncError::AuthUnavailable { .. } => {
            "The mail server could not complete the sign-in. Halcyon will keep trying.".to_string()
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
        // The `\` is a line continuation, not a `\n`. It was written as an escaped newline
        // followed by thirteen spaces of source indentation, which went into the string and
        // only survived unnoticed because the banner's CSS leaves `white-space` at its
        // default and HTML collapses the run.
        SyncError::MissingClientSecret { .. } => {
            "Google needs the client secret for your sign-in application before it will \
             refresh this account. Paste it into Settings — signing in again will not help."
                .to_string()
        }
        SyncError::OauthClientUnusable { configured, .. } => {
            if *configured {
                "The provider rejected Halcyon's sign-in application, not your account. Check \
                 the client ID and secret in Settings → Accounts → Sign-in applications."
                    .to_string()
            } else {
                "This account signs in through a sign-in application, and none is configured. \
                 Add one in Settings → Accounts → Sign-in applications."
                    .to_string()
            }
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
                    // Not `Rejected`. This is Halcyon having no registration with the
                    // provider, which the user fixes in Settings — and as a `Rejected` it
                    // rendered as "Signing in again will fix it", so clearing a client id
                    // stopped every OAuth account at once and offered each one a button that
                    // could not possibly succeed.
                    .ok_or_else(|| SyncError::OauthClientUnusable {
                        provider: provider.id().to_string(),
                        configured: false,
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

            let (expiry, signed_in, issuer) = {
                let reference = reference.clone();
                db.read(move |conn| {
                    Ok((
                        accounts::read_expiry(conn, &reference),
                        accounts::read_signed_in(conn, &reference),
                        accounts::read_issuer(conn, &reference),
                    ))
                })
                .await?
            };

            // A token issued to another sign-in application cannot be refreshed by this one, and
            // the provider's refusal of it reads as a broken registration — "signing in again
            // will not help". It will: that is the only thing that will (M28, 2026-11-14 audit).
            // Said before asking, as the refusal signing in again cures. The access token still
            // works until it expires, so only a refresh is refused.
            if accounts::oauth::needs_refresh(expiry)
                && accounts::client_changed(issuer.as_deref(), &client.client_id)
            {
                tracing::warn!(
                    account_id = account.id,
                    "the sign-in application changed since this account signed in; it has to sign in again"
                );
                return Err(oauth_failure(
                    provider,
                    &account.email,
                    &accounts::oauth::OAuthError::Refused {
                        provider: provider.id().to_string(),
                        error: "invalid_grant".into(),
                        description: Some(
                            "the token was issued to a different sign-in application".into(),
                        ),
                    },
                ));
            }

            let (token, refreshed) = accounts::access_token(expiry, provider, &client, &reference)
                .await
                .map_err(|error| {
                    // The provider's own words, here and nowhere else. `SyncError::Rejected`
                    // deliberately renders without its detail — an IMAP server can echo a
                    // password back — so until this line the log of an expired Google sign-in
                    // read only "rejected the sign-in", the same as a wrong password, and the
                    // cause had to be worked out from timestamps. An `OAuthError` holds
                    // provider ids and error codes and nothing secret.
                    let description = match &error {
                        accounts::oauth::OAuthError::Refused { description, .. } => {
                            description.as_deref().unwrap_or("")
                        }
                        _ => "",
                    };
                    tracing::warn!(
                        account_id = account.id,
                        %error,
                        description,
                        signed_in = %sign_in_age(signed_in, accounts::oauth::now_seconds()),
                        "token refresh failed"
                    );

                    oauth_failure(provider, &account.email, &error)
                })?;

            if let Some(expires_at) = refreshed {
                let reference = reference.clone();
                // A refresh this client could make proves the token is this client's, which is
                // how a sign-in from before the issuer was recorded comes to have one.
                let client_id = issuer.is_none().then(|| client.client_id.clone());
                let _ = db
                    .write(move |tx| {
                        accounts::write_expiry(tx, &reference, expires_at)?;
                        if let Some(client_id) = client_id {
                            accounts::write_issuer(tx, &reference, &client_id)?;
                        }
                        Ok(())
                    })
                    .await;
            }

            Ok(Credential::OAuth(token))
        }
    }
}

/// The accounts that have changes waiting for their server.
fn accounts_with_pending_ops(conn: &rusqlite::Connection) -> Result<Vec<i64>, crate::db::DbError> {
    let mut statement =
        conn.prepare("SELECT DISTINCT account_id FROM pending_op ORDER BY account_id")?;
    let rows = statement.query_map([], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<i64>>>()?)
}

/// Sends one account's queue, and nothing else. See [`SyncEngine::push_soon`].
async fn push_once(app: &dyn Events, db: &Db, account_id: i64) -> Result<(), SyncError> {
    let waiting = db
        .write(move |tx| ops::pending_count(tx, account_id))
        .await?;
    if waiting == 0 {
        // A sync got there while this one waited for the lock.
        return Ok(());
    }

    let Some(account) = db
        .read(move |conn| crate::accounts::store::get(conn, account_id))
        .await?
    else {
        return Ok(());
    };

    // Switched off in Settings: the changes wait, exactly as they do for a sync, which
    // `sync_all` skips for such an account.
    if !account.sync_enabled {
        return Ok(());
    }

    let Some(imap) = account.imap.clone() else {
        return Ok(());
    };

    let credential = credential_for(db, &account).await?;
    let (mut session, caps) = session::connect(&imap, &account.email, &credential).await?;

    let drained = ops::drain(app, db, &mut session, account_id, caps.move_command).await;
    let _ = session.logout().await;

    let sent = drained?;
    tracing::debug!(account_id, sent, "queued changes pushed");
    Ok(())
}

/// The queue, then some of an account's mailboxes. See [`SyncEngine::sync_mailboxes`].
async fn run_some(
    app: &dyn Events,
    db: &Db,
    account_id: i64,
    mailbox_ids: &[i64],
) -> Result<(), SyncError> {
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

    tracing::info!(account_id, mailboxes = ?mailbox_ids, "partial sync starting");

    let credential = credential_for(db, &account).await?;
    let (mut session, caps) = session::connect(&imap, &account.email, &credential).await?;

    // First, for the reason `run_once` gives.
    ops::drain(app, db, &mut session, account_id, caps.move_command).await?;

    for &mailbox_id in mailbox_ids {
        let found: Option<(String, Option<String>)> = db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT remote_path, role FROM mailbox WHERE id = ?1 AND account_id = ?2",
                        rusqlite::params![mailbox_id, account_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .ok())
            })
            .await?;

        // Gone meanwhile, or Gmail's All Mail, which `run_once` never syncs either.
        let Some((path, role)) = found else {
            continue;
        };
        if role.as_deref() == Some(mailboxes::Role::All.as_str()) {
            continue;
        }

        let backfill = role.as_deref() == Some(mailboxes::Role::Inbox.as_str());
        let reopen = Reopen {
            imap: &imap,
            email: &account.email,
            credential: &credential,
        };
        sync_mailbox_readably(
            app,
            db,
            &mut session,
            &reopen,
            caps,
            account_id,
            mailbox_id,
            &path,
            backfill,
        )
        .await?;
    }

    let _ = session.logout().await;
    tracing::info!(account_id, "partial sync finished");
    Ok(())
}

/// One attempt at a full pass.
async fn run_once(app: &dyn Events, db: &Db, account_id: i64) -> Result<(), SyncError> {
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
    ops::drain(app, db, &mut session, account_id, caps.move_command).await?;

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

    let persisted = {
        let to_persist = selectable.clone();
        db.write(move |tx| mailboxes::persist(tx, account_id, &to_persist))
            .await?
    };

    // Paired by path, not by position. `persist` skips a listed mailbox the user has renamed or
    // deleted since the LIST — the server has not heard yet — so its answer can be shorter than
    // the list, and a zip would sync each folder's mail into its neighbour's row.
    let targets: Vec<(mailboxes::Discovered, i64)> = selectable
        .iter()
        .filter_map(|mailbox| {
            persisted
                .iter()
                .find(|(_, path)| path == &mailbox.remote_path)
                .map(|(id, _)| (mailbox.clone(), *id))
        })
        .collect();

    // Folders the server no longer has. `discover` returns an error rather than a short list
    // if the LIST breaks off, so an Ok result of non-zero length is the whole tree — which is
    // the condition `prune` requires before it deletes anything, because deleting a mailbox
    // takes its messages with it.
    if !selectable.is_empty() {
        let keep: Vec<String> = selectable
            .iter()
            .map(|mailbox| mailbox.remote_path.clone())
            .collect();

        let removed = db
            .write(move |tx| mailboxes::prune(tx, account_id, &keep))
            .await?;

        if !removed.is_empty() {
            tracing::info!(
                account_id,
                removed = removed.len(),
                paths = ?removed,
                "mailboxes gone from the server were removed"
            );
        }
    }

    app.emit("mailboxes:changed", payload(&account_id));

    // ---- 2. the Inbox, newest page first ------------------------------------------------
    // docs/03 §5 orders this deliberately: the Inbox is what the user is looking at, and its
    // newest page is what they see. Everything else waits.
    let inbox = targets
        .iter()
        .find(|(mailbox, _)| mailbox.role == Some(mailboxes::Role::Inbox))
        .map(|(mailbox, id)| (mailbox.remote_path.clone(), *id));

    // Counted so "sync finished" can say what it did. A run that logs only its own start and
    // end is indistinguishable from a run that hung — which is exactly how a 46-mailbox
    // account read for the several minutes it was working correctly.
    let mut inserted_total = 0usize;
    let mut synced = 0usize;
    let mut failed = 0usize;

    // ---- 3. everything else, newest 200 each -------------------------------------------
    //
    // In one loop with the Inbox, which goes first. The Inbox used to be synced on its own with
    // `?`, so anything that failed it — one message the parser could not read was enough —
    // ended the account's pass before a single other folder was read, on every pass (M2,
    // 2026-11-14 audit). Now it fails like any other mailbox: counted, and the pass goes on.
    let mut order: Vec<(String, i64, bool)> = Vec::new();
    if let Some((path, mailbox_id)) = inbox {
        order.push((path, mailbox_id, true));
    }
    for (mailbox, mailbox_id) in &targets {
        if mailbox.role == Some(mailboxes::Role::Inbox) {
            continue;
        }

        // Gmail's All Mail contains every message in the account a second time. Syncing it
        // alongside the labels would double the entire mailbox — docs/03 §5 names this.
        if mailbox.role == Some(mailboxes::Role::All) {
            tracing::debug!(path = %mailbox.remote_path, "skipping All Mail; its messages are in their labels");
            continue;
        }

        order.push((mailbox.remote_path.clone(), *mailbox_id, false));
    }

    // One reconnect per pass. A dropped connection used to fail every remaining mailbox in the
    // pass at once, over the dead session — 29 of 31 Yahoo folders in four milliseconds, from
    // one drop (M1, 2026-11-14 audit). Now the session is opened again and the mailbox it failed
    // on is tried once more; a second drop ends the pass, and the next sync starts afresh.
    let mut reconnected = false;
    let reopen = Reopen {
        imap: &imap,
        email: &account.email,
        credential: &credential,
    };

    for (path, mailbox_id, backfill) in &order {
        loop {
            let result = sync_mailbox_readably(
                app,
                db,
                &mut session,
                &reopen,
                caps,
                account_id,
                *mailbox_id,
                path,
                *backfill,
            )
            .await;

            match result {
                Ok(inserted) => {
                    inserted_total += inserted;
                    synced += 1;
                }

                Err(error) if error.connection_lost() && !reconnected => {
                    tracing::warn!(path, %error, "the connection dropped; reconnecting once");
                    reconnected = true;
                    session = reopen.open().await?;
                    continue;
                }

                // Gone again. Every mailbox after this would fail the same way, instantly.
                Err(error) if error.connection_lost() => return Err(error),

                // One unreadable mailbox must not abort the account. A shared folder whose
                // permissions changed is common, and losing the Inbox because of it is not.
                Err(error) => {
                    failed += 1;
                    tracing::warn!(path, %error, "mailbox sync failed; continuing");
                }
            }
            break;
        }
    }

    let _ = session.logout().await;

    // ---- one full threading pass -----------------------------------------------------------
    // The per-batch re-thread above only covers the most recent `RETHREAD_WINDOW` messages,
    // which is right during a sync — a full pass after every 500-message batch would hold the
    // writer for the whole backfill — but it leaves everything older than that window with no
    // thread at all.
    //
    // The window's own comment has claimed since Phase 5 that "a full pass runs once at the end
    // of the initial sync". It did not. The Dovecot gate is what found it: a cold sync of
    // 50,000 messages stored every one of them correctly and left **45,000 without a thread**,
    // because only the newest 5,000 were ever threaded. Nothing failed, no test noticed, and
    // the reader would have shown nine messages in ten as a conversation of one.
    // Only when something actually needs it. The pass costs about thirty seconds on a
    // 50,000-message account, and paying that on every incremental sync would undo the point of
    // having one — on a steady-state mailbox the count is zero and nothing runs.
    let outstanding = db
        .write(move |tx| persist::unthreaded_count(tx, account_id))
        .await?;

    if outstanding > 0 {
        let threaded = db
            .write(move |tx| persist::rethread(tx, account_id, FULL_RETHREAD))
            .await?;

        tracing::debug!(
            account_id,
            outstanding,
            threaded,
            "full threading pass finished"
        );
    }

    app.emit(
        "sync:progress",
        payload(&Progress {
            account_id,
            mailbox: String::new(),
            written: 0,
            usable: true,
            done: true,
        }),
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
    app: &dyn Events,
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
        backfill_uid: mut backfilled_to,
        uid_next: mut stored_uid_next,
        highest_modseq: mut stored_modseq,
    } = stored_state;

    // ---- Rebuild ---------------------------------------------------------------------------
    // Asked for from the mailbox menu (`folders::request_rebuild`). This pass reads the whole
    // mailbox again: every message's envelope and flags, removing what the server no longer
    // has, and downloading every cached body again. Not while anything queued still names the
    // mailbox — the drain at the start of the pass sent what it could, and a change still
    // waiting would be read back from the server before the server had made it.
    let rebuilding = if db
        .read(move |conn| persist::rebuild_requested(conn, mailbox_id))
        .await?
    {
        let owned = path.to_string();
        let waiting = db
            .write(move |tx| ops::names_mailbox(tx, account_id, &owned))
            .await?;

        if waiting {
            tracing::info!(
                path,
                "rebuild waits for changes still on their way to the server"
            );
            false
        } else {
            db.write(move |tx| persist::begin_rebuild(tx, mailbox_id))
                .await?;
            // As `begin_rebuild` left them: the full path, and a backfill from the top.
            backfilled_to = None;
            stored_modseq = None;
            tracing::info!(path, "rebuilding the mailbox");
            true
        }
    } else {
        false
    };
    let refresh = if rebuilding {
        persist::Refresh::Everything
    } else {
        persist::Refresh::Flags
    };

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

            // The state read a moment ago described the mailbox that no longer exists, and
            // `drop_mailbox_contents` has just cleared its stored copy. Forgetting it here too
            // is what makes the re-sync a re-sync.
            //
            // The gate found this by leaving it out: after a reset the backfill marker still
            // said "complete" from before the drop, so the mailbox was refetched to exactly one
            // page — 500 messages of 50,000 — and reported success.
            backfilled_to = None;
            stored_uid_next = None;
            stored_modseq = None;

            fetch::select(session, path, None, caps).await?
        }

        Err(error) => return Err(error),
    };

    // From here on, what this mailbox can be asked: a server with CONDSTORE can still keep no
    // modification sequences for one mailbox, and asking it for them fails every fetch (L2).
    let caps = caps.for_mailbox(selected.highest_modseq);

    if selected.uid_next == 0 && selected.exists == 0 {
        tracing::debug!(path, "mailbox is empty");

        if rebuilding {
            db.write(move |tx| persist::remove_all_numbered(tx, mailbox_id))
                .await?;
            finish_rebuild(app, db, session, account_id, mailbox_id, path).await?;
        }
        return Ok(0);
    }

    tracing::debug!(
        path,
        exists = selected.exists,
        uid_next = selected.uid_next,
        backfill,
        rebuilding,
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
    //
    // **And whenever a backfill is still outstanding**, which is not an optimisation but a
    // correctness rule, and the Dovecot gate is what found it. The first sync of a large
    // mailbox records `uid_next` and the MODSEQ after its *first page*; if it is then
    // interrupted — a closed lid, a dropped connection, the gate stopping the server — the next
    // sync sees a MODSEQ that has not moved, concludes "unchanged since the last sync", and
    // returns before reaching the backfill at all.
    //
    // Measured: a cold sync killed after 5,000 of 50,000 messages, re-run, finished in 1.6
    // seconds having fetched nothing. The remaining 45,000 would never have arrived, and
    // nothing anywhere would have said so.
    let backfill_outstanding = backfill && backfilled_to != Some(1);

    if let (true, false, Some(stored_modseq), Some(stored_uid_next), Some(server_modseq)) = (
        caps.has_modseq(),
        backfill_outstanding,
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

    // Arrivals the page will not reach (M5), fetched after it. The Inbox only: every other folder
    // is held to its newest page by design. Until they are in, the last sync's `uid_next` stays
    // recorded, so a pass interrupted part-way asks for them again instead of forgetting them.
    let missed = if backfill {
        stored_uid_next
            .and_then(|previous| fetch::missed_range(previous, selected.uid_next, first_page))
    } else {
        None
    };
    let recorded_uid_next = match (missed, stored_uid_next) {
        (Some(_), Some(previous)) => previous,
        _ => selected.uid_next,
    };
    let recorded_modseq =
        modseq_to_record(backfill_outstanding, stored_modseq, selected.highest_modseq);

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
            let written = persist::write_batch_as(tx, account_id, mailbox_id, &batch, refresh)?;
            let batch_ms = started.elapsed().as_millis() as u64;

            let started = std::time::Instant::now();
            persist::recount(tx, mailbox_id)?;
            persist::record_mailbox_state(
                tx,
                mailbox_id,
                selected.uid_validity,
                recorded_uid_next,
                recorded_modseq,
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

    app.emit(
        "sync:progress",
        payload(&Progress {
            account_id,
            mailbox: path.to_string(),
            written: written.inserted,
            usable: true,
            done: false,
        }),
    );

    app.emit("messages:added", payload(&mailbox_id));

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

    // ---- arrivals below the newest page (M5) ------------------------------------------------
    let mut inserted_ids = written.inserted_ids.clone();
    let mut missed_inserted = 0usize;

    if let Some((lowest, highest)) = missed {
        let uids = fetch::uids_between(session, lowest, highest).await?;
        tracing::debug!(
            path,
            lowest,
            highest,
            found = uids.len(),
            "fetching arrivals below the newest page"
        );

        let mut cursor = highest.saturating_add(1);
        while let Some((set, bottom)) = fetch::backfill_window(&uids, cursor, BACKFILL_BATCH) {
            let batch = fetch::envelopes(session, &set, caps).await?;
            let more = db
                .write(move |tx| {
                    let written =
                        persist::write_batch_as(tx, account_id, mailbox_id, &batch, refresh)?;
                    persist::recount(tx, mailbox_id)?;
                    persist::rethread(tx, account_id, RETHREAD_WINDOW)?;
                    Ok(written)
                })
                .await?;

            missed_inserted += more.inserted;
            inserted_ids.extend(more.inserted_ids);
            cursor = bottom;
        }

        // All in, so the next sync may start from here. MODSEQ is left as the page recorded it.
        let (validity, uid_next) = (selected.uid_validity, selected.uid_next);
        db.write(move |tx| persist::record_mailbox_state(tx, mailbox_id, validity, uid_next, None))
            .await?;

        if missed_inserted > 0 {
            app.emit("messages:added", payload(&mailbox_id));
        }
    }

    // ---- arrivals and departures, for a server that cannot tell us ----------------------
    //
    // `incremental` does this and is unreachable without CONDSTORE or QRESYNC: it needs a
    // HIGHESTMODSEQ that such a server never sends. So on those accounts rules never ran, junk
    // was never filed, no new-mail notification ever appeared, and mail deleted on another
    // device was never removed here. All four features simply did not happen, and nothing said
    // so — the sync looked entirely healthy.
    //
    // What counts as an arrival is a UID at or above the `uid_next` recorded by the previous
    // sync. That is the same question CONDSTORE answers with a modseq, asked the only other way
    // IMAP offers. A mailbox with no previous `uid_next` has never been synced, so nothing
    // counts — which is exactly the guard `on_arrival` insists on, and the reason its comment
    // warns that running rules over an initial sync would "empty their Inbox on first launch".
    // Keyed on having a baseline rather than on what the server advertises. A CONDSTORE
    // server also takes this path while its backfill is outstanding, and mail arriving during a
    // long backfill deserves its rules and its notification just as much.
    if let Some(previous_uid_next) = stored_uid_next.filter(|next| *next > 0) {
        let arrived = {
            let ids = inserted_ids.clone();
            db.read(move |conn| new_since(conn, mailbox_id, previous_uid_next, &ids))
                .await?
        };

        on_arrival(app, db, account_id, mailbox_id, path, &arrived).await;
    }

    // And what left. Without this a message deleted in webmail stayed here for ever on a
    // server without CONDSTORE, because the only other caller is on the path it cannot reach.
    //
    // Cheap when nothing has gone: `reconcile_expunged` counts the local rows first and returns
    // before touching the network unless there are more here than the server says it has. That
    // also makes it safe during an initial sync or a backfill, when this side is behind rather
    // than ahead. A rebuild asks every time: a message the server lost and another it gained
    // leave the counts agreeing and the copy wrong about both.
    let expunged = reconcile_expunged(
        db,
        session,
        mailbox_id,
        path,
        selected.exists,
        stored_uid_next != Some(selected.uid_next),
        rebuilding,
    )
    .await?;
    if expunged > 0 {
        db.write(move |tx| persist::recount(tx, mailbox_id)).await?;
        app.emit("mailbox:changed", payload(&mailbox_id));
    }

    let mut total = written.inserted + missed_inserted;

    if !backfill {
        if rebuilding {
            refresh_held(
                db,
                session,
                caps,
                account_id,
                mailbox_id,
                written.lowest_uid,
            )
            .await?;
            finish_rebuild(app, db, session, account_id, mailbox_id, path).await?;
        }
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
    let mut cursor = backfill_start(backfilled_to, written.lowest_uid, &uids);

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
                let written =
                    persist::write_batch_as(tx, account_id, mailbox_id, &batch_for_write, refresh)?;
                persist::recount(tx, mailbox_id)?;
                persist::rethread(tx, account_id, RETHREAD_WINDOW)?;
                Ok(written)
            })
            .await?;

        app.emit(
            "sync:progress",
            payload(&Progress {
                account_id,
                mailbox: path.to_string(),
                written: written.inserted,
                usable: true,
                done: false,
            }),
        );
        app.emit("messages:added", payload(&mailbox_id));

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

    if rebuilding {
        finish_rebuild(app, db, session, account_id, mailbox_id, path).await?;
    }

    Ok(total)
}

/// What a finished rebuild tells the window.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Rebuilt {
    account_id: i64,
    mailbox_id: i64,
    /// How many messages the mailbox holds here now.
    messages: i64,
}

/// Reads again the messages a folder holds beyond its newest page. Part of Rebuild.
///
/// The Inbox's backfill reads every message anyway. Any other folder is only ever read to its
/// newest page, and the rest of what it holds here — older mail kept from earlier syncs, mail
/// moved in — would otherwise keep whatever the store had, which is the copy in doubt.
async fn refresh_held(
    db: &Db,
    session: &mut ImapSession,
    caps: Caps,
    account_id: i64,
    mailbox_id: i64,
    below: u32,
) -> Result<(), SyncError> {
    let held = db
        .read(move |conn| persist::held_uids(conn, mailbox_id))
        .await?;

    // Below the page just written — or all of it, when that page came back empty.
    let mut cursor = if below == 0 { u32::MAX } else { below };

    while let Some((set, lowest)) = fetch::backfill_window(&held, cursor, BACKFILL_BATCH) {
        let batch = fetch::envelopes(session, &set, caps).await?;
        db.write(move |tx| {
            persist::write_batch_as(
                tx,
                account_id,
                mailbox_id,
                &batch,
                persist::Refresh::Everything,
            )?;
            Ok(())
        })
        .await?;
        cursor = lowest;
    }

    Ok(())
}

/// The end of a rebuild: every cached body downloaded again, the request cleared, and the
/// window told.
///
/// A body that does not come back keeps the copy already cached — one over the size cap, or one
/// the server has stopped returning, is better kept than lost. A dropped connection is another
/// matter, and ends the pass with the request still set, so the next sync rebuilds again.
async fn finish_rebuild(
    app: &dyn Events,
    db: &Db,
    session: &mut ImapSession,
    account_id: i64,
    mailbox_id: i64,
    path: &str,
) -> Result<(), SyncError> {
    let cached = db
        .read(move |conn| persist::cached_bodies(conn, mailbox_id))
        .await?;
    let root = db.folder().to_path_buf();
    let mut refreshed: Vec<i64> = Vec::new();

    for (message_id, uid) in cached {
        let raw = match bodies::fetch(session, uid).await {
            Ok(raw) if !raw.is_empty() => raw,
            Ok(_) => continue,
            Err(SyncError::Imap(
                async_imap::error::Error::Bad(_) | async_imap::error::Error::No(_),
            )) => {
                tracing::debug!(message_id, uid, "rebuild: body not downloaded again; kept");
                continue;
            }
            Err(error) => return Err(error),
        };

        let parsed = bodies::parse(&raw);
        let cached = bodies::write_cache(&root, account_id, message_id, &raw).ok();
        db.write(move |tx| bodies::persist(tx, message_id, &parsed, cached.as_deref()))
            .await?;
        refreshed.push(message_id);
    }

    let messages = db
        .write(move |tx| persist::finish_rebuild(tx, mailbox_id))
        .await?;

    tracing::info!(path, messages, bodies = refreshed.len(), "mailbox rebuilt");

    if !refreshed.is_empty() {
        app.emit("messages:updated", payload(&refreshed));
    }
    app.emit("mailbox:changed", payload(&mailbox_id));
    app.emit(
        "mailbox:rebuilt",
        payload(&Rebuilt {
            account_id,
            mailbox_id,
            messages,
        }),
    );

    Ok(())
}

/// What it takes to open the account's connection again.
struct Reopen<'a> {
    imap: &'a ServerSettings,
    email: &'a str,
    credential: &'a Credential,
}

impl Reopen<'_> {
    async fn open(&self) -> Result<ImapSession, SyncError> {
        Ok(session::connect(self.imap, self.email, self.credential)
            .await?
            .0)
    }
}

/// [`sync_mailbox`], read again from plain headers when the server's answer could not be parsed.
///
/// A message whose `ENVELOPE` the IMAP parser cannot read failed its whole batch, and the same
/// batch was asked for on every pass, so the mailbox never got past it (M2, 2026-11-14 audit).
/// The session is replaced either way: async-imap keeps the unreadable bytes at the front of its
/// buffer, and nothing more can be read on it. See `Caps::headers_only`.
#[allow(clippy::too_many_arguments)]
async fn sync_mailbox_readably(
    app: &dyn Events,
    db: &Db,
    session: &mut ImapSession,
    reopen: &Reopen<'_>,
    caps: Caps,
    account_id: i64,
    mailbox_id: i64,
    path: &str,
    backfill: bool,
) -> Result<usize, SyncError> {
    let result = sync_mailbox(
        app, db, session, caps, account_id, mailbox_id, path, backfill,
    )
    .await;

    let Err(error) = &result else {
        return result;
    };
    if !error.unparseable() {
        return result;
    }

    tracing::warn!(
        path,
        %error,
        "the server sent something the IMAP parser cannot read; reading this mailbox from its \
         plain headers instead"
    );
    *session = reopen.open().await?;

    let plain = Caps {
        headers_only: true,
        ..caps
    };
    let result = sync_mailbox(
        app, db, session, plain, account_id, mailbox_id, path, backfill,
    )
    .await;

    // Still unreadable — the flags, say, rather than an envelope. The mailbox has failed, and the
    // next one needs a session that can still read.
    if result.as_ref().is_err_and(SyncError::unparseable) {
        *session = reopen.open().await?;
    }

    result
}

/// The MODSEQ a newest-page pass records, or `None` to keep the one stored.
///
/// Held while the Inbox's first download is unfinished (M6, 2026-11-14 audit). The incremental
/// pass that takes over afterwards asks the server for every flag changed since this value, so
/// it has to be no later than the moment the oldest message here was read. Each interrupted pass
/// recorded the server's newest instead, and a message read on a phone after the backfill had
/// stored it — but before that pass — was never asked about again: its flags stayed as the
/// backfill found them. Holding the first value makes the eventual question cover the lot.
fn modseq_to_record(
    backfill_outstanding: bool,
    stored: Option<u64>,
    server: Option<u64>,
) -> Option<u64> {
    if backfill_outstanding && stored.is_some() {
        None
    } else {
        server
    }
}

/// Where the backfill walk should start, given what is stored and what the newest page found.
///
/// `newest_lowest` is 0 when the newest page came back **empty**, and that is the case this
/// function exists for. The newest page is a numeric UID *range* — the 500 slots below
/// `uid_next` — and on a mailbox archived from for years those slots are mostly empty: the real
/// Gmail Inbox described above holds 214 messages spread across 106,287 UIDs, so 500 consecutive
/// slots holding nothing live is unremarkable rather than exotic.
///
/// The arithmetic this replaced was `stored.min(newest_lowest.max(1))`, which turned that 0 into
/// a floor of 1 — or 0 outright on a first sync. `backfill_window` keeps only UIDs strictly
/// below the cursor, so it returned `None` immediately, the walk never ran a single batch, and
/// control fell through to the line that records the backfill as **complete**. A mailbox could
/// therefore be marked fully downloaded having fetched nothing at all, and the marker only ever
/// moves downwards — `record_backfill_progress` uses `MIN(...)` — so nothing short of a
/// UIDVALIDITY change would ever revisit it. Every older message in that mailbox was gone for
/// good, with no error anywhere.
///
/// An empty page now simply carries no information, which is what it is: the walk falls back to
/// the stored progress, or to the whole UID list when there is none.
fn backfill_start(backfilled_to: Option<u32>, newest_lowest: u32, uids: &[u32]) -> u32 {
    // Zero is "the page told us nothing", not "start at the bottom".
    let newest = (newest_lowest > 0).then_some(newest_lowest);

    match (backfilled_to, newest) {
        (Some(stored), Some(bottom)) => stored.min(bottom),
        (Some(stored), None) => stored,
        (None, Some(bottom)) => bottom,
        // Nothing stored and nothing on the newest page: walk everything the server listed.
        // One above the highest known UID, because the window filter is strictly less-than.
        // An empty list leaves this 0, and a mailbox with no messages is correctly complete.
        (None, None) => uids.first().map_or(0, |highest| highest.saturating_add(1)),
    }
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

/// Removes local rows for messages the server no longer has. docs/03 §5.
///
/// ## Why this exists
///
/// Nothing else deletes a message that vanished from the server. Archive a message on your
/// phone, delete one in Gmail's web client, let a rule on the server file one away — the copy
/// here stayed for ever. The symptom is an Inbox full of mail the user has already dealt with
/// somewhere else, and it gets worse every day the app is used alongside another client.
///
/// It also explains a second symptom that looks unrelated: opening one of those messages showed
/// a blank body for ever, because the body fetch asked the server for a UID that no longer
/// existed and got nothing back.
///
/// ## Why a `UID SEARCH` and not `VANISHED`
///
/// docs/03 §5 says to use `VANISHED`, *"if QRESYNC"*. Gmail does not offer QRESYNC — this
/// account reports `qresync=false` — so on the server most people are using, the spec's answer
/// is not available. `UID SEARCH ALL` is: it works everywhere, and the response is a list of
/// integers rather than message data.
///
/// ## Why it is guarded by a count
///
/// The search is cheap but not free, and running it for every mailbox on every sync would be
/// 46 extra round trips a minute for an account where nothing has been deleted. `EXISTS` comes
/// back with the `SELECT` that has already happened, so comparing it to the local count costs
/// nothing and is right whenever it disagrees.
///
/// It is not a complete test: delete one message and receive another between two syncs and the
/// counts match again while the local copy is wrong about both. That case is caught anyway,
/// because the arrival changes `UIDNEXT` and the caller only reaches this point when something
/// changed. The pathological case — an equal number added and removed, with `UIDNEXT` and
/// `HIGHESTMODSEQ` both landing back where they started — cannot happen: `UIDNEXT` only ever
/// increases.
///
/// `force` skips that test, for a rebuild. And a server that says the mailbox is **empty** is
/// believed without a search: that is an answer rather than a fault, and a Bin emptied in webmail
/// used to keep every message here, because the search came back empty and an empty search is
/// (rightly) not trusted.
/// What this store holds of one mailbox, for [`expunge_check`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Held {
    /// Every row, parked moves included.
    all: i64,
    /// Rows with a server UID.
    numbered: i64,
    /// The lowest server UID held.
    lowest: Option<u32>,
}

/// Whether, and over what, to ask the server which messages it still has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpungeCheck {
    /// Nothing can have gone that this store would notice.
    Skip,
    /// The server says the mailbox is empty.
    Empty,
    /// Ask about every UID.
    All,
    /// Ask about this UID and everything above it: the range this store holds.
    From(u32),
}

/// Decides [`ExpungeCheck`]. Separated so it can be tested without a server.
///
/// A mailbox held **in full** keeps the old test: more here than there means something went.
///
/// A mailbox held **in part** — every folder but the Inbox past its newest 200 — can never have
/// more here than there, so the old test never fired and a message deleted elsewhere stayed here
/// for ever as a row that opened blank (H2, 2026-11-14 audit). For those the server is asked about
/// the range this store holds, and only when something has changed since the last look: its
/// count, or its `UIDNEXT`. A count never recorded — every folder the first time this version
/// runs — counts as changed, which clears the ghosts already there.
fn expunge_check(
    held: Held,
    server_exists: u32,
    last_exists: Option<u32>,
    arrived: bool,
    force: bool,
) -> ExpungeCheck {
    if server_exists == 0 {
        return if force || held.all > 0 {
            ExpungeCheck::Empty
        } else {
            ExpungeCheck::Skip
        };
    }

    let in_part = held.numbered < i64::from(server_exists);

    if force {
        return ExpungeCheck::All;
    }

    if !in_part {
        return if held.all > i64::from(server_exists) {
            ExpungeCheck::All
        } else {
            ExpungeCheck::Skip
        };
    }

    let changed = arrived || last_exists != Some(server_exists);
    match (changed, held.lowest) {
        (true, Some(lowest)) => ExpungeCheck::From(lowest),
        _ => ExpungeCheck::Skip,
    }
}

async fn reconcile_expunged(
    db: &Db,
    session: &mut ImapSession,
    mailbox_id: i64,
    path: &str,
    server_exists: u32,
    arrived: bool,
    force: bool,
) -> Result<usize, SyncError> {
    let (held, last_exists) = db
        .read(move |conn| {
            let held = conn
                .query_row(
                    "SELECT COUNT(*), COUNT(CASE WHEN uid > 0 THEN 1 END),
                            MIN(CASE WHEN uid > 0 THEN uid END)
                       FROM message WHERE mailbox_id = ?1",
                    rusqlite::params![mailbox_id],
                    |row| {
                        Ok(Held {
                            all: row.get(0)?,
                            numbered: row.get(1)?,
                            lowest: row
                                .get::<_, Option<i64>>(2)?
                                .and_then(|uid| u32::try_from(uid).ok()),
                        })
                    },
                )
                .unwrap_or(Held {
                    all: 0,
                    numbered: 0,
                    lowest: None,
                });

            let last: Option<u32> = conn
                .query_row(
                    "SELECT server_exists FROM mailbox WHERE id = ?1",
                    rusqlite::params![mailbox_id],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .ok()
                .flatten()
                .and_then(|count| u32::try_from(count).ok());

            Ok((held, last))
        })
        .await?;

    let check = expunge_check(held, server_exists, last_exists, arrived, force);

    tracing::debug!(
        path,
        local = held.all,
        server = server_exists,
        ?check,
        "expunge check"
    );

    let removed = match check {
        ExpungeCheck::Skip => 0,
        ExpungeCheck::Empty => {
            let removed = db
                .write(move |tx| persist::remove_all_numbered(tx, mailbox_id))
                .await?;

            if removed > 0 {
                tracing::info!(
                    path,
                    removed,
                    "the server's copy is empty; removed them here"
                );
            }
            removed
        }
        ExpungeCheck::All | ExpungeCheck::From(_) => {
            let present = match check {
                ExpungeCheck::From(lowest) => fetch::uids_from(session, lowest).await?,
                _ => fetch::all_uids(session).await?,
            };
            reconcile_against(db, mailbox_id, path, server_exists, present).await?
        }
    };

    // Remembered once the check has been made, so the next pass can tell whether anything
    // changed. Written only when it did: on most passes it has not.
    if last_exists != Some(server_exists) {
        db.write(move |tx| {
            tx.execute(
                "UPDATE mailbox SET server_exists = ?2 WHERE id = ?1",
                rusqlite::params![mailbox_id, i64::from(server_exists)],
            )?;
            Ok(())
        })
        .await?;
    }

    Ok(removed)
}

/// Removes the rows whose UIDs the server no longer lists in `present`.
async fn reconcile_against(
    db: &Db,
    mailbox_id: i64,
    path: &str,
    server_exists: u32,
    present: Vec<u32>,
) -> Result<usize, SyncError> {
    // A mailbox that reports messages but returns no UIDs is a server being strange, not a
    // mailbox that emptied. Deleting everything on that answer would destroy the local copy of
    // a mailbox over a transient fault, which is the one outcome worth being paranoid about.
    if present.is_empty() && server_exists > 0 {
        tracing::warn!(
            path,
            "the server reported messages but listed no UIDs; leaving them"
        );
        return Ok(0);
    }

    // The decision itself lives in `persist`, where it is tested without a network.
    let removed = db
        .write(move |tx| persist::remove_missing(tx, mailbox_id, &present))
        .await?;

    if removed > 0 {
        tracing::info!(
            path,
            removed,
            "messages expunged on the server were removed"
        );
    }

    Ok(removed)
}

/// What arrived, for whoever wants to tell the user about it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Arrived {
    account_id: i64,
    message_ids: Vec<i64>,
}

/// Which of these rows are new since the last sync, by UID.
///
/// The stand-in for a modseq on a server that has none. `uid_next` is the UID the server will
/// hand to the next message it receives, so anything at or above the value recorded last time is
/// something that arrived since — and anything below it was already there and must not be
/// treated as an arrival, whatever this pass happened to insert.
fn new_since(
    conn: &rusqlite::Connection,
    mailbox_id: i64,
    previous_uid_next: u32,
    inserted: &[i64],
) -> Result<Vec<i64>, crate::db::DbError> {
    if inserted.is_empty() {
        return Ok(Vec::new());
    }

    // In pieces past SQLite's parameter limit (M7): a long time offline can mean more arrivals
    // than one statement takes. See `db::write::CHUNK`.
    if inserted.len() > crate::db::write::CHUNK {
        let mut arrived = Vec::new();
        for chunk in inserted.chunks(crate::db::write::CHUNK) {
            arrived.extend(new_since(conn, mailbox_id, previous_uid_next, chunk)?);
        }
        return Ok(arrived);
    }

    let placeholders = (0..inserted.len())
        .map(|index| format!("?{}", index + 3))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "SELECT id FROM message
          WHERE mailbox_id = ?1 AND uid >= ?2 AND id IN ({placeholders})
          ORDER BY uid"
    );

    let mut bound: Vec<&dyn rusqlite::ToSql> = vec![&mailbox_id, &previous_uid_next];
    bound.extend(inserted.iter().map(|id| id as &dyn rusqlite::ToSql));

    let rows = conn
        .prepare(&sql)?
        .query_map(bound.as_slice(), |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;

    Ok(rows)
}

/// Whether Inbox arrivals need their bodies before the rules and the junk filter run: when an
/// enabled rule has a condition on the body or the attachments, or the classifier has enough
/// mail to score with and would read the body.
fn arrivals_need_bodies(conn: &rusqlite::Connection) -> Result<bool, crate::db::DbError> {
    let rules = crate::rules::engine::rules_list(conn)?;
    if rules
        .iter()
        .any(|rule| rule.enabled && rule.predicate.needs_body())
    {
        return Ok(true);
    }

    let (ham, junk) = crate::rules::junk::corpus_size(conn)?;
    Ok(ham >= crate::rules::junk::MIN_CORPUS && junk >= crate::rules::junk::MIN_CORPUS)
}

/// Everything that happens because mail **arrived**: rules, the junk filter, the toast.
///
/// Extracted because it used to live inside `incremental` and its comment said it ran "here and
/// nowhere else" — which was true, and was the bug. `incremental` is reachable only on a server
/// advertising CONDSTORE or QRESYNC, so on any other server the app quietly became one that does
/// not run rules, does not file junk and never announces new mail. Nothing failed; those
/// features simply did not happen, and no log line said so.
///
/// The guard that comment insists on is the caller's job now: **only ever pass genuinely new
/// arrivals**. Running these over an initial sync or a backfill "would apply every rule to fifty
/// thousand messages the user has already dealt with, and a rule that files mail would empty
/// their Inbox on first launch".
///
/// Rules first, then the filter, so a rule saying "this is never junk" is not overruled a moment
/// later by a classifier that disagrees.
async fn on_arrival(
    app: &dyn Events,
    db: &Db,
    account_id: i64,
    mailbox_id: i64,
    path: &str,
    arrived: &[i64],
) {
    if arrived.is_empty() {
        return;
    }

    // **Inbox only.** New in Trash, Sent, Drafts, Spam, Gmail's All Mail or a folder a server
    // filter files into is not new mail, and this ran for all of them (H8, 2026-11-14 audit): a
    // message deleted on a phone arrived in Trash and went through the rules — so a rule moving
    // that sender's mail pulled it straight back out — the junk filter scored Sent, and Gmail's
    // spam raised toasts. Mail runs its rules on, and announces, what arrives in the Inbox.
    let role = db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT role FROM mailbox WHERE id = ?1",
                    rusqlite::params![mailbox_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten())
        })
        .await
        .ok()
        .flatten();

    if role.as_deref() != Some("inbox") {
        tracing::debug!(
            path,
            arrived = arrived.len(),
            "arrivals outside the Inbox: no rules, junk scoring or notification"
        );
        return;
    }

    // The bodies, when a rule or the junk filter will read them. Bodies are otherwise fetched
    // when a message is opened, so on arrival every one was empty: a rule on the body or the
    // attachments never matched new mail, and the classifier scored subjects alone (H9,
    // 2026-11-14 audit). Fetched here, before either runs, and only when one of them needs it.
    // If the fetch fails, the rules engine still refuses to judge a body it does not have.
    let needs_bodies = db.read(arrivals_need_bodies).await.unwrap_or(false);
    if needs_bodies {
        if let Err(error) = fetch_body(app, db, account_id, arrived.to_vec()).await {
            tracing::warn!(%error, path, "bodies for arrival rules could not be fetched");
        }
    }

    // Blocked senders first: the user has said so in as many words, which outranks a rule. A
    // message marked junk here is skipped by the classifier and raises no toast.
    let ids = arrived.to_vec();
    match db
        .write(move |tx| crate::rules::vip::mark_blocked_arrivals(tx, &ids))
        .await
    {
        Ok(0) => {}
        Ok(blocked) => tracing::debug!(path, blocked, "blocked senders marked junk on arrival"),
        Err(error) => tracing::warn!(%error, path, "blocked senders not checked on arrival"),
    }

    match crate::rules::engine::run_on_arrival(db, arrived.to_vec()).await {
        Ok(report) if report.matched > 0 => {
            tracing::debug!(path, matched = report.matched, "rules applied on arrival");
        }
        Ok(_) => {}
        // Logged and carried on. A broken rule must not stop mail arriving — the message is
        // already stored, and failing the sync here would mean retrying the fetch forever over
        // something the network had nothing to do with.
        Err(error) => tracing::warn!(%error, path, "rules failed on arrival"),
    }

    match crate::sync::upkeep::score_new_mail(db, mailbox_id, arrived.to_vec()).await {
        Ok(filed) if filed > 0 => {
            tracing::debug!(path, filed, "junk filed on arrival");
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, path, "junk scoring failed on arrival"),
    }

    // After the rules and the filter, never before. A message a rule filed away or the
    // classifier caught should not have raised a toast on its way past — and it would have, if
    // this ran first.
    app.emit(
        "mail:arrived",
        payload(&Arrived {
            account_id,
            message_ids: arrived.to_vec(),
        }),
    );
}

/// The CONDSTORE path: fetch what arrived, reconcile what changed, and nothing else.
///
/// Split out rather than nested in `sync_mailbox` because the two paths share almost nothing:
/// this one never reads an envelope it already holds, and never walks a page.
#[allow(clippy::too_many_arguments)]
async fn incremental(
    app: &dyn Events,
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

        // Still checked, and it costs no round trip: `EXISTS` arrived with the `SELECT` and the
        // local count is a database read. On a CONDSTORE server an expunge bumps
        // `HIGHESTMODSEQ`, so this branch should never see one — but a server without CONDSTORE
        // reports no modseq at all, and for that server a deletion changes nothing this
        // condition looks at. Skipping the check here would leave those accounts with exactly
        // the bug this whole function exists to fix.
        let expunged =
            reconcile_expunged(db, session, mailbox_id, path, selected.exists, false, false)
                .await?;

        if expunged > 0 {
            app.emit("messages:added", payload(&mailbox_id));
        }

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

            on_arrival(app, db, account_id, mailbox_id, path, &written.inserted_ids).await;
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

    // ---- what left ------------------------------------------------------------------------
    // Before the counts are written, so the badge reflects what is actually here. Without this
    // nothing ever removed a message that vanished from the server, and an Inbox used alongside
    // a phone filled up with mail the user had already dealt with elsewhere.
    let expunged =
        // Something changed — a modseq or UIDNEXT moved — so a mailbox held in part is asked
        // about the range it holds. See `expunge_check` (H2).
        reconcile_expunged(db, session, mailbox_id, path, selected.exists, true, false).await?;

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

    if inserted > 0 || changed > 0 || expunged > 0 {
        app.emit(
            "sync:progress",
            payload(&Progress {
                account_id,
                mailbox: path.to_string(),
                written: inserted,
                usable: true,
                done: false,
            }),
        );
        app.emit("messages:added", payload(&mailbox_id));
    }

    Ok(inserted)
}

/// Which of these messages still need a body, as `(id, uid, mailbox path)`, in the order asked.
///
/// Only the ones we do not already hold. The prefetch calls this with the next three rows every
/// time the selection moves, and re-fetching a cached body would turn an arrow-key press into
/// three round trips.
///
/// **Only this account's messages.** The query used to select by message id alone, and the
/// session it fed is the account's. So an id belonging to another account — which the reader's
/// prefetch sent whenever All Inboxes, a smart mailbox or a search put two accounts' rows next
/// to each other — was looked up by *its* UID in *its* folder path on *this* account's server.
/// Folder paths like `INBOX` exist everywhere and UIDs are small integers, so that usually found
/// a message: a different one, belonging to someone else's mailbox, which was then stored as
/// this row's body. The reader showed it, Reply quoted it, and Forward as Attachment sent it.
/// An id from another account is now skipped here and logged, whatever the caller does.
fn bodies_wanted(conn: &rusqlite::Connection, account_id: i64, ids: &[i64]) -> Vec<Wanted> {
    let mut out = Vec::new();

    for &id in ids {
        let row = conn
            .query_row(
                "SELECT m.account_id, m.uid, m.body_state, b.remote_path, m.size, b.uid_validity
                   FROM message m
                   JOIN mailbox b ON b.id = m.mailbox_id
                  WHERE m.id = ?1",
                rusqlite::params![id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                    ))
                },
            )
            .ok();

        let Some((owner, uid, state, path, size, validity)) = row else {
            continue;
        };

        if owner != account_id {
            tracing::warn!(
                message_id = id,
                asked_for = account_id,
                belongs_to = owner,
                "body fetch: message belongs to another account; not fetched"
            );
            continue;
        }

        if state != "full" {
            out.push(Wanted {
                id,
                uid: uid as u32,
                path,
                size,
                validity: validity.and_then(|v| u32::try_from(v).ok()),
            });
        }
    }

    out
}

/// A message whose body is still to be fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Wanted {
    id: i64,
    uid: u32,
    path: String,
    /// What the server said the message's size was, for refusing one too large to fetch.
    size: i64,
    /// The mailbox's `UIDVALIDITY` when `uid` was learned.
    ///
    /// A UID means nothing without it. After a server resets a mailbox, the same number names a
    /// different message, and the body fetched for it was stored under this one (M3, 2026-11-14
    /// audit). The sync drops and re-reads such a mailbox the next time it reaches it; until then
    /// its bodies are not fetched.
    validity: Option<u32>,
}

/// Fetches one message's body, caches the `.eml`, and stores what was parsed out of it.
///
/// docs/06 Phase 5 §3. Opens its own connection rather than borrowing the sync session: the
/// user is waiting for this one, and queueing it behind a backfill that has three hundred
/// batches left would make opening a message take minutes.
pub async fn fetch_body(
    app: &dyn Events,
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

    let wanted = {
        let ids = message_ids.clone();
        db.read(move |conn| Ok(bodies_wanted(conn, account_id, &ids)))
            .await?
    };

    // Logged before the connect, for the reason the module header gives: a call that hangs or
    // returns nothing must still leave a trace, or it looks as though the UI never asked.
    tracing::debug!(
        account_id,
        asked = message_ids.len(),
        needed = wanted.len(),
        "body fetch: starting"
    );

    // Refused by size before a byte is fetched, and before connecting at all. A message over the
    // cap used to be downloaded in full on every open and refused afterwards, and was never shown
    // (M8, 2026-11-14 audit). A sentence saying so is stored in its place, which also stops it
    // being asked for again.
    let (oversize, wanted): (Vec<Wanted>, Vec<Wanted>) =
        wanted.into_iter().partition(|w| bodies::too_large(w.size));
    let refused = oversize.len();
    for w in oversize {
        let body = bodies::too_large_body(w.size);
        db.write(move |tx| bodies::persist(tx, w.id, &body, None))
            .await?;
    }

    if wanted.is_empty() {
        return Ok(refused);
    }

    let credential = credential_for(db, &account).await?;
    let (mut session, caps) = session::connect(&imap, &account.email, &credential).await?;

    // The store's own folder, which for the app is `default_path`'s. A store opened elsewhere
    // used to cache into the app's folder anyway, under ids that belong to the app's messages.
    let root = db.folder().to_path_buf();

    let total = wanted.len();
    let mut stored = 0usize;
    let mut current_mailbox = String::new();

    for Wanted {
        id: message_id,
        uid,
        path,
        validity,
        ..
    } in wanted
    {
        // SELECT only when the mailbox changes. A thread whose messages are all in the Inbox
        // is the common case, and re-selecting per message would triple the round trips.
        if path != current_mailbox {
            // Logged rather than skipped in silence. A failed SELECT skips every message in
            // that mailbox, and without a line here the symptom is a message that renders
            // blank for ever with nothing anywhere to say why — which is exactly what it did.
            if let Err(error) = fetch::select(&mut session, &path, validity, caps).await {
                tracing::warn!(message_id, path, %error, "body fetch: cannot select the mailbox");
                continue;
            }
            current_mailbox = path.clone();
        }

        let raw = match bodies::fetch(&mut session, uid).await {
            Ok(raw) if !raw.is_empty() => raw,
            Ok(_) => {
                // The server had nothing for this UID. Silent before, and it is one of the two
                // ways a message can stay blank with no explanation.
                tracing::warn!(
                    message_id,
                    uid,
                    path,
                    "body fetch: the server returned nothing"
                );
                continue;
            }
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

    // `asked` against `stored` is the pair that matters: a batch that wanted four bodies and
    // stored none is a bug, and reporting only the second number hid that.
    tracing::debug!(
        account_id,
        stored,
        skipped = total - stored,
        "body fetch: finished"
    );

    if stored > 0 {
        app.emit("messages:updated", payload(&message_ids));
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
    fn only_a_refusal_asks_the_user_to_sign_in_again() {
        // The whole point of the split. A provider that says `invalid_grant` means the stored
        // credential is dead; a provider that cannot be reached means nothing about it at all,
        // and treating the second as the first stopped the account and blamed the user.
        let refused = oauth_failure(
            Provider::Google,
            "me@gmail.com",
            &accounts::oauth::OAuthError::Refused {
                provider: "google".into(),
                error: "invalid_grant".into(),
                description: Some("token revoked".into()),
            },
        );

        assert!(
            matches!(refused, SyncError::Rejected { .. }),
            "a refused grant must ask for a new sign-in"
        );
        assert!(describe(&refused).contains("Signing in again"));

        // Everything that is not the provider saying no keeps the account alive. `TimedOut`
        // stands in for the whole class here because a `reqwest::Error` cannot be constructed
        // in a test, and the arm they share is the `_` fallthrough.
        let unavailable = oauth_failure(
            Provider::Google,
            "me@gmail.com",
            &accounts::oauth::OAuthError::TimedOut,
        );

        assert!(
            matches!(unavailable, SyncError::AuthUnavailable { .. }),
            "a provider that did not answer must not be reported as a bad credential"
        );

        let described = describe(&unavailable);
        assert!(!described.contains("Signing in again"), "{described}");
        assert!(described.contains("keep trying"), "{described}");
    }

    /// H16 in the 2026-11-14 audit: a password the server had refused was tried again every five
    /// minutes, by the safety net, by IDLE and by every manual sync — which is how a provider
    /// locks an account. A refused account is now left alone until its sign-in changes.
    #[tokio::test]
    async fn a_refused_sign_in_is_not_tried_again_until_it_changes() {
        let dir = tempfile::tempdir().expect("dir");
        let db = Db::open(&dir.path().join("halcyon.db")).expect("open");
        let engine = SyncEngine::new();

        engine.mark_refused(7);
        let error = engine
            .sync_account(&Recorder::default(), &db, 7)
            .await
            .expect_err("refused");
        assert!(
            matches!(error, SyncError::Rejected { .. }),
            "it tried to connect: {error:?}"
        );

        // A new password or a new sign-in clears it, and the next pass really runs — here it
        // finds no such account, which is the proof that it got past the refusal.
        engine.clear_refusal(7);
        let error = engine
            .sync_account(&Recorder::default(), &db, 7)
            .await
            .expect_err("no such account");
        assert!(matches!(error, SyncError::ShuttingDown), "{error:?}");
    }

    /// M27 in the 2026-11-14 audit: a token endpoint having a bad minute — a 500, a 503, a 429
    /// rate limit — was read as a dead sign-in. The account stopped with "Signing in again will
    /// fix it", and a message being sent at that moment was failed for good.
    #[test]
    fn a_token_endpoint_having_a_bad_minute_is_not_a_dead_sign_in() {
        for code in [
            "http_500",
            "http_503",
            "http_429",
            "temporarily_unavailable",
            "server_error",
        ] {
            let error = oauth_failure(
                Provider::Google,
                "me@gmail.com",
                &accounts::oauth::OAuthError::Refused {
                    provider: "google".into(),
                    error: code.into(),
                    description: None,
                },
            );

            assert!(
                matches!(error, SyncError::AuthUnavailable { .. }),
                "{code} stopped the account: {error:?}"
            );
            assert!(error.is_retryable(), "{code}");
        }
    }

    #[test]
    fn a_rejected_sign_in_application_is_never_reported_as_a_dead_credential() {
        // Both routes to the same wrong sentence, closed together.
        //
        // Google answers `invalid_client` when the client secret is missing or wrong, and that
        // used to land in `Rejected` — so the banner read "The saved sign-in for this account
        // was refused. Signing in again will fix it." Signing in again reran the identical
        // request against the identical broken registration. The other route was a cleared
        // client id, which arrived as a `Rejected` carrying the detail "no oauth client
        // configured" and stopped every OAuth account at once with the same useless advice.
        for code in ["invalid_client", "unauthorized_client"] {
            let mapped = oauth_failure(
                Provider::Google,
                "me@gmail.com",
                &accounts::oauth::OAuthError::Refused {
                    provider: "google".into(),
                    error: code.into(),
                    description: Some("Unauthorized".into()),
                },
            );

            assert!(
                matches!(
                    mapped,
                    SyncError::OauthClientUnusable {
                        configured: true,
                        ..
                    }
                ),
                "{code} must blame the sign-in application, not the account"
            );

            let described = describe(&mapped);
            assert!(!described.contains("Signing in again"), "{described}");
            assert!(described.contains("Sign-in applications"), "{described}");
        }

        // Nothing configured at all: a different sentence, the same destination.
        let absent = SyncError::OauthClientUnusable {
            provider: "google".into(),
            configured: false,
        };

        let described = describe(&absent);
        assert!(!described.contains("Signing in again"), "{described}");
        assert!(described.contains("Sign-in applications"), "{described}");

        // Still non-retryable, like every other configuration fault: waiting fixes none of it,
        // and retrying would back an account off for thirty seconds on every pass.
        assert!(!absent.is_retryable());
        assert!(!SyncError::OauthClientUnusable {
            provider: "google".into(),
            configured: true,
        }
        .is_retryable());

        // And the one refusal that genuinely does mean sign in again still says so.
        let revoked = oauth_failure(
            Provider::Google,
            "me@gmail.com",
            &accounts::oauth::OAuthError::Refused {
                provider: "google".into(),
                error: "invalid_grant".into(),
                description: None,
            },
        );
        assert!(matches!(revoked, SyncError::Rejected { .. }));
        assert!(describe(&revoked).contains("Signing in again"));
    }

    #[test]
    fn no_banner_sentence_carries_stray_whitespace_from_a_broken_continuation() {
        // `MissingClientSecret` held an escaped newline plus thirteen spaces of source
        // indentation, from a `\n` written where a line continuation was meant. It rendered
        // correctly only because the banner leaves `white-space` at its default and HTML
        // collapses the run — so the defect was invisible and would have survived any change
        // to that CSS.
        for error in [
            SyncError::MissingClientSecret {
                provider: "google".into(),
            },
            SyncError::OauthClientUnusable {
                provider: "google".into(),
                configured: true,
            },
            SyncError::OauthClientUnusable {
                provider: "google".into(),
                configured: false,
            },
            SyncError::NotConfigured {
                email: "me@gmail.com".into(),
            },
        ] {
            let described = describe(&error);
            assert!(!described.contains('\n'), "{described:?}");
            assert!(!described.contains("  "), "{described:?}");
        }
    }

    #[test]
    fn the_sign_in_banner_never_fires_for_a_provider_outage() {
        // `needs_reauth` is what actually raises the banner and stops the account, so assert on
        // it rather than only on the sentence.
        for error in [
            accounts::oauth::OAuthError::TimedOut,
            accounts::oauth::OAuthError::NoClient {
                provider: "google".into(),
            },
        ] {
            let mapped = oauth_failure(Provider::Google, "me@gmail.com", &error);
            assert!(
                !matches!(mapped, SyncError::Rejected { .. }),
                "{error} was mapped to a credential rejection"
            );
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

    #[test]
    fn a_refused_refresh_logs_how_old_the_sign_in_was() {
        // The number that would have answered "why does Google keep signing me out" from the
        // log alone: the Gmail account's token was refused one minute after its seventh day.
        let now = 1_790_000_000;

        assert_eq!(sign_in_age(Some(now - 7 * 86_400), now), "7.0 days ago");
        assert_eq!(sign_in_age(Some(now - 36 * 3_600), now), "1.5 days ago");
        // Signed in before the time was recorded. Said plainly rather than shown as fifty years.
        assert_eq!(sign_in_age(None, now), "unknown");
    }

    #[test]
    fn the_modseq_is_held_until_the_inbox_has_been_read_to_the_bottom() {
        // M6: a pass during an unfinished backfill used to record the server's newest MODSEQ,
        // so flag changes to messages the backfill had already stored were never asked about.
        assert_eq!(modseq_to_record(true, Some(100), Some(250)), None, "held");
        assert_eq!(
            modseq_to_record(true, None, Some(250)),
            Some(250),
            "the first page of a first sync is where the held value comes from"
        );
        assert_eq!(modseq_to_record(false, Some(100), Some(250)), Some(250));
        assert_eq!(modseq_to_record(false, None, None), None);
    }

    #[test]
    fn an_empty_newest_page_does_not_end_the_backfill() {
        // The bug, in one assertion. The newest page is a numeric UID range, and on a mailbox
        // archived from for years it can hold nothing live -- 214 messages across 106,287
        // UIDs means 500 consecutive slots are usually empty. That produced `lowest_uid == 0`,
        // the old arithmetic turned it into a cursor of 0 or 1, `backfill_window` found
        // nothing below that, the loop never ran and the mailbox was recorded as fully
        // backfilled having fetched not one message.
        let uids = vec![106_287u32, 90_000, 40_000, 12, 3];

        let cursor = backfill_start(None, 0, &uids);
        assert!(
            cursor > 106_287,
            "an empty newest page must leave the whole UID list still to walk, got {cursor}"
        );
        assert!(
            crate::sync::fetch::backfill_window(&uids, cursor, 500).is_some(),
            "the walk must have something to do"
        );
    }

    #[test]
    fn an_empty_newest_page_does_not_discard_stored_progress() {
        // A resumed sync. The old code took `stored.min(0.max(1))`, which is 1 for every
        // stored value -- so an interrupted backfill that resumed on a day the newest page
        // happened to be empty jumped straight to the bottom and declared itself finished.
        let uids = vec![900u32, 500, 100, 5];
        assert_eq!(backfill_start(Some(400), 0, &uids), 400);
    }

    #[test]
    fn the_walk_still_resumes_from_the_lower_of_the_two() {
        // The ordinary case, unchanged: whichever of stored progress and the newest page
        // reaches further down is where there is still work to do.
        let uids = vec![900u32, 500, 100, 5];
        assert_eq!(backfill_start(Some(400), 700, &uids), 400);
        assert_eq!(backfill_start(Some(800), 700, &uids), 700);
        assert_eq!(backfill_start(None, 700, &uids), 700);
    }

    #[test]
    fn a_mailbox_the_server_lists_as_empty_is_finished() {
        // No stored progress, no newest page and no UIDs at all. There is genuinely nothing
        // to fetch, so a cursor that ends the walk immediately is the right answer here --
        // this is the one case where recording the backfill complete is honest.
        assert_eq!(backfill_start(None, 0, &[]), 0);
        assert!(crate::sync::fetch::backfill_window(&[], 0, 500).is_none());
    }

    fn mailbox_with_uids(uids: &[u32]) -> rusqlite::Connection {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open");
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

        for (index, uid) in uids.iter().enumerate() {
            conn.execute(
                "INSERT INTO message (
                     id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
                     from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged, is_junk
                 ) VALUES (?1, 1, 1, ?2, 'S', 0, 0, 10, 'a@b.test', '', '', 0, 0, 0, 0)",
                rusqlite::params![index as i64 + 1, uid],
            )
            .expect("message");
        }

        conn
    }

    /// Records what was emitted, by name.
    #[derive(Default)]
    struct Recorder(std::sync::Mutex<Vec<String>>);

    impl Events for Recorder {
        fn emit(&self, event: &str, _payload: serde_json::Value) {
            self.0.lock().expect("lock").push(event.to_string());
        }
    }

    /// H8 in the 2026-11-14 audit: arrival handling ran for every mailbox.
    ///
    /// So a message deleted on a phone arrived in Trash and went through the rules — one that
    /// moves mail from a sender pulled it straight back out — the junk filter scored Sent, and
    /// Gmail's spam folder raised new-mail toasts. Mail runs rules on, and announces, mail that
    /// arrives in the Inbox.
    #[tokio::test]
    async fn only_an_inbox_arrival_is_treated_as_new_mail() {
        let dir = tempfile::tempdir().expect("dir");
        let db = Db::open(&dir.path().join("halcyon.db")).expect("open");

        db.write(|tx| {
            tx.execute(
                "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
                 VALUES (1, 'T', 'me@t.test', 'other', 'password', 'halcyon:me')",
                [],
            )?;
            for (id, path, role) in [
                (1, "INBOX", Some("inbox")),
                (2, "Trash", Some("trash")),
                (3, "Sent", Some("sent")),
                (4, "[Gmail]/Spam", Some("junk")),
                (5, "Projects", None),
            ] {
                tx.execute(
                    "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
                     VALUES (?1, 1, ?2, ?2, ?3)",
                    rusqlite::params![id, path, role],
                )?;
                tx.execute(
                    "INSERT INTO message (
                         id, account_id, mailbox_id, uid, subject, date_sent, date_received,
                         size, from_all, to_all, body_text, has_attachment, flag_seen,
                         flag_flagged, is_junk
                     ) VALUES (?1, 1, ?1, 10, 'S', 0, 0, 10, 'a@b.test', '', '', 0, 0, 0, 0)",
                    rusqlite::params![id],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");

        for (mailbox, path) in [
            (2, "Trash"),
            (3, "Sent"),
            (4, "[Gmail]/Spam"),
            (5, "Projects"),
        ] {
            let events = Recorder::default();
            on_arrival(&events, &db, 1, mailbox, path, &[mailbox]).await;

            assert!(
                !events
                    .0
                    .lock()
                    .expect("lock")
                    .iter()
                    .any(|e| e == "mail:arrived"),
                "an arrival in {path} was announced as new mail"
            );
        }

        let events = Recorder::default();
        on_arrival(&events, &db, 1, 1, "INBOX", &[1]).await;
        assert!(
            events
                .0
                .lock()
                .expect("lock")
                .iter()
                .any(|e| e == "mail:arrived"),
            "an arrival in the Inbox must still be announced"
        );
    }

    /// H2 in the 2026-11-14 audit, from the user's own log: Gmail's Sent Mail held 225 of 518
    /// messages, two of those 225 were deleted elsewhere, and they stayed here for hours as rows
    /// that opened blank. The only check was "more here than on the server", which a folder held
    /// in part can never be.
    #[test]
    fn a_folder_held_in_part_is_checked_over_the_range_it_holds() {
        let held = Held {
            all: 225,
            numbered: 225,
            lowest: Some(294),
        };

        // Something changed on the server since the last look: ask about UIDs 294 and up.
        assert_eq!(
            expunge_check(held, 516, Some(518), false, false),
            ExpungeCheck::From(294)
        );
        // Something arrived (UIDNEXT moved) though the count happens to be the same.
        assert_eq!(
            expunge_check(held, 518, Some(518), true, false),
            ExpungeCheck::From(294)
        );
        // Nothing changed at all: no round trip.
        assert_eq!(
            expunge_check(held, 518, Some(518), false, false),
            ExpungeCheck::Skip
        );
        // Never recorded, which is every folder the first time after this version arrives:
        // checked once, which clears the ghosts already there.
        assert_eq!(
            expunge_check(held, 518, None, false, false),
            ExpungeCheck::From(294)
        );
    }

    #[test]
    fn a_folder_held_in_full_keeps_the_old_cheap_test() {
        let held = Held {
            all: 40,
            numbered: 40,
            lowest: Some(1),
        };

        assert_eq!(
            expunge_check(held, 40, Some(41), false, false),
            ExpungeCheck::Skip,
            "as many here as there: nothing removed"
        );
        assert_eq!(
            expunge_check(held, 39, Some(40), false, false),
            ExpungeCheck::All,
            "more here than there"
        );
        assert_eq!(
            expunge_check(held, 0, Some(40), false, false),
            ExpungeCheck::Empty
        );
        assert_eq!(
            expunge_check(held, 40, Some(40), false, true),
            ExpungeCheck::All,
            "a rebuild always asks"
        );
    }

    #[test]
    fn arrivals_fetch_their_bodies_only_when_something_will_read_them() {
        use crate::rules::engine::{rule_save, Action};
        use crate::rules::predicate::{Condition, Field, Op, Predicate};

        let mut conn = mailbox_with_uids(&[]);
        let rule = |field: Field| {
            Predicate::Is(Condition {
                field,
                op: Op::Contains,
                value: "x".into(),
            })
        };

        // Nothing reads a body: no download on arrival, as before.
        assert!(!arrivals_need_bodies(&conn).expect("check"));

        let tx = conn.transaction().expect("tx");
        rule_save(
            &tx,
            None,
            "sender",
            true,
            &rule(Field::From),
            &[Action::Flag],
        )
        .expect("save");
        tx.commit().expect("commit");
        assert!(
            !arrivals_need_bodies(&conn).expect("check"),
            "a header rule needs no body"
        );

        let tx = conn.transaction().expect("tx");
        let id = rule_save(
            &tx,
            None,
            "body",
            false,
            &rule(Field::Body),
            &[Action::Flag],
        )
        .expect("save");
        tx.commit().expect("commit");
        assert!(
            !arrivals_need_bodies(&conn).expect("check"),
            "a disabled body rule needs no body"
        );

        let tx = conn.transaction().expect("tx");
        rule_save(
            &tx,
            Some(id),
            "body",
            true,
            &rule(Field::Body),
            &[Action::Flag],
        )
        .expect("save");
        tx.commit().expect("commit");
        assert!(arrivals_need_bodies(&conn).expect("check"));

        // And for the classifier, once it has enough mail to score with.
        let fresh = mailbox_with_uids(&[]);
        let corpus = crate::rules::junk::MIN_CORPUS;
        fresh
            .execute(
                "INSERT INTO junk_corpus (is_junk, messages) VALUES (0, ?1), (1, ?1)",
                rusqlite::params![corpus],
            )
            .expect("corpus");
        assert!(arrivals_need_bodies(&fresh).expect("check"));
    }

    /// H10 in the 2026-11-14 audit: Block Sender did nothing to future mail.
    ///
    /// Blocking marked the mail already here as junk, and nothing ever looked at the blocked list
    /// again — `vip::is_blocked` had no callers. The next message from that sender arrived in the
    /// Inbox like any other and raised a toast.
    #[tokio::test]
    async fn mail_from_a_blocked_sender_arrives_as_junk() {
        let dir = tempfile::tempdir().expect("dir");
        let db = Db::open(&dir.path().join("halcyon.db")).expect("open");

        db.write(|tx| {
            tx.execute(
                "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
                 VALUES (1, 'T', 'me@t.test', 'other', 'password', 'halcyon:me')",
                [],
            )?;
            tx.execute(
                "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
                 VALUES (1, 1, 'INBOX', 'Inbox', 'inbox')",
                [],
            )?;
            for (id, name, address) in [
                (1, "Pest Co", "Offers@Pest.example"),
                (2, "Ada", "ada@example.test"),
                // Contains the blocked address without being it.
                (3, "Not a pest", "notoffers@pest.example"),
            ] {
                tx.execute(
                    "INSERT INTO message (
                         id, account_id, mailbox_id, uid, subject, date_sent, date_received,
                         size, from_name, from_addr, from_all, to_all, body_text,
                         has_attachment, flag_seen, flag_flagged, is_junk
                     ) VALUES (?1, 1, 1, ?1, 'S', 0, 0, 10, ?2, ?3, ?2 || ' ' || ?3, '', '',
                               0, 0, 0, 0)",
                    rusqlite::params![id, name, address],
                )?;
            }
            crate::rules::vip::block_sender(tx, "offers@pest.example", 0)?;
            // Blocking is retroactive; start from mail that arrives after it.
            tx.execute("UPDATE message SET is_junk = 0, junk_by_user = 0", [])?;
            Ok(())
        })
        .await
        .expect("seed");

        on_arrival(&Recorder::default(), &db, 1, 1, "INBOX", &[1, 2, 3]).await;

        let junk: Vec<(i64, i64)> = db
            .read(|conn| {
                let mut statement = conn.prepare("SELECT id, is_junk FROM message ORDER BY id")?;
                let rows = statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .expect("read");

        assert_eq!(junk, vec![(1, 1), (2, 0), (3, 0)]);
    }

    #[test]
    fn a_body_fetch_never_takes_another_accounts_message() {
        // All Inboxes puts two accounts' rows side by side, and the prefetch used to send the
        // next three ids under the selected row's account. Each was then fetched by its own UID
        // and folder path from the wrong server, and whatever that server had at that UID was
        // stored as its body.
        let conn = mailbox_with_uids(&[10, 11]);

        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (2, 'U', 'other@t.test', 'other', 'password', 'halcyon:other')",
            [],
        )
        .expect("account");
        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (2, 2, 'INBOX', 'Inbox', 'inbox')",
            [],
        )
        .expect("mailbox");
        conn.execute(
            "INSERT INTO message (
                 id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
                 from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged, is_junk
             ) VALUES (3, 2, 2, 10, 'S', 0, 0, 10, 'a@b.test', '', '', 0, 0, 0, 0)",
            [],
        )
        .expect("message");

        // Same UID, same folder name, different account: exactly the case that matched.
        let wanted = bodies_wanted(&conn, 1, &[1, 3, 2]);

        assert_eq!(
            wanted.iter().map(|w| w.id).collect::<Vec<_>>(),
            vec![1, 2],
            "message 3 belongs to account 2 and must not be fetched through account 1"
        );
        assert_eq!(
            bodies_wanted(&conn, 2, &[1, 3, 2])
                .iter()
                .map(|w| w.id)
                .collect::<Vec<_>>(),
            vec![3]
        );
    }

    #[test]
    fn only_uids_at_or_above_the_last_uid_next_count_as_arrivals() {
        // The stand-in for a modseq on a server that has none. Everything below the `uid_next`
        // the previous sync recorded was already here, whatever this pass happened to insert —
        // and treating it as an arrival is what would run every rule over a backfill.
        let conn = mailbox_with_uids(&[10, 11, 12, 13]);
        let all = vec![1, 2, 3, 4];

        // Rows 3 and 4 hold UIDs 12 and 13.
        assert_eq!(new_since(&conn, 1, 12, &all).expect("query"), vec![3, 4]);
        assert_eq!(
            new_since(&conn, 1, 14, &all).expect("query"),
            Vec::<i64>::new()
        );
        assert_eq!(
            new_since(&conn, 1, 10, &all).expect("query"),
            vec![1, 2, 3, 4]
        );
    }

    #[test]
    fn a_backfilled_message_is_not_an_arrival() {
        // The case the guard exists for. A backfill inserts *old* messages — low UIDs — and
        // running rules over them "would apply every rule to fifty thousand messages the user
        // has already dealt with, and a rule that files mail would empty their Inbox".
        let conn = mailbox_with_uids(&[3, 4, 900]);

        // A backfill pass that inserted the two old ones, with the mailbox already synced up to
        // UID 900.
        let backfilled = vec![1, 2];
        assert!(new_since(&conn, 1, 900, &backfilled)
            .expect("query")
            .is_empty());
    }

    #[test]
    fn nothing_inserted_means_nothing_arrived() {
        let conn = mailbox_with_uids(&[1, 2]);
        assert!(new_since(&conn, 1, 1, &[]).expect("query").is_empty());
    }

    #[test]
    fn another_mailboxs_messages_are_not_counted() {
        // The id list comes from one batch and the batch belongs to one mailbox, but the filter
        // says so explicitly rather than trusting the caller.
        let conn = mailbox_with_uids(&[10]);
        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (2, 1, 'Archive', 'Archive', 'archive')",
            [],
        )
        .expect("mailbox");
        conn.execute(
            "INSERT INTO message (
                 id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
                 from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged, is_junk
             ) VALUES (50, 1, 2, 99, 'S', 0, 0, 10, 'a@b.test', '', '', 0, 0, 0, 0)",
            [],
        )
        .expect("message");

        assert_eq!(new_since(&conn, 1, 1, &[1, 50]).expect("query"), vec![1]);
    }

    #[test]
    fn a_push_looks_only_at_accounts_with_something_queued() {
        let mut conn = mailbox_with_uids(&[1]);
        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (2, 'Quiet', 'quiet@t.test', 'other', 'password', 'halcyon:quiet')",
            [],
        )
        .expect("account");

        assert!(accounts_with_pending_ops(&conn).expect("read").is_empty());

        let tx = conn.transaction().expect("tx");
        for _ in 0..2 {
            ops::enqueue(
                &tx,
                1,
                &ops::Op::Flag {
                    mailbox: "INBOX".into(),
                    uids: vec![1],
                    seen: Some(true),
                    flagged: None,
                },
            )
            .expect("queue");
        }
        tx.commit().expect("commit");

        // Once per account, however much it has queued, and never the account with nothing.
        assert_eq!(accounts_with_pending_ops(&conn).expect("read"), vec![1]);
    }
}
