//! Keeping a remembered window on a screen that still exists. docs/06 Phase 11.
//!
//! ## The bug this exists for
//!
//! `tauri-plugin-window-state` remembers where the window was and restores it next launch.
//! When the display it was on has gone — a laptop undocked, a monitor unplugged, a resolution
//! changed — the remembered rectangle is somewhere no screen is, and WebView2 refuses to
//! create a webview for it: `0x80070057, The parameter is incorrect`. Tauri's setup hook then
//! panics with "the underlying handle is not available" and the process exits 101.
//!
//! **The app does not start, and there is nothing the user can do about it.** No window
//! appears, so there is no setting to change; the only cure is deleting a JSON file in
//! `%APPDATA%` that nobody knows the name of. Docking and undocking a laptop is not an unusual
//! thing to do.
//!
//! ## How it was found, which is worth recording
//!
//! It was not the bug being chased. The app had started failing with that exact HRESULT, the
//! remembered window was 2800×1800, the desktop measured 1920×1080, and the inference was
//! obvious and wrong — the real cause was that **the machine was locked**, and a locked session
//! reports the lock screen's metrics rather than the user's displays. Deleting the state file
//! entirely and still failing is what ruled geometry out.
//!
//! The fix stayed because the failure mode it describes is real on its own: a window remembered
//! from a monitor that has since been unplugged genuinely does stop the app starting. What the
//! episode changed is [`on_the_users_desktop`], without which this code corrupts the very thing
//! it protects.
//!
//! ## What it does
//!
//! Runs before Tauri builds, reads the state file, and clamps every remembered window into the
//! virtual screen. Clamped rather than discarded, so a window that is merely half off the edge
//! comes back where the user left it instead of jumping to the middle.

use std::path::{Path, PathBuf};

/// A remembered window rectangle, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// The area a window may occupy: the union of every display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// How much of a window has to remain on screen for it to count as reachable.
///
/// Not the whole window: dragging one mostly off the right edge is something people do on
/// purpose, and restoring it snapped back would be its own annoyance. Enough of the title bar
/// to grab, which is what "reachable" means in practice.
const MIN_VISIBLE: u32 = 120;

/// The smallest window worth restoring.
///
/// A remembered size of zero — which a crash mid-resize can leave behind — is as unusable as
/// one off-screen, and WebView2 rejects it for the same reason.
const MIN_SIZE: u32 = 200;

/// Brings a remembered rectangle back onto the screen.
///
/// Returns `None` when nothing needed changing, so the caller can leave the file alone in the
/// ordinary case and only rewrite it when it was actually wrong.
pub fn clamp(window: Rect, screen: Screen) -> Option<Rect> {
    let mut fixed = window;

    // Size first: a window larger than the screen cannot then be positioned onto it.
    fixed.width = fixed.width.clamp(MIN_SIZE, screen.width.max(MIN_SIZE));
    fixed.height = fixed.height.clamp(MIN_SIZE, screen.height.max(MIN_SIZE));

    // Whether the size had to change is what separates the two cases, and they want opposite
    // treatment:
    //
    // * A window that still fits was *put* where it is. Parking one half off the right edge is
    //   something people do deliberately, and Windows restores such windows exactly as they
    //   were. Only being unreachable is a problem, so all that is enforced is a grabbable strip.
    // * A window that had to be shrunk was remembered from a display that is gone. Its position
    //   is a leftover from a coordinate space that no longer exists and means nothing here, so
    //   it is placed fully on screen.
    let resized = fixed.width != window.width || fixed.height != window.height;

    let right = screen.x.saturating_add(screen.width as i32);
    let bottom = screen.y.saturating_add(screen.height as i32);

    // Compared against the window's own size where it is smaller than the margin, so a small
    // window is not required to show more of itself than it has.
    let visible_x = if resized {
        fixed.width
    } else {
        MIN_VISIBLE.min(fixed.width)
    } as i32;
    let visible_y = if resized {
        fixed.height
    } else {
        MIN_VISIBLE.min(fixed.height)
    } as i32;

    let lowest_x = screen.x.saturating_sub(fixed.width as i32 - visible_x);
    let highest_x = right.saturating_sub(visible_x);
    fixed.x = fixed
        .x
        .clamp(lowest_x.min(highest_x), highest_x.max(lowest_x));

    // Never above the top of the screen: a title bar off the top cannot be dragged back, which
    // is worse than one off the side.
    let highest_y = bottom.saturating_sub(visible_y);
    fixed.y = fixed.y.clamp(screen.y, highest_y.max(screen.y));

    if fixed == window {
        None
    } else {
        Some(fixed)
    }
}

