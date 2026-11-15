//! The compose command surface. docs/03 §4, docs/06 Phase 7.
//!
//! Three commands and no more: prepare a reply, send, and undo. The compose window keeps its
//! own state while the user is typing — a round trip per keystroke would be absurd — and the
//! core is asked only at the two moments that matter.
//!
//! `compose_send` returns as soon as the message is **durably on disk and in the outbox**, not
//! when it has been transmitted. That is deliberate and is what makes Undo Send honest: by the
//! time the window closes the message cannot be lost, and for the length of the hold it has not
//! been sent either.

use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;

use crate::db::Db;
use crate::mail::outgoing::{self, Draft};
use crate::mail::reply;
use crate::sync::drafts;
use crate::sync::envelope::Address;
use crate::sync::outbox;
use crate::sync::sender::Sender;

use super::mail::AppError;

/// One address as the compose window holds it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ComposeAddress {
    pub name: Option<String>,
    pub email: String,
}

impl From<&ComposeAddress> for Address {
    fn from(value: &ComposeAddress) -> Self {
        Address {
            name: value.name.clone(),
            email: value.email.clone(),
        }
    }
}

impl From<&Address> for ComposeAddress {
    fn from(value: &Address) -> Self {
        ComposeAddress {
            name: value.name.clone(),
            email: value.email.clone(),
        }
    }
}

/// What the compose window sends back when the user presses Send.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingMessage {
    #[ts(type = "number")]
    pub account_id: i64,
    pub to: Vec<ComposeAddress>,
    pub cc: Vec<ComposeAddress>,
    pub bcc: Vec<ComposeAddress>,
    pub subject: String,
    pub html: String,
    pub text: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    /// Absolute paths of files to attach. Read by the core rather than sent as bytes.
    pub attachments: Vec<String>,
}

/// Everything the compose window needs to open as a reply.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ReplyDraft {
    #[ts(type = "number")]
    pub account_id: i64,
    pub to: Vec<ComposeAddress>,
    pub cc: Vec<ComposeAddress>,
    pub subject: String,
    /// The quoted original, already sanitised, ready to be placed below the cursor.
    pub quoted_html: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    /// The account's signature, already sanitised, and where the window should put it.
    pub signature_html: String,
    pub signature_placement: String,
    /// Files the draft starts with. Only ever populated by Forward as Attachment.
    ///
    /// The same shape the file picker returns, so the compose window draws the chip and sends
    /// the file by exactly the path it already uses — nothing downstream knows the difference
    /// between a message the user attached and one this put there.
    pub attachments: Vec<PickedFile>,
}

/// How long a message waits in `holding` before it is transmitted.
///
/// docs/01 §6 — 10 seconds by default, and the setting offers 10/20/30/off. Read here rather
/// than passed in, so the window cannot send with a different value from the one the user set.
async fn hold_seconds(db: &Db) -> i64 {
    let stored: Option<String> = db
        .read(|conn| {
            Ok(conn
                .query_row(
                    "SELECT value FROM setting WHERE key = 'compose.undoSeconds'",
                    [],
                    |row| row.get(0),
                )
                .ok())
        })
        .await
        .ok()
        .flatten();

    stored
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(10)
        .clamp(0, 60)
}

/// Passes a message on unaltered. docs/01 §6, docs/06 Phase 7.
///
/// Not a forward, and the difference is the whole feature: the recipient gets the original,
/// from its original sender, so their reply goes back to whoever wrote it. See
/// `mail::outgoing::redirect` for why that requires the cached raw source rather than a
/// rebuild from the stored HTML.
#[tauri::command]
pub async fn compose_redirect(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    sender: State<'_, Sender>,
    message_id: i64,
    to: Vec<ComposeAddress>,
    cc: Vec<ComposeAddress>,
    bcc: Vec<ComposeAddress>,
) -> Result<i64, AppError> {
    let (account_id, subject, raw_path) = db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT account_id, COALESCE(subject, ''), raw_path FROM message WHERE id = ?1",
                    rusqlite::params![message_id],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .ok())
        })
        .await?
        .ok_or_else(|| AppError {
            code: "no-message".into(),
            message: "That message is no longer in the mailbox.".into(),
        })?;

    let account = db
        .read(move |conn| crate::accounts::store::get(conn, account_id))
        .await?
        .ok_or_else(|| AppError {
            code: "no-account".into(),
            message: "That account no longer exists.".into(),
        })?;

    // The original bytes, or nothing. A redirect that quietly fell back to rebuilding the
    // message would send the recipient our reconstruction of it under someone else's name.
    let original = read_source(
        raw_path.as_deref(),
        "This message has not been downloaded in full yet, so it cannot be redirected. Open it \
         once and try again.",
    )?;

    let request = crate::mail::outgoing::Redirect {
        original: &original,
        from: Address {
            name: Some(account.display_name.clone()),
            email: account.email.clone(),
        },
        to: to.iter().map(Address::from).collect(),
        cc: cc.iter().map(Address::from).collect(),
        bcc: bcc.iter().map(Address::from).collect(),
        date: rfc2822_now(),
        resent_message_id: None,
    };

    let built = crate::mail::outgoing::redirect(&request).map_err(|error| AppError {
        code: "build-failed".into(),
        message: error.to_string(),
    })?;

    // Bcc included, exactly as an ordinary send does: the blind addresses exist nowhere in the
    // message and reach the transport only through this list.
    let recipients: Vec<String> = to
        .iter()
        .chain(cc.iter())
        .chain(bcc.iter())
        .map(|address| address.email.trim().to_string())
        .filter(|address| !address.is_empty())
        .collect();

    let root = crate::db::default_path()
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();

    let id = outbox::enqueue(
        db.inner(),
        &root,
        account_id,
        &built,
        &subject,
        &recipients.join(","),
        hold_seconds(db.inner()).await,
    )
    .await?;

    let _ = tauri::Emitter::emit(&app, "outbox:changed", ());
    sender.poke();

    // Held like any other send, and said so. A redirect showed no Undo banner, because only the
    // ordinary send told the main window it was holding a message, and it left when the sender
    // next looked rather than when its hold ran out: up to thirty seconds late (L10).
    let hold = hold_seconds(db.inner()).await;
    held(&app, sender.inner().clone(), id, account_id, hold);

    Ok(id)
}

/// Tells the main window a message is being held for Undo Send, and wakes the sender when the hold
/// runs out.
///
/// The compose window is a different window from the one with the banner, so without the event
/// the Undo Send banner appeared only once the hold was over, which is the whole of the time it
/// exists for. And without the wake the message left at the sender's next idle tick, up to
/// thirty seconds after its hold: Undo Send's timer should decide when it goes.
fn held(
    events: &dyn crate::sync::events::Events,
    sender: crate::sync::sender::Sender,
    id: i64,
    account_id: i64,
    hold: i64,
) {
    events.emit(
        "outbox:progress",
        crate::sync::events::payload(&crate::sync::sender::Progress {
            id,
            account_id,
            state: "holding".to_string(),
            error: None,
        }),
    );

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(hold.max(0) as u64)).await;
        sender.poke();
    });
}

/// The current moment as RFC 5322 asks for it.
///
/// Local offset rather than UTC. `Resent-Date` records when *this* person passed the message
/// on, and a trail of hops that all claim to be in London tells the recipient less than one
/// that says where each hop actually was.
fn rfc2822_now() -> String {
    chrono::Local::now().to_rfc2822()
}

/// The Undo Send delay, for the settings sheet. docs/01 §6.
#[tauri::command]
pub async fn compose_undo_seconds(db: State<'_, Db>) -> Result<i64, AppError> {
    Ok(hold_seconds(&db).await)
}

/// Sets the Undo Send delay. `0` turns it off.
///
/// Clamped rather than validated-and-rejected: the choices in the UI are 0, 10, 20 and 30, and
/// a value from anywhere else is a bug in the caller rather than something to put an error
/// message in front of the user about. The ceiling matters more than the floor — a window that
/// holds a message for an hour looks exactly like a window that failed to send it.
#[tauri::command]
pub async fn compose_set_undo_seconds(db: State<'_, Db>, seconds: i64) -> Result<i64, AppError> {
    let clamped = seconds.clamp(0, 60);

    db.write(move |tx| {
        tx.execute(
            "INSERT INTO setting (key, value) VALUES ('compose.undoSeconds', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![clamped.to_string()],
        )?;
        Ok(())
    })
    .await?;

    Ok(clamped)
}

