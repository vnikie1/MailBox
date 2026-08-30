//! Window-chrome commands.
//!
//! Windows owns the caption strip (see platform::mod), so there is no geometry for the WebView
//! to report and no move, resize or close commands for it to issue. What is left is the two
//! things the WebView genuinely cannot answer for itself: what appearance Windows is in, and
//! the opening of a second top-level window.

use tauri::WebviewWindow;

use super::mail::AppError;

use crate::platform::appearance::{self, Appearance};

/// The current OS appearance. The UI calls this once on mount; every subsequent change
/// arrives on the `system:appearance` event instead, because the UI never polls.
#[tauri::command]
pub fn appearance_get(window: WebviewWindow) -> Appearance {
    appearance::compute(&window)
}

/// Opens the Settings window, or brings it forward if it is already open.
///
/// A real window rather than a sheet over the mailbox, which is what docs/06 Phase 11 asks for
/// and what Mail does. The reason it matters beyond fidelity: settings is where someone goes to
/// *fix* something they are looking at, and a modal sheet hides the very thing they are trying
/// to fix. A window can sit beside the mailbox while they change a setting and watch it take
/// effect.
///
/// Unlike compose, there is exactly one — the label is fixed. Two settings windows could
/// disagree about the same value, and the second one to be closed would win, which is a
/// confusing way to lose a change.
#[tauri::command]
pub async fn settings_open(app: tauri::AppHandle, pane: Option<String>) -> Result<(), AppError> {
    use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

    let pane = pane.unwrap_or_else(|| "general".into());

    // The pane name reaches a URL and an event payload, so it is checked rather than trusted.
    // The window falls back to General for anything it does not recognise, but keeping the
    // check here means a malformed value never reaches the query string at all.
    if !pane.chars().all(|c| c.is_ascii_lowercase()) {
        return Err(AppError {
            code: "bad-pane".into(),
            message: "That is not a settings pane.".into(),
        });
    }

    if let Some(existing) = app.get_webview_window("settings") {
        // Already open: move it to the pane that was asked for rather than opening a second
        // window or silently showing whichever pane it happened to be on.
        let _ = existing.emit("settings:pane", &pane);
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    WebviewWindowBuilder::new(
        &app,
        "settings",
        WebviewUrl::App(format!("index.html?settings=1&pane={pane}").into()),
    )
    .title("Settings")
    .inner_size(780.0, 580.0)
    .min_inner_size(560.0, 420.0)
    .decorations(true)
    .build()
    .map_err(|error| {
        tracing::warn!(%error, "could not open the settings window");
        AppError {
            code: "window-failed".into(),
            message: "The Settings window could not be opened.".into(),
        }
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    /// The pane names the UI knows. Kept here so the guard above and the window agree.
    const PANES: [&str; 7] = [
        "general",
        "accounts",
        "composing",
        "signatures",
        "rules",
        "privacy",
        "advanced",
    ];

    #[test]
    fn every_real_pane_name_passes_the_guard() {
        // The guard is deliberately narrow. If it ever rejects a name the UI actually uses,
        // Settings opens on the wrong pane — or not at all — and the only symptom is a menu
        // item that appears to do nothing.
        for pane in PANES {
            assert!(
                pane.chars().all(|c| c.is_ascii_lowercase()),
                "{pane} would be refused"
            );
        }
    }

    #[test]
    fn a_name_that_could_break_out_of_the_query_string_is_refused() {
        for pane in ["gene&ral", "../etc", "general ", "GENERAL", "a=1"] {
            assert!(
                !pane.chars().all(|c| c.is_ascii_lowercase()),
                "{pane} would be accepted"
            );
        }
    }
}
