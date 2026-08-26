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
