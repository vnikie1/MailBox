//! The Phase 5 exit gate, against Dovecot. docs/04 §Phase 5.
//!
//! `#[ignore]`d — it needs the rig in `test/dovecot` running and its CA trusted, so it never
//! runs in CI or in `npm run verify`.
//!
//! Run with:
//!   cargo test --test dovecot_gate -- --ignored --nocapture --test-threads=1
//!
//! `--test-threads=1` is not optional. Every test here drives the same mailbox on the same
//! server, and two of them running at once would each see the other's changes and blame the
//! engine.
//!
//! ## What this covers that Gmail cannot
//!
//! docs/04 asks for the gate to be met against Dovecot *and* a real Gmail account, and the
//! division is not arbitrary. Gmail has already proved the OAuth path, the label layout and
//! the sheer variety of real mail. It cannot prove these:
//!
//! * **QRESYNC.** Gmail does not advertise it, so that path had never run once.
//! * **A `UIDVALIDITY` reset.** There is no way to make Gmail renumber a mailbox on request,
//!   and "drop and re-sync" is the recovery most likely to be wrong and least likely to be
//!   noticed when it is.
//! * **Rudeness.** Cutting the connection mid-sync is something to do to a server you own.
//!
//! ## What it deliberately does not do
//!
//! It does not touch the user's real store. Every test opens a database of its own in a temp
//! directory, and the credential goes into a Credential Manager entry named for the test
//! account, which `purge` removes at the end. The Gmail tests learned this the hard way —
//! `docs/PHASE-4-VERIFICATION.md` records the session where a test suite deleted a real OAuth
//! client secret.

use std::time::Instant;

use halcyon_lib::accounts::credentials::{self, Kind, Secret};
use halcyon_lib::accounts::provider::Provider;
use halcyon_lib::accounts::provider::{AuthKind, Security, ServerSettings};
use halcyon_lib::accounts::store::{self, NewAccount};
use halcyon_lib::db::Db;
use halcyon_lib::sync::fetch;
use halcyon_lib::sync::session::{self, Credential};

/// Where the rig is. Overridable, because the Docker host is not always the same machine.
fn host() -> String {
    std::env::var("HALCYON_TEST_IMAP_HOST").unwrap_or_else(|_| "192.168.1.15".to_string())
}

fn port() -> u16 {
    std::env::var("HALCYON_TEST_IMAP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(9993)
}

const TEST_EMAIL: &str = "tester@halcyon.test";
const TEST_PASSWORD: &str = "halcyon-test-only";

fn servers() -> (ServerSettings, ServerSettings) {
    let imap = ServerSettings {
        host: host(),
        port: port(),
        security: Security::Tls,
    };
    // No SMTP in the rig yet; Phase 7 adds one. The field is required, so it points at the
    // same host and is never connected to.
    let smtp = ServerSettings {
        host: host(),
        port: 9587,
        security: Security::StartTls,
    };

    (imap, smtp)
}

fn mark(name: &str, started: Instant) {
    println!("  [{:>7.2}s] {name}", started.elapsed().as_secs_f64());
}

/// A store of its own, in a temp directory, with the test account in it.
///
/// Never the user's real database. See the module header.
struct Rig {
    db: Db,
    account_id: i64,
    _dir: tempfile::TempDir,
}

impl Rig {
    async fn open() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("halcyon-gate")
            .tempdir()
            .expect("temp dir");
        let path = dir.path().join("gate.db");
        let db = Db::open(&path).expect("open store");

        let (imap, smtp) = servers();
        let account = NewAccount {
            display_name: "Dovecot gate".into(),
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

        // The password goes through the same Credential Manager path the app uses, so this
        // exercises the real credential lookup rather than a shortcut around it.
        let reference = credentials::reference_for(TEST_EMAIL);
        credentials::store(
            &reference,
            Kind::Password,
            &Secret::new(TEST_PASSWORD.to_string()),
        )
        .expect("store test password");

        Self {
            db,
            account_id,
            _dir: dir,
        }
    }

    async fn connect(&self) -> (halcyon_lib::sync::session::ImapSession, session::Caps) {
        let account = self
            .db
            .read({
                let id = self.account_id;
                move |conn| store::get(conn, id)
            })
            .await
            .expect("read account")
            .expect("account exists");

        let imap = account.imap.clone().expect("imap configured");
        let secret = credentials::load(&credentials::reference_for(TEST_EMAIL), Kind::Password)
            .expect("load password");

        session::connect(&imap, &account.email, &Credential::Password(secret))
            .await
            .expect("connect")
    }

    /// Removes the Credential Manager entry this rig created.
    fn purge(&self) {
        let reference = credentials::reference_for(TEST_EMAIL);
        let _ = credentials::purge(&reference);
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.purge();
    }
}

/// The server is reachable, its certificate validates, and it offers what the gate needs.
///
/// First, and separate, because every other test here fails in a confusing way when this one
/// would have failed clearly.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn the_rig_is_up_and_offers_qresync() {
    let started = Instant::now();
    let rig = Rig::open().await;
    mark("store and account created", started);

    let (mut imap_session, caps) = rig.connect().await;
    mark("connected and authenticated", started);

    println!(
        "  caps: condstore={} qresync={} idle={} move={} uidplus={}",
        caps.condstore, caps.qresync, caps.idle, caps.move_command, caps.uidplus
    );

    assert!(
        caps.condstore,
        "CONDSTORE is what the incremental path needs"
    );
    assert!(
        caps.qresync,
        "QRESYNC is the whole reason this rig exists — Gmail does not offer it"
    );
    assert!(caps.idle);
    assert!(caps.move_command);
    assert!(caps.uidplus);

    let selected = fetch::select(&mut imap_session, "INBOX", None, caps)
        .await
        .expect("select INBOX");

    println!(
        "  INBOX exists={} uid_next={} modseq={:?}",
        selected.exists, selected.uid_next, selected.highest_modseq
    );

    assert!(
        selected.exists >= 50_000,
        "the gate asks for a 50k-message mailbox; found {}. Run test/dovecot/seed.sh 50000",
        selected.exists
    );
    assert!(
        selected.highest_modseq.is_some(),
        "a CONDSTORE server must report HIGHESTMODSEQ on SELECT"
    );

    let _ = imap_session.logout().await;
}

