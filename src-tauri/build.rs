//! Build script.
//!
//! Beyond `tauri_build::build()`, one job: deciding whether the updater's capability file
//! exists at all.
//!
//! ## Why this is not a config setting
//!
//! Tauri validates **every** file in `capabilities/` against the compiled-in plugins, not just
//! the ones `app.security.capabilities` selects. A Store build has no updater plugin, so a
//! capability naming `updater:default` is a hard build failure however carefully the config
//! deselects it. Both were tried; the folder scan wins.
//!
//! So the file has to be absent, and the only place that can know is here — `CARGO_FEATURE_*`
//! is set for the build script and nowhere else.
//!
//! ## Why the source of truth lives in another directory
//!
//! `capabilities-optional/updater.json` is the reviewable file: in version control, next to its
//! siblings, diffed like any other. `capabilities/updater.json` is a copy this script makes, and
//! is ignored by git. Generating the *content* here instead would put a security-relevant
//! permission grant inside a build script, where nobody reviewing the capability surface would
//! think to look — and `capabilities/default.json` staying honest about that surface is a thing
//! this project has already had to fix once.

use std::collections::HashMap;
use std::path::Path;

fn main() {
    sync_optional_capability(
        "self-update",
        Path::new("capabilities-optional/updater.json"),
        Path::new("capabilities/updater.json"),
    );

    inject_oauth_clients(Path::new("oauth"));

    // The icon the resource compiler puts in the exe — what Explorer, the Start menu and the
    // taskbar show. `tauri_build` reads it and does not tell Cargo, and the `rerun-if-changed`
    // lines above switch off Cargo's default of rerunning on any change in the package. So a new
    // `icon.ico` reached nothing until something else reran this script: the first three builds
    // after the 2026-09-17 icon set still carried the old icon, found by extracting it from the
    // installed exe. Rerunning here also recompiles the crate, which is what brings
    // `generate_context!`'s copy — the window icon — up to date with it.
    println!("cargo:rerun-if-changed=icons/icon.ico");

    // The embedded Win32 manifest. Supplied rather than left to the default, because the
    // default carries no DPI awareness and no execution level — see halcyon.exe.manifest for
    // what each element is doing and which App Certification Kit findings prompted it.
    let attributes = tauri_build::Attributes::new().windows_attributes(
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("halcyon.exe.manifest")),
    );

    tauri_build::try_build(attributes).expect("failed to run tauri-build")
}

/// Copies an optional capability into place when its feature is on, and removes it when it is not.
///
/// Both directions matter. Without the removal, switching from a normal build to a Store build in
/// the same working tree leaves the file behind and the Store build fails with an error about a
/// permission rather than about the leftover — which is a confusing half-hour.
fn sync_optional_capability(feature: &str, source: &Path, destination: &Path) {
    println!("cargo:rerun-if-changed={}", source.display());

    let enabled = std::env::var_os(format!(
        "CARGO_FEATURE_{}",
        feature.to_uppercase().replace('-', "_")
    ))
    .is_some();

    if enabled {
        let contents = std::fs::read(source)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", source.display()));

        // Written only when it differs, so an unchanged build does not touch a file that
        // `cargo:rerun-if-changed=capabilities` is watching — which would rebuild every time.
        if std::fs::read(destination).ok().as_deref() != Some(contents.as_slice()) {
            std::fs::write(destination, &contents)
                .unwrap_or_else(|error| panic!("cannot write {}: {error}", destination.display()));
        }
    } else if destination.exists() {
        std::fs::remove_file(destination)
            .unwrap_or_else(|error| panic!("cannot remove {}: {error}", destination.display()));
    }
}

