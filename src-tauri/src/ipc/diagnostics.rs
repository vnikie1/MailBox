//! Crash reports and logs, for the Advanced settings pane. docs/06 Phase 11.
//!
//! Read, reveal, and delete. There is no send: see the note at the top of `crate::diagnostics`
//! for why an upload with no agreed destination would be worse than none, and standing rule 16
//! for why nothing here may become telemetry.

use crate::diagnostics::{self, CrashReport};

use super::mail::AppError;

fn dir() -> std::path::PathBuf {
    diagnostics::directory(
        crate::db::default_path()
            .parent()
            .unwrap_or(std::path::Path::new(".")),
    )
}

/// Every crash report on this machine, newest first.
#[tauri::command]
pub async fn crash_reports() -> Result<Vec<CrashReport>, AppError> {
    Ok(diagnostics::reports(&dir()))
}

/// The full text of one report.
#[tauri::command]
pub async fn crash_report_read(name: String) -> Result<String, AppError> {
    // The name comes from the list above, but it arrives back over IPC and is used to build a
    // path, so it is checked rather than trusted. A name containing a separator or `..` would
    // read any file on the disk — the classic traversal, and the WebView is the place it would
    // come from if a message body ever managed to reach this command.
    if name.contains(['/', '\\']) || name.contains("..") || !name.starts_with("crash-") {
        return Err(AppError {
            code: "bad-name".into(),
            message: "That is not a crash report.".into(),
        });
    }

    std::fs::read_to_string(dir().join(&name)).map_err(|error| AppError {
        code: "read-failed".into(),
        message: error.to_string(),
    })
}

/// Removes every report. The logs are left: they rotate on their own.
#[tauri::command]
pub async fn crash_reports_clear() -> Result<usize, AppError> {
    let directory = dir();
    let mut removed = 0usize;

    for report in diagnostics::reports(&directory) {
        if std::fs::remove_file(directory.join(&report.name)).is_ok() {
            removed += 1;
        }
    }

    Ok(removed)
}

/// Opens the diagnostics folder in Explorer.
///
/// The way a report gets to somebody who can read it. Nothing is attached, uploaded or sent —
/// the user picks the file up themselves and decides who sees it.
#[tauri::command]
pub async fn diagnostics_reveal() -> Result<(), AppError> {
    let directory = dir();

    std::fs::create_dir_all(&directory).map_err(|error| AppError {
        code: "no-folder".into(),
        message: error.to_string(),
    })?;

    // Explorer with a plain directory argument. Deliberately not `ShellExecute` on an arbitrary
    // string: this path is ours, but the habit of handing user-adjacent text to the shell is how
    // a command injection arrives later.
    std::process::Command::new("explorer.exe")
        .arg(&directory)
        .spawn()
        .map_err(|error| AppError {
            code: "open-failed".into(),
            message: error.to_string(),
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_traversing_name_is_refused() {
        // The report name is used to build a path. Without this check, a name of
        // `crash-../../../../Windows/System32/drivers/etc/hosts` reads whatever it likes.
        for name in [
            "crash-../secret.txt",
            "crash-..\\secret.txt",
            "../crash-x.txt",
            "crash-a/b.txt",
        ] {
            let refused = crash_report_read(name.into()).await;
            assert!(refused.is_err(), "{name} was not refused");
        }
    }

    #[tokio::test]
    async fn a_name_that_is_not_a_report_is_refused() {
        // The prefix is the allow-list. `halcyon.log` lives in the same folder and is not a
        // crash report; nor is anything else somebody might name.
        for name in ["halcyon.log", "halcyon.db", "", "notes.txt"] {
            assert!(crash_report_read(name.into()).await.is_err(), "{name}");
        }
    }

    #[tokio::test]
    async fn a_well_formed_name_gets_past_the_check() {
        // The guard must not be so strict that it refuses the real thing. This one gets past
        // validation and fails on the read, which is the correct place for a missing file.
        let error = crash_report_read("crash-000000000001.txt".into())
            .await
            .expect_err("no such file");

        assert_eq!(error.code, "read-failed");
    }
}
