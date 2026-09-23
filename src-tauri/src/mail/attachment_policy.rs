//! Attachments a provider is known to refuse, found before a message is queued.
//!
//! Reported from the app on 2026-09-17: a message carrying a `.docx` and a `.zip` went into the
//! outbox, was rejected by Gmail with `552 5.7.0 This message was blocked because its content
//! presents a potential security issue`, and sat there as a failure banner whose only button was
//! Try Again — which can never succeed, because the rejection is about the content. The zip was a
//! game-modding package holding `dinput8.dll`, `ScriptHookV.dll` and `xinput1_4.dll`, and Gmail
//! blocks `.dll` files even inside an archive. Halcyon's message was well-formed; it simply
//! could not know, until Gmail answered, what it now checks up front.
//!
//! Checking here keeps the user in the compose window with the message intact and a sentence
//! naming the file, instead of a queued message that cannot be sent and cannot be edited. It is
//! a courtesy check, not a guarantee: Gmail also inspects content in ways this does not — macros
//! in documents, other archive formats, nested archives — and the send failure it reports for
//! those is still shown (see `sync::sender`).

use std::io::Cursor;

/// The file types Gmail refuses to send, as Google publishes them ("File types blocked in
/// Gmail"). Gmail applies the list to files inside archives as well.
const GMAIL_BLOCKED: &[&str] = &[
    "ade",
    "adp",
    "apk",
    "appx",
    "appxbundle",
    "bat",
    "cab",
    "chm",
    "cmd",
    "com",
    "cpl",
    "diagcab",
    "diagcfg",
    "diagpack",
    "dll",
    "dmg",
    "ex",
    "ex_",
    "exe",
    "hta",
    "img",
    "ins",
    "iso",
    "isp",
    "jar",
    "jnlp",
    "js",
    "jse",
    "lib",
    "lnk",
    "mde",
    "mjs",
    "msc",
    "msi",
    "msix",
    "msixbundle",
    "msp",
    "mst",
    "nsh",
    "pif",
    "ps1",
    "scr",
    "sct",
    "shb",
    "sys",
    "vb",
    "vbe",
    "vbs",
    "vhd",
    "vxd",
    "wsc",
    "wsf",
    "wsh",
    "xll",
];

/// Why a provider will refuse an attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The file itself is of a blocked type.
    BlockedType { file: String, extension: String },
    /// An archive holds a file of a blocked type.
    BlockedInside { file: String, entry: String },
}

impl Refusal {
    /// The sentence the compose window shows. Names the file, says why, and says what to do —
    /// the three things the provider's own rejection leaves the user to work out.
    pub fn sentence(&self) -> String {
        match self {
            Refusal::BlockedType { file, extension } => format!(
                "Gmail won't send “{file}”: it blocks .{extension} files to protect against \
                 malware. Remove the attachment, or share the file as a Google Drive link instead."
            ),
            Refusal::BlockedInside { file, entry } => format!(
                "Gmail won't send “{file}”: it contains “{entry}”, a type of file Gmail blocks \
                 to protect against malware, even inside a .zip. Remove the attachment, or share \
                 the file as a Google Drive link instead."
            ),
        }
    }
}

/// Whether an account sends through Gmail.
///
/// By its SMTP host as well as its provider: a Google address added through "Other Mail
/// Account" with an app password still sends through `smtp.gmail.com`, and Gmail applies the
/// same rules to it.
pub fn sends_through_gmail(provider: &str, smtp_host: Option<&str>) -> bool {
    provider.eq_ignore_ascii_case("google")
        || smtp_host.is_some_and(|host| {
            let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
            host == "smtp.gmail.com" || host == "smtp.googlemail.com"
        })
}

/// The reason Gmail will refuse this attachment, if it will.
///
/// Takes the bytes the message is about to be built from, rather than the path, so the file is
/// judged exactly as it will be sent. An archive is read only as far as its central directory —
/// the list of names — and nothing in it is decompressed.
pub fn gmail_refusal(filename: &str, bytes: &[u8]) -> Option<Refusal> {
    let extension = extension_of(filename)?;

    if is_blocked(&extension) {
        return Some(Refusal::BlockedType {
            file: filename.to_string(),
            extension,
        });
    }

    if extension == "zip" {
        // An archive that cannot be read is not ours to judge; Gmail will say what it thinks.
        let archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
        let entry = archive
            .file_names()
            .find(|name| extension_of(name).is_some_and(|ext| is_blocked(&ext)))?;

        return Some(Refusal::BlockedInside {
            file: filename.to_string(),
            entry: entry.to_string(),
        });
    }

    None
}

