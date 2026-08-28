//! Send and receive sounds. docs/06 Phase 10 — "off by default".
//!
//! ## Why there is no audio file in this repository
//!
//! Both sounds are Windows' own. The receive sound is `Sound::Mail`, which the toast asks the
//! shell for by name, so it is whatever the user has chosen under Settings → System → Sound →
//! Advanced → New Mail Notification — including a custom one, and including silence. The send
//! sound is a file that ships with Windows, played from `%WINDIR%\Media`.
//!
//! Bundling audio would have been easier and worse: an app that plays its own sound at its own
//! volume, ignoring the theme the rest of the desktop follows, is the thing people mute.
//!
//! ## Why send is a file and receive is not
//!
//! Windows has a registered sound event for new mail and none for sending. `MailBeep` is the
//! only mail alias the shell knows. Reusing it for send would mean both halves of a
//! conversation make the same noise, which is worse than the asymmetry — so send plays
//! `Windows Notify Messaging.wav` directly, and if a given install does not have it, nothing
//! happens.

use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT};

use crate::db::Db;

/// The file Windows ships for a sent message.
///
/// Resolved through `%WINDIR%` rather than hard-coded to `C:\Windows`, because that is not
/// where Windows is on every machine and a missing file here is silent by design — which would
/// make the wrong path impossible to notice.
/// Takes the Windows directory rather than reading it, so the absent case can be tested
/// without mutating the environment — `remove_var` is process-global, and cargo runs tests on
/// several threads, so a test that unset it could fail a different test that was reading it.
fn sent_sound_in(windows: &std::path::Path) -> Option<std::path::PathBuf> {
    let path = windows.join("Media").join("Windows Notify Messaging.wav");

    path.exists().then_some(path)
}

fn sent_sound_path() -> Option<std::path::PathBuf> {
    sent_sound_in(std::path::Path::new(&std::env::var_os("WINDIR")?))
}

/// Plays the sent sound, if the user has sounds on.
///
/// Fire and forget: `SND_ASYNC` returns immediately, so a send is never waiting on audio, and
/// `SND_NODEFAULT` means a missing file plays nothing rather than the system beep. A beep on a
/// successful send would read as an error, which is the opposite of what it is for.
pub fn play_sent() {
    let Some(path) = sent_sound_path() else {
        return;
    };

    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let _ = PlaySoundW(
            PCWSTR(wide.as_ptr()),
            None,
            SND_FILENAME | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

/// Plays the sent sound if this account has sounds enabled.
///
/// The preference is read here rather than by the caller so that "should this make a noise" has
/// exactly one answer, in the same place for both sounds.
#[tauri::command]
pub async fn sound_sent(db: tauri::State<'_, Db>, account_id: i64) -> Result<(), String> {
    let enabled = db
        .read(move |conn| super::notify::prefs_for(conn, account_id))
        .await
        .map(|prefs| prefs.sound)
        .unwrap_or(false);

    if enabled {
        play_sent();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sent_sound_is_looked_up_under_windir() {
        // Not asserted to exist: a stripped install may not have it, and the code handles that.
        // What is asserted is that the lookup is relative to WINDIR rather than a fixed C:\ —
        // the failure this guards against is invisible, because a missing file plays nothing.
        if let Some(path) = sent_sound_path() {
            let windows = std::env::var("WINDIR").expect("WINDIR");

            assert!(
                path.starts_with(&windows),
                "{} is not under {windows}",
                path.display()
            );
            assert_eq!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("wav")
            );
        }
    }

    #[test]
    fn a_directory_without_the_sound_yields_nothing() {
        // The function is called on every successful send. Returning a path that is not there
        // would hand a bad filename to PlaySound on every message.
        assert!(sent_sound_in(std::path::Path::new(r"Z:\definitely-not-windows")).is_none());
    }
}