/// The virtual screen: every display together, as Windows reports it.
#[cfg(windows)]
pub fn virtual_screen() -> Screen {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    let (x, y, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };

    // Zero would mean "no displays", which cannot be true of a process that is drawing. Falling
    // back to a plausible screen keeps the clamp from shrinking every window to nothing on a
    // machine whose metrics are momentarily unavailable — during a display change, for one.
    if width <= 0 || height <= 0 {
        return Screen {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
    }

    Screen {
        x,
        y,
        width: width as u32,
        height: height as u32,
    }
}

/// Where `tauri-plugin-window-state` keeps its file.
pub fn state_path(app_data: &Path) -> PathBuf {
    app_data.join(".window-state.json")
}

/// The state file for this app, computed the way the plugin computes it.
///
/// `%APPDATA%`, not `%LOCALAPPDATA%` — the plugin uses the *config* directory while
/// `db::default_path` uses the data one, and they are different folders on Windows. Derived
/// here rather than asked of Tauri because this runs before the app exists.
pub fn default_state_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    state_path(&base.join("com.uniki.halcyon"))
}

/// Whether the user's own desktop is the one receiving input.
///
/// False when the machine is locked, when a UAC prompt is up, or on any other secure desktop.
/// `OpenInputDesktop` fails in exactly those cases, which is the standard way to ask.
///
/// **This guard is not incidental.** While the session is locked Windows reports the lock
/// screen's display metrics, not the user's — on this machine a 2800×1800 window with a real
/// monitor behind it read as a 1920×1080 desktop. Sanitising against those numbers shrinks a
/// perfectly good window every time the app is launched on a locked session, which is a way of
/// corrupting the user's setup while trying to protect it. It was doing exactly that before
/// this check existed.
#[cfg(windows)]
pub fn on_the_users_desktop() -> bool {
    use windows::Win32::System::StationsAndDesktops::{
        CloseDesktop, OpenInputDesktop, DESKTOP_READOBJECTS,
    };

    unsafe {
        match OpenInputDesktop(Default::default(), false, DESKTOP_READOBJECTS) {
            Ok(desktop) => {
                let _ = CloseDesktop(desktop);
                true
            }
            Err(_) => false,
        }
    }
}

/// Reads the state file, clamps every window in it, and writes it back if anything moved.
///
/// Everything is best-effort. A state file that cannot be read or parsed is left alone and the
/// app starts with its default geometry, which is the outcome this exists to guarantee.
pub fn sanitise(path: &Path, screen: Screen) -> usize {
    let Ok(text) = std::fs::read_to_string(path) else {
        return 0;
    };

    let Ok(mut state) = serde_json::from_str::<serde_json::Value>(&text) else {
        tracing::warn!(
            ?path,
            "the window state file is not valid JSON; ignoring it"
        );
        return 0;
    };

    let Some(windows) = state.as_object_mut() else {
        return 0;
    };

    let mut corrected = 0usize;

    for (label, value) in windows.iter_mut() {
        let Some(entry) = value.as_object_mut() else {
            continue;
        };

        let read = |key: &str| entry.get(key).and_then(serde_json::Value::as_i64);

        let (Some(x), Some(y), Some(width), Some(height)) =
            (read("x"), read("y"), read("width"), read("height"))
        else {
            continue;
        };

        let window = Rect {
            x: x as i32,
            y: y as i32,
            width: width.max(0) as u32,
            height: height.max(0) as u32,
        };

        let Some(fixed) = clamp(window, screen) else {
            continue;
        };

        tracing::info!(
            %label,
            from = ?window,
            to = ?fixed,
            "the remembered window is not on any current display; bringing it back"
        );

        entry.insert("x".into(), fixed.x.into());
        entry.insert("y".into(), fixed.y.into());
        entry.insert("width".into(), fixed.width.into());
        entry.insert("height".into(), fixed.height.into());
        corrected += 1;
    }

    if corrected > 0 {
        if let Ok(text) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(path, text);
        }
    }

    corrected
}

#[cfg(test)]
mod tests {
    use super::*;

