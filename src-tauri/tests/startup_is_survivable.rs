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
/// one is a deliberate act.
///
/// ## The first version of this test under-counted
///
/// It matched lines *ending* in `?`, which misses a `?` in the middle of an expression — and
/// the most consequential fatal step in the hook is exactly that shape:
///
/// ```ignore
/// app.manage(db::Db::open(&path)?);
/// ```
///
/// So it reported two fatal steps when there were three, and the one it could not see was the
/// one most likely to fire on a real machine: a corrupt or unreadable `halcyon.db` after a
/// power cut, a half-restored backup, or a file an antivirus has taken away. Counting `?`
/// anywhere in the line finds all of them.
///
/// That third one has since been given a dialog instead of a panic — see the test below — so
/// the count is back to two. The change to how they are counted stays, because the next one
/// added will not necessarily sit at the end of a line either.
///
/// If this fails, do not raise the number to make it pass. Decide whether the new failure is
/// genuinely worth refusing to start over — and if it is not, log it and carry on.
#[test]
fn the_setup_hook_has_only_the_fatal_steps_it_is_supposed_to() {
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
        .filter(|line| !line.starts_with("//") && !line.starts_with("///"))
        // Anywhere in the line, not just at the end: `app.manage(Db::open(&path)?)` is a fatal
        // step and does not end in `?`.
        .filter(|line| line.contains('?'))
        .collect();

    assert_eq!(
        fatal.len(),
        2,
        "expected exactly two fatal steps in the setup hook, found {}: {fatal:#?}",
        fatal.len()
    );

    assert!(
        fatal
            .iter()
            .any(|line| line.contains("main window missing")),
        "a missing main window is a broken build and is allowed to be fatal: {fatal:#?}"
    );
    assert!(
        fatal.iter().any(|line| line.contains("main.show()")),
        "a window that will not show is not a usable app and is allowed to be fatal: {fatal:#?}"
    );
}

/// A store that will not open must say so rather than vanishing.
///
/// This is the likeliest startup failure a real user will ever meet — a truncated write after a
/// power cut, a half-restored backup, a file an antivirus has quarantined — and it is not
/// survivable: a mail client with no store has nothing to show. So the requirement is not that
/// it carries on, but that it **explains itself** before it stops.
///
/// A `?` here would be a panic, and a panic during setup is a process that exits before
/// painting anything: the user double-clicks the icon and nothing happens, with the explanation
/// written to a crash report only a working installation can display.
#[test]
fn a_store_that_will_not_open_tells_the_user_before_exiting() {
    let source = lib_rs();

    assert!(
        !source.contains("db::Db::open(&path)?"),
        "a question mark on the store open is a silent exit; match on it and call \
         platform::fatal::cannot_start so the user is told why"
    );

    assert!(
        source.contains("platform::fatal::cannot_start"),
        "the store open should report through the fatal dialog"
    );

    // The dialog is worth nothing if it does not say where to look next.
    let fatal_src = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/platform/fatal.rs"
    ))
    .expect("fatal.rs is readable");

    assert!(
        fatal_src.contains("The log is in:"),
        "the dialog should name the diagnostics folder, or the user has been told there is a \
         problem and nothing they can do about it"
    );
}

/// The release profile must not defeat the one `catch_unwind` in the codebase.
///
/// `search/extract.rs` runs the PDF and DOCX parsers inside `catch_unwind` so that a parser
/// panicking on a malformed attachment costs one unindexed attachment rather than the process.
/// `panic = "abort"` in the release profile makes that guard inert — the panic runtime aborts
/// before anything unwinds — and it is inert *only in release*, because debug builds unwind by
/// default. Every test of that path passed while the protection did nothing in the build that
/// ships.
///
/// The input is an email attachment, so this is untrusted input taking down a mail client on a
/// machine where the user did nothing but receive a message.
#[test]
fn release_builds_can_still_catch_a_panicking_parser() {
    let manifest = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("Cargo.toml is readable");

    let release = manifest
        .split_once("[profile.release]")
        .expect("there is a release profile")
        .1;
    let release = release
        .split_once("\n[")
        .map_or(release, |(section, _)| section);

    // Settings only. The comment in that profile explains why `panic = "abort"` is absent and
    // therefore contains the string — a plain substring search over the section matches its own
    // documentation and reports the fix as the bug.
    let sets_abort = release
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .any(|line| line.replace(' ', "") == "panic=\"abort\"");

    assert!(
        !sets_abort,
        "the release profile sets panic = \"abort\", which makes the catch_unwind in \
         search/extract.rs do nothing. A malformed attachment would abort the app."
    );

    let extract = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/search/extract.rs"
    ))
    .expect("extract.rs is readable");

    assert!(
        extract.contains("catch_unwind"),
        "the guard this protects has gone; if attachment extraction no longer needs to catch a \
         panic, delete this test deliberately rather than leaving it passing for the wrong reason"
    );
}