/// Builds the reply a compose window should open with.
#[tauri::command]
pub async fn compose_reply(
    db: State<'_, Db>,
    message_id: i64,
    kind: String,
) -> Result<ReplyDraft, AppError> {
    let kind = match kind.as_str() {
        "reply" => reply::Kind::Reply,
        "replyAll" => reply::Kind::ReplyAll,
        "forward" => reply::Kind::Forward,
        "forwardAsAttachment" => reply::Kind::ForwardAsAttachment,
        other => {
            return Err(AppError {
                code: "bad-kind".into(),
                message: format!("{other} is not a reply kind"),
            })
        }
    };

    let prepared = db
        .read(move |conn| Ok(crate::db::query::reply_source(conn, message_id)))
        .await?;

    let Some(source) = prepared else {
        return Err(AppError {
            code: "not-found".into(),
            message: "That message is no longer in the mailbox.".into(),
        });
    };

    // Every address the user owns, so reply-all does not copy them to themselves.
    let mine: Vec<String> = db
        .read(|conn| {
            let mut statement = conn.prepare("SELECT email FROM account")?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;

    let recipients = reply::recipients(&source.envelope, kind, &mine);
    let signature = signature_for(db.inner(), source.account_id).await;

    // Forward as Attachment is the only kind that carries a file, and it is the only kind that
    // can fail for a reason the user can act on: the original has to have been downloaded.
    let attachments = if kind == reply::Kind::ForwardAsAttachment {
        let raw_path = db
            .read(move |conn| Ok(recorded_source(conn, message_id)))
            .await?;
        vec![attach_original(
            raw_path.as_deref(),
            message_id,
            &source.envelope.subject,
        )?]
    } else {
        Vec::new()
    };

    Ok(ReplyDraft {
        account_id: source.account_id,
        to: recipients.to.iter().map(ComposeAddress::from).collect(),
        cc: recipients.cc.iter().map(ComposeAddress::from).collect(),
        subject: reply::subject(&source.envelope.subject, kind),
        quoted_html: quoted_block(&source, kind),
        in_reply_to: source.envelope.message_id.clone(),
        references: reply::references(&source.references, source.envelope.message_id.as_deref()),
        signature_html: signature.html,
        signature_placement: signature.placement,
        attachments,
    })
}

/// Copies the cached original out to a file the compose window can attach.
///
/// A **copy**, into the temp directory, rather than the cache path itself. The cache is the
/// app's own store: handing its path to the composer would mean a send reading a file the sync
/// engine may rewrite underneath it, and a failed send holding a lock on it.
///
/// The name comes from the subject because the cache names files by id, and a recipient who
/// receives `4213.eml` has no idea what it is.
fn attach_original(
    raw_path: Option<&str>,
    message_id: i64,
    subject: &str,
) -> Result<PickedFile, AppError> {
    let bytes = read_source(
        raw_path,
        "This message has not been downloaded in full yet, so it cannot be forwarded as an \
         attachment. Open it once and try again.",
    )?;

    let name = attachment_name(subject);

    // In a folder of its own, so the *file* can carry the clean name. `compose_send` names an
    // attachment after its file, and this used to write `halcyon-fwd-<id>-<subject>.eml`
    // straight into the temp folder — which is the name recipients saw (L6, 2026-11-14 audit).
    // The folder keeps two forwards of messages with the same subject apart.
    let folder = std::env::temp_dir().join(format!("halcyon-fwd-{message_id}"));
    let path = folder.join(&name);

    std::fs::create_dir_all(&folder)
        .and_then(|()| std::fs::write(&path, &bytes))
        .map_err(|error| AppError {
            code: "write-failed".into(),
            message: format!("The message could not be prepared for forwarding: {error}"),
        })?;

    Ok(PickedFile {
        path: path.to_string_lossy().to_string(),
        filename: name,
        size: bytes.len() as i64,
    })
}

/// The source the row says this message has, if any.
fn recorded_source(conn: &rusqlite::Connection, message_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT raw_path FROM message WHERE id = ?1",
        rusqlite::params![message_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
}

/// A message's cached source, read through its row.
///
/// Through `raw_path`, never `bodies::cache_path`. Ids are reused, so the file named for a
/// message's id can still hold the source of a message deleted before it was — and Redirect, which
/// once read it that way, would have sent that one, under the new message's name, to whoever the
/// user typed in. `raw_path` is recorded only once the file holds this message's own source.
fn read_source(raw_path: Option<&str>, not_cached: &str) -> Result<Vec<u8>, AppError> {
    raw_path
        .and_then(|path| std::fs::read(path).ok())
        .ok_or_else(|| AppError {
            code: "not-cached".into(),
            message: not_cached.into(),
        })
}

/// A filename for a forwarded message, from its subject.
///
/// Everything Windows forbids in a name is replaced rather than removed, so two subjects that
/// differ only in punctuation do not collapse to the same file. Trimmed to a length that
/// leaves room for the prefix and the id within `MAX_PATH`.
fn attachment_name(subject: &str) -> String {
    let cleaned: String = subject
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            c if (c as u32) < 0x20 => '-',
            c => c,
        })
        .collect();

    let trimmed = cleaned.trim().trim_matches('.').trim();
    let short: String = trimmed.chars().take(80).collect();

    if short.is_empty() {
        "Forwarded message.eml".to_string()
    } else {
        format!("{short}.eml")
    }
}

/// Queues a message. Returns the outbox id, which is what Undo Send cancels.
///
/// Returns once the message is on disk and in the outbox — **not** once it has been sent. The
/// window can close immediately and the message cannot be lost, while for the length of the
/// hold nothing has been transmitted and undo is still honest.
#[tauri::command]
pub async fn compose_send(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    sender: State<'_, Sender>,
    message: OutgoingMessage,
    draft_message_id: Option<String>,
) -> Result<i64, AppError> {
    let account = db
        .read({
            let id = message.account_id;
            move |conn| crate::accounts::store::get(conn, id)
        })
        .await?
        .ok_or_else(|| AppError {
            code: "no-account".into(),
            message: "That account no longer exists.".into(),
        })?;

    let from = Address {
        name: Some(account.display_name.clone()),
        email: account.email.clone(),
    };

    // Gmail refuses some attachments outright, and says so only after the message has been
    // queued — leaving a failure that Try Again cannot fix. Known refusals are caught here,
    // while the compose window is still open and the attachment can simply be removed.
    let through_gmail = crate::mail::attachment_policy::sends_through_gmail(
        &account.provider,
        account.smtp.as_ref().map(|smtp| smtp.host.as_str()),
    );

    // Read here rather than carried through the IPC boundary as bytes. A 20MB attachment
    // base64-encoded into a JSON payload is 27MB of string on both sides of the seam, for a
    // file the core can open itself in one call.
    let mut attachments = Vec::with_capacity(message.attachments.len());
    for path in &message.attachments {
        let bytes = std::fs::read(path).map_err(|error| AppError {
            code: "attachment-unreadable".into(),
            message: format!("{path} could not be read: {error}"),
        })?;

        let filename = std::path::Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "attachment".to_string());

        if through_gmail {
            if let Some(refusal) = crate::mail::attachment_policy::gmail_refusal(&filename, &bytes)
            {
                return Err(AppError {
                    code: "attachmentBlocked".into(),
                    message: refusal.sentence(),
                });
            }
        }

        attachments.push(crate::mail::outgoing::Attachment {
            mime: mime_for(&filename),
            filename,
            bytes,
            // Files the user attached, so they are listed beside the message. An image dragged
            // into the body arrives by a different route and carries a `Content-ID`.
            content_id: None,
        });
    }

    let draft = Draft {
        from,
        to: message.to.iter().map(Address::from).collect(),
        cc: message.cc.iter().map(Address::from).collect(),
        bcc: message.bcc.iter().map(Address::from).collect(),
        subject: message.subject.clone(),
        html: message.html.clone(),
        text: message.text.clone(),
        references: message.references.clone(),
        in_reply_to: message.in_reply_to.clone(),
        attachments,
        message_id: None,
    };

    let built = outgoing::build(&draft).map_err(|error| AppError {
        code: "build-failed".into(),
        message: error.to_string(),
    })?;

    // The envelope's recipient list, including Bcc. Stored so the sender does not have to
    // re-derive it, and so the blind recipients survive a restart — they exist nowhere in the
    // message itself, by design.
    let recipients: Vec<String> = message
        .to
        .iter()
        .chain(message.cc.iter())
        .chain(message.bcc.iter())
        .map(|address| address.email.trim().to_string())
        .filter(|address| !address.is_empty())
        .collect();

    let root = crate::db::default_path()
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();

    let id = outbox::enqueue(
        db.inner(),
        &root,
        message.account_id,
        &built,
        &message.subject,
        &recipients.join(","),
        hold_seconds(db.inner()).await,
    )
    .await?;

    // The draft this message was written as, so the sender lets it go once the message has gone
    // — not now: Undo Send deletes the outbox copy, and the draft is then all that is left of it.
    // Nothing let it go before, so a sent message stayed in Drafts everywhere (H4).
    if let Some(draft) = draft_message_id.filter(|id| !drafts::bare(id).is_empty()) {
        db.write(move |tx| {
            tx.execute(
                "UPDATE outbox SET draft_message_id = ?2 WHERE id = ?1",
                rusqlite::params![id, drafts::bracketed(&draft)],
            )?;
            Ok(())
        })
        .await?;
    }

    // Told to the main window now, not when the sender picks it up, and the sender woken when
    // the hold runs out. See `held`.
    let hold = hold_seconds(db.inner()).await;
    held(&app, sender.inner().clone(), id, message.account_id, hold);

    Ok(id)
}

