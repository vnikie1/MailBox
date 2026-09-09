//! What is allowed to stop Halcyon from starting.
//!
//! ## Why this file exists
//!
//! Tauri turns any error returned from the `.setup()` hook into a panic, and `panic = "abort"`
//! turns that panic into a process that dies before painting anything: no window, no dialog, no
//! console. The user double-clicks the icon and nothing happens. A crash report is written, but
//! the only thing that can display one is the app that just failed to start.
//!
//! Six of the nine crash reports on the developer's machine were exactly that, and what they
//! died for was a **decoration** — the Mica backdrop behind the sidebar. WebView2 refused to
//! create the webview (`0x80070057`, six times in the log, one per crash), so there was no
//! window handle, so `platform::install(...)?` returned an error, so the app aborted.
//!
//! Every `?` in that hook is therefore a decision about whether a thing is worth refusing to
//! start over. Two are: a window missing from the config is a broken build, and a window that
//! cannot be shown is not a usable app. Decoration is not, and neither is anything else that
//! only makes the app *nicer*.
//!
//! This reads the source rather than running the app, which is the same trade `capabilities.rs`
//! makes and for the same reason: the failure only exists in a packaged build on a machine
//! whose WebView2 is misbehaving, and no test can arrange that. It cannot prove startup is
//! survivable. It holds the one shape that has already cost a working app.

use std::fs;

fn lib_rs() -> String {
    fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("src/lib.rs is readable")
}

/// The regression itself: the backdrop must not be able to abort startup.
#[test]
fn the_system_backdrop_cannot_stop_the_app_starting() {
    let source = lib_rs();

    assert!(
        source.contains("platform::install(app.handle(), &main);"),
        "platform::install should be called for its effect and not propagate — it applies a \
         decorative backdrop, and six crash reports came from a `?` on this line"
    );

    assert!(
        !source.contains("platform::install(app.handle(), &main)?"),
        "a `?` here aborts the app when WebView2 has not produced a window handle. Log it and \
         carry on, the way the tray four lines below already does."
    );
}

/// The tray is the precedent, and it should stay the precedent.
#[test]
fn the_tray_still_degrades_rather_than_aborting() {
    assert!(
        lib_rs().contains("match platform::tray::install(app.handle())"),
        "the tray handles its own failure; it is what the backdrop was changed to match"
    );
}

/// Nothing new should quietly become fatal.
///
/// Counted rather than named, because the point is not *which* lines use `?` but that adding
/// one is a deliberate act. Two are expected and both are defensible: the main window missing
/// from `tauri.conf.json` is a broken build, and a window that will not show is not an app.
///
/// If this fails, do not raise the number to make it pass. Decide whether the new failure is
/// genuinely worth refusing to start over — and if it is not, log it and continue.
#[test]
fn the_setup_hook_has_only_the_two_fatal_steps_it_is_supposed_to() {
    let source = lib_rs();

    let setup = source
        .split_once(".setup(move |app| {")
        .expect("the setup hook is still spelled this way")
        .1;
    let setup = setup
        .split_once("\n        .on_window_event(")
        .map_or(setup, |(body, _)| body);

    let fatal: Vec<&str> = setup
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//"))
        .filter(|line| line.ends_with("?;") || line.ends_with("?"))
        .collect();

    assert_eq!(
        fatal.len(),
        2,
        "expected exactly two fatal steps in the setup hook, found {}: {fatal:#?}",
        fatal.len()
    );

    assert!(
        fatal.iter().any(|line| line.contains("main window missing")),
        "a missing main window is a broken build and is allowed to be fatal: {fatal:#?}"
    );
    assert!(
        fatal.iter().any(|line| line.contains("main.show()")),
        "a window that will not show is not a usable app and is allowed to be fatal: {fatal:#?}"
    );
}
