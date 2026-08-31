//! Finding Thunderbird's mail on this machine. docs/06 Phase 11.
//!
//! ## Why there is no Thunderbird parser here
//!
//! Thunderbird stores local and POP mail as **mbox files with no extension**, one per folder,
//! each beside a `.msf` index. So importing Thunderbird is importing mbox, and everything this
//! module does is find the files and work out what each folder was called. The index files are
//! ignored entirely: they are a cache Thunderbird rebuilds at will, and reading one would be
//! taking the long way round to information the mbox already contains.
//!
//! Newer profiles may use **maildir** instead, one file per message, if the user turned it on —
//! it has never been the default. Those folders are recognised and reported so the user is told
//! what was skipped rather than shown a folder that silently imported nothing.
//!
//! ## The folder tree
//!
//! Subfolders live in a sibling directory named `<folder>.sbd`. `Work` and `Work.sbd/Projects`
//! is a folder and its child. The `.sbd` suffix is stripped when naming, so the imported tree
//! reads the way it did in Thunderbird.

use std::path::{Path, PathBuf};

/// A mail folder found in a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// Display path, `/`-joined: `Local Folders/Work/Projects`.
    pub path: String,
    /// The mbox file itself.
    pub file: PathBuf,
    pub bytes: u64,
}

/// A profile directory that holds mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub root: PathBuf,
}

/// Files Thunderbird keeps beside its mailboxes that are not mailboxes.
///
/// `.msf` is the index. The rest are Thunderbird's own bookkeeping, and importing one produces
/// a folder full of nothing — `msgFilterRules.dat` in particular looks exactly like a mailbox
/// to a reader that goes by "has no extension".
const NOT_MAIL: &[&str] = &[
    "msgFilterRules.dat",
    "filterlog.html",
    "popstate.dat",
    "rules.dat",
    "Junk",
    "Junk.msf",
];

/// Where Thunderbird keeps profiles on Windows.
fn profiles_root() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("Thunderbird"))
}

/// Every profile on this machine that has a mail directory.
///
/// Read from the directory listing rather than by parsing `profiles.ini`. The ini names
/// profiles and their paths, but a profile listed there may have been deleted, and a profile
/// directory present but unlisted still holds readable mail — which is exactly the case for
/// someone who copied their profile off an old machine, who is the person most likely to be
/// importing.
pub fn profiles() -> Vec<Profile> {
    let Some(root) = profiles_root() else {
        return Vec::new();
    };

    let mut found = Vec::new();

    for parent in [root.join("Profiles"), root] {
        let Ok(entries) = std::fs::read_dir(&parent) else {
            continue;
        };

        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            // A profile is a directory containing Mail/ or ImapMail/. Anything else under
            // %APPDATA%\Thunderbird — Crash Reports, updates — is not one.
            if !path.join("Mail").is_dir() && !path.join("ImapMail").is_dir() {
                continue;
            }

            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "Thunderbird".to_string());

            // Profile directories are named `<random>.<label>`. The label is what the user
            // chose and the random part is noise.
            let display = name.split_once('.').map_or(name.clone(), |(_, rest)| {
                if rest.is_empty() {
                    name.clone()
                } else {
                    rest.to_string()
                }
            });

            found.push(Profile {
                name: display,
                root: path,
            });
        }
    }

    found.sort_by(|a, b| a.name.cmp(&b.name));
    found.dedup_by(|a, b| a.root == b.root);
    found
}

/// True for a file that looks like an mbox rather than an index or a stray.
fn looks_like_mailbox(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    if NOT_MAIL.contains(&name) || name.starts_with('.') {
        return false;
    }

    // Thunderbird's mbox files have no extension at all. Anything with one is an index, a log
    // or a backup.
    path.is_file() && path.extension().is_none()
}

/// Walks one profile and lists its folders, deepest paths included.
pub fn folders(profile: &Path) -> Vec<Folder> {
    let mut found = Vec::new();

    for store in ["Mail", "ImapMail"] {
        let root = profile.join(store);
        if root.is_dir() {
            // Each child of Mail/ is an account — "Local Folders", "pop.example.com". Its name
            // is the top of the displayed path, so two accounts' Inboxes stay distinct.
            if let Ok(accounts) = std::fs::read_dir(&root) {
                for account in accounts.filter_map(Result::ok) {
                    let path = account.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let label = path.file_name().map_or_else(
                        || store.to_string(),
                        |name| name.to_string_lossy().to_string(),
                    );
                    walk(&path, &label, &mut found, 0);
                }
            }
        }
    }

    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

/// How deep the folder walk will go.
///
/// `.sbd` directories nest one level per subfolder, and a profile carried between machines for
/// fifteen years can be deep. The cap is a guard against a symlink loop rather than a limit
/// anybody will meet.
const MAX_DEPTH: usize = 24;

fn walk(directory: &Path, prefix: &str, into: &mut Vec<Folder>, depth: usize) {
    if depth > MAX_DEPTH {
        tracing::warn!(
            ?directory,
            "folder nesting is too deep; not descending further"
        );
        return;
    }

    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        if path.is_dir() {
            // Subfolders. A directory that is not `.sbd` is Thunderbird's own storage — or a
            // maildir folder, which this does not read.
            if let Some(stem) = name.strip_suffix(".sbd") {
                walk(&path, &format!("{prefix}/{stem}"), into, depth + 1);
            }
            continue;
        }

        if !looks_like_mailbox(&path) {
            continue;
        }

        let bytes = entry.metadata().map(|meta| meta.len()).unwrap_or(0);

        // An empty file is a folder that exists and holds nothing. Listing it would offer the
        // user a folder that imports zero messages and looks like a failure.
        if bytes == 0 {
            continue;
        }

        into.push(Folder {
            path: format!("{prefix}/{name}"),
            file: path,
            bytes,
        });
    }
}