/// Cancels a message still in `holding`. This is **Undo Send**.
///
/// Returns false when it is too late — the message is already on its way and no local action
/// recalls it. The UI says so rather than claiming success, because the user would otherwise
/// find out from the recipient.
#[tauri::command]
pub async fn compose_undo(db: State<'_, Db>, id: i64) -> Result<bool, AppError> {
    Ok(outbox::cancel(db.inner(), id).await?)
}

/// Everything waiting or failed in the outbox, for the banner and the outbox view.
#[tauri::command]
pub async fn outbox_list(db: State<'_, Db>) -> Result<Vec<OutboxRow>, AppError> {
    let entries = outbox::pending(db.inner()).await?;

    Ok(entries
        .into_iter()
        .map(|entry| OutboxRow {
            id: entry.id,
            account_id: entry.account_id,
            state: entry.state.as_str().to_string(),
            subject: entry.subject.unwrap_or_default(),
            recipients: entry.recipients.unwrap_or_default(),
            send_after: entry.send_after,
            attempts: entry.attempts,
            last_error: entry.last_error,
        })
        .collect())
}

/// One row of the outbox, as the UI draws it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct OutboxRow {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub account_id: i64,
    pub state: String,
    pub subject: String,
    pub recipients: String,
    #[ts(type = "number")]
    pub send_after: i64,
    #[ts(type = "number")]
    pub attempts: i64,
    pub last_error: Option<String>,
}

/// Opens a compose window. docs/01 §6 — *a separate floating window, not a pane.*
///
/// Percent-encodes a value for the compose window's query string.
///
/// Hand-rolled rather than pulled from a crate because the set that has to be escaped here is
/// small and fixed, and because getting it wrong is loud: an unescaped `&` in a subject would
/// silently truncate it and turn the rest into a parameter of its own.
fn encode_param(value: &str) -> String {
    let mut out = String::with_capacity(value.len());

    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }

    out
}

/// A real OS window rather than a modal inside the main one, because that is what the design
/// calls for and because it is what people actually do with mail: start a reply, go and look
/// something up in another message, come back. A modal makes that impossible, and a pane makes
/// the list unusable while it is open.
///
/// Each window gets a distinct label, so several drafts can be open at once and Windows treats
/// them as separate entries in the taskbar and Alt-Tab, which is the behaviour a separate
/// window is *for*.
#[tauri::command]
pub async fn compose_open(
    app: tauri::AppHandle,
    message_id: Option<i64>,
    kind: Option<String>,
    mailto: Option<crate::platform::links::MailtoRequest>,
) -> Result<String, AppError> {
    let mut url = String::from("index.html?compose=1");
    if let Some(id) = message_id {
        url.push_str(&format!("&message={id}"));
    }
    if let Some(kind) = kind.as_deref() {
        url.push_str(&format!("&kind={kind}"));
    }

    // A `mailto:` arrives as already-sanitised fields, not as the original URL. Handing the raw
    // link to the window would mean parsing it a second time in TypeScript, and the second
    // parser is the one that forgets to drop Bcc — see links.rs. These are the four fields that
    // survived that filter and nothing else can be expressed here.
    if let Some(request) = mailto {
        for address in &request.to {
            url.push_str(&format!("&to={}", encode_param(address)));
        }
        for address in &request.cc {
            url.push_str(&format!("&cc={}", encode_param(address)));
        }
        if !request.subject.is_empty() {
            url.push_str(&format!("&subject={}", encode_param(&request.subject)));
        }
        if !request.body.is_empty() {
            url.push_str(&format!("&body={}", encode_param(&request.body)));
        }
    }

    open_compose_window(&app, url)
}

/// Opens a compose window on `url`, and returns its label.
fn open_compose_window(app: &tauri::AppHandle, url: String) -> Result<String, AppError> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    // Monotonic per process. A label that collided would return the existing window instead of
    // opening a second draft, which is a confusing way to lose what someone just typed.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let label = format!(
        "compose-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );

    // Tauri's native drag-and-drop handler stays ON for compose windows, and that is deliberate
    // — the opposite of the main window, which turns it off in tauri.conf.json.
    //
    // The main window needs HTML5 drag and drop for moving messages between mailboxes, and the
    // native handler swallows every drag before the page sees it. A compose window needs the
    // reverse: files dragged in from Explorer, and only the native handler reports those with
    // real filesystem paths, which is what the attachment pipeline is built on. A WebView2
    // `File` from an HTML5 drop has no path at all. So: do not call
    // `.disable_drag_drop_handler()` here. `ComposeWindow` listens for the drop through
    // `ipc.onFileDrop`.
    WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
        .title("New Message")
        // docs/01 §6 — 700x560 default, and the size is remembered by the window-state plugin.
        .inner_size(700.0, 560.0)
        .min_inner_size(480.0, 360.0)
        .decorations(true)
        .build()
        .map_err(|error| {
            tracing::warn!(%error, "could not open a compose window");
            AppError {
                code: "window-failed".into(),
                message: "The compose window could not be opened.".into(),
            }
        })?;

    Ok(label)
}

/// Deletes a failed message. The Delete button on the failure banner, behind a confirmation.
///
/// Returns false when the message is no longer failed — retried, or deleted from another
/// window — so the banner can refresh rather than claim something it did not do.
#[tauri::command]
pub async fn outbox_discard(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    id: i64,
) -> Result<bool, AppError> {
    let discarded = outbox::discard_failed(db.inner(), id).await?;

    if discarded {
        let _ = tauri::Emitter::emit(&app, "outbox:changed", ());
    }

    Ok(discarded)
}

/// Puts a failed message back in the queue. The Retry button on the failure banner.
#[tauri::command]
pub async fn outbox_retry(
    db: State<'_, Db>,
    sender: State<'_, Sender>,
    id: i64,
) -> Result<bool, AppError> {
    let retried = db.write(move |tx| outbox::retry(tx, id)).await?;

    if retried {
        // Now, not at the next idle tick. The user has just pressed a button and is watching.
        sender.inner().poke();
    }

    Ok(retried)
}

/// Holds a message until a chosen time. **Send Later**. docs/01 §6.
///
/// `send_at` is absolute epoch seconds, computed by the UI: "Tonight 9 PM" means nine in the
/// evening where the user is, and the core has no business guessing at a timezone the window
/// already knows.
#[tauri::command]
pub async fn outbox_schedule(
    db: State<'_, Db>,
    sender: State<'_, Sender>,
    id: i64,
    send_at: i64,
) -> Result<bool, AppError> {
    let moved = db
        .write(move |tx| outbox::reschedule(tx, id, send_at))
        .await?;

    // Woken when the new time comes, if it is soon — as `compose_send` does for the first hold.
    // Send Later's Cancel puts a paused hold back a few seconds out, and without this the message
    // waited for the sender's next idle tick, up to thirty seconds after the countdown said zero.
    let wait = send_at - now_seconds();
    if moved && wait <= 60 * 60 {
        let sender = sender.inner().clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(wait.max(0) as u64)).await;
            sender.poke();
        });
    }

    Ok(moved)
}

