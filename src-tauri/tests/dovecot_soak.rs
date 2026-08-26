//! The last item of the Phase 5 exit gate: *a 12-hour soak shows no leak and no connection
//! storm.* docs/04 §Phase 5.
//!
//! Separate from `dovecot_gate.rs` because it is not a test in the ordinary sense — it runs
//! for half a day and the answer is a trend, not a pass on the last line. It writes a CSV as
//! it goes so a run that is interrupted is still worth reading.
//!
//! Run with:
//!   cargo test --test dovecot_soak -- --ignored --nocapture
//!
//! Shorten it while developing the harness itself:
//!   HALCYON_SOAK_MINUTES=10 cargo test --test dovecot_soak -- --ignored --nocapture
//!
//! ## What it actually measures
//!
//! Two things, and both are stated as numbers rather than impressions.
//!
//! **A leak** is the process's working set climbing without bound. Sampling it is easy; the
//! trap is calling a slow climb a leak. Mail arrives during a soak and the store grows, so
//! some rise is correct. The check is on the *last quarter* against the *second quarter*:
//! after eleven hours a healthy process has stopped growing, whatever it did in its first.
//!
//! **A connection storm** is the client opening far more connections than the work needs. It
//! is measured on the server, not here — `doveadm who` reports what Dovecot actually has open,
//! which is the only number that matters to a provider deciding whether to throttle. docs/05
//! §5 caps Halcyon at 3 per account; sustained breaches of that are the failure.
//!
//! The soak drives the same engine and the same IDLE watcher the app does. It does not drive
//! the UI, which is deliberate: a leak in the WebView is a different bug with a different fix,
//! and mixing the two would make neither measurable.

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use halcyon_lib::accounts::credentials::{self, Kind, Secret};
use halcyon_lib::accounts::provider::{AuthKind, Provider, Security, ServerSettings};
use halcyon_lib::accounts::store::{self, NewAccount};
use halcyon_lib::db::Db;
use halcyon_lib::sync::engine::SyncEngine;
use halcyon_lib::sync::events::Events;
use halcyon_lib::sync::idle;

const TEST_EMAIL: &str = "tester@halcyon.test";
const TEST_PASSWORD: &str = "halcyon-test-only";

/// How often to take a sample, derived from the run length.
///
/// Aims for about two dozen samples whatever the duration: enough for the quarter-against-
/// quarter comparison below to mean something, and few enough that a twelve-hour run is not
/// dominated by its own measuring. A fixed five minutes made short runs — the ones used to
/// develop this harness — produce one sample and fail on the "too few samples" guard, which
/// says nothing about the code under test.
fn sample_every() -> Duration {
    let total = Duration::from_secs(minutes() * 60);
    total
        .div_f64(24.0)
        .clamp(Duration::from_secs(5), Duration::from_secs(5 * 60))
}

/// docs/05 §5. Three per account, and the soak is one account.
const CONNECTION_BUDGET: usize = 3;

fn host() -> String {
    std::env::var("HALCYON_TEST_IMAP_HOST").unwrap_or_else(|_| "192.168.1.15".to_string())
}

fn ssh_target() -> String {
    std::env::var("HALCYON_TEST_SSH").unwrap_or_else(|_| "macstudio".to_string())
}

fn minutes() -> u64 {
    std::env::var("HALCYON_SOAK_MINUTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(12 * 60)
}

/// This process's working set, in megabytes.
fn working_set_mb() -> f64 {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;

    // A failed sample is a gap in the data, not a reason to abandon a twelve-hour run.
    match unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) } {
        Ok(()) => counters.WorkingSetSize as f64 / (1024.0 * 1024.0),
        Err(_) => f64::NAN,
    }
}

/// How many connections Dovecot currently holds for the test user.
///
/// Asked of the server rather than counted here, because the server's answer is the one a
/// provider would act on — and a socket this process believes it closed but has not is exactly
/// the bug worth catching.
fn server_connections() -> usize {
    let output = std::process::Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            &ssh_target(),
            "export PATH=/usr/local/bin:$PATH; \
             docker exec halcyon-dovecot doveadm who -1 2>/dev/null | grep -c imap",
        ])
        .output();

    output
        .ok()
        .and_then(|out| String::from_utf8_lossy(&out.stdout).trim().parse().ok())
        .unwrap_or(0)
}

/// Delivers a message, so IDLE has something to react to.
///
/// A soak against a mailbox nothing ever touches measures an idle socket, not a mail client.
fn deliver(number: u64) {
    let script = format!(
        "export PATH=/usr/local/bin:$PATH; \
         docker exec -i halcyon-dovecot sh -c 'cat > /srv/mail/{TEST_EMAIL}/Maildir/new/soak{number}.halcyon' <<'EOF'\n\
         Message-ID: <soak-{number}@halcyon.test>\n\
         From: Soak <soak@example.test>\n\
         To: Tester <{TEST_EMAIL}>\n\
         Subject: Soak message {number}\n\
         Date: Mon, 01 Jan 2029 00:00:00 +0000\n\
         Content-Type: text/plain; charset=utf-8\n\
         \n\
         Delivered during the soak.\n\
         EOF"
    );

    let _ = std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", &ssh_target(), &script])
        .output();
}

/// Discards events. The soak measures resources, not what was emitted.
struct Quiet;

impl Events for Quiet {
    fn emit(&self, _event: &str, _payload: serde_json::Value) {}
}

#[derive(Clone, Copy)]
struct Sample {
    minute: u64,
    memory_mb: f64,
    connections: usize,
    messages: i64,
}

