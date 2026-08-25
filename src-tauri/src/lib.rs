//! MailBox — application shell.
//!
//! Phase 0 scope: window, chrome, appearance. No database, no network, no mail.
//! The IPC surface here is the seam described in docs/03-architecture.md §4; the
//! commands that belong to it arrive with their phases.

mod ipc;

mod platform;

use tauri::{Manager, WindowEvent};

pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .invoke_handler(tauri::generate_handler![ipc::window::appearance_get,])
        .setup(|app| {
            let main = app
                .get_webview_window("main")
                .ok_or("main window missing from tauri.conf.json")?;

            platform::install(app.handle(), &main)?;

            // The window is created hidden so the DWM backdrop and the theme attribute
            // are both in place before the first frame. Showing it here is what stops
            // the white flash on launch that docs/02 §8 rules out.
            main.show()?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::ThemeChanged(_) = event {
                // Windows reports the theme flip here well before UISettings settles,
                // so this is the fast path for light/dark; appearance::watch covers
                // accent and transparency changes.

                platform::appearance::push(window.app_handle());
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to start MailBox");
}

fn init_tracing() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_env("MAILBOX_LOG").unwrap_or_else(|_| {
        EnvFilter::new(if cfg!(debug_assertions) {
            "mailbox_lib=debug,warn"
        } else {
            "mailbox_lib=info,warn"
        })
    });

    tracing_subscriber::registry()
        .with(fmt::layer().with_target(true).compact())
        .with(filter)
        .init();
}