/// Guesses a content type from a filename.
///
/// A short table rather than sniffing the bytes. The extension is what the *recipient's* client
/// will use anyway, so agreeing with it is more useful than being right about the contents —
/// and a wrong guess degrades to `application/octet-stream`, which every client can save.
fn mime_for(filename: &str) -> String {
    let extension = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    let mime = match extension.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        // Without this a forwarded message goes out as application/octet-stream and the
        // recipient's client offers to save it rather than to open it.
        "eml" => "message/rfc822",
        "txt" | "log" | "md" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    };

    mime.to_string()
}

/// One file the user picked, described for the compose window.
// Deserialize as well as Serialize: it now travels *into* the window on a ReplyDraft as
// well as back out of the file picker.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PickedFile {
    pub path: String,
    pub filename: String,
    #[ts(type = "number")]
    pub size: i64,
}

/// Opens the system file picker and reports what was chosen.
///
/// The paths are returned rather than the contents: the window needs a name and a size to draw
/// a chip, and reading a 20MB file across the IPC boundary to show "20 MB" would be absurd. The
/// core opens them again at send time.
#[tauri::command]
pub async fn compose_pick_files() -> Result<Vec<PickedFile>, AppError> {
    let chosen = tokio::task::spawn_blocking(crate::platform::files::open_files_dialog)
        .await
        .map_err(|_| AppError {
            code: "cancelled".into(),
            message: "Choosing files was interrupted.".into(),
        })?;

    // The dialog only returns files that exist, so there is nothing to skip here — but it goes
    // through the same description as a drop, so a chip reads the same whichever way it came.
    Ok(chosen
        .iter()
        .filter_map(|path| describe_file(path).ok())
        .collect())
}

/// Files dropped on a compose window, described the way the picker describes its choices.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct DescribedFiles {
    pub files: Vec<PickedFile>,
    /// The names of dropped items that cannot be attached — a folder, or a path that has gone
    /// by the time it arrives. Reported rather than dropped silently: an attachment that simply
    /// fails to appear reads as the drop not having worked at all.
    pub skipped: Vec<String>,
}

/// Name and size for one path, or the name to report if it cannot be attached.
///
/// Only the metadata is read. The bytes are read at send time, as they are for a picked file —
/// a drop of a 20 MB file should draw its chip at once, not after the file has crossed the IPC
/// boundary to be counted.
fn describe_file(path: &std::path::Path) -> Result<PickedFile, String> {
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    match std::fs::metadata(path) {
        // A folder would be read at send time as though it were a file, and fail then — far
        // from the moment the user could have done something about it.
        Ok(meta) if meta.is_file() => Ok(PickedFile {
            path: path.to_string_lossy().to_string(),
            filename,
            size: meta.len() as i64,
        }),
        _ => Err(filename),
    }
}

/// Describes files dropped onto a compose window — attaching by dragging, as Mail allows. docs/01
/// §6 specifies only that attachments show as chips; the drop was asked for on 2026-09-17.
///
/// The paths come from the drop itself: Tauri's native drag-and-drop handler, which the compose
/// window keeps (see `compose_open`), hands over real filesystem paths. That is what makes a
/// drop fit the existing attachment pipeline, which is path-based end to end.
#[tauri::command]
pub async fn compose_describe_files(paths: Vec<String>) -> Result<DescribedFiles, AppError> {
    let described = tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        let mut skipped = Vec::new();

        for path in &paths {
            match describe_file(std::path::Path::new(path)) {
                Ok(file) => files.push(file),
                Err(name) => skipped.push(name),
            }
        }

        DescribedFiles { files, skipped }
    })
    .await
    .map_err(|_| AppError {
        code: "cancelled".into(),
        message: "Reading the dropped files was interrupted.".into(),
    })?;

    Ok(described)
}

/// The size at which the compose window warns. `mail::outgoing::ATTACHMENT_WARN_BYTES`.
#[tauri::command]
pub async fn compose_size_limit() -> Result<i64, AppError> {
    Ok(crate::mail::outgoing::ATTACHMENT_WARN_BYTES as i64)
}

/// The signature for an account, and where it goes.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Signature {
    pub html: String,
    /// `above` or `below`, relative to a quoted reply.
    pub placement: String,
}

/// Reads an account's signature.
#[tauri::command]
pub async fn signature_get(db: State<'_, Db>, account_id: i64) -> Result<Signature, AppError> {
    let row: Option<(Option<String>, String)> = db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT signature_html, signature_placement FROM account WHERE id = ?1",
                    rusqlite::params![account_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok())
        })
        .await?;

    let (html, placement) = row.unwrap_or((None, "above".to_string()));

    Ok(Signature {
        html: html.unwrap_or_default(),
        placement,
    })
}

/// Stores an account's signature.
///
/// Sanitised on the way in. It is the user's own markup, but it is markup, and it will be
/// placed into every message they send — a broken tag pasted from a web page would corrupt the
/// structure of every one of them, which is a slow and confusing way to discover a problem.
#[tauri::command]
pub async fn signature_set(
    db: State<'_, Db>,
    account_id: i64,
    html: String,
    placement: String,
) -> Result<(), AppError> {
    let placement = if placement == "below" {
        "below"
    } else {
        "above"
    };
    let clean = crate::mail::render::sanitise_for_enumeration(&html);

    db.write(move |tx| {
        tx.execute(
            "UPDATE account SET signature_html = ?2, signature_placement = ?3 WHERE id = ?1",
            rusqlite::params![account_id, clean, placement],
        )?;
        Ok(())
    })
    .await?;

    Ok(())
}

/// Formats a message's date the way Apple Mail's attribution line does.
///
/// `27 Aug 2026, at 09:34` — in the **user's** local time, not the sender's. A reply that
/// attributes a message to a time the recipient never saw it is confusing in exactly the way
/// that makes someone distrust a client's dates generally.
fn attribution_date(epoch_seconds: i64) -> String {
    use chrono::{Local, TimeZone};

    match Local.timestamp_opt(epoch_seconds, 0).single() {
        Some(when) => when.format("%-d %b %Y, at %H:%M").to_string(),
        // A message with an unparseable date still deserves a reply. Standing rule 13.
        None => "an earlier date".to_string(),
    }
}

/// Escapes text for placement in the reply document.
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The quoted original, with its attribution line, ready to sit below the cursor.
///
/// A forward is not a reply and does not get one: Mail heads a forward with the original's
/// headers — who it was from, when, to whom, what it was about — because the recipient of a
/// forward was never part of the conversation and needs all four.
fn quoted_block(source: &crate::db::query::ReplySource, kind: reply::Kind) -> String {
    // The original travels as the attachment, and that is all. It used to fall through to the
    // reply quote below as well, so the recipient got the message twice (L6, 2026-11-14 audit).
    if kind == reply::Kind::ForwardAsAttachment {
        return "<p><br></p>".to_string();
    }

    let when = attribution_date(source.envelope.date_sent);
    let sender = source.envelope.from.first();

    if kind == reply::Kind::Forward {
        let from = sender
            .map(|address| match &address.name {
                Some(name) if !name.trim().is_empty() => {
                    format!("{} &lt;{}&gt;", escape(name.trim()), escape(&address.email))
                }
                _ => escape(&address.email),
            })
            .unwrap_or_else(|| "unknown".to_string());

        let to = source
            .envelope
            .to
            .iter()
            .map(|address| escape(&address.email))
            .collect::<Vec<_>>()
            .join(", ");

        return format!(
            "<p><br></p><div><p>---------- Forwarded message ----------</p>\
             <p>From: {from}<br>Date: {when}<br>Subject: {}<br>To: {to}</p>{}</div>",
            escape(&source.envelope.subject),
            source.quoted_html
        );
    }

    // The attribution, then the original inside a blockquote. The empty paragraph above is
    // where the caret lands: a reply whose cursor starts inside the quote is the single most
    // annoying thing a mail client can do, and it is the default in more of them than it
    // should be.
    format!(
        "<p><br></p><p>{}</p><blockquote>{}</blockquote>",
        escape(&reply::attribution(sender, &when)),
        source.quoted_html
    )
}

