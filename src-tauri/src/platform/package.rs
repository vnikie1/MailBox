//! Whether this process is the Microsoft Store package, and which app Windows thinks it is.
//!
//! ## Why it is asked at run time rather than decided by the `store` feature
//!
//! The answer that matters is Windows', not the build's. Two things depend on it, and both go
//! wrong without a sound in the build that is not tested on a clean machine:
//!
//! - **Toasts** (H22, 2026-11-14 audit). A toast is addressed to an AppUserModelID. The installer
//!   build is `com.uniki.halcyon`, the name its Start Menu shortcut registers. The Store package
//!   is `<package family name>!Halcyon`, which Windows assigns, and it registers no shortcut
//!   under the other name. A Store build addressing toasts to `com.uniki.halcyon` showed them
//!   only on a computer where the installer build had once been installed, which is this
//!   developer's and nobody else's.
//! - **Run at login** (M31). A packaged app starts at sign-in through the `StartupTask` its
//!   manifest declares. The `Run` key the installer build writes is redirected into the
//!   package's private registry under MSIX, where nothing reads it.
//!
//! A process launched from the package has an identity Windows can report; one launched any
//! other way has none. That is exact, where a feature flag is only what the build intended.

use std::sync::OnceLock;

use windows::core::PWSTR;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
use windows::Win32::Storage::Packaging::Appx::GetCurrentApplicationUserModelId;

/// The AppUserModelID Windows assigned this process from its package, or `None` when it is not
/// running from one: the installer build, or `npm run app:dev`.
///
/// Read once. A process cannot gain or lose a package while it runs.
pub fn app_user_model_id() -> Option<&'static str> {
    static ID: OnceLock<Option<String>> = OnceLock::new();
    ID.get_or_init(read).as_deref()
}

/// Whether this is the Store package.
pub fn is_packaged() -> bool {
    app_user_model_id().is_some()
}

fn read() -> Option<String> {
    let mut length = 0u32;

    // Asked for the length first. Without a package this fails with
    // APPMODEL_ERROR_NO_APPLICATION instead, which is the answer rather than an error.
    let first = unsafe { GetCurrentApplicationUserModelId(&mut length, None) };
    if first != ERROR_INSUFFICIENT_BUFFER || length == 0 {
        return None;
    }

    let mut buffer = vec![0u16; length as usize];
    let second =
        unsafe { GetCurrentApplicationUserModelId(&mut length, Some(PWSTR(buffer.as_mut_ptr()))) };
    if second != ERROR_SUCCESS {
        return None;
    }

    // The length counts the terminating NUL.
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    let id = String::from_utf16(&buffer[..end]).ok()?;

    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_not_started_from_the_package_has_no_package_app_id() {
        // The test runner is an ordinary executable. If this ever said otherwise, every
        // installer build would address its toasts to an app ID it does not have.
        assert_eq!(app_user_model_id(), None);
        assert!(!is_packaged());
    }
}
