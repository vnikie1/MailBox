//! Fixes from the 2026-11-14 audit that only a real server can show, against the rig.
//!
//! `#[ignore]`d — it needs the rig in `test/dovecot` running and its CA trusted, so it never runs
//! in CI or in `npm run verify`. Run with:
//!
//!   cargo test --test audit_gate -- --ignored --nocapture --test-threads=1
//!
//! Not at the same time as `dovecot_gate` or `folders_gate`, nor while the app is running against
//! the rig: all of them keep the rig's password under the same Credential Manager entry, and each
//! removes it when it finishes.
//!
//! - **L3**: a folder whose name holds a quote or a backslash. The IMAP parser hands back a quoted
//!   string with its escapes still in, so `"Q4" plans` was stored as `\"Q4\" plans`, shown so, and
//!   escaped a second time when it was selected, which the server refused.
//! - **L9**: an interrupted Redirect. Recovery asks Sent whether the message got there. It asked
//!   by the original's `Message-ID`, so a redirect of a message already in Sent was marked sent
//!   when it had not gone; and the id went into the search unescaped.
//!
//! Everything this puts on the server carries a per-run suffix and is removed at the end, pass or
//! fail. Nothing here touches the Inbox, whose size and UIDs the other gates depend on.

use std::panic::AssertUnwindSafe;

use futures::FutureExt;

use halcyon_lib::accounts::credentials::{self, Kind, Secret};
use halcyon_lib::accounts::provider::{AuthKind, Provider, Security, ServerSettings};
use halcyon_lib::accounts::store::{self, NewAccount};
use halcyon_lib::db::Db;
use halcyon_lib::mail::outgoing;
use halcyon_lib::sync::envelope::Address;
use halcyon_lib::sync::events::Events;
use halcyon_lib::sync::mailboxes;
use halcyon_lib::sync::sender;
use halcyon_lib::sync::session::{self, Credential, ImapSession};

fn host() -> String {
    std::env::var("HALCYON_TEST_IMAP_HOST").unwrap_or_else(|_| "192.168.1.15".to_string())
}

fn port() -> u16 {
    std::env::var("HALCYON_TEST_IMAP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(9993)
}

const CONNECT: &str = "connect to the rig — if the TLS handshake failed, its certificate may have \
     expired: test/dovecot/README.md, \"The certificate\", says how to renew it";

const TEST_EMAIL: &str = "tester@halcyon.test";
const TEST_PASSWORD: &str = "halcyon-test-only";

/// A store of its own, never the user's, with the rig's account in it.
struct Rig {
    db: Db,
    account_id: i64,
    suffix: String,
    _dir: tempfile::TempDir,
}

impl Rig {
    async fn open() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("halcyon-audit-gate")
            .tempdir()
            .expect("temp dir");
        let db = Db::open(&dir.path().join("gate.db")).expect("open store");

        let account = NewAccount {
            display_name: "Audit gate".into(),
            email: TEST_EMAIL.into(),
            provider: Provider::Other,
            imap: ServerSettings {
                host: host(),
                port: port(),
                security: Security::Tls,
            },
            smtp: ServerSettings {
                host: host(),
                port: 9587,
                security: Security::StartTls,
            },
            auth_kind: AuthKind::Password,
            color: None,
        };

        let account_id = db
            .write(move |tx| store::insert(tx, &account))
            .await
            .expect("insert account");

        credentials::store(
            &credentials::reference_for(TEST_EMAIL),
            Kind::Password,
            &Secret::new(TEST_PASSWORD.to_string()),
        )
        .expect("store test password");

        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis().to_string())
            .unwrap_or_default();

        Self {
            db,
            account_id,
            suffix,
            _dir: dir,
        }
    }

    async fn connect(&self) -> ImapSession {
        self.connect_with_caps().await.0
    }

    async fn connect_with_caps(&self) -> (ImapSession, session::Caps) {
        let imap = ServerSettings {
            host: host(),
            port: port(),
            security: Security::Tls,
        };
        let secret = credentials::load(&credentials::reference_for(TEST_EMAIL), Kind::Password)
            .expect("load password");

        session::connect(&imap, TEST_EMAIL, &Credential::Password(secret))
            .await
            .expect(CONNECT)
    }

    /// What `engine::run_once` does first: list the folders and write them, roles and all.
    async fn sync_tree(&self, imap: &mut ImapSession) -> Vec<mailboxes::Discovered> {
        let listed = mailboxes::discover(imap).await.expect("LIST");
        let account_id = self.account_id;
        let to_persist = listed.clone();
        self.db
            .write(move |tx| mailboxes::persist(tx, account_id, &to_persist))
            .await
            .expect("persist");
        listed
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = credentials::purge(&credentials::reference_for(TEST_EMAIL));
    }
}

