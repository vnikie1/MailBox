//! Run at login, for the Store package. docs/07 §2.5.
//!
//! The installer build starts at sign-in through the `Run` key the autostart plugin writes.
//! Under MSIX that write lands in the package's private copy of the registry, which Windows
//! never reads at sign-in, so turning the setting on did nothing and the checkbox said it had
//! worked (M31, 2026-11-14 audit).
//!
//! A packaged app starts at sign-in through the `StartupTask` its manifest declares, which is
//! what Settings → Apps → Startup lists. The manifest declares it off; this turns it on and off.
//!
//! ## What only the user can change
//!
//! Windows keeps two states the app is not allowed to override. A task the user turned off in
//! Settings or Task Manager stays off until they turn it back on there, and a task an
//! organisation's policy controls stays as the policy says. Both are reported as what they are,
//! because a checkbox that quietly springs back says nothing about why.

use windows::core::HSTRING;
use windows::ApplicationModel::{StartupTask, StartupTaskState};

/// The `TaskId` in `msix/AppxManifest.xml`. A test holds the two together.
const TASK_ID: &str = "HalcyonStartup";

/// Why the setting could not be changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Turned off in Settings → Apps → Startup, or in Task Manager.
    ByUser,
    /// Set by the organisation that manages this PC.
    ByPolicy,
    /// Windows failed to answer.
    Failed(String),
}

impl Refusal {
    /// What the settings pane says.
    pub fn explain(&self) -> String {
        match self {
            Self::ByUser => "Halcyon was turned off in Windows' startup settings, and only you \
                             can turn it back on there: Settings, Apps, Startup."
                .into(),
            Self::ByPolicy => {
                "Your organisation decides whether Halcyon starts with Windows on this PC.".into()
            }
            Self::Failed(error) => format!("Windows would not change the startup setting: {error}"),
        }
    }
}

/// Whether a state means the app will start at sign-in.
fn starts(state: StartupTaskState) -> bool {
    state == StartupTaskState::Enabled || state == StartupTaskState::EnabledByPolicy
}

/// What a state means for a request to turn the task on (`want`) or off.
///
/// `None` when the state already says what was asked, or when the app may change it.
fn blocked(state: StartupTaskState, want: bool) -> Option<Refusal> {
    if starts(state) == want {
        return None;
    }

    if state == StartupTaskState::DisabledByUser {
        Some(Refusal::ByUser)
    } else if state == StartupTaskState::DisabledByPolicy
        || state == StartupTaskState::EnabledByPolicy
    {
        Some(Refusal::ByPolicy)
    } else {
        None
    }
}

fn task() -> Result<StartupTask, Refusal> {
    StartupTask::GetAsync(&HSTRING::from(TASK_ID))
        .and_then(|operation| operation.get())
        .map_err(|error| Refusal::Failed(error.message()))
}

/// Whether the packaged app starts at sign-in. Blocks on Windows, so not on the main thread.
pub fn is_enabled() -> Result<bool, Refusal> {
    let state = task()?
        .State()
        .map_err(|error| Refusal::Failed(error.message()))?;

    Ok(starts(state))
}

/// Turns starting at sign-in on or off. Blocks on Windows, so not on the main thread.
pub fn set(enabled: bool) -> Result<(), Refusal> {
    let task = task()?;
    let failed = |error: windows::core::Error| Refusal::Failed(error.message());

    let state = task.State().map_err(failed)?;
    if let Some(refusal) = blocked(state, enabled) {
        return Err(refusal);
    }
    if starts(state) == enabled {
        return Ok(());
    }

    if !enabled {
        return task.Disable().map_err(failed);
    }

    // A desktop app is not shown a prompt here; Windows answers with the state it is now in,
    // which can still be a refusal if the user or a policy changed it in the meantime.
    let now = task
        .RequestEnableAsync()
        .and_then(|operation| operation.get())
        .map_err(failed)?;

    match blocked(now, true) {
        Some(refusal) => Err(refusal),
        None if starts(now) => Ok(()),
        None => Err(Refusal::Failed("the startup task is still off".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_named_here_is_the_one_the_manifest_declares() {
        // A mismatch fails only in the Store build, with "the system cannot find the file",
        // long after anyone could connect it to a renamed string.
        let manifest = include_str!("../../msix/AppxManifest.xml");
        assert!(
            manifest.contains(&format!("TaskId=\"{TASK_ID}\"")),
            "msix/AppxManifest.xml declares no StartupTask with TaskId {TASK_ID}"
        );
    }

    #[test]
    fn the_manifest_declares_the_task_off() {
        // On would start Halcyon at sign-in from the moment it is installed, unasked.
        let manifest = include_str!("../../msix/AppxManifest.xml");
        assert!(manifest.contains(&format!("TaskId=\"{TASK_ID}\" Enabled=\"false\"")));
    }

    #[test]
    fn a_state_the_user_set_is_theirs_to_change() {
        assert_eq!(
            blocked(StartupTaskState::DisabledByUser, true),
            Some(Refusal::ByUser)
        );
        // Asking for what it already is is no refusal.
        assert_eq!(blocked(StartupTaskState::DisabledByUser, false), None);
    }

    #[test]
    fn a_state_a_policy_set_is_the_policys() {
        assert_eq!(
            blocked(StartupTaskState::DisabledByPolicy, true),
            Some(Refusal::ByPolicy)
        );
        assert_eq!(
            blocked(StartupTaskState::EnabledByPolicy, false),
            Some(Refusal::ByPolicy)
        );
        assert_eq!(blocked(StartupTaskState::EnabledByPolicy, true), None);
    }

    #[test]
    fn an_ordinary_state_is_the_apps_to_change() {
        assert_eq!(blocked(StartupTaskState::Disabled, true), None);
        assert_eq!(blocked(StartupTaskState::Enabled, false), None);
        assert!(starts(StartupTaskState::Enabled));
        assert!(starts(StartupTaskState::EnabledByPolicy));
        assert!(!starts(StartupTaskState::Disabled));
        assert!(!starts(StartupTaskState::DisabledByUser));
    }
}
