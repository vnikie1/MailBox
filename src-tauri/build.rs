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

use std::path::Path;

fn main() {
    sync_optional_capability(
        "self-update",
        Path::new("capabilities-optional/updater.json"),
        Path::new("capabilities/updater.json"),
    );

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