/// Reads a signature without going through the command layer.
async fn signature_for(db: &Db, account_id: i64) -> Signature {
    let row: Option<(Option<String>, String)> = db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT signature_html, signature_placement FROM account WHERE id = ?1",
                    rusqlite::params![account_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok())
        })
        .await
        .ok()
        .flatten();

    let (html, placement) = row.unwrap_or((None, "above".to_string()));

    Signature {
        html: html.unwrap_or_default(),
        placement,
    }
}

/// A new message, with only the signature in it.
///
/// Its own command rather than `compose_reply` with a null id: a new message has no source to
/// read, no recipients to compute and no quote to build, and threading a "maybe there is a
/// parent" branch through all of that would make the reply path harder to follow for the sake
/// of the simpler case.
#[tauri::command]
pub async fn compose_blank(db: State<'_, Db>, account_id: i64) -> Result<ReplyDraft, AppError> {
    let signature = signature_for(db.inner(), account_id).await;

    Ok(ReplyDraft {
        account_id,
        to: Vec::new(),
        cc: Vec::new(),
        subject: String::new(),
        quoted_html: String::new(),
        in_reply_to: None,
        references: Vec::new(),
        signature_html: signature.html,
        signature_placement: signature.placement,
        // A new message starts empty; only Forward as Attachment puts a file on a draft.
        attachments: Vec::new(),
    })
}

/// A draft as it is stored and restored.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct DraftState {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub account_id: i64,
    pub message_id: String,
    pub to: Vec<ComposeAddress>,
    pub cc: Vec<ComposeAddress>,
    pub bcc: Vec<ComposeAddress>,
    pub subject: String,
    pub html: String,
    pub text: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    /// True when this draft was also edited somewhere else.
    ///
    /// Surfaced rather than resolved. Whichever copy an automatic merge discarded would be work
    /// somebody did, and the person who did it is the only one who can say which version
    /// matters — so both are kept on the server and the window says so.
    pub conflict: bool,
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Saves a draft, locally and then to the server.
///
/// **Local first, and the local write is what the caller waits for.** Autosave runs every
/// thirty seconds while someone is typing; if it waited for a round trip, writing a message
/// would become a stream of network stalls, and a draft written with no connection would be no
/// draft at all. The server copy goes through `pending_op`, which already knows how to carry an
/// intent across a lost connection — and is pushed now rather than at the next sync, so the
/// draft is in the Drafts mailbox, where it can be reopened, within seconds.
///
/// `message_id` is stable across saves. It is what lets the tenth save recognise the ninth as
/// the same draft, both here and on the server.
///
/// What changed, and why (2026-11-14 audit):
///
/// * **Every draft is filed on the server**, recipients or not, and with its attachments and
///   Bcc. A draft with nobody in To was kept only as a local row nothing read, so Save as Draft
///   lost it, and no draft ever kept a file (C3). See `outgoing::build_draft`.
/// * **A draft moved to another account starts again.** Changing From kept the identity, so
///   the old account's copy was orphaned and the next save named its UID in the *new*
///   account's Drafts — deleting whatever unrelated draft held that number there (H6).
/// * **A new draft's identity is random**, not `draft-<seconds>`, which two windows saving in
///   the same second shared (L7).
#[tauri::command]
pub async fn compose_save_draft(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    message: OutgoingMessage,
    message_id: Option<String>,
) -> Result<DraftState, AppError> {
    let account = db
        .read({
            let id = message.account_id;
            move |conn| crate::accounts::store::get(conn, id)
        })
        .await?
        .ok_or_else(|| AppError {
            code: "no-account".into(),
            message: "That account no longer exists.".into(),
        })?;

    let account_id = message.account_id;
    let root = db.folder().to_path_buf();

    // A draft that has changed account is let go of where it was, and starts again here.
    let message_id = match message_id.filter(|id| !drafts::bare(id).is_empty()) {
        Some(id) => {
            let keyed = drafts::bracketed(&id);
            let lookup = keyed.clone();
            let owner: Option<i64> = db
                .read(move |conn| {
                    Ok(conn
                        .query_row(
                            "SELECT account_id FROM draft WHERE message_id = ?1",
                            rusqlite::params![lookup],
                            |row| row.get(0),
                        )
                        .ok())
                })
                .await?;

            if owner.is_some_and(|owner| owner != account_id) {
                let root = root.clone();
                let files = db.write(move |tx| drafts::discard(tx, &root, &id)).await?;
                for file in files {
                    let _ = std::fs::remove_file(file);
                }
                None
            } else {
                Some(keyed)
            }
        }
        None => None,
    };

    let from = Address {
        name: Some(account.display_name.clone()),
        email: account.email.clone(),
    };

    // The files themselves, so the server copy carries them and a draft reopened on another
    // machine has them. A file that has gone since it was attached is left out of this save
    // rather than failing it: the user is typing, and a save is not the moment to stop them.
    let mut files = Vec::with_capacity(message.attachments.len());
    for path in &message.attachments {
        match std::fs::read(path) {
            Ok(bytes) => {
                let filename = std::path::Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "attachment".to_string());
                files.push(crate::mail::outgoing::Attachment {
                    mime: mime_for(&filename),
                    filename,
                    bytes,
                    content_id: None,
                });
            }
            Err(error) => tracing::debug!(%error, "draft: an attachment could not be read"),
        }
    }

    let draft = Draft {
        from: from.clone(),
        to: message.to.iter().map(Address::from).collect(),
        cc: message.cc.iter().map(Address::from).collect(),
        bcc: message.bcc.iter().map(Address::from).collect(),
        subject: message.subject.clone(),
        html: message.html.clone(),
        text: message.text.clone(),
        references: message.references.clone(),
        in_reply_to: message.in_reply_to.clone(),
        attachments: files,
        message_id: message_id.clone(),
    };

    // Fails only on an address that cannot be written into a header — one still being typed. The
    // draft is kept here regardless, and filed on the server by the next save that builds.
    let built = outgoing::build_draft(&draft).ok();

    let identity = built
        .as_ref()
        .map(|b| b.message_id.clone())
        .or(message_id)
        .unwrap_or_else(|| outgoing::new_message_id(&from));

    let stored = {
        let identity = identity.clone();
        let to = serde_json::to_string(&message.to).unwrap_or_else(|_| "[]".into());
        let cc = serde_json::to_string(&message.cc).unwrap_or_else(|_| "[]".into());
        let bcc = serde_json::to_string(&message.bcc).unwrap_or_else(|_| "[]".into());
        let attachments =
            serde_json::to_string(&message.attachments).unwrap_or_else(|_| "[]".into());
        let subject = message.subject.clone();
        let html = message.html.clone();
        let text = message.text.clone().unwrap_or_default();
        let in_reply_to = message.in_reply_to.clone();
        let references = message.references.join(" ");

        db.write(move |tx| {
            tx.execute(
                "INSERT INTO draft (
                     account_id, message_id, to_json, cc_json, bcc_json,
                     subject, html, text, in_reply_to, references_, updated_at, attachments_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(message_id) DO UPDATE SET
                     to_json = excluded.to_json,
                     cc_json = excluded.cc_json,
                     bcc_json = excluded.bcc_json,
                     subject = excluded.subject,
                     html = excluded.html,
                     text = excluded.text,
                     in_reply_to = excluded.in_reply_to,
                     references_ = excluded.references_,
                     updated_at = excluded.updated_at,
                     attachments_json = excluded.attachments_json",
                rusqlite::params![
                    account_id,
                    identity,
                    to,
                    cc,
                    bcc,
                    subject,
                    html,
                    text,
                    in_reply_to,
                    references,
                    now_seconds(),
                    attachments
                ],
            )?;

            let id: i64 = tx.query_row(
                "SELECT id FROM draft WHERE message_id = ?1",
                rusqlite::params![identity],
                |row| row.get(0),
            )?;

            Ok(id)
        })
        .await?
    };

    // The server copy, queued. `replaces` is left for the drain to read from the row when the
    // append actually runs — see `ops::drain`.
    if let Some(built) = built {
        let path = drafts::eml_path(&root, stored);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        if std::fs::write(&path, &built.bytes).is_ok() {
            let eml_path = path.to_string_lossy().to_string();
            let message_id_for_op = built.message_id.clone();

            db.write(move |tx| {
                let mailbox = tx
                    .query_row(
                        "SELECT remote_path FROM mailbox
                          WHERE account_id = ?1 AND role = 'drafts'",
                        rusqlite::params![account_id],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap_or_else(|_| "Drafts".to_string());

                crate::sync::ops::enqueue(
                    tx,
                    account_id,
                    &crate::sync::ops::Op::AppendDraft {
                        mailbox,
                        eml_path,
                        replaces: None,
                        message_id: message_id_for_op,
                    },
                )
            })
            .await?;

            super::sync::push_soon(&app);
        }
    }

    // Read after the queue was written, so a conflict the drain found while this save was in
    // flight is reported by this save rather than waiting thirty seconds for the next one.
    let conflict = db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT conflict_at FROM draft WHERE id = ?1",
                    rusqlite::params![stored],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .ok()
                .flatten()
                .is_some())
        })
        .await
        .unwrap_or(false);

    Ok(DraftState {
        conflict,
        id: stored,
        account_id,
        message_id: identity,
        to: message.to,
        cc: message.cc,
        bcc: message.bcc,
        subject: message.subject,
        html: message.html,
        text: message.text.unwrap_or_default(),
        in_reply_to: message.in_reply_to,
        references: message.references,
    })
}

