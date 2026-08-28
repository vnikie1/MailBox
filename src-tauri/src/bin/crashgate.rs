//! Proves the crash hook actually fires in a release build. docs/06 Phase 11.
//!
//! ## Why this needs proving
//!
//! `[profile.release]` sets `panic = "abort"`, and the obvious reading of that is that a panic
//! terminates the process immediately with no chance for a hook to run — which would make the
//! whole of `diagnostics` dead code in exactly the build where it matters.
//!
//! The obvious reading is wrong: the abort runtime still calls the panic hook, and only then
//! aborts. But "I believe the runtime does X" is not the same as knowing, and the cost of being
//! wrong is a released app that silently records nothing about its own crashes. So this asks the
//! real question of the real profile: it installs the hook, panics, and checks whether a file
//! appeared.
//!
//! Run it against the profile you care about:
//!
//! ```text
//! cargo run --release --bin crashgate
//! ```
//!
//! It exits non-zero if no report was written, so it can be a gate rather than a thing somebody
//! reads the output of.

use std::path::PathBuf;
use std::process::ExitCode;

use halcyon_lib::diagnostics;

fn main() -> ExitCode {
    let dir: PathBuf = std::env::temp_dir().join("halcyon-crashgate");
    let _ = std::fs::remove_dir_all(&dir);

    // The child does the panicking. A panic in this process would take the checking with it,
    // which is the one thing the gate cannot afford.
    if std::env::var("HALCYON_CRASHGATE_CHILD").is_ok() {
        diagnostics::install_panic_hook(dir);
        panic!("crashgate: a deliberate panic, to see whether anything records it");
    }

    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("crashgate: could not find its own binary: {error}");
            return ExitCode::FAILURE;
        }
    };

    let status = std::process::Command::new(exe)
        .env("HALCYON_CRASHGATE_CHILD", "1")
        // The child's stderr is the default hook doing its job. Silenced so the gate's own
        // output is the only thing on the terminal.
        .stderr(std::process::Stdio::null())
        .status();

    match status {
        Ok(status) => println!("crashgate: the child exited with {status}"),
        Err(error) => {
            eprintln!("crashgate: could not run the child: {error}");
            return ExitCode::FAILURE;
        }
    }

    let found = diagnostics::reports(&std::env::temp_dir().join("halcyon-crashgate"));

    if found.is_empty() {
        eprintln!();
        eprintln!("FAIL  no crash report was written.");
        eprintln!();
        eprintln!("      The hook did not run. With panic = \"abort\" in the release profile,");
        eprintln!("      that would mean the app records nothing about its own crashes — so");
        eprintln!("      either the profile or diagnostics.rs has to change.");
        return ExitCode::FAILURE;
    }

    println!();
    println!("PASS  {} report(s) written.", found.len());
    for report in &found {
        println!("      {} ({} bytes)", report.name, report.bytes);
        println!("      {}", report.summary);
    }

    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("halcyon-crashgate"));
    ExitCode::SUCCESS
}
