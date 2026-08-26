//! Getting an attachment out of a message. docs/04 Phase 6.
//!
//! The bytes are never stored separately — they live in the cached `.eml` and are decoded on
//! demand. One copy of a 20MB attachment is enough, and a second copy on disk would be a
//! second thing to keep in step and a second thing to forget to delete.
//!
//! **There is deliberately no "open with the default application".** Handing an attachment to
//! whatever the shell associates with its extension is the single most reliable way malware
//! has ever spread through mail, and an "Open" button next to a file called `invoice.pdf.exe`
//! is a loaded gun with a friendly label. docs/04 Phase 6 asks for a *built-in previewer* for
//! images, PDFs and text, and `preview` below is what serves it: bytes into the sandboxed
//! frame, rendered by the WebView, never executed. Anything the previewer cannot show, the
//! user can save and open themselves — deliberately, with the shell's own warnings intact.

use std::path::PathBuf;

use base64::Engine;
use tauri::State;

use crate::db::Db;

use super::mail::AppError;

/// Ceiling on what the built-in previewer will inline.
///
/// The preview crosses the IPC boundary as base64, which costs a third again in size and is
/// held in memory on both sides. A 25MB video is a save, not a preview.
const MAX_PREVIEW_BYTES: usize = 16 * 1024 * 1024;

/// What the previewer can render itself.
///
/// An allow-list, and a short one. Everything here is a format the WebView renders inertly
/// inside the sandboxed frame; anything else is a file that has to leave the app before it
/// means anything, and leaving the app is the user's decision to make.
fn previewable(mime: &str) -> bool {
    let mime = mime.to_ascii_lowercase();

    mime.starts_with("image/")
        || mime.starts_with("text/")
        || mime == "application/pdf"
        || mime == "application/json"
}

/// One attachment's bytes, ready for the previewer.
#[derive(Debug, serde::Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentData {
    pub filename: String,
    pub mime: String,
    /// A `data:` URI. Only ever a type from [`previewable`], so the frame's CSP can allow it
    /// without allowing arbitrary content.
    pub data_url: String,
}

/// Where an attachment lives: which message's cache, and which part of it.
struct Located {
    raw_path: Option<String>,
    part_id: Option<String>,
    filename: Option<String>,
    mime: Option<String>,
}

fn locate(db_row: Located) -> Result<(PathBuf, String, String, String), AppError> {
    let raw_path = db_row.raw_path.ok_or_else(|| AppError {
        code: "not-downloaded".into(),
        message: "This message has not been downloaded yet.".into(),
    })?;

    let part_id = db_row.part_id.ok_or_else(|| AppError {
        code: "no-part".into(),
        message: "This attachment could not be located in the message.".into(),
    })?;

    Ok((
        PathBuf::from(raw_path),
        part_id,
        db_row.filename.unwrap_or_else(|| "attachment".to_string()),
        db_row
            .mime
            .unwrap_or_else(|| "application/octet-stream".to_string()),
    ))
}

async fn read_row(db: &Db, attachment_id: i64) -> Result<Located, AppError> {
    let row = db
        .read(move |conn| {
            let row = conn
                .query_row(
                    "SELECT message.raw_path, attachment.part_id, attachment.filename,
                            attachment.mime
                       FROM attachment
                       JOIN message ON message.id = attachment.message_id
                      WHERE attachment.id = ?1",
                    rusqlite::params![attachment_id],
                    |row| {
                        Ok(Located {
                            raw_path: row.get(0)?,
                            part_id: row.get(1)?,
                            filename: row.get(2)?,
                            mime: row.get(3)?,
                        })
                    },
                )
                .ok();

            Ok(row)
        })
        .await?;

    row.ok_or_else(|| AppError {
        code: "not-found".into(),
        message: "That attachment is no longer in the mailbox.".into(),
    })
}

/// Walks the parsed message to the part named by a dotted path like `1.2`.
///
/// The same numbering `sync::bodies` assigned when it recorded the attachment, so the two
/// agree by construction rather than by a stored offset that could drift.
fn part_at<'a>(
    root: &'a mailparse::ParsedMail<'a>,
    part_id: &str,
) -> Option<&'a mailparse::ParsedMail<'a>> {
    let mut current = root;

    for step in part_id.split('.') {
        let index: usize = step.parse().ok()?;
        // Recorded one-based, because a MIME part path is written that way everywhere else.
        current = current.subparts.get(index.checked_sub(1)?)?;
    }

    Some(current)
}