/// Collects the events the engine emits, so the gate can assert on them.
///
/// The engine used to require a `tauri::AppHandle` purely to call `emit`, which made it
/// undrivable from a test: Tauri's own mock runtime does not load on Windows at all — the test
/// binary dies at start with `STATUS_ENTRYPOINT_NOT_FOUND` before running a line.
///
/// `sync::events::Events` exists because of that, and this is the whole of the test-side
/// implementation. It is also strictly better than a mock handle would have been, because the
/// gate can now check what the engine *said* as well as what it stored.
#[derive(Default)]
struct Recorder {
    seen: std::sync::Mutex<Vec<String>>,
}

impl halcyon_lib::sync::events::Events for Recorder {
    fn emit(&self, event: &str, _payload: serde_json::Value) {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(event.to_string());
        }
    }
}

impl Recorder {
    fn count(&self, event: &str) -> usize {
        self.seen
            .lock()
            .map(|seen| seen.iter().filter(|name| *name == event).count())
            .unwrap_or(0)
    }
}

async fn message_count(db: &Db, account_id: i64) -> i64 {
    db.read(move |conn| {
        Ok(conn
            .query_row(
                "SELECT COUNT(*) FROM message WHERE account_id = ?1",
                rusqlite::params![account_id],
                |row| row.get(0),
            )
            .unwrap_or(0))
    })
    .await
    .expect("count")
}

/// Every `(mailbox_id, uid)` appears at most once.
///
/// The property the whole idempotent-write design exists for, checked directly rather than
/// inferred from a total that happens to look right.
async fn duplicate_count(db: &Db, account_id: i64) -> i64 {
    db.read(move |conn| {
        Ok(conn
            .query_row(
                "SELECT COUNT(*) FROM (
                     SELECT mailbox_id, uid FROM message WHERE account_id = ?1
                     GROUP BY mailbox_id, uid HAVING COUNT(*) > 1
                 )",
                rusqlite::params![account_id],
                |row| row.get(0),
            )
            .unwrap_or(0))
    })
    .await
    .expect("duplicates")
}

