//! The task that actually sends what the outbox holds. docs/06 Phase 7.
//!
//! `outbox` owns the state machine and `smtp` owns the socket; this is the loop that joins
//! them, and the order of its steps is the part that matters.
//!
//! **Submit, then file a copy, then mark sent.** A message that reached the recipient but is
//! missing from Sent is an inconvenience the user can live with; the reverse — a copy in Sent
//! for a message that never left — is a lie the user acts on. So the copy is filed only after
//! the server has accepted, and a failure to file it does not fail the send.
//!
//! The copy in Sent is also load-bearing rather than a courtesy: it is what
//! `outbox::resolve_interrupted` searches when a crash leaves a send in doubt.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

use crate::accounts::credentials::{self, Kind};
use crate::accounts::provider::AuthKind;
use crate::db::Db;

use super::events::{payload, Events};
use super::outbox::{self, Entry, State};
use super::session::{self, SyncError};
use super::smtp;

/// How often to look for work when nothing has asked.
///
/// The outbox is normally poked directly — a send schedules a tick for when its hold expires —
/// so this is a backstop for a message queued while the app was closed, or one whose retry
/// falls due during a quiet period.
const IDLE_TICK: Duration = Duration::from_secs(30);

/// How long to wait before retrying a message the server could not take.
///
/// Deliberately unhurried. A temporary rejection is usually greylisting or a rate limit, and
/// both are made worse by trying again immediately.
const RETRY_AFTER: i64 = 60;

/// What the UI is told about a message on its way.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: i64,
    pub account_id: i64,
    pub state: String,
    /// Present when the state is `failed`, so the banner can say why.
    pub error: Option<String>,
}

/// A handle to the running sender.
#[derive(Clone)]
pub struct Sender {
    wake: Arc<Notify>,
}

impl Default for Sender {
    fn default() -> Self {
        Self::new()
    }
}

impl Sender {
    pub fn new() -> Self {
        Self {
            wake: Arc::new(Notify::new()),
        }
    }

    /// Asks the loop to look now rather than at the next tick.
    ///
    /// Called when a message is queued and when its hold expires, so Undo Send's timer is the
    /// thing that decides when a message goes — not a polling interval that could add thirty
    /// seconds to every send.
    pub fn poke(&self) {
        self.wake.notify_one();
    }

    /// The loop itself. Never returns.
    ///
    /// Deliberately **not** a spawn. `tokio::spawn` panics with "there is no reactor running"
    /// unless it is called from inside a runtime, and Tauri's `setup` — where this is started —
    /// is not. It panicked exactly that way the first time the app was launched after Phase 7,
    /// taking the webview down with it; the whole of Phase 7 had been written and verified
    /// without the app ever being run.
    ///
    /// Returning the future instead lets the caller spawn it on a runtime it actually has, and
    /// keeps this module free of Tauri, which is what `Events` exists for.
    pub fn run(&self, events: Arc<dyn Events>, db: Db, root: PathBuf) -> impl std::future::Future<Output = ()> + Send + 'static {
        let wake = Arc::clone(&self.wake);

        async move {
            // Before anything is sent: resolve whatever a previous run left in doubt. Doing
            // this first means a message that did go out is marked sent before the loop could
            // consider sending it again.
            if let Err(error) = recover(events.as_ref(), &db).await {
                tracing::warn!(%error, "outbox recovery failed; leaving rows in place");
            }

            loop {
                if let Err(error) = run_once(events.as_ref(), &db, &root).await {
                    tracing::warn!(%error, "outbox pass failed");
                }

                tokio::select! {
                    _ = tokio::time::sleep(IDLE_TICK) => {}
                    _ = wake.notified() => {}
                }
            }
        }
    }
}

fn announce(events: &dyn Events, entry: &Entry, state: State, error: Option<&str>) {
    events.emit(
        "outbox:progress",
        payload(&Progress {
            id: entry.id,
            account_id: entry.account_id,
            state: state.as_str().to_string(),
            error: error.map(str::to_string),
        }),
    );
}