/// Throws a draft away: Delete in the compose window's close sheet.
///
/// Everywhere, not only here — see `sync::drafts::discard`. It removed only the local row, and
/// nothing called it (H4, 2026-11-14 audit).
#[tauri::command]
pub async fn compose_discard_draft(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    message_id: String,
) -> Result<(), AppError> {
    let root = db.folder().to_path_buf();
    let files = db
        .write(move |tx| drafts::discard(tx, &root, &message_id))
        .await?;

    for file in files {
        let _ = std::fs::remove_file(file);
    }

    super::sync::push_soon(&app);
    let _ = tauri::Emitter::emit(&app, "mailbox:changed", ());
    Ok(())
}

/// A draft, as a compose window opens with it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct LoadedDraft {
    #[ts(type = "number")]
    pub account_id: i64,
    /// The draft's identity. The window saves under it, so each save replaces this draft rather
    /// than starting another beside it.
    pub message_id: String,
    pub to: Vec<ComposeAddress>,
    pub cc: Vec<ComposeAddress>,
    pub bcc: Vec<ComposeAddress>,
    pub subject: String,
    pub html: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<PickedFile>,
}

/// Opens a compose window on a draft in a Drafts mailbox. Edit Draft.
///
/// Drafts could be saved and never opened again, anywhere in the app (C3, 2026-11-14 audit).
#[tauri::command]
pub async fn compose_open_draft(
    app: tauri::AppHandle,
    message_id: i64,
) -> Result<String, AppError> {
    open_compose_window(&app, format!("index.html?compose=1&draft={message_id}"))
}

/// What the compose window opened by [`compose_open_draft`] starts with.
///
/// This machine's own copy when it has one — it is the newest, and it knows where the attached
/// files are. Otherwise the message itself, downloaded if need be: a draft written on another
/// device, or saved here before drafts were kept. Either way the draft is remembered here with
/// the UID of the server copy it came from, so the first save *replaces* that copy instead of
/// filing a second one beside it.
#[tauri::command]
pub async fn compose_load_draft(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    message_id: i64,
) -> Result<LoadedDraft, AppError> {
    type Row = (i64, Option<String>, i64, Option<String>);

    let row: Row = db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT message.account_id, message.message_id, message.uid, mailbox.role
                       FROM message JOIN mailbox ON mailbox.id = message.mailbox_id
                      WHERE message.id = ?1",
                    rusqlite::params![message_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .ok())
        })
        .await?
        .ok_or_else(|| AppError {
            code: "no-message".into(),
            message: "That draft is no longer in the mailbox.".into(),
        })?;

    let (account_id, header_id, uid, role) = row;
    let identity = header_id
        .filter(|id| !drafts::bare(id).is_empty())
        .map(|id| drafts::bracketed(&id));

    if let Some(identity) = identity.clone() {
        let local = db
            .read(move |conn| Ok(local_draft(conn, &identity)))
            .await?;
        if let Some(local) = local {
            return Ok(local);
        }
    }

    // The source, downloaded first if it has not been. A draft is opened to be edited, and the
    // only alternative to fetching it is refusing.
    let mut raw_path = db
        .read(move |conn| Ok(recorded_source(conn, message_id)))
        .await?;
    if raw_path.is_none() {
        if let Err(error) =
            crate::sync::engine::fetch_body(&app, db.inner(), account_id, vec![message_id]).await
        {
            tracing::warn!(%error, "draft: its source could not be downloaded");
        }
        raw_path = db
            .read(move |conn| Ok(recorded_source(conn, message_id)))
            .await?;
    }

    let raw = read_source(
        raw_path.as_deref(),
        "This draft has not been downloaded yet. Try again once the account is back online.",
    )?;

    let folder = std::env::temp_dir().join(format!("halcyon-fwd-draft-{message_id}"));
    let loaded = draft_from_source(&raw, account_id, identity, &folder)?;

    // Remembered as a draft, with the server copy it came from, so the next save replaces it.
    {
        let loaded = loaded.clone();
        let remote_uid = (role.as_deref() == Some("drafts") && uid > 0).then_some(uid);
        db.write(move |tx| {
            let attachments: Vec<&str> =
                loaded.attachments.iter().map(|f| f.path.as_str()).collect();
            tx.execute(
                "INSERT INTO draft (
                     account_id, message_id, remote_uid, to_json, cc_json, bcc_json, subject,
                     html, in_reply_to, references_, updated_at, attachments_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(message_id) DO NOTHING",
                rusqlite::params![
                    loaded.account_id,
                    loaded.message_id,
                    remote_uid,
                    serde_json::to_string(&loaded.to).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&loaded.cc).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&loaded.bcc).unwrap_or_else(|_| "[]".into()),
                    loaded.subject,
                    loaded.html,
                    loaded.in_reply_to,
                    loaded.references.join(" "),
                    now_seconds(),
                    serde_json::to_string(&attachments).unwrap_or_else(|_| "[]".into()),
                ],
            )?;
            Ok(())
        })
        .await?;
    }

    Ok(loaded)
}