/// Decodes one attachment out of the cached `.eml`.
fn decode(path: &std::path::Path, part_id: &str) -> Result<Vec<u8>, AppError> {
    let raw = std::fs::read(path).map_err(|error| {
        tracing::warn!(%error, "attachment: cached message could not be read");
        AppError {
            code: "cache-missing".into(),
            message: "The downloaded copy of this message is no longer on disk.".into(),
        }
    })?;

    let parsed = mailparse::parse_mail(&raw).map_err(|error| {
        tracing::warn!(%error, "attachment: cached message could not be parsed");
        AppError {
            code: "unreadable".into(),
            message: "This message could not be read.".into(),
        }
    })?;

    let part = part_at(&parsed, part_id).ok_or_else(|| AppError {
        code: "no-part".into(),
        message: "This attachment is not in the downloaded copy of the message.".into(),
    })?;

    part.get_body_raw().map_err(|error| {
        tracing::warn!(%error, "attachment: part could not be decoded");
        AppError {
            code: "unreadable".into(),
            message: "This attachment could not be decoded.".into(),
        }
    })
}

/// Returns an attachment as a `data:` URI, for the built-in previewer.
///
/// Refuses anything outside [`previewable`] rather than handing back bytes the caller would
/// have to decide what to do with. The decision belongs here, where the reasoning is.
#[tauri::command]
pub async fn attachment_preview(
    db: State<'_, Db>,
    attachment_id: i64,
) -> Result<AttachmentData, AppError> {
    let row = read_row(db.inner(), attachment_id).await?;
    let (path, part_id, filename, mime) = locate(row)?;

    if !previewable(&mime) {
        return Err(AppError {
            code: "not-previewable".into(),
            message: "This kind of file cannot be shown here. Save it to open it.".into(),
        });
    }

    let bytes = tokio::task::spawn_blocking(move || decode(&path, &part_id))
        .await
        .map_err(|_| AppError {
            code: "cancelled".into(),
            message: "Reading the attachment was interrupted.".into(),
        })??;

    if bytes.len() > MAX_PREVIEW_BYTES {
        return Err(AppError {
            code: "too-large".into(),
            message: "This file is too large to preview. Save it to open it.".into(),
        });
    }

    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);

    Ok(AttachmentData {
        filename,
        mime: mime.clone(),
        data_url: format!("data:{mime};base64,{encoded}"),
    })
}

/// Saves an attachment, asking the user where.
///
/// Returns the path written, or `None` if the user cancelled — a cancel is an ordinary
/// outcome and not an error to report.
#[tauri::command]
pub async fn attachment_save(
    db: State<'_, Db>,
    attachment_id: i64,
) -> Result<Option<String>, AppError> {
    let row = read_row(db.inner(), attachment_id).await?;
    let (path, part_id, filename, _mime) = locate(row)?;

    let bytes = {
        let path = path.clone();
        let part_id = part_id.clone();
        tokio::task::spawn_blocking(move || decode(&path, &part_id))
            .await
            .map_err(|_| AppError {
                code: "cancelled".into(),
                message: "Reading the attachment was interrupted.".into(),
            })??
    };

    let suggested = crate::platform::files::safe_file_name(&filename);

    let chosen =
        tokio::task::spawn_blocking(move || crate::platform::files::save_file_dialog(&suggested))
            .await
            .map_err(|_| AppError {
                code: "cancelled".into(),
                message: "The save was interrupted.".into(),
            })?;

    let Some(destination) = chosen else {
        return Ok(None);
    };

    let written = destination.clone();
    tokio::task::spawn_blocking(move || std::fs::write(&written, &bytes))
        .await
        .map_err(|_| AppError {
            code: "cancelled".into(),
            message: "The save was interrupted.".into(),
        })?
        .map_err(|error| {
            tracing::warn!(%error, "attachment: could not be written");
            AppError {
                code: "write-failed".into(),
                message: "The file could not be saved to that location.".into(),
            }
        })?;

    Ok(Some(destination.to_string_lossy().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_formats_the_frame_can_render_inertly_are_previewable() {
        for mime in [
            "image/png",
            "IMAGE/JPEG",
            "text/plain",
            "application/pdf",
            "application/json",
        ] {
            assert!(previewable(mime), "{mime}");
        }

        // The dangerous half. An executable, a script, an installer and an Office document
        // with macros must never be handed to anything but a save dialog.
        for mime in [
            "application/x-msdownload",
            "application/vnd.microsoft.portable-executable",
            "application/x-msi",
            "application/vnd.ms-excel.sheet.macroEnabled.12",
            "application/zip",
            "application/octet-stream",
        ] {
            assert!(!previewable(mime), "{mime}");
        }
    }

    #[test]
    fn a_part_path_walks_the_tree_the_way_bodies_numbered_it() {
        // The two sides agree by construction, so a change to either numbering has to break
        // this test to reach production.
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=outer\r\n\r\n",
            "--outer\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "first\r\n",
            "--outer\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "second\r\n",
            "--outer--\r\n"
        );

        let parsed = mailparse::parse_mail(raw.as_bytes()).expect("parse");

        let second = part_at(&parsed, "2").expect("part 2");
        assert_eq!(second.get_body().expect("body").trim(), "second");

        // One-based: there is no part zero, and asking for one must not wrap to the last.
        assert!(part_at(&parsed, "0").is_none());
        assert!(part_at(&parsed, "9").is_none());
        assert!(part_at(&parsed, "not-a-number").is_none());
    }
}