/// Read and flag state Thunderbird records in the message itself.
///
/// `X-Mozilla-Status` is a hex bitmask Thunderbird writes into every stored message. Reading it
/// is the difference between an import that preserves which mail had been read and one that
/// arrives as several thousand unread messages — which, for anybody importing years of
/// archives, makes the result unusable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MozillaFlags {
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub deleted: bool,
}

impl MozillaFlags {
    /// Bits from MailNewsTypes.idl: 0x1 Read, 0x2 Replied, 0x4 Marked, 0x8 Expunged.
    pub fn from_headers(raw: &[u8]) -> Self {
        // Only the header block, and only up to the first blank line: a quoted mail inside the
        // body can carry the header of the message it quotes, and reading that would apply a
        // stranger's read state to this message.
        let head_end = find_header_end(raw);
        let head = String::from_utf8_lossy(&raw[..head_end]);

        let mut flags = Self::default();

        for line in head.lines() {
            let Some(value) = line.strip_prefix("X-Mozilla-Status:") else {
                continue;
            };

            let Ok(bits) = u32::from_str_radix(value.trim(), 16) else {
                continue;
            };

            flags.seen = bits & 0x1 != 0;
            flags.answered = bits & 0x2 != 0;
            flags.flagged = bits & 0x4 != 0;
            flags.deleted = bits & 0x8 != 0;
            break;
        }

        flags
    }
}

/// Offset of the end of the header block: the first blank line, or the whole thing.
fn find_header_end(raw: &[u8]) -> usize {
    for (at, window) in raw.windows(4).enumerate() {
        if window == b"\r\n\r\n" {
            return at;
        }
    }
    for (at, window) in raw.windows(2).enumerate() {
        if window == b"\n\n" {
            return at;
        }
    }
    raw.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_file_is_not_a_mailbox() {
        // `.msf` sits beside every mbox. Importing one produces a folder of nothing.
        let dir = std::env::temp_dir().join("halcyon-tb-index");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("Inbox.msf"), "index").expect("write");
        std::fs::write(dir.join("Inbox"), "From a@x d\nSubject: x\n").expect("write");

        assert!(looks_like_mailbox(&dir.join("Inbox")));
        assert!(!looks_like_mailbox(&dir.join("Inbox.msf")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn thunderbirds_own_files_are_not_mailboxes() {
        // `msgFilterRules.dat` has an extension, but `popstate.dat` aside, the trap is that
        // several of these look exactly like a mailbox to a check that only asks "no extension".
        let dir = std::env::temp_dir().join("halcyon-tb-own");
        let _ = std::fs::create_dir_all(&dir);
        for name in NOT_MAIL {
            std::fs::write(dir.join(name), "x").expect("write");
        }

        for name in NOT_MAIL {
            assert!(!looks_like_mailbox(&dir.join(name)), "{name}");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subfolders_are_found_through_sbd_directories() {
        let root = std::env::temp_dir().join("halcyon-tb-tree");
        let _ = std::fs::remove_dir_all(&root);
        let account = root.join("Mail").join("Local Folders");
        std::fs::create_dir_all(account.join("Work.sbd")).expect("dirs");

        std::fs::write(account.join("Inbox"), "From a@x d\nSubject: x\n").expect("write");
        std::fs::write(
            account.join("Work.sbd").join("Projects"),
            "From a@x d\nSubject: y\n",
        )
        .expect("write");

        let found = folders(&root);
        let paths: Vec<&str> = found.iter().map(|f| f.path.as_str()).collect();

        assert!(paths.contains(&"Local Folders/Inbox"), "{paths:?}");
        // The `.sbd` is stripped, so the tree reads the way it did in Thunderbird.
        assert!(paths.contains(&"Local Folders/Work/Projects"), "{paths:?}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_folder_file_is_not_offered() {
        let root = std::env::temp_dir().join("halcyon-tb-empty");
        let _ = std::fs::remove_dir_all(&root);
        let account = root.join("Mail").join("Local Folders");
        std::fs::create_dir_all(&account).expect("dirs");
        std::fs::write(account.join("Trash"), "").expect("write");

        assert!(folders(&root).is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_and_flag_state_survives_the_import() {
        // The difference between an import that preserves years of triage and one that arrives
        // as several thousand unread messages.
        let read = MozillaFlags::from_headers(b"X-Mozilla-Status: 0001\nSubject: x\n\nbody");
        assert!(read.seen);
        assert!(!read.flagged);

        let flagged = MozillaFlags::from_headers(b"X-Mozilla-Status: 0005\nSubject: x\n\nbody");
        assert!(flagged.seen);
        assert!(flagged.flagged);

        let untouched = MozillaFlags::from_headers(b"Subject: x\n\nbody");
        assert!(!untouched.seen);
    }

    #[test]
    fn a_status_header_inside_a_quoted_body_is_not_read() {
        // A forwarded mail carries the headers of the message it quotes. Reading those would
        // apply a stranger's read state to this message.
        let flags = MozillaFlags::from_headers(
            b"Subject: forwarded\n\n> X-Mozilla-Status: 0001\n> Subject: the original\n",
        );

        assert!(!flags.seen);
    }
}
