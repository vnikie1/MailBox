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
    })
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

    // Told to the main window immediately, not when the sender picks it up. The compose window
    // is a *different* window, so without this the Undo Send banner would not appear until the
    // hold had already expired — which is the entire period it exists to cover.
    {
        use crate::sync::events::{payload, Events};
        let events: &dyn Events = &app;
        events.emit(
            "outbox:progress",
            payload(&crate::sync::sender::Progress {
                id,
                account_id: message.account_id,
                state: "holding".to_string(),
                error: None,
            }),
        );
    }

    // Wake the sender when the hold expires, rather than leaving it to the idle tick. Undo
    // Send's timer should decide when a message goes, not a polling interval.
    let sender = sender.inner().clone();
    let hold = hold_seconds(db.inner()).await;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(hold.max(0) as u64)).await;
        sender.poke();
    });

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
) -> Result<String, AppError> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    // Monotonic per process. A label that collided would return the existing window instead of
    // opening a second draft, which is a confusing way to lose what someone just typed.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let label = format!(
        "compose-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );

    let mut url = String::from("index.html?compose=1");
    if let Some(id) = message_id {
        url.push_str(&format!("&message={id}"));
    }
    if let Some(kind) = kind.as_deref() {
        url.push_str(&format!("&kind={kind}"));
    }

    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
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
pub async fn outbox_schedule(db: State<'_, Db>, id: i64, send_at: i64) -> Result<bool, AppError> {
    Ok(db
        .write(move |tx| outbox::reschedule(tx, id, send_at))
        .await?)
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
#[derive(Debug, Clone, Serialize, TS)]
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

    Ok(chosen
        .into_iter()
        .map(|path| {
            let size = std::fs::metadata(&path)
                .map(|meta| meta.len() as i64)
                .unwrap_or(0);
            let filename = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "attachment".to_string());

            PickedFile {
                path: path.to_string_lossy().to_string(),
                filename,
                size,
            }
        })
        .collect())
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
/// intent across a lost connection.
///
/// `message_id` is stable across saves. It is what lets the tenth save recognise the ninth as
/// the same draft, both here and on the server.
#[tauri::command]
pub async fn compose_save_draft(
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

    let from = Address {
        name: Some(account.display_name.clone()),
        email: account.email.clone(),
    };

    // A draft with no recipients is the normal case — people type the body first. The builder
    // refuses those, so bytes are only produced once there is somewhere to send it; until then
    // the draft lives locally and is restored from the columns below.
    let has_recipients =
        !message.to.is_empty() || !message.cc.is_empty() || !message.bcc.is_empty();

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
        attachments: Vec::new(),
        message_id: message_id.clone(),
    };

    let built = if has_recipients {
        outgoing::build(&draft).ok()
    } else {
        None
    };

    let identity = built
        .as_ref()
        .map(|b| b.message_id.clone())
        .or(message_id)
        .unwrap_or_else(|| format!("<draft-{}@halcyon.invalid>", now_seconds()));

    let account_id = message.account_id;

    let stored = {
        let identity = identity.clone();
        let to = serde_json::to_string(&message.to).unwrap_or_else(|_| "[]".into());
        let cc = serde_json::to_string(&message.cc).unwrap_or_else(|_| "[]".into());
        let bcc = serde_json::to_string(&message.bcc).unwrap_or_else(|_| "[]".into());
        let subject = message.subject.clone();
        let html = message.html.clone();
        let text = message.text.clone().unwrap_or_default();
        let in_reply_to = message.in_reply_to.clone();
        let references = message.references.join(" ");

        db.write(move |tx| {
            tx.execute(
                "INSERT INTO draft (
                     account_id, message_id, to_json, cc_json, bcc_json,
                     subject, html, text, in_reply_to, references_, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(message_id) DO UPDATE SET
                     to_json = excluded.to_json,
                     cc_json = excluded.cc_json,
                     bcc_json = excluded.bcc_json,
                     subject = excluded.subject,
                     html = excluded.html,
                     text = excluded.text,
                     in_reply_to = excluded.in_reply_to,
                     references_ = excluded.references_,
                     updated_at = excluded.updated_at",
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
                    now_seconds()
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

    // The server copy, queued. Only once the message can actually be built: a draft with no
    // recipients has no valid MIME to append.
    if let Some(built) = built {
        let root = crate::db::default_path()
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();

        let path = root.join("drafts").join(format!("{stored}.eml"));
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        if std::fs::write(&path, &built.bytes).is_ok() {
            let mailbox = db
                .read(move |conn| {
                    Ok(conn
                        .query_row(
                            "SELECT remote_path FROM mailbox
                              WHERE account_id = ?1 AND role = 'drafts'",
                            rusqlite::params![account_id],
                            |row| row.get::<_, String>(0),
                        )
                        .ok())
                })
                .await?
                .unwrap_or_else(|| "Drafts".to_string());

            let replaces: Option<u32> = db
                .read(move |conn| {
                    Ok(conn
                        .query_row(
                            "SELECT remote_uid FROM draft WHERE id = ?1",
                            rusqlite::params![stored],
                            |row| row.get::<_, Option<i64>>(0),
                        )
                        .ok()
                        .flatten()
                        .map(|uid| uid as u32))
                })
                .await?;

            let eml_path = path.to_string_lossy().to_string();
            // The draft's stable identity, so the drain can tell our own copy on the server
            // from one another device wrote.
            let message_id_for_op = built.message_id.clone();
            db.write(move |tx| {
                crate::sync::ops::enqueue(
                    tx,
                    account_id,
                    &crate::sync::ops::Op::AppendDraft {
                        mailbox,
                        eml_path,
                        replaces,
                        message_id: message_id_for_op,
                    },
                )
            })
            .await?;
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

/// Removes a draft once its message has been sent or thrown away.
#[tauri::command]
pub async fn compose_discard_draft(db: State<'_, Db>, id: i64) -> Result<(), AppError> {
    db.write(move |tx| {
        tx.execute("DELETE FROM draft WHERE id = ?1", rusqlite::params![id])?;
        Ok(())
    })
    .await?;

    Ok(())
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
