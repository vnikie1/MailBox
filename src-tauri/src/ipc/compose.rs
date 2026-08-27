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

    Ok(ReplyDraft {
        account_id: source.account_id,
        to: recipients.to.iter().map(ComposeAddress::from).collect(),
        cc: recipients.cc.iter().map(ComposeAddress::from).collect(),
        subject: reply::subject(&source.envelope.subject, kind),
        quoted_html: source.quoted_html,
        in_reply_to: source.envelope.message_id.clone(),
        references: reply::references(&source.references, source.envelope.message_id.as_deref()),
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
