//! The taskbar unread badge. docs/06 Phase 10.
//!
//! `ITaskbarList3::SetOverlayIcon` puts a small icon over the app's taskbar button. It is the
//! Windows convention for "there is something here", and it is the only unread indicator most
//! people will see, because the window is usually behind something else.
//!
//! ## Why the icon is drawn rather than shipped
//!
//! The badge shows a *number*, and a number cannot be a static asset — it would mean 100 `.ico`
//! files, or a badge that says "you have mail" and not how much. So each one is drawn at the
//! moment it changes.
//!
//! ## Why it is drawn at the display's size and not at sixteen pixels
//!
//! It used to be a fixed 16×16, on the reasoning that "anything larger is scaled down and looks
//! soft". That had the trade backwards. MSDN asks for "a small icon, measuring 16x16 pixels at
//! 96 dpi" — *at 96 dpi* — and on a 200% display the shell draws the overlay at twice that, so a
//! 16×16 icon was being **enlarged**, doubling every pixel with no filtering. Measured on the
//! machine this was reported from: `GetSystemMetricsForDpi(SM_CXSMICON, 192)` is 32. The result
//! was a one-pixel glyph stroke smeared to two and a staircase around the disc — a badge whose
//! digit could not be read, which is exactly what was reported.
//!
//! Enlarging is the unkind direction. Downscaling a too-large icon costs sharpness; upscaling a
//! too-small one costs the shape of the glyph. So the size is taken from the window's DPI, and
//! where the answer is uncertain — Windows 11 renders a per-monitor taskbar and there is no API
//! for "the DPI of the taskbar showing my button" — it is better to be too big.
//!
//! ## Why the digits come from a font
//!
//! At sixteen pixels a hinted typeface really is a smear, and the 3×5 bitmap font this used to
//! carry was the right answer for that size. At thirty-two it is the wrong one: there is room
//! for real glyphs, and a scaled-up bitmap font stays blocky however smoothly it is filtered.
//! The text is drawn inside the disc, which is fully opaque, so GDI writing colour without
//! touching alpha leaves exactly what is wanted — see `draw_count`.

use std::sync::atomic::{AtomicU32, Ordering};

use windows::core::HSTRING;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
    SelectObject, SetBkMode, SetTextColor, ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER, DT_NOCLIP,
    DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, HGDIOBJ, OUT_TT_PRECIS, TRANSPARENT, VARIABLE_PITCH,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, HICON, ICONINFO, SM_CXSMICON,
};

/// What MSDN specifies, and therefore the floor: 16 pixels at 96 dpi.
const REFERENCE_SIZE: i32 = 16;

/// A ceiling, because `GetSystemMetricsForDpi` is only as sane as the DPI it is handed.
const MAX_SIZE: i32 = 64;

/// Supersampling factor for the disc. Four samples a side is sixteen per pixel, which is past
/// the point where more is visible on a circle this size.
const SAMPLES: i32 = 4;

/// The colour the badge is painted in, as `0x00RRGGBB`, or `UNSET`.
///
/// Pushed down by the UI rather than read from the OS here, and that is the whole point. The
/// app's accent is the OS accent *until the user overrides it in Settings*, and only the UI
/// knows whether they have. Resolving it again in Rust would be a second implementation of the
/// same rule in a second language: the taskbar would show the OS accent while the window showed
/// the user's pick, and nothing would be able to test across the gap.
///
/// The fallback below is a documented degradation, not a parallel implementation: the first
/// badge is drawn during `setup`, before any WebView has mounted to tell us anything.
static FILL: AtomicU32 = AtomicU32::new(UNSET);
static INK: AtomicU32 = AtomicU32::new(UNSET);

const UNSET: u32 = u32::MAX;