struct Quiet;

impl Events for Quiet {
    fn emit(&self, _event: &str, _payload: serde_json::Value) {}
}

/// Removes every message in `mailbox` whose `field` header holds `needle`, by UID. Never `1:*`.
async fn remove_matching(imap: &mut ImapSession, mailbox: &str, field: &str, needle: &str) {
    use futures::StreamExt;

    if imap.select(mailbox).await.is_err() {
        return;
    }
    let Ok(found) = imap
        .uid_search(format!("HEADER {field} \"{needle}\""))
        .await
    else {
        return;
    };
    if found.is_empty() {
        return;
    }

    let set = found
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    if let Ok(stream) = imap.uid_store(&set, "+FLAGS.SILENT (\\Deleted)").await {
        let _: Vec<_> = stream.collect().await;
    }
    if let Ok(stream) = imap.uid_expunge(&set).await {
        futures::pin_mut!(stream);
        while stream.next().await.is_some() {}
    }
}

/// The local row for a folder: its id and the name the sidebar shows.
async fn local(rig: &Rig, path: &str) -> (i64, String) {
    let account_id = rig.account_id;
    let path = path.to_string();
    rig.db
        .read(move |conn| {
            Ok(conn.query_row(
                "SELECT id, display_name FROM mailbox WHERE account_id = ?1 AND remote_path = ?2",
                rusqlite::params![account_id, path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .await
        .expect("the folder's row")
}

/// L3, and the two commands it left: a folder named with a quote, a space or a backslash is
/// listed as itself, opens, takes mail moved into it, and takes a draft.
///
/// The moves go by COPY, as they do on a server without MOVE: async-imap writes COPY's mailbox
/// as given, with no quotes, so a move into any folder with a space in its name was a syntax
/// error. And its APPEND quotes the name without escaping it, so a draft or a sent copy could not
/// be put in a folder with a quote or a backslash in its name. Found running this gate.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn a_folder_named_with_a_quote_or_a_backslash_lists_opens_and_takes_mail() {
    use halcyon_lib::sync::ops;

    let rig = Rig::open().await;
    let mut imap = rig.connect().await;

    let quoted = format!("Audit \"Q4\" plans {}", rig.suffix);
    let slashed = format!("Audit back\\slash {}", rig.suffix);
    let source = format!("Audit source {}", rig.suffix);

    let outcome = AssertUnwindSafe(async {
        for name in [&quoted, &slashed, &source] {
            imap.create(name.as_str()).await.expect("CREATE");
        }
        // Two messages to move, in a folder whose name needs no escaping.
        for n in 1..=2 {
            let message = format!(
                "From: gate@halcyon.test\r\nSubject: audit gate L3 {n}\r\n\
                 Message-ID: <audit-l3-{n}-{}@halcyon.test>\r\n\r\nL3.\r\n",
                rig.suffix
            );
            imap.append(source.as_str(), None, None, message.as_bytes())
                .await
                .expect("APPEND to the source folder");
        }

        let listed = rig.sync_tree(&mut imap).await;
        let paths: Vec<&str> = listed.iter().map(|m| m.remote_path.as_str()).collect();

        for name in [&quoted, &slashed] {
            assert!(
                paths.contains(&name.as_str()),
                "{name:?} was not listed under its own name: {:?}",
                paths
                    .iter()
                    .filter(|path| path.contains(&rig.suffix))
                    .collect::<Vec<_>>()
            );
            let (_, shown) = local(&rig, name).await;
            assert_eq!(&shown, name, "the sidebar would show {shown:?}");
            imap.select(name.as_str())
                .await
                .expect("SELECT by the stored name");
            println!("  {name:?} listed as itself, shown as itself, and opened");
        }

        // Each source message moved into one of them, as `msg_move` leaves it, and sent the way
        // a server without MOVE gets it.
        let uids: Vec<u32> = {
            imap.select(source.as_str()).await.expect("SELECT source");
            let mut found: Vec<u32> = imap
                .uid_search("ALL")
                .await
                .expect("SEARCH")
                .into_iter()
                .collect();
            found.sort_unstable();
            found
        };
        assert_eq!(uids.len(), 2);

        let (source_id, _) = local(&rig, &source).await;
        let (quoted_id, _) = local(&rig, &quoted).await;
        let (slashed_id, _) = local(&rig, &slashed).await;
        let account_id = rig.account_id;
        let destinations = [(quoted.clone(), quoted_id), (slashed.clone(), slashed_id)];
        rig.db
            .write(move |tx| {
                for (index, (to, to_id)) in destinations.iter().enumerate() {
                    let id = 900_301 + index as i64;
                    tx.execute(
                        "INSERT INTO message (id, account_id, mailbox_id, uid, subject, date_sent,
                             date_received, size, from_all, to_all, body_text, has_attachment,
                             flag_seen, flag_flagged, is_junk)
                         VALUES (?1, ?2, ?3, ?4, 'Moving', 0, 0, 1, '', '', '', 0, 1, 0, 0)",
                        rusqlite::params![id, account_id, source_id, uids[index]],
                    )?;
                    for group in ops::locate(tx, &[id])? {
                        ops::enqueue(
                            tx,
                            account_id,
                            &ops::Op::Move {
                                from: group.mailbox,
                                to: to.clone(),
                                uids: group.uids,
                            },
                        )?;
                    }
                    halcyon_lib::db::write::move_to(tx, &[id], *to_id)?;
                }
                Ok(())
            })
            .await
            .expect("queue the moves");

        let drained = ops::drain(&Quiet, &rig.db, &mut imap, account_id, false).await;
        println!("  drained the moves by COPY: {drained:?}");

        for name in [&quoted, &slashed] {
            let selected = imap.select(name.as_str()).await.expect("SELECT");
            assert_eq!(
                selected.exists, 1,
                "the message moved by COPY is not in {name:?}"
            );
        }
        let left = imap.select(source.as_str()).await.expect("SELECT source");
        assert_eq!(left.exists, 0, "a moved message is still in the source");
        println!(
            "  both messages moved by COPY into the folders named with a quote and a backslash"
        );

        // A draft saved into the folder named with a quote, through the queue a save uses.
        let eml = rig._dir.path().join("draft.eml");
        let draft_id = format!("<audit-l3-draft-{}@halcyon.test>", rig.suffix);
        std::fs::write(
            &eml,
            format!(
                "From: tester@halcyon.test\r\nSubject: audit gate draft\r\n\
                 Message-ID: {draft_id}\r\n\r\nA draft.\r\n"
            ),
        )
        .expect("write the draft");
        let draft = ops::Op::AppendDraft {
            mailbox: quoted.clone(),
            eml_path: eml.to_string_lossy().into_owned(),
            replaces: None,
            message_id: draft_id,
        };
        rig.db
            .write(move |tx| ops::enqueue(tx, account_id, &draft))
            .await
            .expect("queue the draft");

        let drained = ops::drain(&Quiet, &rig.db, &mut imap, account_id, true).await;
        println!("  drained the draft: {drained:?}");
        let selected = imap.select(quoted.as_str()).await.expect("SELECT");
        assert_eq!(selected.exists, 2, "the draft is not in {quoted:?}");
        println!("  the draft was appended into the folder named with a quote");
    })
    .catch_unwind()
    .await;

    // Tidy, pass or fail.
    let _ = imap.examine("INBOX").await;
    for name in [&quoted, &slashed, &source] {
        let _ = imap.delete(name.as_str()).await;
    }
    let _ = imap.logout().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// The `Sent` folder's path, from the roles the tree sync wrote.
async fn sent_path(rig: &Rig) -> String {
    let account_id = rig.account_id;
    rig.db
        .read(move |conn| {
            Ok(conn.query_row(
                "SELECT remote_path FROM mailbox WHERE account_id = ?1 AND role = 'sent'",
                rusqlite::params![account_id],
                |row| row.get(0),
            )?)
        })
        .await
        .expect("the rig has a Sent folder")
}

/// An outbox row a crash left `sending`, as `compose_redirect` writes it.
async fn interrupted(rig: &Rig, message_id: &str) -> i64 {
    let account_id = rig.account_id;
    let message_id = message_id.to_string();
    rig.db
        .write(move |tx| {
            tx.execute(
                "INSERT INTO outbox (account_id, eml_path, state, send_after, message_id, created_at)
                 VALUES (?1, 'not-read-by-recovery.eml', 'sending', 0, ?2, 0)",
                rusqlite::params![account_id, message_id],
            )?;
            Ok(tx.last_insert_rowid())
        })
        .await
        .expect("outbox row")
}

async fn state_of(rig: &Rig, id: i64) -> String {
    rig.db
        .read(move |conn| {
            Ok(conn.query_row(
                "SELECT state FROM outbox WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )?)
        })
        .await
        .expect("state")
}

/// L9. A redirect of a message already in Sent is not taken for sent until it is there itself,
/// and an id with a quote and a backslash in it is searched for as written.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn an_interrupted_redirect_is_sent_only_when_sent_holds_it() {
    let rig = Rig::open().await;
    let mut imap = rig.connect().await;
    rig.sync_tree(&mut imap).await;
    let sent = sent_path(&rig).await;
    let suffix = rig.suffix.clone();

    let original_id = format!("<audit-l9-original-{suffix}@stranger.test>");
    let original = format!(
        "From: Stranger <someone@stranger.test>\r\nTo: tester@halcyon.test\r\n\
         Subject: audit gate L9\r\nMessage-ID: {original_id}\r\n\
         Date: Sun, 15 Nov 2026 12:00:00 +0000\r\n\r\nThe original.\r\n"
    );

    let outcome = AssertUnwindSafe(async {
        // The original is in Sent already: a message of the user's own, being redirected.
        imap.append(sent.as_str(), None, None, original.as_bytes())
            .await
            .expect("APPEND the original to Sent");

        let built = outgoing::redirect(&outgoing::Redirect {
            original: original.as_bytes(),
            from: Address {
                name: None,
                email: TEST_EMAIL.into(),
            },
            to: vec![Address {
                name: None,
                email: "someone-else@halcyon.test".into(),
            }],
            cc: Vec::new(),
            bcc: Vec::new(),
            date: "Sun, 15 Nov 2026 12:05:00 +0000".into(),
            resent_message_id: Some(format!("<audit-l9-resent-{suffix}@halcyon.test>")),
        })
        .expect("build the redirect");

        assert_ne!(
            built.message_id, original_id,
            "the redirect is known by the original's id"
        );

        // The crash came before the redirect reached Sent.
        let row = interrupted(&rig, &built.message_id).await;
        sender::recover(&Quiet, &rig.db).await.expect("recover");
        assert_eq!(
            state_of(&rig, row).await,
            "queued",
            "a redirect that never reached Sent was taken for sent, because the original is there"
        );
        println!("  not in Sent: back to the queue, though the original is in Sent");

        // Now it is there.
        imap.append(sent.as_str(), None, None, &built.bytes)
            .await
            .expect("APPEND the redirect to Sent");
        rig.db
            .write(move |tx| {
                tx.execute(
                    "UPDATE outbox SET state = 'sending' WHERE id = ?1",
                    rusqlite::params![row],
                )?;
                Ok(())
            })
            .await
            .expect("interrupt it again");
        sender::recover(&Quiet, &rig.db).await.expect("recover");
        assert_eq!(
            state_of(&rig, row).await,
            "sent",
            "the filed redirect was not found"
        );
        println!("  in Sent: found by its Resent-Message-ID and marked sent");

        // An id with a quote and a backslash in it, as anyone may write one.
        let awkward = format!("<audit-l9 \"quoted\\{suffix}@stranger.test>");
        let message = format!(
            "From: tester@halcyon.test\r\nTo: someone@stranger.test\r\n\
             Subject: audit gate L9 escaping\r\nMessage-ID: {awkward}\r\n\r\nEscaping.\r\n"
        );
        imap.append(sent.as_str(), None, None, message.as_bytes())
            .await
            .expect("APPEND the awkward one");
        let row = interrupted(&rig, &awkward).await;
        sender::recover(&Quiet, &rig.db).await.expect("recover");
        assert_eq!(
            state_of(&rig, row).await,
            "sent",
            "an id with a quote and a backslash was not found; an unescaped search fails here"
        );
        println!("  an id with a quote and a backslash: searched as written, found");
    })
    .catch_unwind()
    .await;

    // Tidy, pass or fail: only this run's messages, by UID.
    remove_matching(&mut imap, &sent, "Message-ID", &suffix).await;
    remove_matching(&mut imap, &sent, "Resent-Message-ID", &suffix).await;
    let _ = imap.logout().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// L2. A mailbox that keeps no mod-sequences, on a server that has CONDSTORE, syncs.
///
/// The rig's `Volatile/` namespace keeps its indexes in memory, so Dovecot answers SELECT there
/// with `NOMODSEQ` while every other mailbox has a `HIGHESTMODSEQ`, and refuses `FETCH MODSEQ`
/// and `CHANGEDSINCE` with "can't be used with non-permanent modseqs". Before the fix every sync
/// of such a mailbox asked for both and failed. Two passes, because the second is the one that
/// asks for changes since the first.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn a_mailbox_without_mod_sequences_syncs_on_a_server_with_condstore() {
    use halcyon_lib::sync::{engine::SyncEngine, fetch};

    let rig = Rig::open().await;
    let (mut imap, caps) = rig.connect_with_caps().await;
    let name = format!("Volatile/Audit {}", rig.suffix);

    let outcome = AssertUnwindSafe(async {
        imap.create(name.as_str()).await.expect(
            "CREATE in Volatile/: the rig needs the in-memory namespace in test/dovecot/dovecot.conf",
        );
        for n in 1..=3 {
            let message = format!("From: gate@halcyon.test\r\nSubject: audit gate L2 {n}\r\n\r\nL2.\r\n");
            imap.append(name.as_str(), None, None, message.as_bytes())
                .await
                .expect("APPEND");
        }

        // The case itself: CONDSTORE on the server, and none in this mailbox.
        let archive = fetch::select(&mut imap, "Archive", None, caps)
            .await
            .expect("SELECT Archive");
        assert!(archive.highest_modseq.is_some(), "the rig no longer keeps mod-sequences at all");
        let volatile = fetch::select(&mut imap, &name, None, caps)
            .await
            .expect("SELECT the volatile mailbox");
        assert!(
            volatile.highest_modseq.is_none(),
            "{name:?} reported mod-sequences, so it does not test L2"
        );
        println!("  Archive has a HIGHESTMODSEQ; {name:?} answered NOMODSEQ");

        rig.sync_tree(&mut imap).await;
        let (folder_id, _) = local(&rig, &name).await;

        let engine = SyncEngine::new();
        let account_id = rig.account_id;
        let held = || async {
            rig.db
                .read(move |conn| {
                    Ok(conn.query_row(
                        "SELECT COUNT(*), COALESCE(SUM(flag_flagged), 0) FROM message
                          WHERE mailbox_id = ?1",
                        rusqlite::params![folder_id],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )?)
                })
                .await
                .expect("read")
        };

        engine
            .sync_mailboxes(&Quiet, &rig.db, account_id, &[folder_id])
            .await
            .expect("the first sync of a mailbox without mod-sequences");
        assert_eq!(held().await, (3, 0));
        println!("  first pass: all three messages");

        // A change made elsewhere: one more message, and one flagged.
        imap.append(
            name.as_str(),
            None,
            None,
            b"From: gate@halcyon.test\r\nSubject: audit gate L2 4\r\n\r\nL2.\r\n".as_slice(),
        )
        .await
        .expect("APPEND");
        imap.select(name.as_str()).await.expect("SELECT");
        {
            use futures::StreamExt;
            let stream = imap
                .uid_store("1", r"+FLAGS.SILENT (\Flagged)")
                .await
                .expect("STORE");
            let _: Vec<_> = stream.collect().await;
        }

        engine
            .sync_mailboxes(&Quiet, &rig.db, account_id, &[folder_id])
            .await
            .expect("the second sync of a mailbox without mod-sequences");
        assert_eq!(held().await, (4, 1), "the second pass missed the new message or the flag");
        println!("  second pass: the new message and the flag set elsewhere");
    })
    .catch_unwind()
    .await;

    let _ = imap.examine("INBOX").await;
    let _ = imap.delete(name.as_str()).await;
    let _ = imap.logout().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
