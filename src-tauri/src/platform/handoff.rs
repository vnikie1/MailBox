//! What Windows asks of the main window, held until the window can hear it.
//!
//! Two requests reach Rust from outside and are acted on by the page: a Jump List task that is a
//! place in the UI (Inbox, Search), and a press on a toast. Both used to be emitted as events
//! the moment they arrived. An event emitted before the page has subscribed reaches nobody, and
//! that was every one of them in two cases:
//!
//! - **At launch** (H19, 2026-11-14 audit). The arguments a cold start carries are read in
//!   `setup`, before the WebView has loaded a line of script.
//! - **After the main window was closed** with a compose or Settings window still open (M30).
//!   Bringing it back makes a new WebView, which has to load before it can listen.
//!
//! So they go through here. An event is held until the main window calls `handoff_take` for it,
//! which hands over what was held and lets later ones straight through. When the main window
//! closes, they are held again.

use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};

use tauri::{AppHandle, Emitter};

/// The events that are held. Anything else is emitted as before.
pub const HELD: [&str; 2] = ["jumplist:task", "notification:action"];

/// More than anyone presses before a window has loaded. Past this the oldest go, so a main window
/// that never asks cannot make this grow.
const MOST: usize = 32;

#[derive(Default)]
struct State {
    listening: HashSet<&'static str>,
    held: Vec<(&'static str, serde_json::Value)>,
}

#[derive(Default)]
pub struct Handoff(Mutex<State>);

impl Handoff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Emits `event` now if the main window is listening for it, and holds it if not.
    pub fn send(&self, app: &AppHandle, event: &'static str, payload: serde_json::Value) {
        if let Some(payload) = self.hold_unless_heard(event, payload) {
            let _ = app.emit(event, payload);
        }
    }

    /// Holds the payload, or gives it back to be emitted.
    fn hold_unless_heard(
        &self,
        event: &'static str,
        payload: serde_json::Value,
    ) -> Option<serde_json::Value> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);

        if state.listening.contains(event) {
            return Some(payload);
        }

        if state.held.len() >= MOST {
            state.held.remove(0);
        }
        state.held.push((event, payload));
        None
    }

    /// The main window is now listening for `event`. Returns what was held for it, in order.
    pub fn take(&self, event: &str) -> Vec<serde_json::Value> {
        let Some(event) = HELD.iter().copied().find(|held| *held == event) else {
            return Vec::new();
        };

        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.listening.insert(event);

        let (taken, kept) = std::mem::take(&mut state.held)
            .into_iter()
            .partition(|(name, _)| *name == event);
        state.held = kept;

        taken.into_iter().map(|(_, payload)| payload).collect()
    }

    /// The main window has closed. Whatever comes next waits for the one that replaces it.
    pub fn window_closed(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .listening
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn what_arrives_before_the_window_listens_is_kept_for_it() {
        // H19: a Search task that started the app arrived before the page had loaded.
        let handoff = Handoff::new();
        assert_eq!(
            handoff.hold_unless_heard("jumplist:task", json!("--search")),
            None
        );

        assert_eq!(handoff.take("jumplist:task"), vec![json!("--search")]);
    }

    #[test]
    fn once_the_window_listens_nothing_more_is_held() {
        let handoff = Handoff::new();
        assert!(handoff.take("jumplist:task").is_empty());

        assert_eq!(
            handoff.hold_unless_heard("jumplist:task", json!("--inbox")),
            Some(json!("--inbox"))
        );
        assert!(handoff.take("jumplist:task").is_empty());
    }

    #[test]
    fn each_event_is_handed_only_to_whoever_asked_for_it() {
        // The Jump List and the toasts are subscribed in different components, which load at
        // different moments. One asking must not take the other's.
        let handoff = Handoff::new();
        handoff.hold_unless_heard("jumplist:task", json!("--inbox"));
        handoff.hold_unless_heard("notification:action", json!({ "action": "open" }));

        assert_eq!(handoff.take("jumplist:task"), vec![json!("--inbox")]);
        assert_eq!(
            handoff.take("notification:action"),
            vec![json!({ "action": "open" })]
        );
    }

    #[test]
    fn a_closed_window_has_its_requests_held_for_the_next_one() {
        // M30: the main window was closed and is being brought back. The new page has not
        // subscribed yet, so a toast pressed now has to wait for it.
        let handoff = Handoff::new();
        handoff.take("notification:action");
        handoff.window_closed();

        assert_eq!(
            handoff.hold_unless_heard("notification:action", json!(1)),
            None
        );
        assert_eq!(handoff.take("notification:action"), vec![json!(1)]);
    }

    #[test]
    fn an_event_that_is_not_held_cannot_be_taken() {
        let handoff = Handoff::new();
        assert!(handoff.take("mailbox:changed").is_empty());
        assert!(!handoff
            .0
            .lock()
            .unwrap()
            .listening
            .contains("mailbox:changed"));
    }

    #[test]
    fn a_window_that_never_asks_cannot_make_the_queue_grow() {
        let handoff = Handoff::new();
        for index in 0..(MOST + 5) {
            handoff.hold_unless_heard("jumplist:task", json!(index));
        }

        let taken = handoff.take("jumplist:task");
        assert_eq!(taken.len(), MOST);
        assert_eq!(taken[0], json!(5), "the oldest went first");
    }
}