/// The accent the app is actually showing, and the colour that reads on it.
///
/// Both come from the UI, which computes the foreground by WCAG relative luminance — the same
/// function that decides `--accent-fg` for every button in the app. A badge that picked its own
/// would eventually disagree with the window it belongs to.
pub fn set_paint(fill_rgb: u32, ink_rgb: u32) {
    FILL.store(fill_rgb & 0x00FF_FFFF, Ordering::Relaxed);
    INK.store(ink_rgb & 0x00FF_FFFF, Ordering::Relaxed);
}

/// The paint to use, falling back to the OS accent for the badge drawn before the UI is up.
fn paint() -> (u32, u32) {
    let fill = match FILL.load(Ordering::Relaxed) {
        UNSET => super::appearance::accent_rgb().unwrap_or(0x00_C4_2B_1C),
        value => value,
    };

    let ink = match INK.load(Ordering::Relaxed) {
        UNSET => 0x00_FF_FF_FF,
        value => value,
    };

    (fill, ink)
}

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
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};

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

    let icon = draw(overlay_size(hwnd), count)?;

    // The overlay's accessible name. Narrator reads this and nothing else about the badge, so
    // without it the count is a purely visual signal — which for a screen-reader user is the
    // same as the badge not being there. docs/06 Phase 10 asks for an accessibility pass.
    let description = HSTRING::from(match count {
        1 => "1 unread".to_string(),
        many => format!("{many} unread"),
    });

    let result = unsafe { taskbar.SetOverlayIcon(hwnd, icon, &description) };

    // Destroyed after the shell has taken its copy. Leaking one per unread change is a handle
    // leak that only shows up after a long session, which is the hardest kind to attribute.
    unsafe {
        let _ = DestroyIcon(icon);
    }

    result
}

/// How big the shell wants the overlay, in device pixels.
///
/// `GetDpiForWindow` is meaningful here because the app declares `PerMonitorV2` in
/// `halcyon.exe.manifest`, so this is the real DPI of the monitor the window is on rather than
/// the system value. That is not quite the question — the taskbar showing the button may be on
/// a different monitor, and Windows offers no way to ask which — but of the two available
/// answers it is much the better one, and being too large degrades far more gracefully than
/// being too small.
fn overlay_size(hwnd: HWND) -> i32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        return REFERENCE_SIZE;
    }

    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) };
    size.clamp(REFERENCE_SIZE, MAX_SIZE)
}

/// The filled disc, anti-aliased, as premultiplied BGRA.
///
/// Pure, and separated for that reason: everything below it is COM and GDI that cannot run in a
/// test, and this is the part with arithmetic worth checking.
///
/// **Premultiplied** is not optional. The shell composites a 32-bit icon with `AlphaBlend`,
/// which expects colour already scaled by alpha; the old badge got away with straight values
/// only because every pixel was fully opaque or fully transparent, where the two are identical.
/// The moment the rim carries partial alpha, straight colour draws a bright halo around it.
fn disc(size: i32, fill: u32) -> Vec<u32> {
    let mut pixels = vec![0_u32; (size * size) as usize];

    let red = ((fill >> 16) & 0xFF) as f32;
    let green = ((fill >> 8) & 0xFF) as f32;
    let blue = (fill & 0xFF) as f32;

    let centre = size as f32 / 2.0;
    let radius = centre;
    let step = 1.0 / SAMPLES as f32;

    for y in 0..size {
        for x in 0..size {
            // Coverage by supersampling rather than by a distance-to-edge approximation: the
            // same loop is correct at every size, and at sixteen pixels the approximation's
            // error is a visible flat spot on a circle that small.
            let mut hits = 0;

            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let px = x as f32 + (sx as f32 + 0.5) * step;
                    let py = y as f32 + (sy as f32 + 0.5) * step;
                    let dx = px - centre;
                    let dy = py - centre;

                    if dx * dx + dy * dy <= radius * radius {
                        hits += 1;
                    }
                }
            }

            if hits == 0 {
                continue;
            }

            let coverage = hits as f32 / (SAMPLES * SAMPLES) as f32;
            let alpha = (coverage * 255.0).round();

            let premultiply = |channel: f32| ((channel * alpha / 255.0).round() as u32) & 0xFF;

            pixels[(y * size + x) as usize] = ((alpha as u32) << 24)
                | (premultiply(red) << 16)
                | (premultiply(green) << 8)
                | premultiply(blue);
        }
    }

    pixels
}