/// The mean of a slice of samples' memory, ignoring failed reads.
fn mean_memory(samples: &[Sample]) -> f64 {
    let usable: Vec<f64> = samples
        .iter()
        .map(|s| s.memory_mb)
        .filter(|value| value.is_finite())
        .collect();

    if usable.is_empty() {
        return f64::NAN;
    }

    usable.iter().sum::<f64>() / usable.len() as f64
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs for twelve hours; needs the Dovecot rig"]
async fn gate_5_twelve_hour_soak() {
    let total = Duration::from_secs(minutes() * 60);
    let interval = sample_every();
    println!(
        "soak: {} minutes, sampling every {}s",
        minutes(),
        interval.as_secs()
    );

    let dir = tempfile::Builder::new()
        .prefix("halcyon-soak")
        .tempdir()
        .expect("temp dir");
    let db = Db::open(&dir.path().join("soak.db")).expect("open store");

    let imap = ServerSettings {
        host: host(),
        port: 9993,
        security: Security::Tls,
    };
    let smtp = ServerSettings {
        host: host(),
        port: 9587,
        security: Security::StartTls,
    };

    let account = NewAccount {
        display_name: "Soak".into(),
        email: TEST_EMAIL.into(),
        provider: Provider::Other,
        imap,
        smtp,
        auth_kind: AuthKind::Password,
        color: None,
    };

    let account_id = db
        .write(move |tx| store::insert(tx, &account))
        .await
        .expect("insert account");

    let reference = credentials::reference_for(TEST_EMAIL);
    credentials::store(
        &reference,
        Kind::Password,
        &Secret::new(TEST_PASSWORD.to_string()),
    )
    .expect("store password");

    let engine = SyncEngine::new();
    let events: Arc<dyn Events> = Arc::new(Quiet);

    // The initial sync is not part of the measurement — a cold sync of 50,000 messages
    // legitimately allocates, and including it would make the first sample an outlier that
    // every later comparison is drawn against.
    println!("initial sync (not measured) ...");
    engine
        .sync_account(events.as_ref(), &db, account_id)
        .await
        .expect("initial sync");
    println!("initial sync done; starting the watcher");

    let watcher = idle::watch(Arc::clone(&events), db.clone(), engine.clone(), account_id);

    let csv_path = std::env::temp_dir().join("halcyon-soak.csv");
    let mut csv = std::fs::File::create(&csv_path).expect("csv");
    writeln!(csv, "minute,memory_mb,connections,messages").ok();
    println!("writing samples to {}", csv_path.display());

    let started = Instant::now();
    let mut samples: Vec<Sample> = Vec::new();
    let mut delivered = 0u64;
    let mut over_budget = 0usize;
    let mut peak_connections = 0usize;

    while started.elapsed() < total {
        tokio::time::sleep(interval).await;

        // Deliver every other sample, so the watcher has real work and the mailbox grows the
        // way a real one does.
        delivered += 1;
        deliver(delivered);

        let connections = server_connections();
        peak_connections = peak_connections.max(connections);
        if connections > CONNECTION_BUDGET {
            over_budget += 1;
        }

        let messages = db
            .read(move |conn| {
                Ok(conn
                    .query_row("SELECT COUNT(*) FROM message", [], |row| row.get(0))
                    .unwrap_or(0))
            })
            .await
            .unwrap_or(0);

        let sample = Sample {
            minute: started.elapsed().as_secs() / 60,
            memory_mb: working_set_mb(),
            connections,
            messages,
        };
        samples.push(sample);

        writeln!(
            csv,
            "{},{:.1},{},{}",
            sample.minute, sample.memory_mb, sample.connections, sample.messages
        )
        .ok();
        csv.flush().ok();

        println!(
            "  [{:>4}m] memory={:>7.1}MB connections={} messages={}",
            sample.minute, sample.memory_mb, sample.connections, sample.messages
        );
    }

    watcher.stop();
    let _ = credentials::purge(&reference);

    // ---- the verdict -----------------------------------------------------------------------
    assert!(
        samples.len() >= 8,
        "too few samples ({}) to say anything about a trend",
        samples.len()
    );

    let quarter = samples.len() / 4;
    let second_quarter = mean_memory(&samples[quarter..quarter * 2]);
    let last_quarter = mean_memory(&samples[samples.len() - quarter..]);
    let growth = (last_quarter - second_quarter) / second_quarter * 100.0;

    println!();
    println!("=== soak verdict ===");
    println!("  samples            {}", samples.len());
    println!("  memory, 2nd qtr    {second_quarter:.1} MB");
    println!("  memory, last qtr   {last_quarter:.1} MB");
    println!("  growth             {growth:+.1}%");
    println!("  peak connections   {peak_connections} (budget {CONNECTION_BUDGET})");
    println!("  samples over budget {over_budget}");
    println!("  messages delivered {delivered}");
    println!("  csv                {}", csv_path.display());

    // Compared late-against-middle rather than late-against-start: the store legitimately grows
    // as mail arrives, and a process that has settled by the second quarter is not leaking
    // whatever it did while warming up.
    assert!(
        growth < 25.0,
        "working set grew {growth:+.1}% between the second and last quarter, which is a leak"
    );

    // One brief overlap is a sync starting as an idle connection closes. A sustained breach is
    // the storm docs/05 §5 is about.
    assert!(
        over_budget <= samples.len() / 10,
        "{over_budget} of {} samples exceeded the {CONNECTION_BUDGET}-connection budget",
        samples.len()
    );
}