fn is_blocked(extension: &str) -> bool {
    GMAIL_BLOCKED.contains(&extension)
}

/// The extension of the last path component, lowercased — `None` for a directory entry or a
/// name without one.
fn extension_of(name: &str) -> Option<String> {
    let last = name.rsplit(['/', '\\']).next()?;
    let (stem, extension) = last.rsplit_once('.')?;
    if stem.is_empty() || extension.is_empty() {
        return None;
    }
    Some(extension.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(names: &[&str]) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default();
            for name in names {
                if name.ends_with('/') {
                    writer.add_directory(*name, options).expect("dir");
                } else {
                    writer.start_file(*name, options).expect("start");
                    writer.write_all(b"contents").expect("write");
                }
            }
            writer.finish().expect("finish");
        }
        buffer.into_inner()
    }

    #[test]
    fn the_reported_archive_is_refused_and_named() {
        // The exact shape of the zip Gmail rejected: a folder, three DLLs, an .asi and text.
        let bytes = zip_of(&[
            "bin/",
            "bin/args.txt",
            "bin/dinput8.dll",
            "bin/NativeTrainer.asi",
            "bin/ScriptHookV.dll",
            "bin/xinput1_4.dll",
            "HOW_TO_INSTALL_2025.txt",
            "readme.txt",
        ]);

        let refusal = gmail_refusal("ScriptHookV_3725.0_1013.20.zip", &bytes).expect("refused");

        assert_eq!(
            refusal,
            Refusal::BlockedInside {
                file: "ScriptHookV_3725.0_1013.20.zip".into(),
                entry: "bin/dinput8.dll".into(),
            }
        );
        let sentence = refusal.sentence();
        assert!(
            sentence.contains("ScriptHookV_3725.0_1013.20.zip"),
            "{sentence}"
        );
        assert!(sentence.contains("bin/dinput8.dll"), "{sentence}");
        assert!(sentence.contains("Google Drive"), "{sentence}");
    }

    #[test]
    fn an_ordinary_document_and_archive_pass() {
        // The .docx that travelled with it was fine, and so is an archive of documents. A .docx
        // is itself a zip; it is judged by its own extension and never opened.
        assert_eq!(
            gmail_refusal("New Rent agreement format Commercial.docx", b"PK..."),
            None
        );

        let photos = zip_of(&["holiday/", "holiday/one.jpg", "holiday/notes.txt"]);
        assert_eq!(gmail_refusal("holiday.zip", &photos), None);
    }

    #[test]
    fn a_blocked_file_is_refused_by_its_own_extension_whatever_its_case() {
        assert_eq!(
            gmail_refusal("Setup.EXE", b"MZ"),
            Some(Refusal::BlockedType {
                file: "Setup.EXE".into(),
                extension: "exe".into(),
            })
        );
        assert!(gmail_refusal("deploy.ps1", b"").is_some());
        assert!(gmail_refusal("tool.js", b"").is_some());
    }

    #[test]
    fn a_zip_that_cannot_be_read_is_left_for_gmail_to_judge() {
        // Refusing a message over an archive we could not parse would be guessing.
        assert_eq!(gmail_refusal("broken.zip", b"not a zip at all"), None);
    }

    #[test]
    fn names_without_a_real_extension_are_not_mistaken_for_one() {
        assert_eq!(extension_of("README"), None);
        assert_eq!(extension_of(".dll"), None, "a dotfile has no extension");
        assert_eq!(extension_of("bin/"), None, "a directory entry has none");
        assert_eq!(extension_of("C:\\tools\\x.DLL").as_deref(), Some("dll"));
        assert_eq!(extension_of("archive.tar.gz").as_deref(), Some("gz"));
    }

    #[test]
    fn gmail_is_recognised_by_provider_or_by_its_smtp_host() {
        assert!(sends_through_gmail("google", None));
        assert!(sends_through_gmail("Google", Some("smtp.gmail.com")));
        // A Google address added as "Other Mail Account" with an app password.
        assert!(sends_through_gmail("other", Some("SMTP.Gmail.com")));
        assert!(sends_through_gmail("other", Some("smtp.googlemail.com.")));

        assert!(!sends_through_gmail("yahoo", Some("smtp.mail.yahoo.com")));
        assert!(!sends_through_gmail(
            "other",
            Some("smtp.gmail.com.example.net")
        ));
        assert!(!sends_through_gmail("microsoft", None));
    }
}
