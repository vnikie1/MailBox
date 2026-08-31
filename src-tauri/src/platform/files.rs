//! Asking Windows where to put a file, and making a mail filename safe to put anywhere.
//!
//! The system dialog rather than a plugin, for the same reason `open_external` calls
//! `ShellExecuteW` directly: it is one call, the user already knows the dialog, and it keeps
//! the app's capability surface — the thing `capabilities/default.json` stays honest about —
//! from growing a filesystem permission the WebView could then ask for on its own.

use std::path::PathBuf;

/// Characters Windows refuses in a filename, plus the ones that are merely a bad idea.
const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Device names Windows still reserves, whatever the extension.
///
/// `CON.txt` is not a file; it is the console. A sender who names an attachment that gets an
/// error at best and something stranger at worst, so the name is defused before it is offered.
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Turns a filename from a message into one that is safe to suggest.
///
/// **This is attacker-controlled text.** A `Content-Disposition` filename is written by whoever
/// sent the mail, and the two things it must never be able to do are escape the directory the
/// user chose and impersonate a different file type by hiding the real extension.
///
/// So: no separators, no traversal, no control characters, and — the one that is easy to miss
/// — no right-to-left override. `invoice\u{202E}fdp.exe` renders in a file dialog as
/// `invoiceexe.pdf`, which is a genuine technique and not a hypothetical one.
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control())
        // Bidirectional overrides and other invisible reordering marks.
        .filter(|c| !matches!(*c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}'))
        .map(|c| if FORBIDDEN.contains(&c) { '_' } else { c })
        .collect();

    // Leading dots hide the file; trailing dots and spaces are silently stripped by the
    // filesystem, which is how "report.exe " becomes something other than what was shown.
    let trimmed = cleaned.trim().trim_matches('.').trim();

    if trimmed.is_empty() {
        return "attachment".to_string();
    }

    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    if RESERVED
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        return format!("_{trimmed}");
    }

    // Long names fail at the path limit rather than at the filename, so the failure appears
    // to be about the folder the user picked.
    if trimmed.chars().count() > 200 {
        return trimmed.chars().take(200).collect();
    }

    trimmed.to_string()
}

/// Shows the system "Save As" dialog. `None` means the user cancelled.
///
/// Blocking, and must be called from a blocking context — it runs a modal message loop.
pub fn save_file_dialog(suggested: &str) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::core::PWSTR;
    use windows::Win32::UI::Controls::Dialogs::{
        GetSaveFileNameW, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };

    // The dialog writes the chosen path back into this buffer, so it is sized for the long
    // path limit rather than for the suggestion.
    let mut buffer: Vec<u16> = vec![0; 32_768];

    for (index, unit) in std::ffi::OsStr::new(suggested)
        .encode_wide()
        .take(buffer.len() - 1)
        .enumerate()
    {
        buffer[index] = unit;
    }

    let mut options = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        // OVERWRITEPROMPT so the shell asks before replacing, and NOCHANGEDIR so picking a
        // folder here does not silently move the process's working directory.
        Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };

    // Returns false both on cancel and on error; the two are indistinguishable here and mean
    // the same thing to the caller — no file was chosen.
    let chosen = unsafe { GetSaveFileNameW(&mut options) };
    if !chosen.as_bool() {
        return None;
    }

    let length = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
    if length == 0 {
        return None;
    }

    Some(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..length],
    )))
}

/// Shows the system "Open" dialog, allowing several files. Empty when cancelled.
///
/// Blocking, like [`save_file_dialog`], and for the same reason: it runs a modal message loop.
///
/// Multi-select returns its result in a shape worth spelling out, because it is easy to get
/// subtly wrong and the failure is silent. With one file the buffer holds a full path. With
/// several it holds the *directory*, then a NUL, then each bare filename, then a NUL, and a
/// final NUL to end the list. Reading it as a single path gives the directory and no files at
/// all — an attachment button that appears to do nothing.
pub fn open_files_dialog() -> Vec<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::core::PWSTR;
    use windows::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
        OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };

    // Large, because it has to hold every selected name at once. A user attaching forty
    // photographs is not doing anything unusual.
    let mut buffer: Vec<u16> = vec![0; 65_536];

    let mut options = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        Flags: OFN_EXPLORER
            | OFN_ALLOWMULTISELECT
            | OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR,
        ..Default::default()
    };

    if !unsafe { GetOpenFileNameW(&mut options) }.as_bool() {
        return Vec::new();
    }

    // Split on NULs, stopping at the empty string that ends the list.
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0usize;
    for index in 0..buffer.len() {
        if buffer[index] != 0 {
            continue;
        }
        if index == start {
            break;
        }
        parts.push(
            std::ffi::OsString::from_wide(&buffer[start..index])
                .to_string_lossy()
                .to_string(),
        );
        start = index + 1;
    }

    match parts.len() {
        0 => Vec::new(),
        // One entry is a complete path: the user chose a single file.
        1 => vec![PathBuf::from(&parts[0])],
        // Otherwise the first entry is the directory and the rest are bare names.
        _ => {
            let directory = PathBuf::from(&parts[0]);
            parts[1..].iter().map(|name| directory.join(name)).collect()
        }
    }
}