/// What the badge says. Two digits fit; three do not, at any size the shell will draw.
fn label(count: u32) -> String {
    if count > 99 {
        "99+".to_string()
    } else {
        count.to_string()
    }
}

/// Draws the badge and wraps it as an `HICON`.
fn draw(size: i32, count: u32) -> windows::core::Result<HICON> {
    let (fill, ink) = paint();
    let pixels = disc(size, fill);
    let text = label(count);

    unsafe {
        let dc = CreateCompatibleDC(None);

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                // Negative: a top-down bitmap, so row 0 is the top. Windows DIBs are
                // bottom-up by default and the badge would be drawn upside down.
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();

        // Every early return from here on has to free the DC. The previous version returned on
        // this call's error with `?` and leaked one each time.
        let colour = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => bitmap,
            Err(error) => {
                let _ = DeleteDC(dc);
                return Err(error);
            }
        };

        if !bits.is_null() {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u32>(), pixels.len());
        }

        draw_count(dc, colour, size, &text, ink);

        // Zeroed explicitly. `CreateBitmap` with a null pointer leaves the contents
        // **undefined** — the old comment claimed an all-zero mask and simply got lucky, which
        // is a bug that would only ever appear on someone else's machine. A set bit in the AND
        // mask punches a hole in a 32-bit icon.
        let mask_bits = vec![0_u8; ((size + 15) / 16 * 2 * size) as usize];
        let mask = windows::Win32::Graphics::Gdi::CreateBitmap(
            size,
            size,
            1,
            1,
            Some(mask_bits.as_ptr().cast()),
        );

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

