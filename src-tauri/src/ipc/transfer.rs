//! Import and export, over IPC. docs/06 Phase 11.
//!
//! Both are long. A Thunderbird profile of fifteen years is tens of thousands of messages, and
//! an import that returns nothing until it finishes looks identical to one that has hung. So
//! the work runs on a blocking thread and reports on `transfer:progress`, and the commands
//! themselves return as soon as the work is scheduled.
//!
//! **The writes are batched per folder rather than per message, and per import for nothing.**
//! One transaction around fifty thousand inserts holds the writer for minutes and cannot be
//! interrupted; one per message turns an import into fifty thousand fsyncs. Per folder is the
//! unit the user thinks in, and it is the unit progress is reported in.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use ts_rs::TS;

use crate::db::Db;
use crate::transfer::{export, import, mbox, thunderbird};

use super::mail::AppError;

/// A Thunderbird profile found on this machine, with the folders in it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ImportSource {
    pub name: String,
    /// Absolute path to the profile directory.
    pub root: String,
    pub folders: Vec<ImportFolder>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ImportFolder {
    /// Display path, `/`-joined.
    pub path: String,
    /// Absolute path to the mbox file.
    pub file: String,
    #[ts(type = "number")]
    pub bytes: u64,
}

/// Progress, as the UI shows it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    /// What is being worked on now — a folder name, or a mailbox.
    pub label: String,
    pub done: usize,
    pub total: usize,
    pub messages: usize,
    /// Set once, at the end. Absent while the work is running.
    pub finished: Option<TransferResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TransferResult {
    pub folders: usize,
    pub messages: usize,
    /// Items that could not be read: oversized, or not mail at all.
    pub skipped: usize,
    /// Where an export put things. Empty for an import.
    pub files: Vec<String>,
    /// Present when the whole run failed rather than an item within it.
    pub error: Option<String>,
}

