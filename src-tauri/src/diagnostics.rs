//! Logs and crash reports, kept on this machine. docs/06 Phase 11.
//!
//! ## Why this exists
//!
//! Until now every log line went to stdout. A release build has no console attached, so in the
//! hands of an actual user the entire log stream went nowhere — and the one moment anybody wants
//! a log is after something went wrong, which is exactly when there was nothing to look at.
//!
//! ## What is written, and what is not
//!
//! Standing rule 16 is no telemetry, and nothing here contacts a network. Reports are files in
//! the app's own data directory; the user can read them, hand one to somebody, or delete them,
//! and if they do nothing the ring buffer below eventually drops them.
//!
//! docs/06 asks for "opt-in upload". There is deliberately **no upload here**, because there is
//! nowhere to upload to: building a crash-collection endpoint is a service, with its own
//! retention and its own privacy questions, and shipping a client that posts to a server nobody
//! has decided on yet would be worse than shipping none. What the UI offers instead is the
//! report and the folder it is in. When a destination exists, opt-in sending is a small addition
//! on top; the hard part — capturing something worth sending — is what this does.
//!
//! ## Secrets
//!
//! Standing rule 12 keeps credentials out of logs. That is enforced upstream, by the error types
//! themselves: `tests/secrets.rs` asserts that nothing formattable can print a secret. This file
//! writes panic messages and backtraces, which are the *shape* of the program rather than its
//! data — function names and line numbers, not mailbox contents.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use ts_rs::TS;

/// How many crash reports to keep.
///
/// A crash that repeats writes a report every launch, and a directory that grows without bound
/// is its own small bug. Ten is enough to see a pattern and few enough to read.
const KEEP_REPORTS: usize = 10;

/// How many rotated log files to keep, and how large each may grow.
///
/// Sized so the whole of a session survives — a sync of a large mailbox is chatty — without the
/// logs becoming a noticeable fraction of the mail they describe.
const KEEP_LOGS: usize = 5;
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

/// One crash report, as the UI lists it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CrashReport {
    /// The file name, which is also its id. Timestamped, so the list sorts by name.
    pub name: String,
    /// First line of the report — the panic message, which is the useful part in a list.
    pub summary: String,
    #[ts(type = "number")]
    pub bytes: u64,
}

/// Where logs and reports live: `%LOCALAPPDATA%\com.uniki.halcyon\diagnostics`.
///
/// Beside the database rather than in a temp directory, so that "delete the app's data" is one
/// folder and the uninstaller has one place to look. docs/06's exit gate asks that uninstalling
/// leave nothing behind.
pub fn directory(data_dir: &Path) -> PathBuf {
    data_dir.join("diagnostics")
}

/// Trims a directory to the newest `keep` files matching a prefix.
fn trim(dir: &Path, prefix: &str, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();

    // By name, which is by timestamp: the names are written to sort that way precisely so this
    // does not have to ask the filesystem for modification times it may not keep accurately.
    files.sort();

    if files.len() <= keep {
        return;
    }

    for stale in &files[..files.len() - keep] {
        let _ = fs::remove_file(stale);
    }
}

/// Rotates the log if it has grown past the cap, then returns a handle to append to.
fn open_log(dir: &Path, stamp: &str) -> Option<fs::File> {
    let current = dir.join("halcyon.log");

    if let Ok(meta) = fs::metadata(&current) {
        if meta.len() > MAX_LOG_BYTES {
            let _ = fs::rename(&current, dir.join(format!("halcyon-{stamp}.log")));
            trim(dir, "halcyon-", KEEP_LOGS);
        }
    }

    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&current)
        .ok()
}

/// A sortable, filename-safe timestamp.
///
/// Hand-rolled from the seconds since the epoch rather than pulled from `chrono`, because the
/// only property needed is that it sorts and is unique per second — and this runs inside a panic
/// hook, where the fewer things that can themselves panic, the better.
fn stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    format!("{secs:012}")
}

/// Installs the panic hook that writes a crash report.
///
/// Chained rather than replacing: the default hook prints to stderr, which is what a developer
/// running from a terminal expects to see, and losing it would make debugging worse in exchange
/// for a file they could already read.
pub fn install_panic_hook(dir: PathBuf) {
    let _ = fs::create_dir_all(&dir);

    let previous = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        previous(info);

        let path = dir.join(format!("crash-{}.txt", stamp()));

        // Everything here is best-effort. A panic inside a panic hook aborts the process, and an
        // abort during a crash report loses the very thing being written.
        if let Ok(mut file) = fs::File::create(&path) {
            let _ = writeln!(file, "Halcyon {}", env!("CARGO_PKG_VERSION"));
            let _ = writeln!(file, "{}", info);
            let _ = writeln!(file);
            let _ = writeln!(file, "{}", std::backtrace::Backtrace::force_capture());
        }

        trim(&dir, "crash-", KEEP_REPORTS);
    }));
}

