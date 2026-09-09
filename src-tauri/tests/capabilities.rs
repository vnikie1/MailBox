//! What the WebView is allowed to ask the core for.
//!
//! ## Why this file exists
//!
//! Tauri v2 denies a command with no entry in `capabilities/default.json`, and denies it *in the
//! webview* — the core never sees the call, so nothing appears in the log and no Rust test can
//! observe it. The only trace is a console message inside a window nobody has devtools open on.
//! `CLAUDE.md` names this as one of the things that will bite you, and it has now bitten twice.
//!
//! The second time cost a released build. A compose window with anything typed in it could not
//! be closed at all: pressing Delete, Save as Draft or Send did nothing, for ever. The list held
//! `core:window:allow-close` and not `core:window:allow-destroy`, and those are two different
//! calls — `close()` *asks*, and a window whose `onCloseRequested` handler allows the close is
//! then shut by `destroy`. Everything looked right, including the permission that looked like
//! the relevant one.
//!
//! These tests read the shipped file. They cannot prove the list is complete — only the running
//! app can do that — but they hold the pairings where one permission without its partner
//! produces a feature that half works.

use std::collections::HashSet;

fn permissions() -> HashSet<String> {
    let raw = include_str!("../capabilities/default.json");
    let parsed: serde_json::Value = serde_json::from_str(raw).expect("capabilities is valid JSON");

    parsed["permissions"]
        .as_array()
        .expect("permissions is an array")
        .iter()
        .map(|value| {
            value
                .as_str()
                .expect("a permission is a string")
                .to_string()
        })
        .collect()
}

fn windows() -> Vec<String> {
    let raw = include_str!("../capabilities/default.json");
    let parsed: serde_json::Value = serde_json::from_str(raw).expect("capabilities is valid JSON");

    parsed["windows"]
        .as_array()
        .expect("windows is an array")
        .iter()
        .map(|value| value.as_str().expect("a window is a string").to_string())
        .collect()
}

#[test]
fn a_window_that_may_close_may_also_be_destroyed() {
    // The pairing that shipped broken. `close()` raises close-requested; allowing it then calls
    // `destroy`. With only the first, every window carrying a close handler asks its question
    // and then refuses to go — and the refusal is silent everywhere the user can see.
    let granted = permissions();

    assert!(
        !granted.contains("core:window:allow-close")
            || granted.contains("core:window:allow-destroy"),
        "core:window:allow-close is granted without core:window:allow-destroy — a window with an          onCloseRequested handler will ask whether to close and then refuse to"
    );
}

#[test]
fn every_window_the_app_opens_is_covered() {
    // A window left off this list has *every* core call denied, not merely one — which is the
    // shape that reads as "the app is broken" rather than "one button does nothing".
    let patterns = windows();

    for expected in ["main", "compose-", "eml-", "settings"] {
        assert!(
            patterns.iter().any(|pattern| pattern.starts_with(expected)),
            "no capability entry covers {expected} windows"
        );
    }
}

#[test]
fn the_list_is_granular_rather_than_core_default() {
    // `core:default` would have hidden the bug above by granting everything, and this app holds
    // someone's entire private correspondence. The description in that file says so; this keeps
    // it true.
    let granted = permissions();

    assert!(
        !granted.contains("core:default"),
        "the capability list has been widened to core:default, which grants the whole core          surface to the webview"
    );
}
