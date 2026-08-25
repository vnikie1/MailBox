//! Window-chrome commands.
//!
//! Phase 0 needs exactly one. Windows owns the caption strip (see platform::mod), so
//! there is no geometry for the WebView to report and no window commands for it to
//! issue. The rest of the IPC contract (docs/03-architecture.md §4) arrives with the
//! phase that implements it.

use tauri::WebviewWindow;

use crate::platform::appearance::{self, Appearance};

/// The current OS appearance. The UI calls this once on mount; every subsequent change
/// arrives on the `system:appearance` event instead, because the UI never polls.
#[tauri::command]
pub fn appearance_get(window: WebviewWindow) -> Appearance {
    appearance::compute(&window)
}
