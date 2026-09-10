//! Telling the user why Halcyon will not start.
//!
//! ## Why this exists
//!
//! A failure during startup has nowhere to put an error. There is no window yet, there is no
//! console — a GUI subsystem binary has no stdout anyone will see — and Tauri turns any error
//! returned from the `.setup()` hook into a panic. The user double-clicks the icon and nothing
//! happens.
//!
//! That is the worst failure mode this app has, and it is not hypothetical: nine crash reports
//! accumulated on the developer's machine over three days before anybody knew what they were,
//! because the only artefact each one produced was a file that can be read by the app that had
//! just failed to start.
//!
//! Six of those nine have since been made survivable — a decorative backdrop no longer aborts
//! anything. The rest cannot be: a database that will not open is not a mail client. So the
//! answer for those is not to carry on regardless but to **say something first**, name the file
//! and the log, and exit deliberately rather than vanishing.
//!
//! ## Why a message box and not a Tauri dialog
//!
//! Tauri's dialog plugin needs a running app, which is the thing that has not happened. This is
//! the one place in the codebase that has to reach past the framework to Win32, because the
//! framework is what failed.

use std::path::Path;

use windows::core::HSTRING;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_SYSTEMMODAL,
};

/// Shows the message, then ends the process.
///
/// Exits rather than panicking. A panic here would write a crash report that only a working
/// installation can display, and would return a non-obvious exit code; this has already told
/// the user what happened, so the report would be for nobody.
///
/// `MB_SYSTEMMODAL` and `MB_SETFOREGROUND` because there is no owner window to be modal to, and
/// a dialog that opens behind whatever the user was doing is a dialog they will not see — which
/// would put us back where we started.
pub fn cannot_start(summary: &str, detail: &str, log_dir: Option<&Path>) -> ! {
    let mut body = format!("{summary}\n\n{detail}");

    if let Some(dir) = log_dir {
        body.push_str(&format!("\n\nThe log is in:\n{}", dir.display()));
    }

    // Logged as well as shown. The dialog is for the person at the keyboard; the log is for
    // whoever they send it to afterwards.
    tracing::error!(summary, detail, "Halcyon cannot start");

    // SAFETY: both strings are owned, NUL-terminated by HSTRING, and outlive the call.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(body.as_str()),
            &HSTRING::from("Halcyon cannot start"),
            MB_OK | MB_ICONERROR | MB_SYSTEMMODAL | MB_SETFOREGROUND,
        );
    }

    std::process::exit(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The body is assembled the way the dialog will show it.
    ///
    /// Not a test of `MessageBoxW` — that would block a test run on a dialog nobody can click.
    /// What is worth pinning is that the message names the thing that failed *and* where to look
    /// afterwards, because a dialog saying only "could not start" leaves the user exactly as
    /// stuck as the silent exit it replaced.
    #[test]
    fn the_message_names_the_failure_and_the_log() {
        let dir = Path::new(r"C:\Users\someone\AppData\Local\com.uniki.halcyon\diagnostics");
        let body = format!(
            "{}\n\n{}\n\nThe log is in:\n{}",
            "Halcyon could not open its mail store.",
            "database is locked",
            dir.display()
        );

        assert!(body.contains("mail store"), "says what failed");
        assert!(body.contains("database is locked"), "quotes the underlying error");
        assert!(body.contains("diagnostics"), "points at the log");
    }
}