/// The sign-in applications a build may carry, and where each comes from.
///
/// ## Why they are build inputs
///
/// Google and Microsoft only let a registered application sign anyone in, and a build with none
/// shows every new user a note telling them to go and register one. So the developer's own
/// builds carry theirs. The source is public, though, and a secret in public source is not a
/// secret (docs/05 §9) — so the values are never in the repository. They come from the
/// environment, or from `oauth/clients.env`, which is gitignored; the environment wins, so CI
/// can supply them without a file. A build with neither behaves exactly as a build from a clean
/// checkout should: no built-in client, and the bring-your-own panel in Settings.
///
/// The same arrangement the updater's signing key already uses.
///
/// ## What is watched, and why it changes with the file
///
/// Cargo treats a `rerun-if-changed` path that does not exist as stale on *every* build. On any
/// machine without `clients.env` — every clean checkout, every CI run — watching the file would
/// rerun this script and recompile the whole crate each time. So when the file is absent, the
/// **directory** is watched instead: it always exists, its README and example being committed,
/// and a directory is watched recursively, so creating `clients.env` triggers exactly one
/// rebuild.
///
/// Once the file exists, only the file is watched. Watching the directory then as well would
/// recompile the crate for an edit to the README beside it, which is a five-minute release build
/// for a documentation change. Deleting the file is still noticed: a watched file that has gone
/// missing is stale, which reruns this script, which then watches the directory again.
fn inject_oauth_clients(dir: &Path) {
    let path = dir.join("clients.env");

    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    } else {
        println!("cargo:rerun-if-changed={}", dir.display());
    }

    let file = read_env_file(&path);

    let value = |name: &str| -> Option<String> {
        println!("cargo:rerun-if-env-changed={name}");

        // An empty variable is treated as unset rather than as "override the file with
        // nothing", which is what `set NAME=` on Windows actually leaves behind.
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| file.get(name).cloned())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };

    let google_id = value("HALCYON_GOOGLE_CLIENT_ID");
    let google_secret = value("HALCYON_GOOGLE_CLIENT_SECRET");
    let microsoft_id = value("HALCYON_MICROSOFT_CLIENT_ID");

    // Google needs both halves or neither. An id alone would light the Google tile up, send the
    // user through a whole browser consent, and fail at the token exchange — so it is not
    // compiled in at all, and the build says why. The secret itself is never printed.
    match (google_id, google_secret) {
        (Some(id), Some(secret)) => {
            if !id.ends_with(".apps.googleusercontent.com") {
                warn(&format!(
                    "HALCYON_GOOGLE_CLIENT_ID is `{id}`, which does not end in \
                     .apps.googleusercontent.com — check it is the client ID and not the \
                     project number or the secret"
                ));
            }
            emit("HALCYON_GOOGLE_CLIENT_ID", &id);
            emit("HALCYON_GOOGLE_CLIENT_SECRET", &secret);
        }
        (Some(_), None) => warn(
            "HALCYON_GOOGLE_CLIENT_ID is set but HALCYON_GOOGLE_CLIENT_SECRET is not, so no \
             Google client was built in. Google refuses a Desktop client without its secret; \
             add it to src-tauri/oauth/clients.env.",
        ),
        (None, Some(_)) => warn(
            "HALCYON_GOOGLE_CLIENT_SECRET is set without HALCYON_GOOGLE_CLIENT_ID, so it was \
             ignored.",
        ),
        (None, None) => {}
    }

    if let Some(id) = microsoft_id {
        if !looks_like_guid(&id) {
            warn(&format!(
                "HALCYON_MICROSOFT_CLIENT_ID is `{id}`, which is not shaped like the \
                 Application (client) ID GUID Entra shows on the app's Overview page"
            ));
        }
        emit("HALCYON_MICROSOFT_CLIENT_ID", &id);
    }
}

/// Hands a value to the crate, where `option_env!` picks it up.
fn emit(name: &str, value: &str) {
    // A newline would end the directive early and turn the rest of the value into a second,
    // unintended one. Nothing legitimate here contains whitespace at all.
    if value.chars().any(char::is_whitespace) {
        warn(&format!("{name} contains whitespace and was not built in"));
        return;
    }

    println!("cargo:rustc-env={name}={value}");
}

fn warn(message: &str) {
    println!("cargo:warning={message}");
}

/// `NAME=value` lines, `#` comments, optional quotes. Missing file is an empty map.
fn read_env_file(path: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };

    // Notepad has, at various points, saved UTF-8 with a byte-order mark. Left in, it becomes
    // part of the first variable's *name*, and that variable silently never matches.
    text.trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.trim().to_string(), unquote(value.trim()).to_string()))
        .collect()
}

fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, hex. Only used to warn, never to refuse.
fn looks_like_guid(value: &str) -> bool {
    let groups: Vec<&str> = value.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(group, len)| group.len() == len && group.chars().all(|c| c.is_ascii_hexdigit()))
}