/// Every Thunderbird profile on this machine, with its folders.
///
/// Returns an empty list rather than an error when Thunderbird is not installed: "we found
/// nothing" is the honest answer and it is not a failure.
#[tauri::command]
pub async fn import_sources() -> Result<Vec<ImportSource>, AppError> {
    let found = tauri::async_runtime::spawn_blocking(|| {
        thunderbird::profiles()
            .into_iter()
            .map(|profile| {
                let folders = thunderbird::folders(&profile.root)
                    .into_iter()
                    .map(|folder| ImportFolder {
                        path: folder.path,
                        file: folder.file.to_string_lossy().to_string(),
                        bytes: folder.bytes,
                    })
                    .collect();

                ImportSource {
                    name: profile.name,
                    root: profile.root.to_string_lossy().to_string(),
                    folders,
                }
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|error| AppError {
        code: "scan-failed".into(),
        message: error.to_string(),
    })?;

    Ok(found)
}

/// Asks the user for mbox files to import. Empty when cancelled.
#[tauri::command]
pub async fn import_pick_files() -> Result<Vec<String>, AppError> {
    let picked = tauri::async_runtime::spawn_blocking(crate::platform::files::open_files_dialog)
        .await
        .unwrap_or_default();

    Ok(picked
        .into_iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect())
}

/// Asks the user where an export should go. `None` when cancelled.
#[tauri::command]
pub async fn export_pick_folder() -> Result<Option<String>, AppError> {
    let picked = tauri::async_runtime::spawn_blocking(|| {
        crate::platform::files::pick_folder_dialog("Choose where to save the exported mail")
    })
    .await
    .unwrap_or(None);

    Ok(picked.map(|path| path.to_string_lossy().to_string()))
}

/// One thing to import: a display path and the mbox file behind it.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    /// Where it lands, `/`-joined. For a loose mbox this is just the file's name.
    pub path: String,
    pub file: String,
}

fn report(app: &AppHandle, progress: TransferProgress) {
    let _ = app.emit("transfer:progress", progress);
}

/// Imports mbox files into the local account.
///
/// Returns once the work is scheduled. Everything after that arrives on `transfer:progress`,
/// which the UI is already listening to — the same shape as sync, and for the same reason.
#[tauri::command]
pub async fn import_run(
    app: AppHandle,
    db: State<'_, Db>,
    requests: Vec<ImportRequest>,
) -> Result<(), AppError> {
    let db = db.inner().clone();
    let cache_root = crate::db::default_path()
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();

    tauri::async_runtime::spawn(async move {
        let total = requests.len();
        let mut messages = 0usize;
        let mut skipped = 0usize;
        let mut folders = 0usize;
        let mut failure: Option<String> = None;

        for (index, request) in requests.iter().enumerate() {
            report(
                &app,
                TransferProgress {
                    label: request.path.clone(),
                    done: index,
                    total,
                    messages,
                    finished: None,
                },
            );

            // Kept for the log line below: the closure takes ownership of the request, and a
            // folder that fails is the one case where its name is worth having.
            let label = request.path.clone();
            let request = request.clone();
            let cache_root = cache_root.clone();

            // A `.pst` is a different shape of work: it holds a whole folder tree rather than
            // one mailbox, and its reader is not `Send`, so it cannot run inside the writer's
            // closure. It gets its own path.
            if is_pst(&request.file) {
                match import_pst(&db, &cache_root, &request.file).await {
                    Ok((written, unreadable)) => {
                        messages += written;
                        skipped += unreadable;
                        folders += 1;
                    }
                    Err(error) => {
                        tracing::warn!(file = %request.file, %error, "a .pst could not be imported");
                        skipped += 1;
                        failure.get_or_insert_with(|| error.to_string());
                    }
                }
                continue;
            }

            // One transaction per folder. See the module note.
            let outcome = db
                .write(move |tx| {
                    let account = import::local_account(tx, "On My PC")?;
                    let mailbox = import::mailbox_for(tx, account, &request.path)?;
                    let mut uid = import::next_uid(tx, mailbox)?;

                    let file =
                        std::fs::File::open(&request.file).map_err(crate::db::DbError::from)?;
                    let reader = std::io::BufReader::new(file);

                    let mut written = 0usize;

                    let counts = mbox::read(reader, |raw| {
                        match import::write_message(
                            tx,
                            &cache_root,
                            account,
                            mailbox,
                            uid,
                            raw,
                            None,
                        ) {
                            Ok(Some(_)) => {
                                written += 1;
                                uid += 1;
                            }
                            Ok(None) => {}
                            Err(error) => {
                                return Err(std::io::Error::other(error.to_string()));
                            }
                        }
                        Ok(())
                    })
                    .map_err(crate::db::DbError::from)?;

                    import::finish(tx, account, &[mailbox])?;

                    Ok((written, counts.oversized))
                })
                .await;

            match outcome {
                Ok((written, oversized)) => {
                    messages += written;
                    skipped += oversized;
                    folders += 1;
                }
                Err(error) => {
                    // One folder failing does not end the import — a single unreadable file in a
                    // profile of two hundred should not cost the other hundred and ninety-nine.
                    tracing::warn!(folder = %label, %error, "a folder could not be imported");
                    skipped += 1;
                    failure.get_or_insert_with(|| error.to_string());
                }
            }
        }

        report(
            &app,
            TransferProgress {
                label: String::new(),
                done: total,
                total,
                messages,
                finished: Some(TransferResult {
                    folders,
                    messages,
                    skipped,
                    files: Vec::new(),
                    error: failure,
                }),
            },
        );

        // The sidebar and the lists are stale now: an account and its mailboxes appeared.
        let _ = app.emit("accounts:changed", ());
        let _ = app.emit("mailboxes:changed", ());
    });

    Ok(())
}

/// Exports mailboxes to a directory.
#[tauri::command]
pub async fn export_run(
    app: AppHandle,
    db: State<'_, Db>,
    mailbox_ids: Vec<i64>,
    format: String,
    directory: String,
) -> Result<(), AppError> {
    let db = db.inner().clone();
    let as_tree = format == "eml";

    tauri::async_runtime::spawn(async move {
        let total = mailbox_ids.len();
        let mut result = export::Exported::default();
        let mut failure: Option<String> = None;

        for (index, mailbox_id) in mailbox_ids.iter().enumerate() {
            let mailbox_id = *mailbox_id;
            let directory = std::path::PathBuf::from(&directory);

            report(
                &app,
                TransferProgress {
                    label: String::new(),
                    done: index,
                    total,
                    messages: result.messages,
                    finished: None,
                },
            );

            // A read rather than a write: exporting must never block the writer, and a large
            // export would otherwise hold it for the whole run while mail was arriving.
            let one = db
                .read(move |conn| {
                    let mut counts = export::Exported::default();
                    if as_tree {
                        export::to_eml_tree(conn, mailbox_id, &directory, &mut counts)?;
                    } else {
                        export::to_mbox(conn, mailbox_id, &directory, &mut counts)?;
                    }
                    Ok(counts)
                })
                .await;

            match one {
                Ok(counts) => {
                    result.mailboxes += counts.mailboxes;
                    result.messages += counts.messages;
                    result.without_body += counts.without_body;
                    result.files.extend(counts.files);
                }
                Err(error) => {
                    tracing::warn!(mailbox_id, %error, "a mailbox could not be exported");
                    failure.get_or_insert_with(|| error.to_string());
                }
            }
        }

        report(
            &app,
            TransferProgress {
                label: String::new(),
                done: total,
                total,
                messages: result.messages,
                finished: Some(TransferResult {
                    folders: result.mailboxes,
                    messages: result.messages,
                    skipped: result.without_body,
                    files: result.files,
                    error: failure,
                }),
            },
        );
    });

    Ok(())
}

/// True for a file that should be read as an Outlook store rather than as mbox.
fn is_pst(file: &str) -> bool {
    std::path::Path::new(file)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pst"))
}

/// How many messages are held before being written.
///
/// The reader produces them faster than SQLite takes them, and one transaction per message
/// would turn an archive of fifty thousand into fifty thousand commits. Two hundred is small
/// enough that a failure loses little and large enough that the commit cost disappears.
const PST_BATCH: usize = 200;

/// Imports one `.pst`.
///
/// ## Why a channel
///
/// `outlook-pst` hands back `Rc`-shaped objects, which are deliberately not `Send`: the store,
/// its folders and its messages all share one file handle and one cache. The database writer is
/// on its own thread and takes a `Send` closure, so the two cannot be in the same scope.
///
/// So the reader runs on a blocking thread and pushes finished messages — plain owned bytes,
/// which *are* `Send` — down a bounded channel. The bound matters: an unbounded one would let a
/// ten-gigabyte archive be read into memory faster than it could be written, which is the same
/// out-of-memory failure the mbox reader streams to avoid.
async fn import_pst(
    db: &Db,
    cache_root: &std::path::Path,
    file: &str,
) -> Result<(usize, usize), String> {
    use crate::transfer::pst;

    let (sender, mut receiver) = tokio::sync::mpsc::channel::<pst::Extracted>(PST_BATCH * 2);

    let path = std::path::PathBuf::from(file);
    let reader = tauri::async_runtime::spawn_blocking(move || {
        pst::read(&path, |message| {
            // The receiver going away means the import was abandoned; stopping is correct.
            sender
                .blocking_send(message)
                .map_err(|_| std::io::Error::other("the import was stopped"))
        })
    });

    let mut written = 0usize;
    let mut batch: Vec<pst::Extracted> = Vec::with_capacity(PST_BATCH);

    loop {
        let got = receiver.recv().await;
        let finished = got.is_none();

        if let Some(message) = got {
            batch.push(message);
        }

        if batch.len() >= PST_BATCH || (finished && !batch.is_empty()) {
            let work = std::mem::take(&mut batch);
            let cache_root = cache_root.to_path_buf();

            written += db
                .write(move |tx| {
                    let account = import::local_account(tx, "On My PC")?;
                    let mut touched: Vec<i64> = Vec::new();
                    let mut count = 0usize;

                    for message in work {
                        let mailbox = import::mailbox_for(tx, account, &message.path)?;
                        let uid = import::next_uid(tx, mailbox)?;

                        // Read state comes from a MAPI property rather than from the headers,
                        // so it is handed over explicitly — the synthesised message has no
                        // header that carries it.
                        if import::write_message(
                            tx,
                            &cache_root,
                            account,
                            mailbox,
                            uid,
                            &message.raw,
                            Some(message.seen),
                        )?
                        .is_some()
                        {
                            count += 1;
                        }

                        if !touched.contains(&mailbox) {
                            touched.push(mailbox);
                        }
                    }

                    import::finish(tx, account, &touched)?;
                    Ok(count)
                })
                .await
                .map_err(|error| error.to_string())?;
        }

        if finished {
            break;
        }
    }

    let counts = reader
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;

    // Everything the reader could not do, reported as skipped rather than quietly dropped. An
    // archive that imports looking complete and is not is the one failure this must never have.
    Ok((written, counts.failed + counts.rtf_only))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_machine_without_thunderbird_reports_nothing_rather_than_failing() {
        // "We found nothing" is the honest answer to "what can I import", and it is not an
        // error. An error here would put a red banner in front of somebody whose only mistake
        // was not using Thunderbird.
        let found = import_sources().await;
        assert!(found.is_ok());
    }
}
