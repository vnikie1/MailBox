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

use std::cell::Cell;
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

/// A path in `dir` named `{prefix}{stamp}-NN{suffix}` that nothing is at yet, created empty.
///
/// The stamp has one-second resolution, and two crash reports, or two log rotations, can come
/// within a second of each other. Named by the stamp alone, the second replaced the first (L39).
/// The number keeps them apart and keeps the names sorting by time.
fn create_new(dir: &Path, prefix: &str, stamp: &str, suffix: &str) -> Option<(PathBuf, fs::File)> {
    (0..100).find_map(|number| {
        let path = dir.join(format!("{prefix}{stamp}-{number:02}{suffix}"));
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .ok()
            .map(|file| (path, file))
    })
}

/// The log file, rotated when it passes its cap: at start and, since L39, while running.
///
/// It used to be checked only when the app started, and Halcyon runs for weeks from the tray, so
/// one session's log grew without limit. Now every write counts towards the cap.
pub struct Log {
    dir: PathBuf,
    file: Option<fs::File>,
    written: u64,
    limit: u64,
}

impl Log {
    fn open(dir: &Path, limit: u64) -> Option<Self> {
        let mut log = Self {
            dir: dir.to_path_buf(),
            file: None,
            written: 0,
            limit,
        };
        log.reopen();
        log.file.is_some().then_some(log)
    }

    fn current(&self) -> PathBuf {
        self.dir.join("halcyon.log")
    }

    /// Opens the current file to append to, rotating it first if it is already past the cap.
    fn reopen(&mut self) {
        // Closed before it is renamed: Windows will not rename a file this process has open.
        self.file = None;

        let current = self.current();
        let size = fs::metadata(&current).map(|meta| meta.len()).unwrap_or(0);

        if size > self.limit {
            // The name is claimed by creating it, and closed before the log is renamed over it.
            let rotated = create_new(&self.dir, "halcyon-", &stamp(), ".log").map(|(path, _)| path);
            if let Some(rotated) = rotated {
                let _ = fs::rename(&current, rotated);
            }
            trim(&self.dir, "halcyon-", KEEP_LOGS);
        }

        self.file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&current)
            .ok();
        self.written = fs::metadata(&current).map(|meta| meta.len()).unwrap_or(0);
    }
}

impl Write for Log {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.written > self.limit || self.file.is_none() {
            self.reopen();
        }

        let file = self
            .file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("the log file could not be opened"))?;
        let written = file.write(buf)?;
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.as_mut().map_or(Ok(()), Write::flush)
    }
}

thread_local! {
    /// Whether a panic on this thread is one its code catches and recovers from.
    static PANICS_EXPECTED: Cell<bool> = const { Cell::new(false) };
}

/// Runs `work`, whose caller catches its panics and carries on.
///
/// Such a panic is not a crash and gets no report. The attachment text extractors are the case
/// this is for: a malformed PDF can panic the parser, the caller catches it and leaves that one
/// attachment unindexed, and every one used to write a crash report, so a mailbox of bad PDFs
/// filled all ten places and pushed out the report of a real crash (L39). The panic hook runs
/// before the stack unwinds, so it still sees the flag set; the guard puts it back afterwards.
pub fn panics_expected<T>(work: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            PANICS_EXPECTED.with(|expected| expected.set(self.0));
        }
    }

    let _restore = Restore(PANICS_EXPECTED.with(|expected| expected.replace(true)));
    work()
}

/// Whether a panic happening now, on this thread, is a crash.
fn is_crash() -> bool {
    !PANICS_EXPECTED.with(Cell::get)
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

        if !is_crash() {
            return;
        }

        // Everything here is best-effort. A panic inside a panic hook aborts the process, and an
        // abort during a crash report loses the very thing being written.
        if let Some((_, mut file)) = create_new(&dir, "crash-", &stamp(), ".txt") {
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
pub fn log_writer(dir: &Path) -> Option<Log> {
    fs::create_dir_all(dir).ok()?;
    Log::open(dir, MAX_LOG_BYTES)
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
    fn two_reports_in_the_same_second_are_two_reports() {
        // Named by the second alone, the second report replaced the first (L39).
        let dir = temp("samesecond");

        let first = create_new(&dir, "crash-", "000000000042", ".txt").expect("first");
        let second = create_new(&dir, "crash-", "000000000042", ".txt").expect("second");

        assert_ne!(first.0, second.0);
        assert_eq!(reports(&dir).len(), 2);
        // Still in the order they were written.
        assert!(first.0 < second.0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_panic_its_caller_catches_is_not_a_crash() {
        // Each malformed PDF the text extractor choked on wrote a crash report, and ten of them
        // pushed every real one out (L39).
        assert!(is_crash());

        // Noted rather than asserted inside: an assertion failing there would be one more panic
        // the catch swallows.
        let mut inside = None;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            panics_expected(|| {
                inside = Some(is_crash());
                panic!("a malformed attachment");
            })
        }));

        assert!(caught.is_err());
        assert_eq!(inside, Some(false), "a caught panic was taken for a crash");
        assert!(is_crash(), "the flag outlived the panic");
        assert_eq!(panics_expected(|| 7), 7);
        assert!(is_crash());
    }

    #[test]
    fn the_log_is_rotated_while_the_app_runs() {
        // It was rotated only at start, and the app runs for weeks from the tray (L39).
        let dir = temp("rotate");
        let mut log = Log::open(&dir, 64).expect("open");

        for line in 0..20 {
            writeln!(log, "line {line} of a long session").expect("write");
        }
        log.flush().expect("flush");

        let rotated = fs::read_dir(&dir)
            .expect("list")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("halcyon-"))
            .count();
        let current = fs::metadata(dir.join("halcyon.log"))
            .expect("current")
            .len();

        assert!(rotated >= 1, "nothing was rotated");
        assert!(rotated <= KEEP_LOGS);
        assert!(current < 200, "the current log is {current} bytes");

        drop(log);
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