/// Resolves sends that a previous process left in flight. See `outbox`'s module header.
pub async fn recover(events: &dyn Events, db: &Db) -> Result<(), SyncError> {
    let stranded = outbox::interrupted(db).await?;
    if stranded.is_empty() {
        return Ok(());
    }

    tracing::info!(count = stranded.len(), "resolving interrupted sends");

    for entry in stranded {
        // Without an id there is nothing to look for, and guessing is exactly what this
        // mechanism exists to avoid. Back to the queue: a duplicate is recoverable by the
        // recipient, a lost message is not recoverable by anyone.
        let Some(message_id) = entry.message_id.clone() else {
            let id = entry.id;
            db.write(move |tx| outbox::resolve_interrupted(tx, id, false).map(|_| ()))
                .await?;
            continue;
        };

        let found = match was_filed(db, entry.account_id, &message_id).await {
            Ok(found) => found,
            Err(error) => {
                // The server could not be asked. Leave the row as it is and try again next
                // start rather than deciding without evidence.
                tracing::warn!(id = entry.id, %error, "could not check Sent; leaving in doubt");
                continue;
            }
        };

        let id = entry.id;
        let state = db
            .write(move |tx| outbox::resolve_interrupted(tx, id, found))
            .await?;

        tracing::info!(
            id,
            found_in_sent = found,
            state = state.as_str(),
            "resolved"
        );
        announce(events, &entry, state, None);
    }

    Ok(())
}

/// Whether a message with this id is already in the account's Sent mailbox.
async fn was_filed(db: &Db, account_id: i64, message_id: &str) -> Result<bool, SyncError> {
    let (mut imap_session, _caps) = connect(db, account_id).await?;
    let mailbox = sent_mailbox(db, account_id).await?;

    let result = async {
        imap_session.select(&mailbox).await?;

        // Searching by header rather than by UID: the UID is the server's and we never learned
        // it, whereas the Message-ID is ours and travels with the message.
        let found = imap_session
            .uid_search(format!("HEADER Message-ID \"{message_id}\""))
            .await?;

        Ok::<bool, SyncError>(!found.is_empty())
    }
    .await;

    let _ = imap_session.logout().await;
    result
}

/// The remote path of the account's Sent mailbox.
async fn sent_mailbox(db: &Db, account_id: i64) -> Result<String, SyncError> {
    let path: Option<String> = db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT remote_path FROM mailbox WHERE account_id = ?1 AND role = 'sent'",
                    rusqlite::params![account_id],
                    |row| row.get(0),
                )
                .ok())
        })
        .await?;

    // "Sent" is the near-universal fallback, and filing to a mailbox the server then creates is
    // better than not filing at all — the copy is what crash recovery reads.
    Ok(path.unwrap_or_else(|| "Sent".to_string()))
}

async fn connect(
    db: &Db,
    account_id: i64,
) -> Result<(session::ImapSession, session::Caps), SyncError> {
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

    let credential = super::engine::credential_for(db, &account).await?;
    session::connect(&imap, &account.email, &credential).await
}

/// One pass: send everything due.
pub async fn run_once(events: &dyn Events, db: &Db, root: &Path) -> Result<(), SyncError> {
    let due = outbox::claim_due(db, None).await?;
    if due.is_empty() {
        return Ok(());
    }

    tracing::debug!(count = due.len(), "outbox: sending");

    for entry in due {
        announce(events, &entry, State::Sending, None);

        match send_one(db, root, &entry).await {
            Ok(()) => {
                let id = entry.id;
                db.write(move |tx| outbox::mark_sent(tx, id)).await?;
                tracing::info!(id, "outbox: sent");
                announce(events, &entry, State::Sent, None);
            }

            Err(error) => {
                let detail = describe(&error);
                let retryable = error.is_retryable();
                let id = entry.id;

                // A permanent refusal is not worth five attempts. Exhausting the count now puts
                // it in front of the user immediately rather than an hour later.
                let after = if retryable { RETRY_AFTER } else { 0 };
                let state = {
                    let detail = detail.clone();
                    db.write(move |tx| {
                        if !retryable {
                            tx.execute(
                                "UPDATE outbox SET attempts = ?2 WHERE id = ?1",
                                rusqlite::params![id, outbox::MAX_ATTEMPTS - 1],
                            )?;
                        }
                        outbox::mark_attempt_failed(tx, id, &detail, after)
                    })
                    .await?
                };

                tracing::warn!(id, %error, state = state.as_str(), "outbox: send failed");
                announce(events, &entry, state, Some(&detail));
            }
        }
    }

    Ok(())
}