    const HD: Screen = Screen {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn a_window_that_fits_is_left_alone() {
        // The ordinary case, and it must not rewrite the file: a clamp that always "corrects"
        // would move the window a little on every launch.
        let window = Rect {
            x: 100,
            y: 100,
            width: 1200,
            height: 800,
        };

        assert_eq!(clamp(window, HD), None);
    }

    #[test]
    fn a_window_from_a_larger_display_is_brought_back() {
        // The bug. 2800x1800 at (251,55) remembered from a big monitor, restored on a 1920x1080
        // laptop screen: WebView2 refuses the rectangle and the app does not start at all.
        let window = Rect {
            x: 251,
            y: 55,
            width: 2800,
            height: 1800,
        };

        let fixed = clamp(window, HD).expect("this window is off-screen");

        assert!(fixed.width <= HD.width, "{fixed:?}");
        assert!(fixed.height <= HD.height, "{fixed:?}");
        assert!(fixed.x >= 0 && fixed.y >= 0, "{fixed:?}");
        assert!(fixed.x + fixed.width as i32 <= HD.width as i32, "{fixed:?}");
    }

    #[test]
    fn a_window_off_the_right_edge_keeps_a_grabbable_strip() {
        let window = Rect {
            x: 5000,
            y: 100,
            width: 800,
            height: 600,
        };

        let fixed = clamp(window, HD).expect("off screen");
        assert!(fixed.x < HD.width as i32, "{fixed:?}");
        assert!(
            fixed.x + fixed.width as i32 > 0,
            "the window is entirely off the left: {fixed:?}"
        );
    }

    #[test]
    fn a_window_above_the_top_comes_down() {
        // A title bar off the top of the screen cannot be dragged back, which is worse than one
        // off the side — there is no way to reach it with the mouse at all.
        let window = Rect {
            x: 100,
            y: -900,
            width: 800,
            height: 600,
        };

        let fixed = clamp(window, HD).expect("off screen");
        assert!(fixed.y >= 0, "{fixed:?}");
    }

    #[test]
    fn a_window_slightly_off_the_edge_is_not_snapped_to_the_middle() {
        // People park windows half off the side on purpose. Restoring them centred would be an
        // annoyance of its own, so only the unreachable ones move.
        let window = Rect {
            x: 1800,
            y: 100,
            width: 800,
            height: 600,
        };

        assert_eq!(clamp(window, HD), None, "a mostly-visible window moved");
    }

    #[test]
    fn a_zero_sized_window_is_given_a_usable_size() {
        // What a crash mid-resize can leave behind. WebView2 rejects it exactly as it rejects
        // an off-screen one.
        let window = Rect {
            x: 100,
            y: 100,
            width: 0,
            height: 0,
        };

        let fixed = clamp(window, HD).expect("unusable");
        assert!(
            fixed.width >= MIN_SIZE && fixed.height >= MIN_SIZE,
            "{fixed:?}"
        );
    }

    #[test]
    fn a_second_monitor_left_of_the_primary_is_a_valid_place_to_be() {
        // The virtual screen can start at a negative x. Treating 0 as the left edge would drag
        // every window off the secondary display on a very common desktop setup.
        let wide = Screen {
            x: -1920,
            y: 0,
            width: 3840,
            height: 1080,
        };
        let window = Rect {
            x: -1800,
            y: 50,
            width: 1000,
            height: 700,
        };

        assert_eq!(clamp(window, wide), None);
    }

    #[test]
    fn the_state_file_is_rewritten_only_when_something_was_wrong() {
        let dir = std::env::temp_dir().join("halcyon-window-state");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dirs");

        let path = state_path(&dir);
        std::fs::write(
            &path,
            r#"{"main":{"width":2800,"height":1800,"x":251,"y":55,"maximized":false}}"#,
        )
        .expect("write");

        assert_eq!(sanitise(&path, HD), 1);
        // And running again changes nothing, because the first pass fixed it.
        assert_eq!(sanitise(&path, HD), 0);

        // The keys the plugin cares about that this does not touch have to survive.
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("maximized"), "{text}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_state_file_is_left_alone_rather_than_treated_as_geometry() {
        let dir = std::env::temp_dir().join("halcyon-window-state-broken");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dirs");

        let path = state_path(&dir);
        std::fs::write(&path, "{not json at all").expect("write");

        assert_eq!(sanitise(&path, HD), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_state_file_is_not_an_error() {
        // A first launch. There is nothing to correct and nothing to complain about.
        assert_eq!(sanitise(Path::new("C:/nowhere/.window-state.json"), HD), 0);
    }
}