/// This machine's copy of a draft, if it has one.
fn local_draft(conn: &rusqlite::Connection, identity: &str) -> Option<LoadedDraft> {
    type Stored = (
        i64,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
    );

    let row: Stored = conn
        .query_row(
            "SELECT account_id, to_json, cc_json, bcc_json, subject, html, in_reply_to,
                    references_, attachments_json
               FROM draft WHERE message_id = ?1",
            rusqlite::params![identity],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .ok()?;

    let (account_id, to, cc, bcc, subject, html, in_reply_to, references, attachments) = row;
    let addresses =
        |json: &str| serde_json::from_str::<Vec<ComposeAddress>>(json).unwrap_or_default();
    let paths = serde_json::from_str::<Vec<String>>(&attachments).unwrap_or_default();

    Some(LoadedDraft {
        account_id,
        message_id: identity.to_string(),
        to: addresses(&to),
        cc: addresses(&cc),
        bcc: addresses(&bcc),
        subject,
        html,
        in_reply_to,
        references: references
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect(),
        // Files moved or deleted since are left off rather than failing the open.
        attachments: paths
            .iter()
            .filter_map(|path| describe_file(std::path::Path::new(path)).ok())
            .collect(),
    })
}

/// A draft read from its source: recipients, Bcc included, the body, threading, and its files
/// written out to `folder` so they can be attached again.
fn draft_from_source(
    raw: &[u8],
    account_id: i64,
    identity: Option<String>,
    folder: &std::path::Path,
) -> Result<LoadedDraft, AppError> {
    use mailparse::MailHeaderMap;

    let unreadable = || AppError {
        code: "unreadable".into(),
        message: "This draft could not be read.".into(),
    };

    let parsed = crate::mail::mime::parse(raw).map_err(|_| unreadable())?;
    let envelope = crate::sync::envelope::from_headers(&parsed.headers);

    let mut html: Option<String> = None;
    let mut text: Option<String> = None;
    let mut files: Vec<PickedFile> = Vec::new();

    let mut pending = vec![&parsed];
    while let Some(part) = pending.pop() {
        if !part.subparts.is_empty() {
            // Reversed so the parts are visited in the order they were written.
            pending.extend(part.subparts.iter().rev());
            continue;
        }

        let disposition = part.get_content_disposition();
        let filename = disposition
            .params
            .get("filename")
            .or_else(|| part.ctype.params.get("name"))
            .cloned();
        let mime = part.ctype.mimetype.to_ascii_lowercase();

        if let Some(name) = filename {
            let Ok(bytes) = part.get_body_raw() else {
                continue;
            };
            let name = crate::platform::files::safe_file_name(&name);
            let path = folder.join(&name);

            if std::fs::create_dir_all(folder)
                .and_then(|()| std::fs::write(&path, &bytes))
                .is_ok()
            {
                files.push(PickedFile {
                    path: path.to_string_lossy().to_string(),
                    filename: name,
                    size: bytes.len() as i64,
                });
            }
            continue;
        }

        match mime.as_str() {
            "text/html" if html.is_none() => html = part.get_body().ok(),
            "text/plain" if text.is_none() => text = part.get_body().ok(),
            _ => {}
        }
    }

    // Sanitised like anything else read from a mailbox: a draft from another device is markup
    // nobody here has looked at, and it is about to be edited and sent under the user's name.
    let html = match (html, text) {
        (Some(html), _) => crate::mail::render::sanitise_for_enumeration(&html),
        (None, Some(text)) => text
            .replace("\r\n", "\n")
            .split("\n\n")
            .map(|paragraph| format!("<p>{}</p>", escape(paragraph).replace('\n', "<br>")))
            .collect(),
        (None, None) => String::new(),
    };

    let identity = identity
        .or_else(|| envelope.message_id.as_deref().map(drafts::bracketed))
        .ok_or_else(unreadable)?;

    let references = parsed
        .headers
        .get_first_value("References")
        .map(|value| crate::sync::envelope::parse_references(&value))
        .unwrap_or_default();

    Ok(LoadedDraft {
        account_id,
        message_id: identity,
        to: envelope.to.iter().map(ComposeAddress::from).collect(),
        cc: envelope.cc.iter().map(ComposeAddress::from).collect(),
        bcc: envelope.bcc.iter().map(ComposeAddress::from).collect(),
        subject: envelope.subject,
        html,
        in_reply_to: envelope.in_reply_to,
        references,
        attachments: files,
    })
}

/// One suggested recipient.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub name: Option<String>,
    pub email: String,
    /// How many messages this address appears on. Drives the ordering.
    #[ts(type = "number")]
    pub seen: i64,
}