/// Turns a send failure into a sentence the outbox banner can show.
///
/// The server's own text is kept where there is one — "550 mailbox full" tells the user
/// something they can act on, and paraphrasing it into "could not send" does not.
fn describe(error: &smtp::SendError) -> String {
    match error {
        smtp::SendError::Refused { detail, .. } => detail.clone(),
        smtp::SendError::Temporary { detail, .. } => detail.clone(),
        smtp::SendError::Insecure { host, port } => {
            format!("{host}:{port} is not an encrypted submission port.")
        }
        smtp::SendError::Envelope { detail } => format!("A recipient is not usable: {detail}"),
        other => other.to_string(),
    }
}

async fn send_one(db: &Db, root: &Path, entry: &Entry) -> Result<(), smtp::SendError> {
    let account = db
        .read({
            let id = entry.account_id;
            move |conn| crate::accounts::store::get(conn, id)
        })
        .await
        .ok()
        .flatten()
        .ok_or_else(|| smtp::SendError::Envelope {
            detail: "the account no longer exists".into(),
        })?;

    let server = account
        .smtp
        .clone()
        .ok_or_else(|| smtp::SendError::Envelope {
            detail: "this account has no outgoing server configured".into(),
        })?;

    let path = if entry.eml_path.is_empty() {
        outbox::eml_path(root, entry.account_id, entry.id)
    } else {
        PathBuf::from(&entry.eml_path)
    };

    let raw = std::fs::read(&path)?;

    let recipients: Vec<String> = entry
        .recipients
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .map(str::to_string)
        .collect();

    let envelope = smtp::envelope_for(&account.email, &recipients)?;

    // OAuth accounts cannot use SMTP AUTH PLAIN with a password. Phase 7 covers password
    // submission; XOAUTH2 for SMTP is the same shape as the IMAP one and arrives with it.
    if account.auth_kind != AuthKind::Password {
        return Err(smtp::SendError::Envelope {
            detail: "sending from an OAuth account is not available yet".into(),
        });
    }

    let secret = credentials::load(&credentials::reference_for(&account.email), Kind::Password)
        .map_err(|_| smtp::SendError::Envelope {
            detail: "no password is stored for this account".into(),
        })?;

    smtp::send(&server, &account.email, &secret, &envelope, &raw).await?;

    // Filed only now, after the server has taken it. A copy in Sent for a message that never
    // left is a lie the user acts on; a message delivered but missing from Sent is merely
    // untidy. A failure here is logged and does not fail the send.
    if let Err(error) = file_in_sent(db, entry.account_id, &raw).await {
        tracing::warn!(id = entry.id, %error, "sent, but could not file a copy in Sent");
    }

    Ok(())
}

/// Appends a copy of a sent message to the account's Sent mailbox.
async fn file_in_sent(db: &Db, account_id: i64, raw: &[u8]) -> Result<(), SyncError> {
    let (mut imap_session, _caps) = connect(db, account_id).await?;
    let mailbox = sent_mailbox(db, account_id).await?;

    // `\Seen`, because the user wrote it. A Sent folder that shows unread mail the user typed
    // themselves is a badge that can never be cleared by reading anything.
    let result = imap_session
        .append(&mailbox, Some("(\\Seen)"), None, raw)
        .await
        .map_err(SyncError::from);

    let _ = imap_session.logout().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_message_reaches_the_user_rather_than_a_paraphrase() {
        // "550 mailbox full" tells the user something they can act on. "Could not send" does
        // not, and it is what they would be left with.
        let refused = smtp::SendError::Refused {
            host: "smtp.example.test".into(),
            detail: "550 5.2.2 mailbox full".into(),
        };

        assert_eq!(describe(&refused), "550 5.2.2 mailbox full");
    }

    #[test]
    fn a_misconfigured_port_is_explained_rather_than_echoed() {
        // Here the server never spoke, so there is nothing to quote and the app has to say
        // what is wrong itself.
        let insecure = smtp::SendError::Insecure {
            host: "smtp.example.test".into(),
            port: 25,
        };

        let described = describe(&insecure);
        assert!(described.contains("smtp.example.test:25"), "{described}");
        assert!(described.contains("encrypted"), "{described}");
    }
}
