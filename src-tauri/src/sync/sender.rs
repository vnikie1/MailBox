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
    pub fn run(
        &self,
        events: Arc<dyn Events>,
        db: Db,
        root: PathBuf,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
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
    // First: rows from an `enqueue` that never finished writing its bytes. They can never be
    // sent, and `claim_due` will not pick them up, so without this they sit in the outbox
    // looking like a message still on its way.
    match outbox::sweep_unwritten(db).await {
        Ok(0) => {}
        Ok(count) => tracing::warn!(count, "outbox rows had no message bytes; marked failed"),
        Err(error) => tracing::warn!(%error, "could not sweep unwritten outbox rows"),
    }

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
        // Gmail's content block, explained before it is quoted. Its own wording — "blocked
        // because its content presents a potential security issue" — does not say that an
        // attachment is the cause, that it can be one *inside a .zip*, or that retrying is
        // pointless, and the banner offered Try Again as the only thing to do. Reported on
        // 2026-09-17 with a zip of three DLLs. `compose_send` now catches the known cases before
        // queueing (`mail::attachment_policy`); this covers what only Gmail can see, such as
        // macros in a document or another archive format.
        smtp::SendError::Refused { detail, .. } if is_gmail_content_block(detail) => format!(
            "Gmail blocked this message because of an attachment — usually a program or a \
             file like a .dll, which Gmail refuses even inside a .zip. Sending it again will \
             not help: delete it, and share the file as a Google Drive link instead. \
             Gmail said: {detail}"
        ),
        smtp::SendError::Refused { detail, .. } => detail.clone(),
        smtp::SendError::Temporary { detail, .. } => detail.clone(),
        smtp::SendError::Insecure { host, port } => {
            format!("{host}:{port} is not an encrypted submission port.")
        }
        smtp::SendError::Envelope { detail } => format!("A recipient is not usable: {detail}"),

        // The same two sentences the sidebar uses for the same two situations, because they
        // are the same two situations and a user meeting both should not have to work out
        // whether they mean the same thing.
        smtp::SendError::Credential {
            retryable: false, ..
        } => "The saved sign-in for this account was refused. Signing in again will fix it."
            .to_string(),
        smtp::SendError::Credential { .. } => {
            "The account's sign-in could not be checked. Halcyon will try again.".to_string()
        }

        other => other.to_string(),
    }
}

/// Gmail's refusal of a message's content, by the two things it always carries: the `5.7.0`
/// status and a link to its "BlockedMessage" help page. Matching the link rather than the
/// sentence, because the link is what Google keeps stable.
fn is_gmail_content_block(detail: &str) -> bool {
    detail.contains("5.7.0") && detail.contains("p=BlockedMessage")
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

    // The same resolver the IMAP side uses, which is the point: it loads a password, or
    // refreshes an OAuth token when it is close to expiring and writes the new expiry back.
    // Duplicating that here would mean a second copy of the refresh logic, and the copy that
    // sends mail would be the one nobody noticed had gone stale.
    //
    // Until this call existed, `send_one` refused outright for anything but a password account,
    // and the only account type Gmail offers is OAuth -- so the app could not send at all. The
    // refusal was deliberate and documented; what was not written down is that it left the send
    // half of Phase 7 unreachable, and it stayed that way until somebody tried to send a message.
    let credential = super::engine::credential_for(db, &account)
        .await
        .map_err(|error| smtp::SendError::Credential {
            detail: error.to_string(),
            // Whether waiting could help is the sign-in layer's decision, and it already makes
            // it: a refused credential is not retryable, a provider that did not answer is.
            // This used to report every one of them as a malformed envelope, which is never
            // retryable, so the loop below set `attempts` to the maximum and the message went
            // straight to "was not sent" -- for a network blip, on the first attempt, having
            // never reached a mail server at all.
            retryable: error.is_retryable(),
        })?;

    smtp::send(&server, &account.email, &credential, &envelope, &raw).await?;

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
    fn gmail_blocking_an_attachment_is_explained_and_still_quoted() {
        // Word for word what Gmail answered on 2026-09-17 for a zip of three DLLs. The banner
        // showed exactly this and offered Try Again, which cannot work.
        let gmail = "permanent error (552): 5.7.0 This message was blocked because its content \
                     presents a potential security issue. To review our message content and \
                     attachment content guidelines, go to \
                     https://support.google.com/mail/?p=BlockedMessage \
                     d9443c01a7336-2dd89e93316sm22023355ad.14 - gsmtp";
        let refused = smtp::SendError::Refused {
            host: "smtp.gmail.com".into(),
            detail: gmail.into(),
        };

        let described = describe(&refused);
        assert!(described.starts_with("Gmail blocked this message because of an attachment"));
        assert!(described.contains(".zip"), "{described}");
        assert!(described.contains("will not help"), "{described}");
        // The server's own words survive, after the explanation.
        assert!(described.ends_with(gmail), "{described}");

        // And an ordinary refusal is still left alone.
        let full = smtp::SendError::Refused {
            host: "smtp.gmail.com".into(),
            detail: "552 5.2.2 The recipient's inbox is out of storage space".into(),
        };
        assert_eq!(
            describe(&full),
            "552 5.2.2 The recipient's inbox is out of storage space"
        );
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

    #[test]
    fn a_credential_that_could_not_be_checked_is_retried() {
        // The failure this exists for. Refreshing an OAuth token needs the network, and losing
        // it for a moment says nothing about whether the credential is good. This used to be
        // reported as `Envelope` — "the message has no usable envelope" — which is never
        // retryable, so the loop set `attempts` to the maximum and the message went straight to
        // "was not sent" on the first attempt, having never reached a mail server.
        let transient = smtp::SendError::Credential {
            detail: "network error talking to google".into(),
            retryable: true,
        };

        assert!(
            transient.is_retryable(),
            "a sign-in that could not be checked must be tried again"
        );

        let described = describe(&transient);
        assert!(described.ends_with('.'), "{described}");
        assert!(
            !described.contains("Signing in again"),
            "a transient failure must not send the user through a sign-in: {described}"
        );
    }

    #[test]
    fn a_refused_credential_is_not_retried() {
        // The other half. Hammering a credential the provider has refused is how an account
        // gets locked out, and the user has something to do about it.
        let refused = smtp::SendError::Credential {
            detail: "google refused the request: invalid_grant".into(),
            retryable: false,
        };

        assert!(!refused.is_retryable());
        assert!(describe(&refused).contains("Signing in again"));
    }

    #[test]
    fn a_credential_failure_never_reads_as_a_bad_recipient() {
        // What the old classification told the user. The envelope was fine; nobody had looked
        // at it yet.
        for retryable in [true, false] {
            let described = describe(&smtp::SendError::Credential {
                detail: "whatever went wrong".into(),
                retryable,
            });

            assert!(
                !described.contains("recipient"),
                "a sign-in problem was described as a recipient problem: {described}"
            );
            assert!(
                !described.contains("whatever went wrong"),
                "protocol detail reached the user: {described}"
            );
        }
    }
}
