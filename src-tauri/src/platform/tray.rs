//! The tray icon and the unread count that drives it. docs/06 Phase 10.
//!
//! ## Why the tray and the taskbar badge live together
//!
//! They answer the same question — "is there anything new?" — from one number, and that number
//! has to be computed the same way for both. Split across two modules they would drift: one
//! counting every mailbox, the other only the Inbox, and the user seeing two different answers
//! on the same screen.
//!
//! ## What the tray does not do
//!
//! It does not hide the window on close. An app that vanishes into the tray when you press the
//! close button is a well-known annoyance, and Windows has spent a decade teaching people that
//! close means close. The tray is a *shortcut back*, not a place the app hides.

use std::sync::atomic::{AtomicU32, Ordering};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::db::{Db, DbError};

/// The last count shown, so an unchanged one costs nothing.
///
/// A sync that touches fifty mailboxes emits fifty change events, and redrawing a 16×16 icon
/// fifty times to show the same number is fifty COM calls the shell has to service.
static SHOWN: AtomicU32 = AtomicU32::new(u32::MAX);

/// Unread across every account's Inbox.
///
/// The Inbox alone, deliberately. Counting every mailbox would include Junk, Trash and the
/// archive, so a badge would sit there for ever showing unread spam nobody intends to read —
/// and a badge that never reaches zero is one people stop looking at.
pub fn unread_count(conn: &rusqlite::Connection) -> Result<u32, DbError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
               FROM message
               JOIN mailbox ON mailbox.id = message.mailbox_id
              WHERE mailbox.role = 'inbox'
                AND message.flag_seen = 0
                AND message.is_junk = 0
                AND (message.snooze_until IS NULL OR message.snooze_until <= ?1)",
            rusqlite::params![now()],
            |row| row.get(0),
        )
        .unwrap_or(0);

    Ok(u32::try_from(count).unwrap_or(u32::MAX))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// Builds the tray icon and its menu.
pub fn install(app: &AppHandle) -> Result<TrayIcon, Box<dyn std::error::Error>> {
    let open = MenuItem::with_id(app, "open", "Open Halcyon", true, None::<&str>)?;
    let compose = MenuItem::with_id(app, "compose", "New Message", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&open, &compose, &quit])?;

    let tray = TrayIconBuilder::with_id("halcyon")
        .tooltip("Halcyon")
        .menu(&menu)
        // The menu belongs on right-click, which is the Windows convention. Showing it on left
        // click as well would mean a single click never just opens the app.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show(app),
            "compose" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = compose_from_tray(&app).await {
                        tracing::warn!(%error, "could not compose from the tray");
                    }
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Left click opens. Anything else — right click, hover — is the menu's business or
            // nobody's.
            if let TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                show(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(tray)
}

/// Brings the window back, from wherever it is.
///
/// All three calls are needed and each covers a different state: hidden, minimised, and open
/// but behind something. Doing only `show` leaves a minimised window minimised.
pub fn show(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

async fn compose_from_tray(app: &AppHandle) -> Result<(), String> {
    let db = app.state::<Db>();

    let has_account: bool = db
        .read(|conn| {
            Ok(conn
                .query_row("SELECT COUNT(*) FROM account", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap_or(0)
                > 0)
        })
        .await
        .map_err(|error| error.to_string())?;

    if !has_account {
        // No accounts yet. Showing the window is the useful thing — it is where they would add
        // one — and opening an empty compose window from nowhere would be baffling.
        show(app);
        return Ok(());
    }

    // The same command the New Message shortcut calls, so a draft opened from the tray is
    // identical to one opened from the window. Two paths would drift.
    crate::ipc::compose::compose_open(app.clone(), None, None, None)
        .await
        .map(|_| ())
        .map_err(|error| error.message)
}

/// Recomputes the count and updates the tray tooltip and the taskbar badge.
///
/// Both from one number, so they cannot disagree. Cheap when nothing changed, which matters:
/// this runs on every `mailbox:changed`, and a sync raises a lot of those.
pub async fn refresh(app: &AppHandle) {
    let db = app.state::<Db>();

    let Ok(count) = db.read(unread_count).await else {
        return;
    };

    if SHOWN.swap(count, Ordering::Relaxed) == count {
        return;
    }

    if let Some(window) = app.get_webview_window("main") {
        crate::platform::badge::refresh(&window, count);
    }

    if let Some(tray) = app.tray_by_id("halcyon") {
        let tooltip = match count {
            0 => "Halcyon".to_string(),
            1 => "Halcyon — 1 unread".to_string(),
            many => format!("Halcyon — {many} unread"),
        };

        let _ = tray.set_tooltip(Some(&tooltip));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn store() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open");
        crate::db::migrate::run(&mut conn).expect("migrate");

        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'T', 'me@t.test', 'other', 'password', 'halcyon:me')",
            [],
        )
        .expect("account");

        for (id, path, role) in [(1, "INBOX", "inbox"), (2, "Junk", "junk")] {
            conn.execute(
                "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
                 VALUES (?1, 1, ?2, ?2, ?3)",
                rusqlite::params![id, path, role],
            )
            .expect("mailbox");
        }

        conn
    }

    fn add(conn: &Connection, id: i64, mailbox: i64, seen: bool, junk: bool, snooze: Option<i64>) {
        conn.execute(
            "INSERT INTO message (
                 id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
                 from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged, is_junk,
                 snooze_until
             ) VALUES (?1, 1, ?2, ?1, 'S', 0, 0, 10, 'a@b.test', '', '', 0, ?3, 0, ?4, ?5)",
            rusqlite::params![id, mailbox, i64::from(seen), i64::from(junk), snooze],
        )
        .expect("message");
    }

    #[test]
    fn unread_inbox_mail_is_counted() {
        let conn = store();
        add(&conn, 1, 1, false, false, None);
        add(&conn, 2, 1, false, false, None);
        add(&conn, 3, 1, true, false, None);

        assert_eq!(unread_count(&conn).expect("count"), 2);
    }

    #[test]
    fn junk_is_not_counted() {
        // A badge that includes unread spam never reaches zero, and a badge that never reaches
        // zero is one people stop looking at.
        let conn = store();
        add(&conn, 1, 2, false, true, None);
        add(&conn, 2, 1, false, true, None);

        assert_eq!(unread_count(&conn).expect("count"), 0);
    }

    #[test]
    fn a_snoozed_message_is_not_counted_until_it_is_due() {
        // The whole point of Remind Me is not to be told about it yet. A badge counting it
        // would be telling them anyway.
        let conn = store();
        add(&conn, 1, 1, false, false, Some(i64::MAX / 2));
        assert_eq!(unread_count(&conn).expect("count"), 0);

        add(&conn, 2, 1, false, false, Some(1));
        assert_eq!(unread_count(&conn).expect("count"), 1);
    }

    #[test]
    fn only_the_inbox_counts() {
        let conn = store();
        add(&conn, 1, 2, false, false, None);

        assert_eq!(unread_count(&conn).expect("count"), 0);
    }

    #[test]
    fn an_empty_store_counts_zero_rather_than_failing() {
        let mut conn = Connection::open_in_memory().expect("open");
        crate::db::migrate::run(&mut conn).expect("migrate");

        assert_eq!(unread_count(&conn).expect("count"), 0);
    }
}
