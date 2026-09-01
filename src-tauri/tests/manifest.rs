//! The embedded Win32 manifest is valid, and stays valid. docs/06 Phase 11.
//!
//! ## Why this is a test rather than a review comment
//!
//! `halcyon.exe.manifest` is embedded into the binary by `build.rs`, and two separate mistakes
//! in it produced the *same* distant symptom: the App Certification Kit reporting "Failed to
//! process the binary" and then "the app is not DPI Aware".
//!
//! The first was an em-dash. The manifest tooling reads the embedded resource as ANSI, so the
//! three utf-8 bytes of `—` arrive as three unrelated characters mid-document.
//!
//! The second was the obvious fix for the first: replacing each em-dash with two hyphens. A
//! double hyphen may not appear inside an XML comment, so the encoding error became a parse
//! error and looked like progress.
//!
//! Neither is visible in a diff — both are punctuation inside a comment — and neither shows up
//! until somebody runs a twenty-minute certification pass and reads a warning about DPI. That
//! is a long way to travel to be told about a dash.

const MANIFEST: &str = include_str!("../halcyon.exe.manifest");

/// The comment bodies, without their `<!--` and `-->` delimiters.
fn comment_bodies(source: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = source;

    while let Some(start) = rest.find("<!--") {
        let after = &rest[start + 4..];
        let Some(end) = after.find("-->") else {
            break;
        };
        found.push(&after[..end]);
        rest = &after[end + 3..];
    }

    found
}

#[test]
fn the_manifest_is_ascii_only() {
    // The Windows manifest tooling reads the resource as ANSI. Anything outside ASCII arrives as
    // different characters than were written, in the middle of an XML document.
    let offenders: Vec<(usize, char)> = MANIFEST
        .char_indices()
        .filter(|(_, character)| !character.is_ascii())
        .collect();

    assert!(
        offenders.is_empty(),
        "non-ASCII in halcyon.exe.manifest at {:?} - the embedded resource is read as ANSI, so \
         these will not survive: {:?}",
        offenders.iter().map(|(at, _)| *at).collect::<Vec<_>>(),
        offenders
            .iter()
            .map(|(_, character)| *character)
            .collect::<Vec<_>>()
    );
}

#[test]
fn no_comment_contains_a_double_hyphen() {
    // XML forbids `--` inside a comment. This is what the obvious fix for the encoding problem
    // introduced, and it fails one layer later with a message about XML rather than about text.
    for body in comment_bodies(MANIFEST) {
        assert!(
            !body.contains("--"),
            "an XML comment contains a double hyphen, which is not allowed: {:?}",
            body.lines()
                .find(|line| line.contains("--"))
                .unwrap_or(body)
                .trim()
        );
    }
}

#[test]
fn the_common_controls_dependency_is_declared() {
    // The one that actually mattered, and the one the other tests would have let through.
    //
    // Supplying a custom manifest REPLACES the one tauri-build writes, and that manifest exists
    // almost entirely to declare this dependency. Without it the app calls into comdlg32 and
    // comctl32 for its file dialogs, side-by-side resolution fails, and the process dies before
    // `main` with 0xC0000139 or "the side-by-side configuration is incorrect" - no window, no
    // log line, no crash report, nothing in the event log.
    //
    // It went unnoticed for hours because every launch in between happened to be under MSIX
    // package identity, where side-by-side resolution does not apply. The certification kit
    // started the packaged app two dozen times without complaint while the ordinary build could
    // not start at all.
    assert!(
        MANIFEST.contains("Microsoft.Windows.Common-Controls"),
        "the Common Controls v6 dependency is missing. A custom manifest replaces tauri-build's, \
         and without this the app cannot start outside an MSIX package - see the note at the top \
         of halcyon.exe.manifest."
    );

    // The exact identity matters: a wrong publicKeyToken resolves to nothing, which fails the
    // same silent way as omitting it.
    for part in [
        "version=\"6.0.0.0\"",
        "publicKeyToken=\"6595b64144ccf1df\"",
        "processorArchitecture=\"*\"",
    ] {
        assert!(
            MANIFEST.contains(part),
            "the Common Controls dependency is missing {part}, so it will not resolve"
        );
    }
}

#[test]
fn the_manifest_declares_what_it_is_there_to_declare() {
    // The four settings the certification kit and Windows actually read. A manifest that parses
    // but declares nothing would pass the two tests above and change nothing about the app.
    for required in [
        "PerMonitorV2",
        "<dpiAware",
        "asInvoker",
        "longPathAware",
        "UTF-8",
    ] {
        assert!(
            MANIFEST.contains(required),
            "the manifest no longer declares {required}"
        );
    }
}

#[test]
fn the_manifest_is_well_formed_enough_to_embed() {
    // Not a full XML parser — a dependency for one file would be its own bad trade — but the
    // failures that actually happen: unbalanced tags, an unterminated comment, a missing root.
    assert!(
        MANIFEST.trim_start().starts_with("<?xml"),
        "no XML declaration"
    );
    assert!(MANIFEST.contains("<assembly"), "no <assembly> root");
    assert!(
        MANIFEST.trim_end().ends_with("</assembly>"),
        "unclosed root"
    );

    assert_eq!(
        MANIFEST.matches("<!--").count(),
        MANIFEST.matches("-->").count(),
        "an XML comment is opened and not closed"
    );

    // Every element opened by name is closed. Self-closing tags are counted out first, since
    // `<supportedOS ... />` has no partner.
    let without_self_closing = MANIFEST.replace("/>", ">");

    for element in [
        "assembly",
        "trustInfo",
        "compatibility",
        "application",
        "windowsSettings",
    ] {
        let opened = opening_tags(&without_self_closing, element);
        let closed = without_self_closing
            .matches(&format!("</{element}>"))
            .count();
        assert_eq!(
            opened, closed,
            "<{element}> is opened {opened} times and closed {closed}"
        );
    }
}

/// How many times `<name>` is opened, counting `<name>` and `<name ...>` but not `<nameOther>`.
///
/// The obvious `matches("<assembly")` counts `<assemblyIdentity` too, which made this test report
/// the manifest as unbalanced when it was not. A tag name ends at whitespace or `>`.
fn opening_tags(source: &str, element: &str) -> usize {
    let needle = format!("<{element}");

    source
        .match_indices(&needle)
        .filter(|(at, _)| {
            source[at + needle.len()..]
                .chars()
                .next()
                .is_some_and(|next| next.is_whitespace() || next == '>')
        })
        .count()
}
