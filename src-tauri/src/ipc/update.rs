//! Checking for and installing updates. docs/06 Phase 11, docs/07 §2.3.
//!
//! ## Why the check is not automatic and not silent
//!
//! Standing rule 16 is no telemetry, and an update check is the one outbound request this app
//! makes that is not mail. It is a GET for a static JSON file on GitHub, and the only thing the
//! server can learn from it is that some IP asked — no account, no message counts, no
//! identifier of any kind. That is a small enough thing to be worth the safety of knowing about
//! a security fix, but it is not nothing, so it is a setting and the setting can be turned off.
//!
//! There is no automatic install. `dialog: false` in the config means Tauri's own prompt is
//! disabled and this app asks in its own words, in the Settings window, because an installer
//! that restarts the app while somebody is typing a message is not an acceptable trade for
//! being current.
//!
//! ## Why the signature matters more than the transport
//!
//! TLS proves the file came from GitHub. It does not prove the file is *ours* — a compromised
//! account, a mis-scoped token, or a release uploaded by anyone with write access all serve
//! over perfectly good TLS. The updater verifies a minisign signature against the public key
//! compiled into the binary, so an update nobody signed with the private key is refused no
//! matter where it came from. That key is not in this repository and must never be.
//!
//! ## Store builds
//!
//! Compiled out entirely under `--no-default-features --features store`. The Store installs its
//! own updates, and two mechanisms fighting produces duplicate installs and fails
//! certification. The commands still exist so the UI does not have to know which build it is
//! in; they report that updates are handled elsewhere.

use serde::Serialize;
use ts_rs::TS;

use super::mail::AppError;

/// What a check found.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// False in a Store build, where updates arrive through the Store.
    pub supported: bool,
    pub available: bool,
    /// The version on offer, when there is one.
    pub version: Option<String>,
    /// Release notes, as published. Shown as plain text, never as markup.
    pub notes: Option<String>,
    /// Set when the check itself failed — offline, or GitHub unreachable.
    pub error: Option<String>,
}

impl UpdateStatus {
    fn none(supported: bool) -> Self {
        Self {
            supported,
            available: false,
            version: None,
            notes: None,
            error: None,
        }
    }
}

/// Asks whether a newer version exists. Never installs anything.
#[cfg(feature = "self-update")]
#[tauri::command]
pub async fn update_check(app: tauri::AppHandle) -> Result<UpdateStatus, AppError> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(error) => {
            return Ok(UpdateStatus {
                error: Some(error.to_string()),
                ..UpdateStatus::none(true)
            })
        }
    };

    match updater.check().await {
        Ok(Some(update)) => Ok(UpdateStatus {
            supported: true,
            available: true,
            version: Some(update.version.clone()),
            notes: update.body.clone(),
            error: None,
        }),
        Ok(None) => Ok(UpdateStatus::none(true)),
        Err(error) => Ok(UpdateStatus {
            // Reported rather than raised. Being offline is not a fault, and a red banner every
            // time somebody opens Settings on a train would teach them to ignore the one that
            // matters.
            error: Some(error.to_string()),
            ..UpdateStatus::none(true)
        }),
    }
}

/// Downloads and installs, then relaunches. Only ever called from an explicit button.
#[cfg(feature = "self-update")]
#[tauri::command]
pub async fn update_install(app: tauri::AppHandle) -> Result<(), AppError> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = app.updater().map_err(|error| AppError {
        code: "updater-unavailable".into(),
        message: error.to_string(),
    })?;

    let Some(update) = updater.check().await.map_err(|error| AppError {
        code: "check-failed".into(),
        message: error.to_string(),
    })?
    else {
        return Err(AppError {
            code: "no-update".into(),
            message: "There is no update to install.".into(),
        });
    };

    // The signature is verified inside `download_and_install` before anything is run. There is
    // deliberately no path here that writes the downloaded bytes anywhere first.
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| AppError {
            code: "install-failed".into(),
            message: error.to_string(),
        })?;

    // The installer has replaced the binary; the running process is the old one.
    app.restart();
}

