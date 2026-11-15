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
pub mod browser_keys;
pub mod fatal;
pub mod files;
pub mod handoff;
pub mod jumplist;
pub mod links;
pub mod notify;
pub mod package;
pub mod sound;
pub mod startup;
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
    decorate(window);

    // Outside `decorate` on purpose. This is not decoration: it is what pushes theme, accent
    // and transparency changes from Windows into the UI, and it needs no window handle. It
    // finds the main window by name each time, so a main window made again needs no second one.
    appearance::watch(app.clone());
}

/// The system backdrop, which every main window needs before it is first shown.
fn decorate(window: &WebviewWindow) {
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
}

/// Makes the main window again, after it was closed while another window kept the app running.
///
/// Built from the same entry in `tauri.conf.json` as the first one, so it is the same window:
/// hidden until the backdrop is on it, and placed where the window-state plugin last saw it.
/// The badge is drawn on a window rather than on the app, so it is drawn again too (M30).
///
/// One at a time. The tray, a second launch and a toast can all ask in the same moment, and a
/// second build under the label `main` would fail.
pub fn reopen_main(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::Manager;

    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let window = match app.get_webview_window("main") {
        Some(window) => window,
        None => {
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "main")
                .ok_or("tauri.conf.json has no main window")?
                .clone();

            let window = tauri::WebviewWindowBuilder::from_config(app, &config)?.build()?;
            decorate(&window);
            tracing::info!("the main window was closed, and has been made again");
            window
        }
    };

    window.show()?;
    let _ = window.unminimize();
    let _ = window.set_focus();

    let app = app.clone();
    tauri::async_runtime::spawn(async move { tray::repaint(&app).await });

    Ok(())
}
