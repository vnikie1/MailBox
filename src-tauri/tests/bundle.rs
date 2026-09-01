//! Nothing but the app itself gets built into a release, and therefore shipped. docs/06 Phase 11.
//!
//! ## What went wrong
//!
//! Every `.rs` file in `src/bin/` is auto-discovered by cargo as a binary of this package. A
//! release build produces all of them in `target/release`, and the NSIS bundler picks some up
//! from there. A clean install of 1.0.0 -- performed after a full uninstall, so nothing was
//! stale -- put these in `%LOCALAPPDATA%\Halcyon`:
//!
//!     crashgate.exe     306,176
//!     halcyon.exe    12,123,136
//!     seed.exe        1,402,880
//!     uninstall.exe      82,108
//!
//! `seed.exe` writes fabricated mail into the user's database. `crashgate.exe` crashes the app on
//! purpose. Neither is an attack, and both are inert unless run, but they are development
//! scaffolding sitting in a shipped product and one of them can corrupt somebody's mailbox.
//!
//! A comment in `Cargo.toml` had asserted for months that "Tauri bundles only the productName
//! binary". It was wrong, and being written down made it less likely anybody would check.
//!
//! ## What this test actually guarantees
//!
//! Not much on its own, and it is worth being honest about that: it reads `Cargo.toml` and checks
//! the two things that keep the dev tools out of a release build -- `autobins = false`, so a new
//! file in `src/bin/` is not silently picked up, and `required-features` on every binary that is
//! not the app, so none of them is built unless somebody asks. It cannot see inside an installer.
//!
//! What it does do is fail in one second, on the change that would reintroduce the problem,
//! instead of a month later on somebody's machine.

const CARGO_TOML: &str = include_str!("../Cargo.toml");

/// The `[[bin]]` sections, as (name, has_required_features).
fn binaries(source: &str) -> Vec<(String, bool)> {
    let mut found = Vec::new();
    let mut current: Option<(String, bool)> = None;
    let mut in_bin = false;

    for line in source.lines() {
        let line = line.trim();

        if line.starts_with('[') {
            // A new section ends the one before it.
            if let Some(entry) = current.take() {
                found.push(entry);
            }
            in_bin = line == "[[bin]]";
            continue;
        }

        if !in_bin {
            continue;
        }

        if let Some(rest) = line.strip_prefix("name = ") {
            current = Some((rest.trim_matches('"').to_string(), false));
        } else if line.starts_with("required-features") {
            if let Some(entry) = current.as_mut() {
                entry.1 = true;
            }
        }
    }

    if let Some(entry) = current {
        found.push(entry);
    }

    found
}

#[test]
fn binaries_are_declared_rather_than_discovered() {
    // Without this, adding a file to src/bin/ ships it. The failure is silent in every way that
    // matters: the build succeeds, the tests pass, and the extra binary only appears if somebody
    // lists the contents of an installed copy.
    assert!(
        CARGO_TOML.contains("autobins = false"),
        "autobins is not disabled, so every file in src/bin/ is built into target/release and \
         can be picked up by the bundler"
    );
}

#[test]
fn only_the_app_is_built_without_asking() {
    let binaries = binaries(CARGO_TOML);

    assert!(
        binaries.iter().any(|(name, _)| name == "halcyon"),
        "the app itself is no longer declared as a [[bin]]; with autobins = false that means it \
         does not get built at all"
    );

    for (name, gated) in &binaries {
        if name == "halcyon" {
            assert!(
                !gated,
                "the app is behind required-features, so an ordinary build produces no app"
            );
            continue;
        }

        assert!(
            gated,
            "the binary {name} has no required-features, so a release build produces \
             target/release/{name}.exe and the NSIS bundler can ship it to users"
        );
    }
}

#[test]
fn every_file_in_src_bin_is_accounted_for() {
    // The declarations above are only worth something if they cover everything actually present.
    // A new tool added to src/bin/ without a matching entry would not be built -- which is safe --
    // but the author would find that confusing, so say so plainly rather than let it be a mystery.
    let declared = binaries(CARGO_TOML);

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin");
    let entries = std::fs::read_dir(&dir).expect("src/bin should exist");

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("a file name")
            .to_string();

        assert!(
            declared.iter().any(|(name, _)| *name == stem),
            "src/bin/{stem}.rs has no [[bin]] entry in Cargo.toml. With autobins = false it will \
             never be built -- add one with required-features = [\"devtools\"]"
        );
    }
}
