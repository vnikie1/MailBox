//! Where the sync engine reports what it is doing.
//!
//! The engine used to take a `tauri::AppHandle` purely so it could call `emit`. That is a
//! large dependency for a small need, and it had a concrete cost: the Phase 5 exit gate has to
//! drive the engine that ships, and the engine could not be driven at all without a GUI
//! runtime. Tauri's own mock runtime does not load on Windows — the test binary fails at
//! start with `STATUS_ENTRYPOINT_NOT_FOUND` before a line of it runs.
//!
//! So the engine now takes this instead. `AppHandle` implements it, so the app is unchanged;
//! the gate implements it in six lines and additionally gets to *assert on what was emitted*,
//! which the `AppHandle` version could not do either.
//!
//! It also matches what docs/03 already says the architecture is: the Rust core owns
//! everything that touches a network, and the UI hears about it over a defined seam. That the
//! seam happened to be a Tauri type was incidental.

/// Something that can be told what happened.
///
/// Deliberately fire-and-forget. A sync must not fail because nothing was listening — the app
/// closing mid-sync is ordinary, and the engine's job is the mailbox, not the audience.
pub trait Events: Send + Sync {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

impl<R: tauri::Runtime> Events for tauri::AppHandle<R> {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        // The error is discarded on purpose, exactly as the call sites used to discard it: it
        // means no window is listening, which is not a sync failure.
        let _ = tauri::Emitter::emit(self, event, payload);
    }
}

/// Serialises a payload for [`Events::emit`].
///
/// A payload that will not serialise is a bug in the payload type, not a reason to abandon a
/// sync that is otherwise working — so it degrades to null and says so in the log. Standing
/// rule 13.
pub fn payload<T: serde::Serialize>(value: &T) -> serde_json::Value {
    match serde_json::to_value(value) {
        Ok(json) => json,
        Err(error) => {
            tracing::warn!(%error, "sync event payload could not be serialised");
            serde_json::Value::Null
        }
    }
}
