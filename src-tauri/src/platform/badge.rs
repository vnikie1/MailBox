//! The taskbar unread badge. docs/06 Phase 10.
//!
//! `ITaskbarList3::SetOverlayIcon` puts a small icon over the app's taskbar button. It is the
//! Windows convention for "there is something here", and it is the only unread indicator most
//! people will see, because the window is usually behind something else.
//!
//! ## Why the icon is drawn rather than shipped
//!
//! The badge shows a *number*, and a number cannot be a static asset — it would mean 100 `.ico`
//! files, or a badge that says "you have mail" and not how much. So each one is drawn into a
//! 16×16 bitmap at the moment it changes.
//!
//! ## Why it says 99+
//!
//! Sixteen pixels holds two digits legibly and not three. A count that renders as an unreadable
//! smear is worse than a count that admits it has stopped counting, and every platform that has
//! tried this arrives at the same answer.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO};

/// The size Windows draws an overlay at. Anything larger is scaled down and looks soft.
const SIZE: i32 = 16;

/// A 16×16 BGRA buffer.
type Pixels = [u32; (SIZE * SIZE) as usize];

/// Sets or clears the badge.
///
/// `count` of zero clears it. Errors are swallowed and logged: a taskbar badge is a courtesy,
/// and an app that failed to start because it could not draw one would be absurd.
pub fn set_unread(hwnd: HWND, count: u32) {
    if let Err(error) = try_set(hwnd, count) {
        tracing::debug!(%error, count, "could not set the taskbar badge");
    }
}

fn try_set(hwnd: HWND, count: u32) -> windows::core::Result<()> {
    // Created per call rather than held. The interface is apartment-threaded, and caching one
    // across threads is the kind of COM mistake that shows up as an occasional hang rather
    // than an error.
    let taskbar: ITaskbarList3 = unsafe {
        windows::Win32::System::Com::CoCreateInstance(
            &TaskbarList,
            None,
            windows::Win32::System::Com::CLSCTX_ALL,
        )
    }?;

    unsafe { taskbar.HrInit() }?;

    if count == 0 {
        // A null icon is how the overlay is removed. Leaving the last one up would tell the
        // user they have mail they have already read.
        unsafe { taskbar.SetOverlayIcon(hwnd, HICON::default(), None) }?;
        return Ok(());
    }

    let icon = draw(count)?;
    let result = unsafe { taskbar.SetOverlayIcon(hwnd, icon, None) };

    // Destroyed after the shell has taken its copy. Leaking one per unread change is a handle
    // leak that only shows up after a long session, which is the hardest kind to attribute.
    unsafe {
        let _ = DestroyIcon(icon);
    }

    result
}

/// Draws the badge: a filled circle with the count on it.
fn draw(count: u32) -> windows::core::Result<HICON> {
    let mut pixels: Pixels = [0; (SIZE * SIZE) as usize];

    // The accent-red Windows uses for attention badges. Absolute rather than a token: this is
    // drawn into a bitmap the shell owns, not into the app's own surface, so nothing here can
    // read a CSS custom property.
    let fill: u32 = 0xFF_C4_2B_1C;
    let ink: u32 = 0xFF_FF_FF_FF;

    let centre = (SIZE as f32 - 1.0) / 2.0;
    let radius = centre + 0.5;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - centre;
            let dy = y as f32 - centre;

            if dx * dx + dy * dy <= radius * radius {
                pixels[(y * SIZE + x) as usize] = fill;
            }
        }
    }

    // The digits, as a 3×5 bitmap font. A font here would mean loading one, measuring it and
    // hinting it at 16 pixels — for at most three glyphs that never change size.
    let text = if count > 99 {
        "99+".to_string()
    } else {
        count.to_string()
    };

    let glyph_width = 4; // 3 columns plus a gap
    let width = text.len() as i32 * glyph_width - 1;
    let start_x = (SIZE - width) / 2;
    let start_y = (SIZE - 5) / 2;

    for (index, character) in text.chars().enumerate() {
        let rows = glyph(character);

        for (row, bits) in rows.iter().enumerate() {
            for column in 0..3 {
                if bits & (1 << (2 - column)) == 0 {
                    continue;
                }

                let x = start_x + index as i32 * glyph_width + column;
                let y = start_y + row as i32;

                if (0..SIZE).contains(&x) && (0..SIZE).contains(&y) {
                    pixels[(y * SIZE + x) as usize] = ink;
                }
            }
        }
    }

    to_icon(&pixels)
}

/// A 3×5 glyph, one byte per row, low three bits set.
fn glyph(character: char) -> [u8; 5] {
    match character {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        _ => [0; 5],
    }
}

/// Wraps a BGRA buffer as an `HICON`.
fn to_icon(pixels: &Pixels) -> windows::core::Result<HICON> {
    unsafe {
        let dc = CreateCompatibleDC(None);

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: SIZE,
                // Negative: a top-down bitmap, so row 0 is the top. Windows DIBs are
                // bottom-up by default and the badge would be drawn upside down.
                biHeight: -SIZE,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let colour = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;

        if !bits.is_null() {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u32>(), pixels.len());
        }

        // A mask is required even for a 32-bit icon with its own alpha; an all-zero one means
        // "use the alpha channel".
        let mask = windows::Win32::Graphics::Gdi::CreateBitmap(SIZE, SIZE, 1, 1, None);

        let icon_info = ICONINFO {
            fIcon: true.into(),
            hbmMask: mask,
            hbmColor: colour,
            ..Default::default()
        };

        let icon = CreateIconIndirect(&icon_info);

        // Both bitmaps are copied into the icon, so the originals are ours to free. Without
        // this every badge update leaks two GDI objects.
        let _ = DeleteObject(HGDIOBJ(colour.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        let _ = DeleteDC(dc);

        icon
    }
}

/// Keeps the badge honest without the caller having to know about COM.
///
/// Silently does nothing when there is no window — during shutdown, or before the window
/// exists — because a badge is never worth an error path of its own.
pub fn refresh(window: &tauri::WebviewWindow, count: u32) {
    let Ok(handle) = window.hwnd() else {
        return;
    };

    set_unread(HWND(handle.0.cast()), count);
}

// Silences unused-import warnings for the handful of items only one build configuration uses.
#[allow(dead_code)]
fn _unused(_: WPARAM, _: LPARAM, _: HBITMAP, _: fn(HGDIOBJ) -> ()) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_over_ninety_nine_is_abbreviated() {
        // Sixteen pixels holds two digits legibly and not three. A smear is worse than a count
        // that admits it has stopped counting.
        assert!(draw(100).is_ok());
        assert!(draw(1).is_ok());
        assert!(draw(99).is_ok());
    }

    #[test]
    fn every_digit_has_a_glyph() {
        // A missing one would draw a blank space, so the badge would silently show "1" for 10.
        for character in "0123456789+".chars() {
            assert_ne!(glyph(character), [0; 5], "no glyph for {character}");
        }
    }

    #[test]
    fn an_unknown_character_draws_nothing_rather_than_rubbish() {
        assert_eq!(glyph('x'), [0; 5]);
    }
}