#[cfg(not(feature = "self-update"))]
#[tauri::command]
pub async fn update_check() -> Result<UpdateStatus, AppError> {
    Ok(UpdateStatus::none(false))
}

#[cfg(not(feature = "self-update"))]
#[tauri::command]
pub async fn update_install() -> Result<(), AppError> {
    Err(AppError {
        code: "store-build".into(),
        message: "This copy is updated through the Microsoft Store.".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_that_cannot_self_update_says_so_rather_than_failing() {
        // The UI asks one question of both builds. A Store build answering with an error would
        // put "update check failed" in front of somebody whose updates are working fine.
        let status = UpdateStatus::none(false);
        assert!(!status.supported);
        assert!(!status.available);
        assert!(status.error.is_none());
    }

    #[test]
    fn the_shipped_config_carries_no_dangerous_updater_flags() {
        // The updater plugin has three escape hatches: allow plain HTTP, accept invalid
        // certificates, accept mismatched hostnames. Each one is exactly what somebody needs to
        // test an update against a local server, and exactly what must never reach a release --
        // any of them turns "an update is signed and came from us over TLS" into "an update came
        // from whoever answered".
        //
        // They are used, deliberately, in a throwaway config passed to `tauri build --config`
        // for the update test. That is why this test exists: the mechanism to enable them is a
        // one-line edit to a JSON file that looks exactly like the real one.
        let config = std::fs::read_to_string("tauri.conf.json").expect("config");

        for flag in [
            "dangerousInsecureTransportProtocol",
            "dangerousAcceptInvalidCerts",
            "dangerousAcceptInvalidHostnames",
        ] {
            assert!(
                !config.contains(flag),
                "tauri.conf.json sets {flag}. That belongs in a test-only --config override and \
                 never in the shipped configuration."
            );
        }

        // And the endpoint is https, which is the property those flags exist to disable.
        assert!(
            config.contains("https://github.com/"),
            "the updater endpoint is not an https GitHub URL"
        );
    }

    #[test]
    fn the_public_key_is_in_the_config_and_the_private_key_is_not_in_the_repository() {
        // The whole security argument for the updater. TLS proves the file came from GitHub; the
        // signature proves it came from us. If the private key were ever committed, anyone with
        // the repository could sign an update for every installed copy.
        let config = std::fs::read_to_string("tauri.conf.json").expect("config");
        assert!(
            config.contains("\"pubkey\""),
            "the updater has no public key, so any file served would be accepted"
        );

        // A Tauri signing key carries this header — `rsign`, not minisign, which is the first
        // thing the first version of this got wrong. The second was that the key file is
        // **base64 as a whole**, so the plaintext header never appears in it and searching for
        // the plaintext found nothing. Both forms are checked, and the encoded one is derived
        // rather than pasted so the two cannot drift.
        //
        // Assembled at runtime rather than written as a literal, because otherwise this file
        // contains the string and the search finds itself — which it did, reporting the test as
        // the leak.
        use base64::Engine;

        let plain = format!("{} {}", "untrusted comment: rsign", "encrypted secret key");
        let encoded = base64::engine::general_purpose::STANDARD.encode(&plain);

        for entry in walk(std::path::Path::new(".")) {
            let Ok(text) = std::fs::read_to_string(&entry) else {
                continue;
            };

            for needle in [&plain, &encoded] {
                assert!(
                    !text.contains(needle),
                    "a private signing key is committed at {}",
                    entry.display()
                );
            }
        }
    }

    /// Every file under `dir`, skipping build output.
    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut found = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return found;
        };

        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            if name == "target" || name == "node_modules" || name == ".git" {
                continue;
            }

            if path.is_dir() {
                found.extend(walk(&path));
            } else {
                found.push(path);
            }
        }

        found
    }
}
