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
//! and it learns one from a Start Menu shortcut written by an installer — which is Phase 11. So:
//!
//! - **Running app, installed:** toast appears, buttons work, actions route through the UI.
//! - **Running app, not installed** (`npm run app:dev`): Windows refuses the AUMID, `show()`
//!   returns an error, and `notify::announce` logs it at debug and moves on. Nothing breaks;
//!   there is simply no toast. This is the state during development and it is expected.
//! - **App not running:** needs the COM activator docs/06 names, which can only be registered
//!   against an installed AUMID. Deferred to Phase 11 with the installer, deliberately.
//!
//! ## Why the actions route through the UI
//!
//! Archiving from a toast could be done here — the store functions are right there. It is done by
//! emitting an event the frontend handles instead, because the frontend already owns these
//! mutations along with the cache invalidation that makes the list update. Doing it twice would
//! mean two implementations of "archive", and the one in this file would be the one that forgets
//! to refresh the message list, or drifts the day archive learns to do something else.

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_winrt_notification::{Sound, Toast};

/// The AUMID the installer build is registered under.
///
/// Must match the identifier in `tauri.conf.json`, because that is what the installer will use
/// when it writes the Start Menu shortcut. If these ever disagree, toasts stop appearing in
/// release builds and nowhere else — which is the hardest kind of bug to notice.
const APP_ID: &str = "com.uniki.halcyon";

/// The AUMID toasts are addressed to: the package's own when this is the Store build.
///
/// The Store package is `<package family name>!Halcyon` and registers no shortcut under
/// `APP_ID`. Addressing its toasts to `APP_ID` showed them only on a PC where the installer
/// build had once been installed (H22, 2026-11-14 audit). See `platform::package`.
pub fn app_id() -> &'static str {
    super::package::app_user_model_id().unwrap_or(APP_ID)
}

/// What the user pressed, on which message.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToastAction {
    /// `reply`, `archive`, `read`, or `open` when the toast body itself was clicked.
    pub action: String,
    pub message_id: i64,
}

/// A press, with the fingerprint of the message the toast was shown for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pressed {
    action: String,
    message_id: i64,
    fingerprint: u64,
}

/// The argument string a button sends back: `archive:41:<fingerprint>`.
///
/// Deliberately dull, because it crosses a Windows API boundary as an opaque string and anything
/// structured would have to be escaped by hand. The fingerprint is what stops a toast acting on
/// another message that has since been given the same row id (L37). See `notify::fingerprint`.
fn argument(action: &str, message_id: i64, fingerprint: u64) -> String {
    format!("{action}:{message_id}:{fingerprint:016x}")
}

/// Parses what `argument` wrote.
///
/// A toast from before the fingerprint, `archive:41`, is refused: it is exactly the kind that
/// could name the wrong message.
fn parse_action(argument: &str) -> Option<Pressed> {
    let mut parts = argument.split(':');
    let (action, id, fingerprint) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }

    // An allow-list. The argument comes back from the shell, and while Windows only returns what
    // we put in, treating it as trusted is a habit rather than a fact.
    if !matches!(action, "reply" | "archive" | "read" | "open") {
        return None;
    }

    Some(Pressed {
        action: action.to_string(),
        message_id: id.parse().ok()?,
        fingerprint: u64::from_str_radix(fingerprint, 16).ok()?,
    })
}

/// Acts on a press, if the message it names is still the one the toast was shown for.
///
/// The window comes forward either way. A toast for a message that has gone, pressed, should
/// still open the app; it should not archive whatever now has that id.
async fn act(app: AppHandle, pressed: Pressed) {
    super::tray::show(&app);

    let db = app.state::<crate::db::Db>();
    let id = pressed.message_id;
    let now = db
        .read(move |conn| super::notify::fingerprint_of(conn, id))
        .await
        .ok()
        .flatten();

    if now != Some(pressed.fingerprint) {
        tracing::info!(
            message_id = id,
            action = pressed.action,
            "a toast was pressed for a message that is no longer here; nothing was done"
        );
        return;
    }

    let action = ToastAction {
        action: pressed.action,
        message_id: id,
    };

    // Held if the main window is not listening yet: it is being brought back, or the app has
    // only just started. See `platform::handoff`.
    app.state::<super::handoff::Handoff>().send(
        &app,
        "notification:action",
        serde_json::to_value(action).unwrap_or_default(),
    );
}

