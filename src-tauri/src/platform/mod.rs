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
pub mod window_state;

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
///
/// ## Why this cannot fail the app
///
/// It used to return a `Result` and the setup hook used `?` on it. Tauri turns any error out
/// of the setup hook into a panic, and `panic = "abort"` turns that into a process that dies
/// before painting anything — no window, no dialog, nothing the user can read.
///
/// **Six of the nine crash reports on the developer's machine are that.** All six say
///
/// ```text
/// Failed to setup app: error encountered during setup hook: the underlying handle is not available
/// ```
///
/// and each is preceded in the log, in the same millisecond, by WebView2 refusing to create
/// the webview with `0x80070057` — six occurrences in the log, one per crash, and none since.
/// With no webview there is no window handle, so `hwnd_of` fails, so `?` killed the app.
///
/// What it died for is **a decoration**: the Mica backdrop behind the sidebar. The tray four
/// lines below it in `lib.rs` already had this right — it matches on its own failure and
/// carries on — and there was never a reason for the backdrop to be treated more harshly than
/// the tray.
///
/// So this takes no `?` out to its caller. A missing handle is logged at ERROR, saying what it
/// actually means rather than repeating Tauri's wording, and the app goes on to show its
/// window. If the webview really is dead the user still gets an empty frame rather than a
/// silent exit — which is not good, but it is the first version of this failure they can see
/// and report.
pub fn install(app: &AppHandle, window: &WebviewWindow) {
    match hwnd_of(window) {
        Ok(hwnd) => {
            let effective = backdrop::apply(
                hwnd,
                appearance::preferred_backdrop(),
                appearance::is_dark(window),
            );
            tracing::info!(?effective, "system backdrop applied");
        }
        Err(error) => {
            tracing::error!(
                %error,
                "no window handle, so the system backdrop was skipped; this usually means \
                 WebView2 refused to create the webview (look for 0x80070057 just above)"
            );
        }
    }

    // Outside the match on purpose. This is not decoration: it is what pushes theme, accent
    // and transparency changes from Windows into the UI, and it needs no window handle.
    appearance::watch(app.clone());
}
