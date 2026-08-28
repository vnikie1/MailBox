//! The Windows toast itself, with its buttons. docs/06 Phase 10.
//!
//! Split out from `notify` because the two answer different questions. `notify` decides *whether*
//! a message deserves a notification — four gates, all testable, none of them touching Windows.
//! This file decides what the toast looks like and what its buttons do, none of which a test can
//! observe. Keeping them apart is what lets the policy have real tests.
//!
//! ## Why not the Tauri plugin
//!
//! `tauri-plugin-notification` wraps `tauri-winrt-notification` but exposes only
//! `action_type_id`, which is the mobile notion of an action — a category registered up front.
//! `add_button`, the thing that actually draws Reply / Archive / Mark as Read on a Windows toast,
//! is not reachable through the plugin. docs/06 asks for those buttons, so this path uses the
//! crate directly. The plugin stays as the dependency that installs the permission plumbing.
//!
//! ## What works now and what waits for the installer
//!
//! A toast is addressed to an AppUserModelID. Windows only shows toasts for an AUMID it knows,
//! and it learns one from a Start Menu shortcut written by an installer — which is Phase 12. So:
//!
//! - **Running app, installed:** toast appears, buttons work, actions route through the UI.
//! - **Running app, not installed** (`npm run app:dev`): Windows refuses the AUMID, `show()`
//!   returns an error, and `notify::announce` logs it at debug and moves on. Nothing breaks;
//!   there is simply no toast. This is the state during development and it is expected.
//! - **App not running:** needs the COM activator docs/06 names, which can only be registered
//!   against an installed AUMID. Deferred to Phase 12 with the installer, deliberately.
//!
//! ## Why the actions route through the UI
//!
//! Archiving from a toast could be done here — the store functions are right there. It is done by
//! emitting an event the frontend handles instead, because the frontend already owns these
//! mutations along with the cache invalidation that makes the list update. Doing it twice would
//! mean two implementations of "archive", and the one in this file would be the one that forgets
//! to refresh the message list, or drifts the day archive learns to do something else.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_winrt_notification::{Sound, Toast};

/// The AUMID an installed Halcyon is registered under.
///
/// Must match the identifier in `tauri.conf.json`, because that is what the installer will use
/// when it writes the Start Menu shortcut. If these ever disagree, toasts stop appearing in
/// release builds and nowhere else — which is the hardest kind of bug to notice.
const APP_ID: &str = "com.uniki.halcyon";

/// What the user pressed, on which message.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToastAction {
    /// `reply`, `archive`, `read`, or `open` when the toast body itself was clicked.
    pub action: String,
    pub message_id: i64,
}

/// Parses the argument string a button sends back.
///
/// The encoding is deliberately dull — `archive:41` — because it crosses a Windows API boundary
/// as an opaque string and anything structured would have to be escaped by hand.
fn parse_action(argument: &str) -> Option<ToastAction> {
    let (action, id) = argument.split_once(':')?;

    // An allow-list. The argument comes back from the shell, and while Windows only returns what
    // we put in, treating it as trusted is a habit rather than a fact.
    if !matches!(action, "reply" | "archive" | "read" | "open") {
        return None;
    }

    Some(ToastAction {
        action: action.to_string(),
        message_id: id.parse().ok()?,
    })
}

/// Shows one toast for one message, with the three actions docs/06 asks for.
pub fn show_message(
    app: &AppHandle,
    message_id: i64,
    from: &str,
    subject: &str,
    sound: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.clone();

    Toast::new(APP_ID)
        .title(from)
        .text1(subject)
        // The system's own mail sound rather than one shipped with the app, so it matches
        // whatever the user has chosen in Windows' sound settings — including silence, and
        // including a custom one. `None` is a silent toast, not the default sound.
        .sound(if sound { Some(Sound::Mail) } else { None })
        // Order matters: the leftmost is the one people press without reading, and between the
        // three, Reply is the only one that is not silently destructive to do by accident.
        .add_button("Reply", &format!("reply:{message_id}"))
        .add_button("Archive", &format!("archive:{message_id}"))
        .add_button("Mark as Read", &format!("read:{message_id}"))
        .on_activated(move |argument| {
            // A button sends its own argument back. Clicking the toast *body* sends nothing, and
            // that is the common case — Windows makes the body by far the easiest target. An
            // absent argument therefore means "show me this message", not "something went wrong".
            let action = match argument.as_deref().filter(|value| !value.is_empty()) {
                Some(value) => parse_action(value),
                None => Some(ToastAction {
                    action: "open".to_string(),
                    message_id,
                }),
            };

            if let Some(action) = action {
                // The window comes forward here rather than in the UI. Every one of these
                // actions is a reply to something the user did while looking at the desktop, and
                // an archive that happens behind a window they cannot see is indistinguishable
                // from one that did not happen.
                super::tray::show(&handle);
                let _ = handle.emit("notification:action", action);
            }

            Ok(())
        })
        .show()?;

    Ok(())
}

/// One toast for a batch. No buttons: there is no single message for them to act on.
pub fn show_summary(
    app: &AppHandle,
    count: usize,
    sound: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.clone();

    Toast::new(APP_ID)
        .title(&format!("{count} new messages"))
        .text1("Halcyon")
        .sound(if sound { Some(Sound::Mail) } else { None })
        .on_activated(move |_| {
            // No message to open, so this brings the app forward and lets the user look.
            super::tray::show(&handle);
            Ok(())
        })
        .show()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_button_argument_round_trips() {
        let action = parse_action("archive:41").expect("parsed");

        assert_eq!(action.action, "archive");
        assert_eq!(action.message_id, 41);
    }

    #[test]
    fn every_button_this_file_writes_can_be_read_back() {
        // The encoding is written in one place and parsed in another, and a typo in either would
        // show up only as a button that silently does nothing.
        for action in ["reply", "archive", "read", "open"] {
            let parsed = parse_action(&format!("{action}:7")).expect("parsed");

            assert_eq!(parsed.action, action);
            assert_eq!(parsed.message_id, 7);
        }
    }

    #[test]
    fn an_unknown_action_is_refused() {
        assert!(parse_action("delete:41").is_none());
    }

    #[test]
    fn a_malformed_argument_is_refused() {
        assert!(parse_action("archive").is_none());
        assert!(parse_action("archive:").is_none());
        assert!(parse_action("archive:not-a-number").is_none());
        assert!(parse_action("").is_none());
    }

    #[test]
    fn the_app_id_matches_the_bundle_identifier() {
        // These have to agree or toasts vanish in release builds only. Asserted here so the
        // disagreement is a failing test rather than a silent absence months later.
        let config = include_str!("../../tauri.conf.json");
        assert!(
            config.contains(&format!("\"identifier\": \"{APP_ID}\"")),
            "tauri.conf.json identifier does not match toast::APP_ID ({APP_ID})"
        );
    }
}
