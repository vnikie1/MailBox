//! Window-chrome commands.
//!
//! Windows owns the caption strip (see platform::mod), so there is no geometry for the WebView
//! to report and no move, resize or close commands for it to issue. What is left is the two
//! things the WebView genuinely cannot answer for itself: what appearance Windows is in, and
//! the opening of a second top-level window.

use rusqlite::OptionalExtension;
use tauri::{State, WebviewWindow};

use super::mail::AppError;

use crate::db::Db;
use crate::platform::appearance::{self, Appearance};

/// The current OS appearance. The UI calls this once on mount; every subsequent change
/// arrives on the `system:appearance` event instead, because the UI never polls.
#[tauri::command]
pub fn appearance_get(window: WebviewWindow) -> Appearance {
    appearance::compute(&window)
}

/// The key display preferences are stored under.
///
/// One row holding the whole object rather than a row per field: it is written and read as a
/// unit, and a partial write is the one failure that would be worse than none.
const DISPLAY_PREFERENCES_KEY: &str = "display.preferences";

/// The user's display preferences, as stored. `None` before they have ever changed one.
///
/// ## Why these are in the database rather than in the WebView
///
/// They used to live only in `localStorage`, and on this machine that turned out not to be
/// storage at all. WebView2 keeps it in a LevelDB write-ahead log, and that log became corrupt:
/// LevelDB's own diagnostic says `dropping 3706 bytes; Corruption: checksum mismatch` on every
/// single launch, always at the same offset, and it discards everything written after that
/// point. Because the directory had never been compacted, each run appended past the damage
/// into the region the next run would throw away — so every preference the user set was
/// faithfully written and silently lost, for ever.
///
/// The visible symptom was oddly specific and worth recording: the *theme* came back and the
/// *accent* did not. The last record that survived recovery predated the accent feature, so it
/// carried a theme and no accent key at all, and a shallow merge left the accent at its
/// default — which resolves to the Windows accent, which on that machine is orange.
///
/// The database is the right place for this regardless: it is what the rest of the app's state
/// is already trusted to, it is backed up with everything else, and SQLite has real durability
/// guarantees where a webview's cache has none. `localStorage` is still written, but only as a
/// cache that lets the first frame paint before this can be read — see `store/settings.ts`.
#[tauri::command]
pub async fn display_preferences_get(db: State<'_, Db>) -> Result<Option<String>, AppError> {
    db.read(|conn| {
        Ok(conn
            .query_row(
                "SELECT value FROM setting WHERE key = ?1",
                rusqlite::params![DISPLAY_PREFERENCES_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()?)
    })
    .await
    .map_err(AppError::from)
}

/// Stores them. The value is opaque here: the shape belongs to the window that wrote it.
#[tauri::command]
pub async fn display_preferences_set(db: State<'_, Db>, value: String) -> Result<(), AppError> {
    db.write(move |tx| {
        tx.execute(
            "INSERT INTO setting (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![DISPLAY_PREFERENCES_KEY, value],
        )?;
        Ok(())
    })
    .await
    .map_err(AppError::from)
}

/// The colour the taskbar badge is drawn in, pushed from the UI.
///
/// The UI owns this because it is the only thing that knows the answer: the app draws with the
/// OS accent until the user pins one in Settings, and the pinned value lives in the WebView.
/// Resolving it again here would be a second copy of the same rule in a second language, and
/// the taskbar would show one colour while the window showed another.
///
/// Both values are `0x00RRGGBB`. The foreground comes down with the fill rather than being
/// worked out here, because it is decided by the same WCAG luminance function that gives every
/// button in the app its `--accent-fg`, and a badge that picked its own would drift from it.
#[tauri::command]
pub async fn badge_paint(app: tauri::AppHandle, fill: u32, ink: u32) -> Result<(), AppError> {
    // Logged because this is the one place the *resolved* accent crosses out of the WebView,
    // which makes it the only way to see from outside what colour the window actually chose.
    // A theme that silently falls back to the OS accent looks identical to one that was never
    // set, and this line is what tells the two apart.
    tracing::info!(
        fill = format!("#{fill:06X}"),
        ink = format!("#{ink:06X}"),
        "accent resolved"
    );
    crate::platform::badge::set_paint(fill, ink);
    crate::platform::tray::repaint(&app).await;
    Ok(())
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
///
/// `account` picks out one account on the Accounts pane — the mailbox menu's `Edit "Account"…`,
/// which used to open the pane and leave the user to find the account in it. An id cannot
/// break out of a query string, so it needs no check beyond being a number.
#[tauri::command]
pub async fn settings_open(
    app: tauri::AppHandle,
    pane: Option<String>,
    account: Option<i64>,
) -> Result<(), AppError> {
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
        if let Some(account) = account {
            let _ = existing.emit("settings:account", account);
        }
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    let address = match account {
        Some(account) => format!("index.html?settings=1&pane={pane}&account={account}"),
        None => format!("index.html?settings=1&pane={pane}"),
    };

    WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App(address.into()))
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
