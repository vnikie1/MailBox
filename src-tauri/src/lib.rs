//! Halcyon — application shell.
//!
//! The IPC surface here is the seam described in docs/03-architecture.md §4. Phase 3 adds
//! the local store and the mail commands; networking arrives in Phase 5. The commands that
//! have no behaviour behind them yet are deliberately absent rather than stubbed.

pub mod accounts;
pub mod db;
pub mod mail;
pub mod rules;
pub mod search;
pub mod sync;
pub mod undo;

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
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_deep_link::init())
        // Off unless the user asks. An app that added itself to startup without being told is
        // one people uninstall.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        // A second launch — from a mailto: link, or the Start menu — hands its arguments to the
        // running instance and exits. Without this, opening a mailto: link would start a whole
        // second copy of the app against the same database.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            use tauri::Manager;

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }

            platform::links::handle_arguments(app, &argv);
        }))
        .invoke_handler(tauri::generate_handler![
            ipc::window::appearance_get,
            platform::sound::sound_sent,
            ipc::mail::accounts_list,
            ipc::mail::mailboxes_tree,
            ipc::mail::messages_page,
            ipc::mail::message_get,
            ipc::mail::thread_get,
            ipc::mail::search,
            ipc::mail::msg_set_flags,
            ipc::mail::msg_toggle_read,
            ipc::mail::msg_toggle_flag,
            ipc::mail::msg_archive,
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
            ipc::compose::compose_open,
            ipc::compose::outbox_retry,
            ipc::compose::outbox_schedule,
            ipc::compose::compose_pick_files,
            ipc::compose::compose_size_limit,
            ipc::compose::signature_get,
            ipc::compose::signature_set,
            ipc::compose::compose_blank,
            ipc::compose::compose_save_draft,
            ipc::compose::compose_redirect,
            ipc::compose::compose_undo_seconds,
            ipc::compose::compose_set_undo_seconds,
            ipc::organise::smart_list,
            ipc::organise::smart_save,
            ipc::organise::smart_delete,
            ipc::organise::smart_messages,
            ipc::organise::rules_list,
            ipc::organise::rule_save,
            ipc::organise::rule_delete,
            ipc::organise::rules_run,
            ipc::organise::flag_names,
            ipc::organise::flag_rename,
            ipc::organise::flag_set,
            ipc::organise::vips_list,
            ipc::organise::vip_add,
            ipc::organise::vip_remove,
            ipc::organise::junk_status,
            ipc::organise::junk_mark,
            ipc::organise::junk_scan,
            ipc::organise::junk_training_mode,
            ipc::organise::junk_set_training_mode,
            ipc::organise::blocked_list,
            ipc::organise::block_sender,
            ipc::organise::unblock_sender,
            ipc::organise::snooze,
            ipc::organise::unsnooze,
            ipc::organise::mute_thread,
            ipc::organise::follow_ups_detect,
            ipc::organise::notify_prefs,
            ipc::organise::notify_set_prefs,
            ipc::organise::run_at_login,
            ipc::organise::set_run_at_login,
            ipc::organise::undo_available,
            ipc::organise::undo_perform,
            ipc::organise::redo_perform,
            ipc::compose::compose_discard_draft,
            ipc::compose::contacts_suggest,
            ipc::search::search_run,
            ipc::search::search_suggest,
            ipc::search::search_history,
            ipc::search::search_remember,
            ipc::search::search_history_clear,
            ipc::search::search_save_as_smart,
            ipc::body::message_body,
            ipc::body::remote_images_enabled,
            ipc::body::set_remote_images_enabled,
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
            // Behind an `Arc` because `Db::write` moves its closure to the writer thread, so
            // an undo command has to hand the stack across that boundary rather than borrow it.
            app.manage(std::sync::Arc::new(undo::Stack::new()));

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
                // `tauri::async_runtime::spawn` rather than `tokio::spawn`: `setup` does not run
                // inside a runtime, and the bare form panics there.
                tauri::async_runtime::spawn(sender.run(events, db, root));
            }
            app.manage(sender);

            // The upkeep tick: reminders that come due, follow-up detection. Started here for
            // the same reason as the sender — a reminder that fell due while the app was closed
            // has to fire on the first tick after launch, not be lost.
            {
                let events: std::sync::Arc<dyn sync::events::Events> =
                    std::sync::Arc::new(app.handle().clone());
                let db: db::Db = app.state::<db::Db>().inner().clone();
                tauri::async_runtime::spawn(sync::upkeep::run(db, events));
            }

            platform::install(app.handle(), &main)?;

            // The taskbar right-click menu. Installed once; the shell remembers it until the
            // next CommitList, so there is nothing to refresh and nothing to tear down.
            platform::jumplist::install();

            // The tray and the taskbar badge. Both driven by one unread count, so they cannot
            // show the user two different answers on the same screen.
            match platform::tray::install(app.handle()) {
                Ok(_) => {
                    let handle = app.handle().clone();

                    // Refreshed on the same event the list invalidates on. The count is cheap
                    // and the update is skipped when the number has not moved, which matters:
                    // a sync raises this event once per mailbox.
                    tauri::Listener::listen(app.handle(), "mailbox:changed", move |_| {
                        let handle = handle.clone();
                        tauri::async_runtime::spawn(async move {
                            platform::tray::refresh(&handle).await;
                        });
                    });

                    // Once at startup, or the badge stays blank until the first sync finishes —
                    // which on a cold start is exactly when someone wants to know.
                    let handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        platform::tray::refresh(&handle).await;
                    });

                    // New mail. Raised by the sync *after* rules and the junk filter have run,
                    // so nothing a rule filed away or the classifier caught raises a toast.
                    let handle = app.handle().clone();
                    tauri::Listener::listen(app.handle(), "mail:arrived", move |event| {
                        #[derive(serde::Deserialize)]
                        #[serde(rename_all = "camelCase")]
                        struct Arrived {
                            account_id: i64,
                            message_ids: Vec<i64>,
                        }

                        let Ok(arrived) = serde_json::from_str::<Arrived>(event.payload()) else {
                            return;
                        };

                        let handle = handle.clone();
                        tauri::async_runtime::spawn(async move {
                            let db = tauri::Manager::state::<db::Db>(&handle);
                            platform::notify::announce(
                                &handle,
                                db.inner(),
                                arrived.account_id,
                                arrived.message_ids,
                            )
                            .await;
                        });
                    });
                }
                // Logged, not fatal. A tray icon is a convenience, and a mail client that
                // refused to start because the shell would not give it one would be absurd.
                Err(error) => tracing::warn!(%error, "could not create the tray icon"),
            }

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