/// Suggests recipients from the people already in the mailbox.
///
/// docs/06 Phase 7 — *autocomplete from contacts + previous recipients.* There is no address
/// book to read on Windows that every user has, so the mailbox is the address book: the people
/// who have written to the user, and the people the user has written to.
///
/// Ranked by how often an address appears rather than how recently. Recency puts whoever sent
/// the last newsletter at the top of every field; frequency puts the people actually
/// corresponded with there, which is what the feature is for.
#[tauri::command]
pub async fn contacts_suggest(
    db: State<'_, Db>,
    prefix: String,
    limit: Option<i64>,
) -> Result<Vec<Contact>, AppError> {
    let needle = prefix.trim().to_lowercase();

    // Nothing typed yet means no suggestion. A dropdown that opens on focus with the twenty
    // most-mailed people covers the field the moment it is clicked.
    if needle.is_empty() {
        return Ok(Vec::new());
    }

    let limit = limit.unwrap_or(8).clamp(1, 25);

    let rows = db
        .read(move |conn| {
            // Senders only. The `to_all` column is a denormalised search string rather than a
            // parsed list, so mining it for addresses would produce fragments; the people the
            // user writes to are overwhelmingly also people who write back, and Phase 8's
            // contact work can widen this properly.
            let mut statement = conn.prepare(
                "SELECT from_addr,
                        MAX(from_name) AS display_name,
                        COUNT(*)       AS seen
                   FROM message
                  WHERE from_addr IS NOT NULL
                    AND from_addr <> ''
                    AND (LOWER(from_addr) LIKE ?1 OR LOWER(COALESCE(from_name, '')) LIKE ?1)
                  GROUP BY LOWER(from_addr)
                  ORDER BY seen DESC, display_name ASC
                  LIMIT ?2",
            )?;

            let pattern = format!("%{needle}%");
            let found = statement
                .query_map(rusqlite::params![pattern, limit], |row| {
                    Ok(Contact {
                        email: row.get::<_, String>(0)?,
                        name: row.get::<_, Option<String>>(1)?,
                        seen: row.get::<_, i64>(2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            Ok(found)
        })
        .await?;

    Ok(rows)
}

#[cfg(test)]
mod describe_tests {
    use super::*;

    #[tokio::test]
    async fn a_drop_attaches_files_and_names_what_it_could_not() {
        // A drop can carry anything Explorer can drag: files, a folder, and — by the time the
        // event arrives — a path that has just been deleted. Only the first can be attached, and
        // the rest have to be reported rather than quietly left out, or the drop looks like it
        // did nothing.
        let dir = tempfile::tempdir().expect("tempdir");

        let file = dir.path().join("Rent agreement.docx");
        std::fs::write(&file, b"twelve bytes").expect("write");

        let folder = dir.path().join("Photos");
        std::fs::create_dir(&folder).expect("mkdir");

        let gone = dir.path().join("deleted.txt");

        let described = compose_describe_files(vec![
            file.to_string_lossy().to_string(),
            folder.to_string_lossy().to_string(),
            gone.to_string_lossy().to_string(),
        ])
        .await
        .expect("describe");

        assert_eq!(described.files.len(), 1);
        let attached = &described.files[0];
        assert_eq!(attached.filename, "Rent agreement.docx");
        assert_eq!(attached.size, 12);
        assert_eq!(attached.path, file.to_string_lossy());

        assert_eq!(described.skipped, vec!["Photos", "deleted.txt"]);
    }

    #[tokio::test]
    async fn an_empty_drop_is_not_an_error() {
        let described = compose_describe_files(Vec::new()).await.expect("describe");
        assert!(described.files.is_empty());
        assert!(described.skipped.is_empty());
    }
}

#[cfg(test)]
mod held_tests {
    use super::*;

    struct Recorder(std::sync::Mutex<Vec<(String, serde_json::Value)>>);

    impl crate::sync::events::Events for Recorder {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            if let Ok(mut seen) = self.0.lock() {
                seen.push((event.to_string(), payload));
            }
        }
    }

    #[tokio::test]
    async fn a_held_message_is_announced_to_the_main_window() {
        // The redirect's half of Undo Send was missing: no banner (L10).
        let recorder = Recorder(std::sync::Mutex::new(Vec::new()));

        held(&recorder, crate::sync::sender::Sender::new(), 7, 2, 0);

        let seen = recorder.0.lock().expect("lock");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "outbox:progress");
        assert_eq!(seen[0].1["id"], 7);
        assert_eq!(seen[0].1["state"], "holding");
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;

    fn store_with(raw_path: Option<&str>) -> rusqlite::Connection {
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
        conn.execute(
            "INSERT INTO message (id, account_id, mailbox_id, uid, date_sent, date_received, raw_path)
             VALUES (7, 1, 1, 7, 0, 0, ?1)",
            rusqlite::params![raw_path],
        )
        .expect("message");
        conn
    }

    #[test]
    fn a_stale_file_at_a_reused_id_is_never_sent_as_this_message() {
        // Message 7 was deleted with its source still on disk, and the id was handed to a new
        // message whose body has not been downloaded. The file named for id 7 is the old one;
        // Forward as Attachment and Redirect used to read it by that name and send it.
        let dir = tempfile::tempdir().expect("temp dir");
        let stale = crate::sync::bodies::write_cache(dir.path(), 1, 7, b"the deleted message")
            .expect("write");
        assert!(stale.exists());

        let conn = store_with(None);
        let recorded = recorded_source(&conn, 7);

        assert_eq!(recorded, None);
        let refused = attach_original(recorded.as_deref(), 7, "New message").expect_err("refused");
        assert_eq!(refused.code, "not-cached");
        assert!(
            !refused.message.contains("  "),
            "the message has a run of spaces in it"
        );
    }

    #[test]
    fn a_recorded_source_is_attached_as_a_copy() {
        const SOURCE: &[u8] = b"Subject: Hi\r\n\r\nbody";
        let dir = tempfile::tempdir().expect("temp dir");
        let source = crate::sync::bodies::write_cache(dir.path(), 1, 7, SOURCE).expect("write");
        let path = source.to_string_lossy().to_string();

        let conn = store_with(Some(&path));
        let recorded = recorded_source(&conn, 7);
        assert_eq!(recorded.as_deref(), Some(path.as_str()));

        let attached =
            attach_original(recorded.as_deref(), 7, "Quarterly figures").expect("attached");
        assert_eq!(attached.size, SOURCE.len() as i64);
        assert!(attached.filename.contains("Quarterly figures"));
        assert_ne!(
            attached.path, path,
            "the cache file itself was handed to the composer"
        );
        assert_eq!(std::fs::read(&attached.path).expect("copy"), SOURCE);

        let _ = std::fs::remove_file(&attached.path);
    }

    /// L6 in the 2026-11-14 audit. `compose_send` names an attachment after its file, and the
    /// copy was written as `halcyon-fwd-<id>-<subject>.eml`, so that is what recipients saw —
    /// the app's name and an internal id, in the name of a file in someone else's inbox.
    #[test]
    fn a_forwarded_message_reaches_the_recipient_named_after_its_subject() {
        const SOURCE: &[u8] = b"Subject: Hi\r\n\r\nbody";
        let dir = tempfile::tempdir().expect("temp dir");
        let source = crate::sync::bodies::write_cache(dir.path(), 1, 7, SOURCE).expect("write");
        let path = source.to_string_lossy().to_string();

        let attached = attach_original(Some(&path), 7, "Quarterly figures").expect("attached");

        let on_disk = std::path::Path::new(&attached.path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string());
        assert_eq!(on_disk.as_deref(), Some("Quarterly figures.eml"));
        assert_eq!(attached.filename, "Quarterly figures.eml");

        let _ =
            std::fs::remove_dir_all(std::path::Path::new(&attached.path).parent().expect("dir"));
    }

    /// L6 again: Forward as Attachment fell through to the reply quote, so the original went
    /// twice — as the attachment, and quoted in full in the body above it.
    #[test]
    fn forward_as_attachment_does_not_also_quote_the_original() {
        let source = crate::db::query::ReplySource {
            account_id: 1,
            envelope: crate::sync::envelope::Envelope {
                subject: "Quarterly figures".into(),
                ..Default::default()
            },
            references: Vec::new(),
            quoted_html: "<p>THE ORIGINAL BODY</p>".into(),
        };

        let body = quoted_block(&source, reply::Kind::ForwardAsAttachment);
        assert!(!body.contains("THE ORIGINAL BODY"), "{body}");

        // The ordinary kinds still quote it.
        assert!(quoted_block(&source, reply::Kind::Forward).contains("THE ORIGINAL BODY"));
        assert!(quoted_block(&source, reply::Kind::Reply).contains("THE ORIGINAL BODY"));
    }
}

#[cfg(test)]
mod draft_tests {
    use super::*;

    fn address(name: Option<&str>, email: &str) -> Address {
        Address {
            name: name.map(str::to_string),
            email: email.to_string(),
        }
    }

    /// C3 in the 2026-11-14 audit: a saved draft could not be opened again anywhere. What it is
    /// opened from, when this machine has no copy of its own, is its source — so everything the
    /// draft held has to come back out of it: recipients, the blind ones too, the body, the
    /// threading and the files.
    #[test]
    fn a_draft_reopens_with_everything_it_was_saved_with() {
        let draft = Draft {
            from: address(Some("Me"), "me@halcyon.test"),
            to: vec![address(Some("Ada Lovelace"), "ada@example.test")],
            cc: vec![address(None, "grace@example.test")],
            bcc: vec![address(None, "secret@example.test")],
            subject: "Quarterly figures".into(),
            html: "<p>The numbers are <b>attached</b>.</p>".into(),
            text: None,
            references: vec!["<root@example.test>".into(), "<parent@example.test>".into()],
            in_reply_to: Some("<parent@example.test>".into()),
            attachments: vec![crate::mail::outgoing::Attachment {
                filename: "figures.csv".into(),
                mime: "text/csv".into(),
                bytes: b"q1,q2\r\n10,20\r\n".to_vec(),
                content_id: None,
            }],
            message_id: Some("<draft-1@halcyon.test>".into()),
        };

        let built = crate::mail::outgoing::build_draft(&draft).expect("built");
        let folder = tempfile::tempdir().expect("folder");

        let loaded = draft_from_source(&built.bytes, 1, None, folder.path()).expect("loaded");

        assert_eq!(loaded.message_id, "<draft-1@halcyon.test>");
        assert_eq!(loaded.to.len(), 1);
        assert_eq!(loaded.to[0].email, "ada@example.test");
        assert_eq!(loaded.to[0].name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(loaded.cc[0].email, "grace@example.test");
        assert_eq!(loaded.bcc.len(), 1, "the blind copy was lost");
        assert_eq!(loaded.bcc[0].email, "secret@example.test");
        assert_eq!(loaded.subject, "Quarterly figures");
        assert!(loaded.html.contains("attached"), "{}", loaded.html);
        assert_eq!(loaded.in_reply_to.as_deref(), Some("parent@example.test"));
        assert_eq!(loaded.references.len(), 2);

        assert_eq!(loaded.attachments.len(), 1, "the file was lost");
        let file = &loaded.attachments[0];
        assert_eq!(file.filename, "figures.csv");
        assert_eq!(
            std::fs::read(&file.path).expect("written"),
            b"q1,q2\r\n10,20\r\n"
        );
    }

    #[test]
    fn a_draft_written_as_plain_text_reopens_as_paragraphs() {
        let raw = b"From: me@halcyon.test\r\nMessage-ID: <p@x>\r\nSubject: s\r\n\r\nFirst line\r\n<not a tag>\r\n\r\nSecond";
        let folder = tempfile::tempdir().expect("folder");

        let loaded = draft_from_source(raw, 1, None, folder.path()).expect("loaded");

        assert_eq!(
            loaded.html,
            "<p>First line<br>&lt;not a tag&gt;</p><p>Second</p>"
        );
        assert_eq!(loaded.message_id, "<p@x>");
    }

    #[test]
    fn this_machines_own_copy_is_preferred_and_keeps_its_files() {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open");
        crate::db::migrate::run(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'T', 'me@t.test', 'other', 'password', 'halcyon:me')",
            [],
        )
        .expect("account");

        let dir = tempfile::tempdir().expect("dir");
        let kept = dir.path().join("plan.pdf");
        std::fs::write(&kept, b"pdf").expect("file");
        let gone = dir.path().join("moved-away.txt");

        conn.execute(
            "INSERT INTO draft (account_id, message_id, to_json, bcc_json, subject, html,
                                references_, attachments_json, updated_at)
             VALUES (1, '<d@x>', '[{\"name\":null,\"email\":\"ada@example.test\"}]',
                     '[{\"name\":null,\"email\":\"b@example.test\"}]', 'Plan', '<p>hi</p>',
                     '<a@x> <b@x>', ?1, 0)",
            rusqlite::params![serde_json::to_string(&[
                kept.to_string_lossy(),
                gone.to_string_lossy()
            ])
            .expect("json")],
        )
        .expect("draft");

        let loaded = local_draft(&conn, "<d@x>").expect("found");

        assert_eq!(loaded.to[0].email, "ada@example.test");
        assert_eq!(loaded.bcc[0].email, "b@example.test");
        assert_eq!(loaded.references, vec!["<a@x>", "<b@x>"]);
        assert_eq!(
            loaded
                .attachments
                .iter()
                .map(|f| f.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["plan.pdf"],
            "a file that has gone is left off rather than failing the open"
        );
    }
}