/// **Gate 1** — a cold sync of a 50,000-message mailbox completes, and is correct.
///
/// "Completes" is the easy half. "Correct" is checked three ways: the count matches what the
/// server reported, no `(mailbox_id, uid)` pair appears twice, and every message has been
/// given a thread.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn gate_1_cold_sync_of_fifty_thousand() {
    let started = Instant::now();
    let rig = Rig::open().await;
    let app = Recorder::default();
    let engine = halcyon_lib::sync::engine::SyncEngine::new();

    println!("  cold sync starting (this is the long one)");
    engine
        .sync_account(&app, &rig.db, rig.account_id)
        .await
        .expect("cold sync");
    mark("cold sync finished", started);

    let total = message_count(&rig.db, rig.account_id).await;
    let duplicates = duplicate_count(&rig.db, rig.account_id).await;

    let threadless: i64 = rig
        .db
        .read({
            let id = rig.account_id;
            move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT COUNT(*) FROM message WHERE account_id = ?1 AND thread_id IS NULL",
                        rusqlite::params![id],
                        |row| row.get(0),
                    )
                    .unwrap_or(-1))
            }
        })
        .await
        .expect("threadless");

    println!("  stored={total} duplicates={duplicates} without a thread={threadless}");
    println!(
        "  events: progress={} messages:added={}",
        app.count("sync:progress"),
        app.count("messages:added")
    );

    assert_eq!(duplicates, 0, "a message was stored twice");
    assert!(
        total >= 50_000,
        "expected the whole mailbox, stored {total}"
    );

    // The assertion this gate was written for, and the one that caught a real bug on its first
    // run: every message stored correctly, and 45,000 of 50,000 with no thread, because only
    // the newest `RETHREAD_WINDOW` were ever threaded and the promised full pass at the end of
    // a sync did not exist. Nothing failed and no unit test noticed — the reader would simply
    // have shown nine messages in ten as a conversation of one.
    assert_eq!(
        threadless, 0,
        "{threadless} of {total} messages have no thread; the full pass at the end of a sync \
         is what covers everything older than the batch window"
    );

    // The seed builds threads of twenty, so a correctly threaded 50,000-message mailbox has
    // about 2,500 threads. An implementation that gave every message its own thread would pass
    // the check above and fail this one.
    let threads: i64 = rig
        .db
        .read({
            let id = rig.account_id;
            move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT COUNT(*) FROM thread WHERE account_id = ?1",
                        rusqlite::params![id],
                        |row| row.get(0),
                    )
                    .unwrap_or(-1))
            }
        })
        .await
        .expect("threads");

    println!("  threads={threads} (the seed builds chains of twenty)");
    assert!(
        (2_000..=3_500).contains(&threads),
        "expected roughly 2,500 threads from chains of twenty, found {threads}"
    );
}

/// Runs a shell command on the Docker host over SSH.
///
/// The gate needs to be rude to the server — stop it mid-sync, renumber a mailbox, change a
/// flag as if from another device — and none of that is expressible over IMAP as a client.
fn on_host(command: &str) -> String {
    let output = std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", &ssh_target(), command])
        .output()
        .expect("ssh to the docker host");

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn ssh_target() -> String {
    std::env::var("HALCYON_TEST_SSH").unwrap_or_else(|_| "macstudio".to_string())
}

/// `docker compose` in the rig's directory, with the PATH Docker Desktop needs.
fn compose(args: &str) -> String {
    on_host(&format!(
        "export PATH=/usr/local/bin:$PATH; export DOCKER_BUILDKIT=0; \
         cd ~/halcyon-test/dovecot && docker compose {args} 2>&1 | tail -5"
    ))
}

/// **Gate 2** — killing the connection mid-sync loses nothing and duplicates nothing.
///
/// The server is stopped while a cold sync is in flight, then started again and the sync
/// re-run. What matters is not that the first attempt fails — it must — but that the second
/// one arrives at exactly the same mailbox it would have reached uninterrupted.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn gate_2_a_killed_connection_loses_nothing() {
    let rig = Rig::open().await;
    let app = Recorder::default();
    let engine = halcyon_lib::sync::engine::SyncEngine::new();
    let started = Instant::now();

    // Start a sync and pull the server out from under it. Five seconds is comfortably inside
    // the cold sync of a 50,000-message mailbox, which takes over a minute.
    let killer = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let out = compose("stop");
        println!("  server stopped mid-sync: {out}");
    });

    let first = engine.sync_account(&app, &rig.db, rig.account_id).await;
    killer.await.expect("killer");
    mark(&format!("first sync ended: {first:?}"), started);

    let after_kill = message_count(&rig.db, rig.account_id).await;
    println!("  stored before the kill: {after_kill}");

    assert!(
        first.is_err(),
        "the sync reported success while the server was being stopped"
    );

    compose("start");
    // The container needs a moment to accept connections again.
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    mark("server back up", started);

    engine
        .sync_account(&app, &rig.db, rig.account_id)
        .await
        .expect("second sync");
    mark("second sync finished", started);

    let total = message_count(&rig.db, rig.account_id).await;
    let duplicates = duplicate_count(&rig.db, rig.account_id).await;

    println!("  after recovery: stored={total} duplicates={duplicates}");

    // Both halves matter. Duplicates would mean the idempotent write is not; a short count
    // would mean the interrupted range was skipped rather than retried.
    assert_eq!(duplicates, 0, "recovery created duplicates");
    assert_eq!(
        total, 50_000,
        "recovery did not reach the whole mailbox: {total}"
    );
}

