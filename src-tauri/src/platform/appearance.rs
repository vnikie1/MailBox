//! OS appearance: theme, accent colour, and the transparency setting.
//!
//! docs/01 §11 — "read the user's accent colour from the OS and use it as the default
//! accent ... matching the OS accent is a large part of feeling native". docs/03 §8
//! routes that through `UISettings.ColorValuesChanged`.
//!
//! Note what is NOT computed here: the accent *foreground*. White on a yellow accent is
//! ~1.6:1 and fails the contrast item in docs/02 §8, so it has to be derived from the
//! accent's luminance — but the accent can also come from the in-app override that never
//! reaches Rust, so that rule lives once, in src/lib/appearance.ts, and covers both.

use std::sync::OnceLock;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Theme, WebviewWindow};
use windows::Foundation::TypedEventHandler;
use windows::UI::ViewManagement::{UIColorType, UISettings};

use super::{backdrop, hwnd_of};

const EVENT_APPEARANCE: &str = "system:appearance";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    /// "light" or "dark", matching the `data-theme` values in semantic.css.
    pub theme: &'static str,
    /// The OS accent as `#RRGGBB`, or `None` if Windows would not tell us — in which
    /// case the CSS fallback in semantic.css supplies Apple blue. The fallback value is
    /// deliberately not duplicated here.
    pub accent: Option<String>,
    pub reduce_transparency: bool,
    /// The material actually on the window, so the UI knows whether to use the CSS
    /// fallback rather than painting a transparent sidebar over nothing.
    pub backdrop: backdrop::Kind,
}

fn last() -> &'static Mutex<Option<Appearance>> {
    static LAST: OnceLock<Mutex<Option<Appearance>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

pub fn is_dark(window: &WebviewWindow) -> bool {
    matches!(window.theme(), Ok(Theme::Dark))
}

/// The wire format for an accent colour. src/lib/appearance.ts parses exactly this, so
/// the shape is a contract rather than a formatting choice.
fn hex_from_rgb(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

fn accent_hex() -> Option<String> {
    let colour = UISettings::new()
        .ok()?
        .GetColorValue(UIColorType::Accent)
        .ok()?;
    Some(hex_from_rgb(colour.R, colour.G, colour.B))
}

/// The Windows "Transparency effects" setting. Off means the user has asked every app to
/// stop being translucent, which the definition of done requires us to honour.
fn advanced_effects_enabled() -> bool {
    UISettings::new()
        .ok()
        .and_then(|settings| settings.AdvancedEffectsEnabled().ok())
        .unwrap_or(true)
}

pub fn preferred_backdrop() -> backdrop::Kind {
    if advanced_effects_enabled() {
        backdrop::Kind::MicaAlt
    } else {
        backdrop::Kind::None
    }
}

/// Read the current OS appearance, re-apply the window attributes that depend on it, and
/// report what actually took effect.
pub fn compute(window: &WebviewWindow) -> Appearance {
    let dark = is_dark(window);
    let effects = advanced_effects_enabled();

    let want = if effects {
        backdrop::Kind::MicaAlt
    } else {
        backdrop::Kind::None
    };

    let effective = match hwnd_of(window) {
        Ok(hwnd) => backdrop::apply(hwnd, want, dark),
        Err(err) => {
            tracing::warn!(%err, "no HWND; backdrop not applied");
            backdrop::Kind::None
        }
    };

    Appearance {
        theme: if dark { "dark" } else { "light" },
        accent: accent_hex(),
        reduce_transparency: !effects,
        backdrop: effective,
    }
}

/// Recompute and emit, but only when something actually changed — Windows raises the
/// underlying events several times per user action.
pub fn push(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let next = compute(&window);

    {
        let mut previous = last().lock();
        if previous.as_ref() == Some(&next) {
            return;
        }
        *previous = Some(next.clone());
    }

    tracing::debug!(?next, "appearance changed");
    let _ = app.emit(EVENT_APPEARANCE, next);
}

pub fn watch(app: AppHandle) {
    let Ok(settings) = UISettings::new() else {
        tracing::warn!("UISettings unavailable; accent and transparency will not follow the OS");
        return;
    };

    let colour_app = app.clone();
    let colour_changed = TypedEventHandler::new(move |_, _| {
        schedule_refresh(colour_app.clone());
        Ok(())
    });
    if let Err(err) = settings.ColorValuesChanged(&colour_changed) {
        tracing::warn!(%err, "could not subscribe to ColorValuesChanged");
    }

    let effects_app = app;
    let effects_changed = TypedEventHandler::new(move |_, _| {
        schedule_refresh(effects_app.clone());
        Ok(())
    });
    if let Err(err) = settings.AdvancedEffectsEnabledChanged(&effects_changed) {
        tracing::warn!(%err, "could not subscribe to AdvancedEffectsEnabledChanged");
    }

    // Dropping the UISettings instance unregisters both handlers. It is meant to live as
    // long as the process, so leaking it is the intent rather than an oversight.
    std::mem::forget(settings);
}

fn schedule_refresh(app: AppHandle) {
    std::thread::spawn(move || {
        // WinRT raises ColorValuesChanged *before* the new values are readable, and
        // raises it several times per change. Letting it settle first collapses the
        // burst into one correct update.
        std::thread::sleep(Duration::from_millis(120));

        // UISettings must not be read from an uninitialised apartment, so the read
        // itself is marshalled back onto the main thread.
        let handle = app.clone();
        if let Err(err) = app.run_on_main_thread(move || push(&handle)) {
            tracing::warn!(%err, "could not marshal appearance refresh to the main thread");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_hex_is_uppercase_six_digit_with_a_hash() {
        // src/lib/appearance.ts parses this with /^#?([0-9a-f]{6})$/i and rejects any
        // other shape rather than guessing, so short or lowercase output would silently
        // drop the OS accent.
        assert_eq!(hex_from_rgb(0x00, 0x7A, 0xFF), "#007AFF");
        assert_eq!(hex_from_rgb(0xF7, 0x63, 0x0C), "#F7630C");
    }

    #[test]
    fn accent_hex_pads_single_digit_channels() {
        assert_eq!(hex_from_rgb(0, 0, 0), "#000000");
        assert_eq!(hex_from_rgb(1, 2, 3), "#010203");
        assert_eq!(hex_from_rgb(0xFF, 0xFF, 0xFF), "#FFFFFF");
    }

    #[test]
    fn appearance_serialises_to_the_shape_the_ui_expects() {
        // These key names and the backdrop spelling are the IPC contract with
        // src/lib/appearance.ts. Renaming a field here without renaming it there would
        // fail silently at runtime, not at compile time.
        let json = serde_json::to_value(Appearance {
            theme: "dark",
            accent: Some("#F7630C".to_owned()),
            reduce_transparency: false,
            backdrop: backdrop::Kind::MicaAlt,
        })
        .expect("Appearance must serialise");

        assert_eq!(json["theme"], "dark");
        assert_eq!(json["accent"], "#F7630C");
        assert_eq!(json["reduceTransparency"], false);
        assert_eq!(json["backdrop"], "micaAlt");
    }

    #[test]
    fn a_missing_accent_serialises_as_null_so_the_css_fallback_applies() {
        let json = serde_json::to_value(Appearance {
            theme: "light",
            accent: None,
            reduce_transparency: true,
            backdrop: backdrop::Kind::None,
        })
        .expect("Appearance must serialise");

        assert!(json["accent"].is_null());
        assert_eq!(json["backdrop"], "none");
    }
}
