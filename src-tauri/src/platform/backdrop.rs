//! DWM system backdrop.
//!
//! docs/01 §12 wants the sidebar to sample the desktop the way NSVisualEffectView's
//! `sidebar` material does. On Windows that is **Mica Alt**, not Acrylic: Mica samples
//! the desktop wallpaper and desaturates when the window deactivates — which is exactly
//! the macOS behaviour docs/01 §3 describes — whereas Acrylic blurs whatever happens to
//! be behind the window, and Microsoft scopes it to transient surfaces.
//!
//! Acrylic (`DWMSBT_TRANSIENTWINDOW`) is still the right material for menus and popovers.
//! It is not here yet because there are none until Phase 1, and carrying an unused variant
//! is the sort of thing standing rule 18 exists to prevent.
//!
//! `backdrop-filter` inside the WebView cannot do this job at all: it blurs page content
//! behind the element, never the desktop. It is the fallback path only, selected by the
//! UI when this returns `Kind::None`.

use std::ffi::c_void;
use std::mem::size_of;

use serde::Serialize;
use windows::core::BOOL;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_NONE, DWMSBT_TABBEDWINDOW,
    DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND, DWM_SYSTEMBACKDROP_TYPE, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::UI::Controls::MARGINS;

/// Which material is actually on the window. Reported to the UI so it can switch to the
/// CSS fallback rather than rendering a transparent sidebar over nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// `DWMSBT_TABBEDWINDOW` — samples the wallpaper. The default for the main window.
    MicaAlt,
    /// No material: opaque surfaces plus the CSS fallback.
    None,
}

/// Apply the backdrop and the theme-dependent frame attributes.
///
/// Returns the material that is actually in effect, which is `None` if DWM refused —
/// on a Windows build without `DWMWA_SYSTEMBACKDROP_TYPE`, or with composition off.
pub fn apply(hwnd: HWND, want: Kind, dark: bool) -> Kind {
    unsafe {
        // The app draws its own titlebar, but DWM still paints the 1px frame and the
        // drop shadow. Without this they stay light in dark mode and the window reads
        // as broken at the edges.
        let dark_mode = BOOL::from(dark);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            std::ptr::addr_of!(dark_mode).cast::<c_void>(),
            size_of::<BOOL>() as u32,
        );

        // docs/01 §2: window corner radius 10. Windows 11's `round` is 8px, which is
        // the closest the OS offers and the only value that keeps the frame, the shadow
        // and the snap animation consistent. A CSS radius here would clip the content
        // but leave a square DWM frame behind it.
        let corner: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::addr_of!(corner).cast::<c_void>(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );

        if want == Kind::None {
            let none: DWM_SYSTEMBACKDROP_TYPE = DWMSBT_NONE;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                std::ptr::addr_of!(none).cast::<c_void>(),
                size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
            );
            return Kind::None;
        }

        // Sheet-of-glass: negative margins extend the frame across the whole client
        // area, which is what lets the material show through the entire window rather
        // than only under a caption that no longer exists.
        let margins = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        if DwmExtendFrameIntoClientArea(hwnd, &margins).is_err() {
            tracing::warn!("DwmExtendFrameIntoClientArea failed; falling back to opaque chrome");
            return Kind::None;
        }

        let kind: DWM_SYSTEMBACKDROP_TYPE = match want {
            Kind::MicaAlt => DWMSBT_TABBEDWINDOW,
            Kind::None => DWMSBT_NONE,
        };
        match DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            std::ptr::addr_of!(kind).cast::<c_void>(),
            size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        ) {
            Ok(()) => want,
            Err(err) => {
                tracing::warn!(%err, "DWMWA_SYSTEMBACKDROP_TYPE unsupported; using CSS fallback");
                Kind::None
            }
        }
    }
}