/// Every crash report on disk, newest first.
pub fn reports(dir: &Path) -> Vec<CrashReport> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut found: Vec<CrashReport> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.to_string();

            if !name.starts_with("crash-") {
                return None;
            }

            let text = fs::read_to_string(&path).unwrap_or_default();

            Some(CrashReport {
                // The panic message, which is line two: line one is the version.
                summary: text
                    .lines()
                    .nth(1)
                    .unwrap_or("(no detail recorded)")
                    .chars()
                    .take(160)
                    .collect(),
                bytes: entry.metadata().map(|m| m.len()).unwrap_or(0),
                name,
            })
        })
        .collect();

    found.sort_by(|a, b| b.name.cmp(&a.name));
    found
}

/// The file logger, as a `tracing` writer.
///
/// Returns `None` when the directory cannot be created — a read-only or full disk — in which
/// case logging stays on stdout and the app runs. Failing to start because a log file could not
/// be opened would be a worse trade than losing the log.
pub fn log_writer(dir: &Path) -> Option<fs::File> {
    fs::create_dir_all(dir).ok()?;
    open_log(dir, &stamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own.
    ///
    /// Named after the caller rather than the clock. The first version used `stamp()`, which has
    /// one-second resolution, so every test in this module shared a directory and deleted each
    /// other's fixtures — four failures that looked like logic bugs and were a naming bug.
    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halcyon-diag-{name}"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn the_directory_sits_beside_the_database() {
        // The uninstaller and "delete my data" both work on one folder. A log written to a temp
        // directory instead would survive an uninstall that promised to leave nothing behind.
        let data = Path::new("C:/Users/someone/AppData/Local/com.uniki.halcyon");
        assert!(directory(data).starts_with(data));
    }

    #[test]
    fn an_empty_directory_has_no_reports() {
        let dir = temp("empty");
        assert!(reports(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_report_is_summarised_by_its_panic_message() {
        let dir = temp("summary");
        fs::write(
            dir.join("crash-000000000001.txt"),
            "Halcyon 0.1.0\npanicked at 'the mailbox was not there', src/x.rs:9\n\nstack…",
        )
        .expect("write");

        let found = reports(&dir);
        assert_eq!(found.len(), 1);
        assert!(found[0].summary.contains("the mailbox was not there"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_are_listed_newest_first() {
        let dir = temp("order");
        for name in ["crash-000000000001.txt", "crash-000000000009.txt"] {
            fs::write(dir.join(name), "Halcyon\nboom\n").expect("write");
        }

        let found = reports(&dir);
        assert_eq!(found[0].name, "crash-000000000009.txt");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_crash_files_are_listed() {
        // The log lives in the same folder. Listing it as a crash report would put a 4MB file in
        // front of somebody looking for a one-line panic message.
        let dir = temp("onlycrash");
        fs::write(dir.join("halcyon.log"), "ordinary logging\n").expect("write");
        fs::write(dir.join("crash-000000000002.txt"), "Halcyon\nboom\n").expect("write");

        let found = reports(&dir);
        assert_eq!(found.len(), 1);
        assert!(found[0].name.starts_with("crash-"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_reports_are_dropped_and_the_newest_are_kept() {
        // A crash that repeats writes a report every launch. Without this the folder grows for
        // ever, and the reports that matter are buried under the ones that came after.
        let dir = temp("trim");

        for index in 1..=(KEEP_REPORTS + 4) {
            fs::write(
                dir.join(format!("crash-{index:012}.txt")),
                "Halcyon\nboom\n",
            )
            .expect("write");
        }

        trim(&dir, "crash-", KEEP_REPORTS);

        let found = reports(&dir);
        assert_eq!(found.len(), KEEP_REPORTS);

        // The survivors are the newest, not an arbitrary ten.
        assert_eq!(found[0].name, format!("crash-{:012}.txt", KEEP_REPORTS + 4));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn trimming_leaves_other_files_alone() {
        let dir = temp("others");
        fs::write(dir.join("halcyon.log"), "keep me").expect("write");
        for index in 1..=(KEEP_REPORTS + 2) {
            fs::write(dir.join(format!("crash-{index:012}.txt")), "x").expect("write");
        }

        trim(&dir, "crash-", KEEP_REPORTS);

        assert!(dir.join("halcyon.log").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
