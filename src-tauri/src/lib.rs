//! Halcyon — application shell.
//!
//! The IPC surface here is the seam described in docs/03-architecture.md §4. Phase 3 adds
//! the local store and the mail commands; networking arrives in Phase 5. The commands that
//! have no behaviour behind them yet are deliberately absent rather than stubbed.

pub mod accounts;
pub mod db;
pub mod mail;
pub mod sync;

mod ipc;

mod platform;

use tauri::{Manager, WindowEvent};

pub fn run() {
    // docs/03 §5 budgets cold start to painted UI at under 800ms. This measures the half
    // the core is responsible for — process start to the window being shown, which
    // includes opening and migrating the store. The WebView's own load and first paint are
    // not included and cannot be from here; a true end-to-end figure needs a release build
    // with bundled assets, which is Phase 11's measurement.
    let started = std::time::Instant::now();

    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            ipc::window::appearance_get,
            ipc::mail::accounts_list,
            ipc::mail::mailboxes_tree,
            ipc::mail::messages_page,
            ipc::mail::message_get,
            ipc::mail::thread_get,
            ipc::mail::search,
            ipc::mail::msg_set_flags,
            ipc::mail::msg_move,
            ipc::mail::msg_delete,
            ipc::accounts::providers_list,
            ipc::accounts::account_discover,
            ipc::accounts::account_test,
            ipc::accounts::account_add_password,
            ipc::accounts::account_add_oauth,
            ipc::accounts::accounts_detail,
            ipc::accounts::account_update,
            ipc::accounts::accounts_reorder,
            ipc::accounts::account_remove,
            ipc::accounts::account_credential_status,
            ipc::accounts::oauth_client_get,
            ipc::accounts::oauth_client_set,
            ipc::accounts::provider_open_setup,
            ipc::sync::sync_now,
            ipc::sync::sync_all,
            ipc::sync::bodies_ensure,
            ipc::sync::sync_watch,
            ipc::compose::compose_reply,
            ipc::compose::compose_send,
            ipc::compose::compose_undo,
            ipc::compose::outbox_list,
            ipc::body::message_body,
            ipc::body::open_external,
            ipc::body::open_external_confirmed,
            ipc::attachments::attachment_preview,
            ipc::attachments::attachment_save,
        ])
        .setup(move |app| {
            let main = app
                .get_webview_window("main")
                .ok_or("main window missing from tauri.conf.json")?;

            // Opened before the window is shown, so the first frame the user sees is
            // already backed by the real store rather than by an empty one that fills in.
            let path = db::default_path();
            tracing::info!(path = %path.display(), "opening mail store");
            app.manage(db::Db::open(&path)?);
            app.manage(sync::engine::SyncEngine::new());
            app.manage(sync::idle::Watchers::new());

            // The outbox sender. Started here rather than on first send, because its first
            // job is to resolve whatever a previous run left in flight — a message that was
            // mid-send when the app was killed must be settled before anything else is sent.
            let sender = sync::sender::Sender::new();
            {
                let events: std::sync::Arc<dyn sync::events::Events> =
                    std::sync::Arc::new(app.handle().clone());
                let db: db::Db = app.state::<db::Db>().inner().clone();
                let root = path
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_default();
                sender.start(events, db, root);
            }
            app.manage(sender);

            platform::install(app.handle(), &main)?;

            // The window is created hidden so the DWM backdrop and the theme attribute
            // are both in place before the first frame. Showing it here is what stops
            // the white flash on launch that docs/02 §8 rules out.
            main.show()?;

            tracing::info!(
                elapsed_ms = started.elapsed().as_millis() as u64,
                "window shown (core side of cold start; excludes WebView paint)"
            );

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
        .expect("failed to start Halcyon");
}

fn init_tracing() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_env("HALCYON_LOG").unwrap_or_else(|_| {
        EnvFilter::new(if cfg!(debug_assertions) {
            "halcyon_lib=debug,warn"
        } else {
            "halcyon_lib=info,warn"
        })
    });

    tracing_subscriber::registry()
        .with(fmt::layer().with_target(true).compact())
        .with(filter)
        .init();
}
