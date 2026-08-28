//! Windows platform integration. docs/03-architecture.md §8.
//!
//! ## Why the window is decorated
//!
//! The obvious way to reproduce macOS Mail's unified 52px titlebar is an undecorated
//! window with the caption buttons drawn in the page, and `WM_NCHITTEST` answering
//! `HTMAXBUTTON` over the maximise button so Windows 11 offers the Snap Layouts flyout.
//! That was built, and it does not work — not because of a mistake, but because of how
//! Tauri hosts its WebView. An undecorated Tauri window has this hierarchy:
//!
//! ```text
//! Tauri Window                     <- a subclass here only ever sees the border
//! ├─ TAURI_DRAG_RESIZE_BORDERS     <- covers the entire client area
//! ├─ WRY_WEBVIEW
//! ├─ Chrome_WidgetWin_0 / _1       <- owned by msedgewebview2.exe
//! └─ Chrome_RenderWidgetHostHWND   <- owned by msedgewebview2.exe
//! ```
//!
//! Every child spans the full client rect, so Windows routes `WM_NCHITTEST` to the
//! deepest child under the pointer. Instrumenting the top-level window confirmed this
//! precisely: hit tests arrive for the resize borders (`HTTOP`, `HTRIGHT`) and never once
//! for a point inside the client area. The usual escape — subclass the children and
//! return `HTTRANSPARENT` so the test falls through — cannot work either, because the
//! `Chrome_*` windows belong to the WebView2 process.
//!
//! So Windows keeps the caption strip. It draws and hit-tests the three buttons itself,
//! which means Snap Layouts, hover, press, `Alt`+`Space`, double-click-to-maximise and
//! screen-reader support are all native and cannot regress. `docs/02` §6.1 already
//! specified those buttons as native metrics with Segoe Fluent glyphs, so drawing them
//! ourselves was only ever an imitation of what Windows was going to draw anyway.
//!
//! The cost is real and is not hidden: the window now has a ~32px system caption above
//! our toolbar rather than one unified 52px bar. See docs/PHASE-0-VERIFICATION.md §2.

pub mod appearance;
pub mod backdrop;
pub mod badge;
pub mod files;
pub mod jumplist;
pub mod links;
pub mod notify;
pub mod sound;
pub mod toast;
pub mod tray;

use std::ffi::c_void;

use tauri::{AppHandle, WebviewWindow};
use windows::Win32::Foundation::HWND;

/// Recover the raw HWND.
///
/// Only the pointer value crosses the boundary, so this does not depend on Tauri's
/// `windows` crate version matching ours.
pub fn hwnd_of(window: &WebviewWindow) -> Result<HWND, Box<dyn std::error::Error>> {
    let raw = window.hwnd()?;
    Ok(HWND(raw.0.cast::<c_void>()))
}

/// Attach the platform layer to a freshly created window, before it is shown.
pub fn install(app: &AppHandle, window: &WebviewWindow) -> Result<(), Box<dyn std::error::Error>> {
    let hwnd = hwnd_of(window)?;

    let effective = backdrop::apply(
        hwnd,
        appearance::preferred_backdrop(),
        appearance::is_dark(window),
    );
    tracing::info!(?effective, "system backdrop applied");

    appearance::watch(app.clone());

    Ok(())
}
