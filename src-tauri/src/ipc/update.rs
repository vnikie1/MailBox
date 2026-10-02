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

/// Why a check produced no answer.
///
/// ## Why a string was not enough
///
/// `error` used to be the only signal, and the UI said one thing for all of it: *"Could not
/// reach the update server. This is usually just being offline."* That sentence was wrong for
/// the case that has actually been happening on this machine — **69 failed checks and zero
/// successes since 2026-09-03** — because the server was reached, answered, and said 404: the
/// GitHub repository is public and real, and has no releases published, so
/// `releases/latest/download/latest.json` has nothing to serve.
///
/// Telling somebody they are offline when they are not is the kind of error message that sends
/// them to look at their router. The two cases are cleanly distinguishable inside the plugin —
/// a non-2xx response leaves `last_error` unset and falls through to `ReleaseNotFound`, while a
/// transport failure returns `Reqwest` — so the distinction is made here, once, and the UI
/// branches on a value instead of guessing from a sentence.
///
/// ## Why a Store build allows it to be unused
///
/// Every Store build warned that all five variants are never constructed, and it was right: with
/// the updater compiled out, `update_check` only ever answers "not supported here". The type
/// stays anyway, because it is part of `UpdateStatus` and so of the IPC contract both builds
/// share — the window is the same code in each. Allowed for that build only, so a variant left
/// unused by the self-updating build still warns where it means something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(feature = "self-update"), allow(dead_code))]
pub enum UpdateProblem {
    /// Nothing answered: no network, DNS failure, a refused connection, TLS.
    Unreachable,
    /// Something answered, and it was not a release. A repository with no releases published
    /// gives a 404 here, and that is not the user's connection.
    NoRelease,
    /// Reached and answered, and the answer could not be understood.
    Malformed,
    /// There is a release, and nothing in it for this machine.
    Unsupported,
    /// The updater is not configured in this build.
    Unavailable,
}

/// Which of those a plugin error is.
///
/// `tauri_plugin_updater::Error` is `#[non_exhaustive]`, so the catch-all is required rather
/// than lazy. It lands on `Malformed` because an error this code has never seen is, from the
/// user's side, the server having said something unusable — and that reads better than
/// asserting anything about their connection.
#[cfg(feature = "self-update")]
fn classify(error: &tauri_plugin_updater::Error) -> UpdateProblem {
    use tauri_plugin_updater::Error as E;

    match error {
        E::Reqwest(_) | E::Network(_) | E::Io(_) => UpdateProblem::Unreachable,
        E::ReleaseNotFound => UpdateProblem::NoRelease,
        E::Serialization(_) | E::Semver(_) | E::UrlParse(_) => UpdateProblem::Malformed,
        E::TargetNotFound(_) | E::TargetsNotFound(_) | E::UnsupportedArch | E::UnsupportedOs => {
            UpdateProblem::Unsupported
        }
        E::EmptyEndpoints => UpdateProblem::Unavailable,
        _ => UpdateProblem::Malformed,
    }
}

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
    /// The underlying message, kept for the log and for a report. Not what the UI says.
    pub error: Option<String>,
    /// What kind of failure it was, for the UI to say something true about.
    pub problem: Option<UpdateProblem>,
}

impl UpdateStatus {
    fn none(supported: bool) -> Self {
        Self {
            supported,
            available: false,
            version: None,
            notes: None,
            error: None,
            problem: None,
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
                problem: Some(UpdateProblem::Unavailable),
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
            problem: None,
        }),
        Ok(None) => Ok(UpdateStatus::none(true)),
        Err(error) => {
            let problem = classify(&error);

            // Logged here because the UI deliberately does not show the raw message, and
            // without this the only record of *why* a check failed is the plugin's own line,
            // which does not say which endpoint or what the app then told the user.
            tracing::warn!(%error, ?problem, "update check failed");

            Ok(UpdateStatus {
                // Reported rather than raised. Being offline is not a fault, and a red banner
                // every time somebody opens Settings on a train would teach them to ignore the
                // one that matters.
                error: Some(error.to_string()),
                problem: Some(problem),
                ..UpdateStatus::none(true)
            })
        }
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

    /// The distinction the UI was getting wrong, pinned to the two plugin errors that carry it.
    ///
    /// A GitHub repository with no releases published answers `releases/latest/download/...`
    /// with a 404. The plugin treats a non-2xx response as "no release here", leaves its
    /// `last_error` unset and falls through to `ReleaseNotFound` — so the app was reaching the
    /// server perfectly well and telling the user they were offline. That happened 69 times on
    /// the developer's machine before anybody read the log.
    #[cfg(feature = "self-update")]
    #[test]
    fn a_server_that_answered_is_not_the_same_as_no_server() {
        use tauri_plugin_updater::Error;

        assert_eq!(
            classify(&Error::ReleaseNotFound),
            UpdateProblem::NoRelease,
            "a 404 from a repository with nothing published is not a connection problem"
        );

        assert_eq!(
            classify(&Error::Network("connection refused".into())),
            UpdateProblem::Unreachable,
            "a transport failure is the case the old copy described"
        );

        assert_eq!(
            classify(&Error::EmptyEndpoints),
            UpdateProblem::Unavailable,
            "no endpoints configured is a build problem, not the user's"
        );

        assert_eq!(
            classify(&Error::UnsupportedArch),
            UpdateProblem::Unsupported,
            "a release with nothing for this machine is its own answer"
        );
    }

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