/// Puts the count on the disc.
///
/// GDI writes colour and leaves alpha alone, which would normally ruin a 32-bit DIB — but the
/// text lands inside the disc, where alpha is already 255 and premultiplied colour is the same
/// as straight colour. So the one place GDI text is safe in a premultiplied bitmap is exactly
/// the place it is wanted. Anything that spilled past the rim would land on transparent pixels
/// and simply not show, which is a benign way to be wrong.
unsafe fn draw_count(
    dc: windows::Win32::Graphics::Gdi::HDC,
    _bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
    size: i32,
    text: &str,
    ink: u32,
) {
    let previous = SelectObject(dc, HGDIOBJ(_bitmap.0));

    // Sized from the badge and the number of digits: one digit can be tall, three cannot. The
    // fractions are chosen so the glyphs sit inside the disc rather than touching its rim,
    // where the anti-aliased edge would eat them.
    let fraction = match text.chars().count() {
        1 => 0.66,
        2 => 0.54,
        _ => 0.40,
    };

    let height = -((size as f32 * fraction).round() as i32);

    let face = HSTRING::from("Segoe UI");
    let font = CreateFontW(
        height,
        0,
        0,
        0,
        // Semibold. A regular weight at this size is thin enough to disappear against a
        // saturated fill once the shell has scaled it.
        600,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        OUT_TT_PRECIS,
        CLIP_DEFAULT_PRECIS,
        // Greyscale rather than ClearType: subpixel rendering assumes it knows the geometry of
        // the display it lands on, and this bitmap is handed to the shell to composite wherever
        // it likes. Coloured fringes on a two-character glyph are worse than slightly softer
        // edges.
        ANTIALIASED_QUALITY,
        (VARIABLE_PITCH.0 | FF_DONTCARE.0) as u32,
        &face,
    );

    let previous_font = SelectObject(dc, HGDIOBJ(font.0));

    SetBkMode(dc, TRANSPARENT);
    // COLORREF is 0x00BBGGRR, the other way round from the 0x00RRGGBB used everywhere else here.
    let colourref = windows::Win32::Foundation::COLORREF(
        ((ink & 0xFF) << 16) | (ink & 0xFF00) | ((ink >> 16) & 0xFF),
    );
    SetTextColor(dc, colourref);

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: size,
        bottom: size,
    };

    let mut wide: Vec<u16> = text.encode_utf16().collect();
    DrawTextW(
        dc,
        &mut wide,
        &mut rect,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP,
    );

    SelectObject(dc, previous_font);
    let _ = DeleteObject(HGDIOBJ(font.0));
    SelectObject(dc, previous);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_over_ninety_nine_is_abbreviated() {
        // Two digits fit on the disc and three do not, at any size the shell will draw. A count
        // that renders as an unreadable smear is worse than one that admits it stopped counting.
        assert_eq!(label(1), "1");
        assert_eq!(label(99), "99");
        assert_eq!(label(100), "99+");
        assert_eq!(label(4_000), "99+");
    }

    #[test]
    fn the_disc_is_premultiplied_so_the_shell_does_not_draw_a_halo() {
        // The shell composites the icon with `AlphaBlend`, which expects colour already scaled
        // by alpha. Straight colour on a partly transparent rim draws a bright fringe, and the
        // old badge only escaped it by having no partial alpha anywhere.
        let pixels = disc(32, 0x00_FF_FF_FF);

        for pixel in &pixels {
            let alpha = pixel >> 24;
            for shift in [16, 8, 0] {
                let channel = (pixel >> shift) & 0xFF;
                assert!(
                    channel <= alpha,
                    "a channel brighter than its own alpha is not premultiplied: \
                     {channel} > {alpha}"
                );
            }
        }
    }

    #[test]
    fn the_disc_has_a_soft_edge_rather_than_a_staircase() {
        // The point of the rewrite. A hard-thresholded circle has only 0 and 255; this asserts
        // there is a real gradient, which is what stops the rim looking like steps once the
        // shell scales it.
        let pixels = disc(32, 0x00_FF_63_0C);
        let partial = pixels
            .iter()
            .filter(|pixel| {
                let alpha = *pixel >> 24;
                alpha > 0 && alpha < 255
            })
            .count();

        assert!(
            partial > 16,
            "only {partial} pixels are partly covered; the edge is not anti-aliased"
        );
    }

    #[test]
    fn the_disc_fills_its_square_and_stops_at_the_edge() {
        let size = 32;
        let pixels = disc(size, 0x00_FF_FF_FF);
        let at = |x: i32, y: i32| pixels[(y * size + x) as usize] >> 24;

        assert_eq!(at(size / 2, size / 2), 255, "the middle should be solid");
        assert_eq!(at(0, 0), 0, "the corners are outside the circle");
        assert_eq!(at(size - 1, 0), 0);
        assert_eq!(at(0, size - 1), 0);
        assert_eq!(at(size - 1, size - 1), 0);
    }

    #[test]
    fn the_paint_falls_back_rather_than_drawing_nothing() {
        // Before any WebView has mounted there is no accent to have been pushed, and the first
        // badge is drawn during `setup`. It has to come out as something.
        let (fill, ink) = paint();
        assert!(fill <= 0x00FF_FFFF);
        assert!(ink <= 0x00FF_FFFF);
    }

    #[test]
    fn a_pushed_accent_is_what_gets_painted() {
        set_paint(0x00_FF_63_0C, 0x00_00_00_00);
        assert_eq!(paint(), (0x00_FF_63_0C, 0x00_00_00_00));

        // Restored, because these statics outlive the test and the fallback test above reads
        // them. Tests in one binary share the process.
        FILL.store(UNSET, Ordering::Relaxed);
        INK.store(UNSET, Ordering::Relaxed);
    }
}