/// Shows the system folder picker. `None` means the user cancelled.
///
/// `IFileOpenDialog` with `FOS_PICKFOLDERS` rather than the old `SHBrowseForFolder`, which
/// still exists and still shows the small tree-in-a-box from Windows 2000 — visibly not the
/// dialog the rest of the system uses.
///
/// Blocking, like the other two here: it runs a modal message loop.
pub fn pick_folder_dialog(title: &str) -> Option<PathBuf> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, IShellItem, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };

    unsafe {
        // The dialog is apartment-threaded. Initialising here rather than at start-up keeps the
        // COM lifetime to the call — and `CoInitializeEx` returning "already initialised" on
        // this thread is a success, not an error, which is why the result is not checked.
        let started = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let result = (|| -> Option<PathBuf> {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;

            // The options are read and merged rather than set outright: replacing them would
            // drop the defaults the shell relies on, and the dialog misbehaves in ways that
            // look like a Windows bug.
            let options = dialog.GetOptions().ok()?;
            dialog.SetOptions(options | FOS_PICKFOLDERS).ok()?;

            let heading = HSTRING::from(title);
            let _ = dialog.SetTitle(PCWSTR(heading.as_ptr()));

            // Cancelling returns an error, and it is the ordinary path rather than a failure.
            dialog.Show(None).ok()?;

            let item: IShellItem = dialog.GetResult().ok()?;
            let wide = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let path = wide.to_string().ok()?;
            windows::Win32::System::Com::CoTaskMemFree(Some(wide.0 as *const _));

            Some(PathBuf::from(path))
        })();

        if started.is_ok() {
            CoUninitialize();
        }

        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filename_cannot_escape_the_folder_the_user_chose() {
        // The whole point. A `Content-Disposition` filename is written by the sender.
        assert!(!safe_file_name("../../windows/system32/evil.dll").contains('/'));
        assert!(!safe_file_name(r"..\..\evil.dll").contains('\\'));
        assert!(!safe_file_name("C:\\Windows\\evil.dll").contains(':'));
    }

    #[test]
    fn a_right_to_left_override_cannot_disguise_the_extension() {
        // U+202E reverses what follows, so this renders as "invoiceexe.pdf" in a file dialog
        // while remaining an executable. A real technique, not a hypothetical one.
        let disguised = "invoice\u{202E}fdp.exe";
        let safe = safe_file_name(disguised);

        assert!(!safe.contains('\u{202E}'), "{safe:?}");
        assert!(
            safe.ends_with(".exe"),
            "the real extension must stay visible: {safe:?}"
        );
    }

    #[test]
    fn reserved_device_names_are_defused() {
        // `CON.txt` is not a file on Windows, whatever the extension says.
        assert_eq!(safe_file_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_file_name("nul"), "_nul");
        // A name that merely starts with those letters is fine.
        assert_eq!(safe_file_name("console.log"), "console.log");
    }

    #[test]
    fn trailing_dots_and_spaces_are_removed() {
        // The filesystem strips them silently, so "report.exe " is saved as "report.exe"
        // while the dialog showed something else.
        assert_eq!(safe_file_name("report.exe "), "report.exe");
        assert_eq!(safe_file_name("report.pdf."), "report.pdf");
        assert_eq!(safe_file_name("  spaced.txt  "), "spaced.txt");
    }

    #[test]
    fn a_name_that_sanitises_to_nothing_still_produces_a_file() {
        // Refusing to save is worse than saving under a dull name.
        assert_eq!(safe_file_name(""), "attachment");
        assert_eq!(safe_file_name("..."), "attachment");
        assert_eq!(safe_file_name("\u{202E}\u{200F}"), "attachment");
    }

    #[test]
    fn an_ordinary_name_is_left_alone() {
        // A sanitiser that mangles normal input gets worked around rather than trusted.
        assert_eq!(
            safe_file_name("Q3 report (final).pdf"),
            "Q3 report (final).pdf"
        );
        assert_eq!(safe_file_name("photo-2026.jpeg"), "photo-2026.jpeg");
        assert_eq!(safe_file_name("naïve café.txt"), "naïve café.txt");
    }
}