/// Shows one toast for one message, with the three actions docs/06 asks for.
pub fn show_message(
    app: &AppHandle,
    message_id: i64,
    fingerprint: u64,
    from: &str,
    subject: &str,
    sound: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.clone();

    Toast::new(app_id())
        .title(from)
        .text1(subject)
        // The system's own mail sound rather than one shipped with the app, so it matches
        // whatever the user has chosen in Windows' sound settings — including silence, and
        // including a custom one. `None` is a silent toast, not the default sound.
        .sound(if sound { Some(Sound::Mail) } else { None })
        // Order matters: the leftmost is the one people press without reading, and between the
        // three, Reply is the only one that is not silently destructive to do by accident.
        .add_button("Reply", &argument("reply", message_id, fingerprint))
        .add_button("Archive", &argument("archive", message_id, fingerprint))
        .add_button("Mark as Read", &argument("read", message_id, fingerprint))
        .on_activated(move |pressed| {
            // A button sends its own argument back. Clicking the toast *body* sends nothing, and
            // that is the common case — Windows makes the body by far the easiest target. An
            // absent argument therefore means "show me this message", not "something went wrong".
            let pressed = match pressed.as_deref().filter(|value| !value.is_empty()) {
                Some(value) => parse_action(value),
                None => Some(Pressed {
                    action: "open".to_string(),
                    message_id,
                    fingerprint,
                }),
            };

            match pressed {
                // The window comes forward in Rust rather than in the UI. Every one of these
                // actions is a reply to something the user did while looking at the desktop, and
                // an archive that happens behind a window they cannot see is indistinguishable
                // from one that did not happen.
                Some(pressed) => {
                    tauri::async_runtime::spawn(act(handle.clone(), pressed));
                }
                None => super::tray::show(&handle),
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

    Toast::new(app_id())
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
        let pressed = parse_action(&argument("archive", 41, u64::MAX)).expect("parsed");

        assert_eq!(pressed.action, "archive");
        assert_eq!(pressed.message_id, 41);
        assert_eq!(pressed.fingerprint, u64::MAX);
    }

    #[test]
    fn every_button_this_file_writes_can_be_read_back() {
        // The encoding is written in one place and parsed in another, and a typo in either would
        // show up only as a button that silently does nothing.
        for action in ["reply", "archive", "read", "open"] {
            let parsed = parse_action(&argument(action, 7, 0x00ab)).expect("parsed");

            assert_eq!(parsed.action, action);
            assert_eq!(parsed.message_id, 7);
            assert_eq!(parsed.fingerprint, 0x00ab);
        }
    }

    #[test]
    fn an_unknown_action_is_refused() {
        assert!(parse_action("delete:41:00").is_none());
    }

    #[test]
    fn a_malformed_argument_is_refused() {
        assert!(parse_action("archive").is_none());
        assert!(parse_action("archive:").is_none());
        assert!(parse_action("archive:not-a-number:00").is_none());
        assert!(parse_action("archive:41:not-hex").is_none());
        assert!(parse_action("archive:41:00:extra").is_none());
        assert!(parse_action("").is_none());
    }

    #[test]
    fn a_toast_from_before_the_fingerprint_is_refused() {
        // L37. `archive:41` names a row id alone, which may by now be another message.
        assert!(parse_action("archive:41").is_none());
    }

    #[test]
    fn an_unpackaged_build_addresses_toasts_to_the_installers_app_id() {
        // H22's other half: only the Store package may use its own. The test runner has none.
        assert_eq!(app_id(), APP_ID);
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