/// **Gate 3** — a flag changed on another device shows up within five seconds.
///
/// `doveadm` plays the other device. The point is the *reconciliation*, not IDLE: the flag is
/// changed on the server by something that is not us, and the next sync has to notice — which
/// is the CONDSTORE path, and the half of "stays correct" a UID-range sync cannot see.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn gate_3_a_flag_changed_elsewhere_arrives() {
    let rig = Rig::open().await;
    let app = Recorder::default();
    let engine = halcyon_lib::sync::engine::SyncEngine::new();

    engine
        .sync_account(&app, &rig.db, rig.account_id)
        .await
        .expect("initial sync");

    // A message deep in the mailbox, not in the newest page. That is the whole point:
    // re-reading the newest page would find a change near the top and miss this one.
    let uid = 137;
    let before: bool = rig
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT flag_flagged FROM message WHERE uid = ?1",
                    rusqlite::params![uid],
                    |row| row.get(0),
                )
                .unwrap_or(false))
        })
        .await
        .expect("read flag");

    println!("  uid {uid} flagged before: {before}");
    assert!(!before, "the fixture message is already flagged");

    on_host(&format!(
        "export PATH=/usr/local/bin:$PATH; \
         docker exec halcyon-dovecot doveadm flags add -u tester@halcyon.test \
         '\\Flagged' MAILBOX INBOX UID {uid}"
    ));
    println!("  flag set on the server by doveadm");

    let started = Instant::now();
    engine
        .sync_account(&app, &rig.db, rig.account_id)
        .await
        .expect("reconciling sync");
    let elapsed = started.elapsed();

    let after: bool = rig
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT flag_flagged FROM message WHERE uid = ?1",
                    rusqlite::params![uid],
                    |row| row.get(0),
                )
                .unwrap_or(false))
        })
        .await
        .expect("read flag");

    println!(
        "  uid {uid} flagged after: {after} (sync took {:.2}s)",
        elapsed.as_secs_f64()
    );

    assert!(after, "a flag set on the server was not picked up");
    assert!(
        elapsed.as_secs() < 5,
        "the gate asks for five seconds; the reconciling sync took {:.2}s",
        elapsed.as_secs_f64()
    );

    // Put it back, so the test can run twice.
    on_host(&format!(
        "export PATH=/usr/local/bin:$PATH; \
         docker exec halcyon-dovecot doveadm flags remove -u tester@halcyon.test \
         '\\Flagged' MAILBOX INBOX UID {uid}"
    ));
}

/// **Gate 4** — a `UIDVALIDITY` reset is handled.
///
/// The recovery docs/03 §5 specifies is blunt on purpose: *drop and re-sync that mailbox. Do
/// not try to be clever.* Every UID held for it now refers to a different message, so keeping
/// any of them would silently attach the wrong flags — and the wrong read state — to the wrong
/// mail. Wrong quietly, which is the worst way to be wrong in a mail client.
///
/// It cannot be provoked on Gmail at all, which is a large part of why this rig exists. Here
/// it is caused the way a server really would: the UID list is deleted, and Dovecot rebuilds
/// it with a new validity and renumbers every message from one.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn gate_4_a_uidvalidity_reset_is_recovered() {
    let rig = Rig::open().await;
    let app = Recorder::default();
    let engine = halcyon_lib::sync::engine::SyncEngine::new();
    let started = Instant::now();

    engine
        .sync_account(&app, &rig.db, rig.account_id)
        .await
        .expect("initial sync");
    mark("initial sync", started);

    let before = message_count(&rig.db, rig.account_id).await;
    let uid_before: i64 = rig
        .db
        .read(move |conn| {
            Ok(conn
                .query_row("SELECT MAX(uid) FROM message", [], |row| row.get(0))
                .unwrap_or(0))
        })
        .await
        .expect("max uid");

    println!("  before the reset: stored={before} max uid={uid_before}");

    // Stop, remove the UID list and indexes, start. Dovecot rebuilds both, picks a new
    // UIDVALIDITY, and renumbers the same messages from 1 — exactly what a restore from backup
    // looks like to a client.
    compose("stop");
    on_host(
        "export PATH=/usr/local/bin:$PATH; \
         docker run --rm -v dovecot_maildata:/srv/mail alpine sh -c \
         'rm -f /srv/mail/tester@halcyon.test/Maildir/dovecot-uidlist \
                /srv/mail/tester@halcyon.test/Maildir/dovecot.index* \
                /srv/mail/tester@halcyon.test/Maildir/dovecot.list.index*'",
    );
    compose("start");
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    mark("mailbox renumbered by the server", started);

    engine
        .sync_account(&app, &rig.db, rig.account_id)
        .await
        .expect("sync after the reset");
    mark("sync after the reset", started);

    let after = message_count(&rig.db, rig.account_id).await;
    let duplicates = duplicate_count(&rig.db, rig.account_id).await;

    println!("  after recovery: stored={after} duplicates={duplicates}");

    // The mailbox is whole, and holds each message exactly once. A client that kept the old
    // UIDs would show either double the mail or half of it.
    assert_eq!(duplicates, 0, "the reset left duplicate rows behind");
    assert_eq!(
        after, 50_000,
        "the mailbox was not fully re-synced after the reset: {after}"
    );
}
