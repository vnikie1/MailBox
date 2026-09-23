//! The mailbox context menu's server half, against a real IMAP server.
//!
//! `#[ignore]`d — it needs the rig in `test/dovecot` running and its CA trusted, so it never runs
//! in CI or in `npm run verify`. Run with:
//!
//!   cargo test --test folders_gate -- --ignored --nocapture --test-threads=1
//!
//! Not at the same time as `dovecot_gate`: both keep the rig's password under the same
//! Credential Manager entry, and each removes it when it finishes.
//!
//! ## What this proves that the unit tests cannot
//!
//! `sync::folders` and `ops` are unit-tested against a database, which shows what gets queued.
//! Whether Dovecot then does what the queue asks — creates the folder under the name that was
//! encoded, renames it with its children and its mail, deletes it, empties the Bin — and what the
//! drain makes of a real refusal, is only answered by sending it. Every folder this makes carries
//! a per-run suffix and is removed at the end, pass or fail.
//!
//! What it does not do is run a whole sync: the rig's Inbox holds fifty thousand messages, and a
//! full pass is `dovecot_gate`'s business. The pieces a sync runs around the queue — `discover`,
//! `persist`, `prune` — are called directly, in the order `engine::run_once` calls them.

use std::sync::Mutex;

use halcyon_lib::accounts::credentials::{self, Kind, Secret};
use halcyon_lib::accounts::provider::{AuthKind, Provider, Security, ServerSettings};
use halcyon_lib::accounts::store::{self, NewAccount};
use halcyon_lib::db::Db;
use halcyon_lib::sync::events::Events;
use halcyon_lib::sync::mailboxes::{self, Role};
use halcyon_lib::sync::session::{self, Caps, Credential, ImapSession};
use halcyon_lib::sync::{folders, ops};

fn host() -> String {
    std::env::var("HALCYON_TEST_IMAP_HOST").unwrap_or_else(|_| "192.168.1.15".to_string())
}

fn port() -> u16 {
    std::env::var("HALCYON_TEST_IMAP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(9993)
}

/// What a failed connection says. The likeliest cause after a quiet spell is the rig's
/// certificate expiring, which otherwise reads as an unexplained TLS failure.
const CONNECT: &str = "connect to the rig — if the TLS handshake failed, its certificate may have \
     expired: test/dovecot/README.md, \"The certificate\", says how to renew it";

const TEST_EMAIL: &str = "tester@halcyon.test";
const TEST_PASSWORD: &str = "halcyon-test-only";

/// A store of its own, never the user's, with the rig's account in it.
struct Rig {
    db: Db,
    account_id: i64,
    suffix: String,
    dir: tempfile::TempDir,
}

impl Rig {
    async fn open() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("halcyon-folders-gate")
            .tempdir()
            .expect("temp dir");
        let db = Db::open(&dir.path().join("gate.db")).expect("open store");

        let account = NewAccount {
            display_name: "Folders gate".into(),
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
            dir,
        }
    }

    async fn connect(&self) -> (ImapSession, Caps) {
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

    /// A name no other run has used.
    fn name(&self, stem: &str) -> String {
        format!("{stem} {}", self.suffix)
    }

    /// What `engine::run_once` does around the queue: list, write, prune.
    async fn sync_tree(&self, imap: &mut ImapSession) -> Vec<mailboxes::Discovered> {
        let listed: Vec<_> = mailboxes::discover(imap)
            .await
            .expect("LIST")
            .into_iter()
            .filter(|mailbox| mailbox.selectable)
            .collect();

        let account_id = self.account_id;
        let to_persist = listed.clone();
        self.db
            .write(move |tx| mailboxes::persist(tx, account_id, &to_persist))
            .await
            .expect("persist");

        let keep: Vec<String> = listed.iter().map(|m| m.remote_path.clone()).collect();
        self.db
            .write(move |tx| mailboxes::prune(tx, account_id, &keep))
            .await
            .expect("prune");

        listed
    }

    async fn drain(&self, imap: &mut ImapSession, caps: Caps, events: &Recorder) -> usize {
        ops::drain(events, &self.db, imap, self.account_id, caps.move_command)
            .await
            .expect("drain")
    }

    async fn pending(&self) -> i64 {
        let account_id = self.account_id;
        self.db
            .write(move |tx| ops::pending_count(tx, account_id))
            .await
            .expect("count")
    }

    /// The local row for a path: its id and its display name.
    async fn local(&self, path: &str) -> Option<(i64, String)> {
        let account_id = self.account_id;
        let path = path.to_string();
        self.db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT id, display_name FROM mailbox
                          WHERE account_id = ?1 AND remote_path = ?2",
                        rusqlite::params![account_id, path],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .ok())
            })
            .await
            .expect("read")
    }

    /// Removes whatever this run left on the server under its suffix.
    async fn tidy(&self) {
        let (mut imap, _) = self.connect().await;
        let Ok(listed) = mailboxes::discover(&mut imap).await else {
            return;
        };

        // Deepest first, as the real delete does.
        let mut ours: Vec<String> = listed
            .into_iter()
            .map(|mailbox| mailbox.remote_path)
            .filter(|path| path.contains(&self.suffix))
            .collect();
        ours.sort_by_key(|path| std::cmp::Reverse(path.len()));

        let _ = imap.examine("INBOX").await;
        for path in ours {
            if let Err(error) = imap.delete(&path).await {
                println!("  could not tidy {path}: {error}");
            }
        }

        let _ = imap.logout().await;
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = credentials::purge(&credentials::reference_for(TEST_EMAIL));
    }
}

/// What the drain told the window.
#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<(String, serde_json::Value)>>,
}

impl Events for Recorder {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push((event.to_string(), payload));
        }
    }
}

impl Recorder {
    /// The mailboxes a rebuild finished for, with how many messages each holds now.
    fn rebuilt(&self) -> Vec<(i64, i64)> {
        self.seen
            .lock()
            .map(|seen| {
                seen.iter()
                    .filter(|(event, _)| event == "mailbox:rebuilt")
                    .filter_map(|(_, payload)| {
                        Some((
                            payload["mailboxId"].as_i64()?,
                            payload["messages"].as_i64()?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn refusals(&self) -> Vec<String> {
        self.seen
            .lock()
            .map(|seen| {
                seen.iter()
                    .filter(|(event, _)| event == "mailbox:refused")
                    .filter_map(|(_, payload)| payload["message"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn paths(listed: &[mailboxes::Discovered]) -> Vec<&str> {
    listed.iter().map(|m| m.remote_path.as_str()).collect()
}

const MESSAGE: &str = "From: gate@halcyon.test\r\n\
To: tester@halcyon.test\r\n\
Subject: folders gate\r\n\
Message-ID: <folders-gate@halcyon.test>\r\n\
\r\n\
Put here by folders_gate.\r\n";

async fn append(imap: &mut ImapSession, path: &str, count: usize) {
    for _ in 0..count {
        imap.append(path, None, None, MESSAGE.as_bytes())
            .await
            .expect("APPEND");
    }
}

async fn exists(imap: &mut ImapSession, path: &str) -> u32 {
    imap.select(path).await.expect("SELECT").exists
}

/// New, Rename and Delete Mailbox, one after another, as the menu sends them.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn a_folder_is_made_renamed_with_its_mail_and_children_and_deleted() {
    let rig = Rig::open().await;
    let events = Recorder::default();
    let (mut imap, caps) = rig.connect().await;
    rig.sync_tree(&mut imap).await;

    let account_id = rig.account_id;
    let plain = rig.name("Gate");
    let accented = rig.name("Reçus");

    // ---- New Mailbox, twice: once plain, once with a name that has to be encoded.
    let (gate_id, recus_id) = {
        let (plain, accented) = (plain.clone(), accented.clone());
        rig.db
            .write(move |tx| {
                let first = folders::create(tx, account_id, &plain).expect("create");
                let second = folders::create(tx, account_id, &accented).expect("create");
                Ok((first, second))
            })
            .await
            .expect("write")
    };

    assert_eq!(rig.drain(&mut imap, caps, &events).await, 2);
    assert_eq!(rig.pending().await, 0);

    let listed = rig.sync_tree(&mut imap).await;
    let wire = format!("Re&AOc-us {}", rig.suffix);
    assert!(
        paths(&listed).contains(&plain.as_str()),
        "{:?}",
        paths(&listed)
    );
    assert!(
        paths(&listed).contains(&wire.as_str()),
        "the accented name reached the server encoded: {:?}",
        paths(&listed)
    );

    // The sync kept the rows it was given, under the names the user typed.
    assert_eq!(rig.local(&plain).await, Some((gate_id, plain.clone())));
    assert_eq!(rig.local(&wire).await, Some((recus_id, accented.clone())));

    // ---- Some mail in it, and a folder inside it made by another client.
    append(&mut imap, &plain, 2).await;
    let child = format!("{plain}/Child");
    imap.create(&child).await.expect("CREATE child");
    rig.sync_tree(&mut imap).await;
    let (child_id, _) = rig.local(&child).await.expect("child row");

    // ---- Rename Mailbox.
    let renamed = rig.name("Renamed");
    {
        let renamed = renamed.clone();
        rig.db
            .write(move |tx| {
                folders::rename(tx, gate_id, &renamed).expect("rename");
                Ok(())
            })
            .await
            .expect("write");
    }

    assert_eq!(rig.drain(&mut imap, caps, &events).await, 1);

    let listed = rig.sync_tree(&mut imap).await;
    let renamed_child = format!("{renamed}/Child");
    assert!(
        paths(&listed).contains(&renamed.as_str()),
        "{:?}",
        paths(&listed)
    );
    assert!(paths(&listed).contains(&renamed_child.as_str()));
    assert!(
        !paths(&listed).contains(&plain.as_str()),
        "the old name is gone"
    );
    assert_eq!(
        exists(&mut imap, &renamed).await,
        2,
        "the mail went with it"
    );

    // Same rows, new paths.
    assert_eq!(rig.local(&renamed).await, Some((gate_id, renamed.clone())));
    assert_eq!(
        rig.local(&renamed_child).await.map(|(id, _)| id),
        Some(child_id)
    );

    // ---- Delete Mailbox, which takes the child too.
    let deleted = rig
        .db
        .write(move |tx| Ok(folders::delete(tx, gate_id).expect("delete")))
        .await
        .expect("write");
    assert_eq!(deleted.mailbox_ids, vec![child_id, gate_id]);

    rig.db
        .write(move |tx| {
            folders::delete(tx, recus_id).expect("delete");
            Ok(())
        })
        .await
        .expect("write");

    assert_eq!(rig.drain(&mut imap, caps, &events).await, 2);

    let listed = rig.sync_tree(&mut imap).await;
    for gone in [&renamed, &renamed_child, &wire] {
        assert!(
            !paths(&listed).contains(&gone.as_str()),
            "{gone} is still on the server"
        );
    }
    assert!(rig.local(&renamed).await.is_none());

    assert!(events.refusals().is_empty(), "{:?}", events.refusals());

    let _ = imap.logout().await;
    rig.tidy().await;
}

/// The race the pending-tree check exists for, with a real listing.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn a_sync_that_listed_the_old_name_does_not_undo_a_rename() {
    let rig = Rig::open().await;
    let events = Recorder::default();
    let (mut imap, caps) = rig.connect().await;

    let before = rig.name("Race");
    let after = rig.name("Raced");
    imap.create(&before).await.expect("CREATE");
    append(&mut imap, &before, 1).await;
    rig.sync_tree(&mut imap).await;
    let (id, _) = rig.local(&before).await.expect("row");

    // A LIST taken, and *then* the user renames: the listing still says "Race".
    let stale: Vec<_> = mailboxes::discover(&mut imap)
        .await
        .expect("LIST")
        .into_iter()
        .filter(|m| m.selectable)
        .collect();

    {
        let after = after.clone();
        rig.db
            .write(move |tx| {
                folders::rename(tx, id, &after).expect("rename");
                Ok(())
            })
            .await
            .expect("write");
    }

    // The stale listing is applied, exactly as the rest of a sync pass would apply it.
    let account_id = rig.account_id;
    {
        let keep: Vec<String> = stale.iter().map(|m| m.remote_path.clone()).collect();
        rig.db
            .write(move |tx| mailboxes::persist(tx, account_id, &stale))
            .await
            .expect("persist");
        rig.db
            .write(move |tx| mailboxes::prune(tx, account_id, &keep))
            .await
            .expect("prune");
    }

    assert!(
        rig.local(&before).await.is_none(),
        "the old name was written back"
    );
    assert_eq!(
        rig.local(&after).await.map(|(row, _)| row),
        Some(id),
        "the renamed folder was pruned"
    );

    // And the rename still lands.
    assert_eq!(rig.drain(&mut imap, caps, &events).await, 1);
    let listed = rig.sync_tree(&mut imap).await;
    assert!(paths(&listed).contains(&after.as_str()));
    assert_eq!(rig.local(&after).await.map(|(row, _)| row), Some(id));

    let _ = imap.logout().await;
    rig.tidy().await;
}

/// Erase Deleted Items and Erase Junk Mail, including mail the store never downloaded.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn erasing_empties_the_bin_and_junk_on_the_server() {
    let rig = Rig::open().await;
    let events = Recorder::default();
    let (mut imap, caps) = rig.connect().await;

    let listed = rig.sync_tree(&mut imap).await;
    let bin = listed
        .iter()
        .find(|m| m.role == Some(Role::Trash))
        .map(|m| m.remote_path.clone())
        .expect("the rig has a Trash");
    let junk = listed
        .iter()
        .find(|m| m.role == Some(Role::Junk))
        .map(|m| m.remote_path.clone())
        .expect("the rig has a Junk");

    // Put straight on the server and never synced: the local store knows none of these, which
    // is the case a UID-list erase would have left behind.
    append(&mut imap, &bin, 3).await;
    append(&mut imap, &junk, 2).await;
    let inbox_before = exists(&mut imap, "INBOX").await;
    assert!(exists(&mut imap, &bin).await >= 3);

    let account_id = rig.account_id;
    rig.db
        .write(move |tx| {
            folders::erase(tx, account_id, Role::Trash).expect("erase bin");
            folders::erase(tx, account_id, Role::Junk).expect("erase junk");
            Ok(())
        })
        .await
        .expect("write");

    assert_eq!(rig.drain(&mut imap, caps, &events).await, 2);

    assert_eq!(exists(&mut imap, &bin).await, 0, "the Bin is empty");
    assert_eq!(exists(&mut imap, &junk).await, 0, "Junk is empty");
    assert_eq!(
        exists(&mut imap, "INBOX").await,
        inbox_before,
        "and nothing else was touched"
    );

    // Erasing what is already empty is not an error.
    rig.db
        .write(move |tx| {
            folders::erase(tx, account_id, Role::Trash).expect("erase again");
            Ok(())
        })
        .await
        .expect("write");
    assert_eq!(rig.drain(&mut imap, caps, &events).await, 1);

    assert!(events.refusals().is_empty(), "{:?}", events.refusals());
    let _ = imap.logout().await;
}

/// Mark All Messages as Read over a mailbox the size of the rig's Inbox.
///
/// Every UID used to go in one command. Twenty thousand scattered UIDs — no two consecutive, so
/// nothing collapses into a range — come to about 115 KB, which this Dovecot happens to accept
/// and RFC 7162 asks clients not to send. Now split into commands of at most 7,000 octets; this
/// proves the split pieces still add up to the whole selection on a real server. `\Flagged` is
/// used rather than `\Seen` so the Inbox's read state, which other gates measure, is never
/// touched; and every flag is put back.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn a_flag_change_over_a_huge_scattered_selection_reaches_the_server() {
    let rig = Rig::open().await;
    let events = Recorder::default();
    let (mut imap, caps) = rig.connect().await;
    rig.sync_tree(&mut imap).await;

    imap.select("INBOX").await.expect("SELECT");
    let already: std::collections::HashSet<u32> = imap.uid_search("FLAGGED").await.expect("SEARCH");

    let mut candidates: Vec<u32> = imap
        .uid_search("1:*")
        .await
        .expect("SEARCH")
        .into_iter()
        .filter(|uid| !already.contains(uid))
        .collect();
    candidates.sort_unstable();
    let chosen: Vec<u32> = candidates.into_iter().step_by(2).take(20_000).collect();
    assert!(
        chosen.len() >= 10_000,
        "the rig's Inbox is smaller than the gate assumes"
    );

    let enqueue_flag = |flagged: bool| {
        let chosen = chosen.clone();
        let account_id = rig.account_id;
        rig.db.write(move |tx| {
            ops::enqueue(
                tx,
                account_id,
                &ops::Op::Flag {
                    mailbox: "INBOX".into(),
                    uids: chosen,
                    seen: None,
                    flagged: Some(flagged),
                },
            )
        })
    };

    enqueue_flag(true).await.expect("queue");
    assert_eq!(rig.drain(&mut imap, caps, &events).await, 1);

    imap.select("INBOX").await.expect("SELECT");
    let flagged: std::collections::HashSet<u32> = imap.uid_search("FLAGGED").await.expect("SEARCH");
    let missing = chosen.iter().filter(|uid| !flagged.contains(uid)).count();
    assert_eq!(missing, 0, "{missing} of {} were not flagged", chosen.len());

    enqueue_flag(false).await.expect("queue");
    assert_eq!(rig.drain(&mut imap, caps, &events).await, 1);

    imap.select("INBOX").await.expect("SELECT");
    let after: std::collections::HashSet<u32> = imap.uid_search("FLAGGED").await.expect("SEARCH");
    assert_eq!(after, already, "the Inbox's flags are as they were");

    let _ = imap.logout().await;
}

/// A change reaches the server within seconds, without waiting for a sync to start.
///
/// The queue used to go only at the start of a sync, and with IDLE a sync starts when the Inbox
/// changes or every five minutes — found by driving the built app, where Mark All Messages as
/// Read on a folder left it unread on the server for minutes.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn a_queued_change_is_pushed_without_waiting_for_a_sync() {
    let rig = Rig::open().await;
    let (mut imap, _) = rig.connect().await;
    rig.sync_tree(&mut imap).await;

    let folder = rig.name("Pushed");
    imap.create(&folder).await.expect("CREATE");
    append(&mut imap, &folder, 2).await;
    imap.select(&folder).await.expect("SELECT");
    let uids: Vec<u32> = imap
        .uid_search("UNSEEN")
        .await
        .expect("SEARCH")
        .into_iter()
        .collect();
    assert_eq!(uids.len(), 2);

    let account_id = rig.account_id;
    {
        let folder = folder.clone();
        let uids = uids.clone();
        rig.db
            .write(move |tx| {
                ops::enqueue(
                    tx,
                    account_id,
                    &ops::Op::Flag {
                        mailbox: folder,
                        uids,
                        seen: Some(true),
                        flagged: None,
                    },
                )
            })
            .await
            .expect("queue");
    }

    let engine = halcyon_lib::sync::engine::SyncEngine::new();
    let events: std::sync::Arc<dyn Events> = std::sync::Arc::new(Recorder::default());
    let started = std::time::Instant::now();
    engine.push_soon(events, rig.db.clone()).await;

    // The push hands each account to a task of its own; give it the time a connection takes.
    let mut unseen = uids.len();
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        imap.select(&folder).await.expect("SELECT");
        unseen = imap.uid_search("UNSEEN").await.expect("SEARCH").len();
        if unseen == 0 {
            break;
        }
    }

    println!("  read on the server after {:?}", started.elapsed());
    assert_eq!(unseen, 0, "the change never left the queue");
    assert_eq!(rig.pending().await, 0);

    let _ = imap.logout().await;
    rig.tidy().await;
}

/// A name Dovecot will not take: the folder is taken back, its mail goes home, and the user is
/// told in the server's words.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn a_folder_the_server_refuses_is_taken_back_and_the_user_is_told() {
    let rig = Rig::open().await;
    let events = Recorder::default();
    let (mut imap, caps) = rig.connect().await;
    let listed = rig.sync_tree(&mut imap).await;
    let _ = listed;

    // A local message standing for the Inbox's first one, parked in the doomed folder by a
    // queued move, exactly as `msg_move` leaves it.
    let first_uid: u32 = {
        imap.select("INBOX").await.expect("SELECT");
        let found = imap.uid_search("1:5").await.expect("SEARCH");
        found.into_iter().min().expect("the Inbox has mail")
    };
    let (inbox_id, _) = rig.local("INBOX").await.expect("inbox row");

    // Begins with the separator: Dovecot refuses it outright. Written past the name check,
    // which would never let a user type it.
    let refused = format!("/Refused {}", rig.suffix);
    let account_id = rig.account_id;
    let unrelated = rig.name("Unrelated");

    let phantom = {
        let refused = refused.clone();
        let unrelated = unrelated.clone();
        rig.db
            .write(move |tx| {
                tx.execute(
                    "INSERT INTO mailbox (account_id, remote_path, display_name, delimiter)
                     VALUES (?1, ?2, 'Refused', '/')",
                    rusqlite::params![account_id, refused],
                )?;
                let phantom = tx.last_insert_rowid();
                ops::enqueue(
                    tx,
                    account_id,
                    &ops::Op::CreateMailbox {
                        path: refused.clone(),
                    },
                )?;

                tx.execute(
                    "INSERT INTO message (id, account_id, mailbox_id, uid, subject, date_sent,
                         date_received, size, from_all, to_all, body_text, has_attachment,
                         flag_seen, flag_flagged, is_junk)
                     VALUES (900001, ?1, ?2, ?3, 'Parked', 0, 0, 1, '', '', '', 0, 1, 0, 0)",
                    rusqlite::params![account_id, inbox_id, first_uid],
                )?;
                for group in ops::locate(tx, &[900_001])? {
                    ops::enqueue(
                        tx,
                        account_id,
                        &ops::Op::Move {
                            from: group.mailbox,
                            to: refused.clone(),
                            uids: group.uids,
                        },
                    )?;
                }
                halcyon_lib::db::write::move_to(tx, &[900_001], phantom)?;

                // Something that has nothing to do with it, which must still go through.
                folders::create(tx, account_id, &unrelated).expect("create");
                Ok(phantom)
            })
            .await
            .expect("write")
    };

    let sent = rig.drain(&mut imap, caps, &events).await;
    assert_eq!(sent, 1, "only the unrelated folder was sent");
    assert_eq!(rig.pending().await, 0, "nothing is left to fail again");

    let refusals = events.refusals();
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    println!("  refused: {}", refusals[0]);
    assert!(refusals[0].starts_with("The server would not create the mailbox “Refused”"));
    assert!(refusals[0].contains("The server said:"));

    // The folder is gone here, and the message is back where the server has it.
    assert!(rig.local(&refused).await.is_none());
    let home: (i64, i64) = rig
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                "SELECT mailbox_id, uid FROM message WHERE id = 900001",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .await
        .expect("read");
    assert_eq!(home, (inbox_id, i64::from(first_uid)));
    assert_ne!(home.0, phantom);

    let listed = rig.sync_tree(&mut imap).await;
    assert!(paths(&listed).contains(&unrelated.as_str()));
    assert!(!paths(&listed).iter().any(|path| path.contains("Refused")));

    let _ = imap.logout().await;
    rig.tidy().await;
}

/// A rename the server turns down — the new name was taken meanwhile by another client.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn a_rename_the_server_refuses_puts_the_old_name_back() {
    let rig = Rig::open().await;
    let events = Recorder::default();
    let (mut imap, caps) = rig.connect().await;

    let keep = rig.name("Keep");
    let wanted = rig.name("Wanted");
    imap.create(&keep).await.expect("CREATE");
    rig.sync_tree(&mut imap).await;
    let (id, _) = rig.local(&keep).await.expect("row");

    {
        let wanted = wanted.clone();
        rig.db
            .write(move |tx| {
                folders::rename(tx, id, &wanted).expect("rename");
                Ok(())
            })
            .await
            .expect("write");
    }

    // Another client takes the name before the rename is sent.
    imap.create(&wanted).await.expect("CREATE elsewhere");

    rig.drain(&mut imap, caps, &events).await;

    let refusals = events.refusals();
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    println!("  refused: {}", refusals[0]);
    assert!(refusals[0].contains("so it keeps its old name"));

    assert_eq!(rig.local(&keep).await, Some((id, keep.clone())));
    assert_eq!(rig.pending().await, 0);

    let listed = rig.sync_tree(&mut imap).await;
    assert!(paths(&listed).contains(&keep.as_str()));
    // The other client's folder is its own, and now has a row of its own.
    assert!(rig.local(&wanted).await.is_some_and(|(row, _)| row != id));

    let _ = imap.logout().await;
    rig.tidy().await;
}
/// One message with a subject of its own, so a rebuild can be seen to have read it again.
async fn append_numbered(imap: &mut ImapSession, path: &str, count: usize, stem: &str) {
    for index in 1..=count {
        let message = format!(
            "From: gate@halcyon.test\r\n\
             To: tester@halcyon.test\r\n\
             Subject: {stem} {index}\r\n\
             Message-ID: <{stem}-{index}@halcyon.test>\r\n\
             \r\n\
             Put here by folders_gate, {index}.\r\n"
        );
        imap.append(path, None, None, message.as_bytes())
            .await
            .expect("APPEND");
    }
}

/// Empties a mailbox the way another client would: `\Deleted` on everything, then expunge.
///
/// Never an Inbox. This is a real expunge on a real server, and the rig's Inbox is fifty
/// thousand seeded messages that every other gate measures against — an earlier version of this
/// file passed `"INBOX"` here where it meant the account's Bin, and emptied it. Cheap to refuse,
/// and the refusal is the reason the mistake cannot be made twice.
async fn empty_on_server(imap: &mut ImapSession, path: &str) {
    assert!(
        !path.eq_ignore_ascii_case("INBOX"),
        "the gate never empties an Inbox"
    );

    imap.select(path).await.expect("SELECT");
    {
        let mut stream = imap
            .uid_store("1:*", "+FLAGS.SILENT (\\Deleted)")
            .await
            .expect("STORE");
        while let Some(item) = futures::StreamExt::next(&mut stream).await {
            item.expect("STORE response");
        }
    }
    {
        let stream = imap.expunge().await.expect("EXPUNGE");
        futures::pin_mut!(stream);
        while let Some(item) = futures::StreamExt::next(&mut stream).await {
            item.expect("EXPUNGE response");
        }
    }
}

/// A folder made inside another, renamed with it, and deleted with it.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn a_folder_is_made_inside_another_and_moves_with_it() {
    let rig = Rig::open().await;
    let events = Recorder::default();
    let (mut imap, caps) = rig.connect().await;
    rig.sync_tree(&mut imap).await;

    let account_id = rig.account_id;
    let parent = rig.name("Nested");

    let (parent_id, child_id) = {
        let parent = parent.clone();
        rig.db
            .write(move |tx| {
                let parent_id = folders::create(tx, account_id, &parent).expect("create");
                // A name that has to be encoded, inside a folder: the separator goes between the
                // parent's path and the encoded leaf, not into the encoding.
                let child_id =
                    folders::create_in(tx, account_id, Some(parent_id), "Reçus").expect("inside");
                Ok((parent_id, child_id))
            })
            .await
            .expect("write")
    };

    assert_eq!(rig.drain(&mut imap, caps, &events).await, 2);
    assert_eq!(rig.pending().await, 0);

    let child = format!("{parent}/Re&AOc-us");
    let listed = rig.sync_tree(&mut imap).await;
    assert!(
        paths(&listed).contains(&child.as_str()),
        "{:?}",
        paths(&listed)
    );
    assert_eq!(
        rig.local(&child).await,
        Some((child_id, "Reçus".to_string())),
        "the same row, under the name the user typed"
    );

    // The sidebar's tree: the child is inside the parent, and the parent knows it holds one.
    let tree = rig
        .db
        .read(move |conn| halcyon_lib::db::query::mailboxes_tree(conn, Some(account_id)))
        .await
        .expect("tree");
    let row = |id: i64| tree.iter().find(|row| row.id == id).expect("row");
    assert_eq!(row(child_id).parent_id, Some(parent_id));
    assert_eq!(row(parent_id).descendants, 1);
    assert!(row(parent_id).can_contain);

    // Renamed, the child goes with it — the server moves it, and the row follows.
    let renamed = rig.name("Nested renamed");
    {
        let renamed = renamed.clone();
        rig.db
            .write(move |tx| {
                folders::rename(tx, parent_id, &renamed).expect("rename");
                Ok(())
            })
            .await
            .expect("write");
    }
    assert_eq!(rig.drain(&mut imap, caps, &events).await, 1);

    let moved = format!("{renamed}/Re&AOc-us");
    let listed = rig.sync_tree(&mut imap).await;
    assert!(
        paths(&listed).contains(&moved.as_str()),
        "{:?}",
        paths(&listed)
    );
    assert_eq!(rig.local(&moved).await.map(|(id, _)| id), Some(child_id));

    // Deleted, both go.
    let deleted = rig
        .db
        .write(move |tx| Ok(folders::delete(tx, parent_id).expect("delete")))
        .await
        .expect("write");
    assert_eq!(deleted.mailbox_ids, vec![child_id, parent_id]);
    assert_eq!(rig.drain(&mut imap, caps, &events).await, 1);

    let listed = rig.sync_tree(&mut imap).await;
    assert!(!paths(&listed).iter().any(|path| path.contains(&rig.suffix)));

    assert!(events.refusals().is_empty(), "{:?}", events.refusals());
    let _ = imap.logout().await;
    rig.tidy().await;
}

/// Use This Mailbox As: the choice survives a real listing, and the app files by it.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn a_chosen_bin_outlasts_the_servers_own_and_is_what_erase_empties() {
    let rig = Rig::open().await;
    let events = Recorder::default();
    let (mut imap, caps) = rig.connect().await;

    let listed = rig.sync_tree(&mut imap).await;
    let server_bin = listed
        .iter()
        .find(|mailbox| mailbox.role == Some(Role::Trash))
        .map(|mailbox| mailbox.remote_path.clone())
        .expect("the rig has a Trash");

    let chosen = rig.name("Deleted Messages");
    imap.create(&chosen).await.expect("CREATE");
    append_numbered(&mut imap, &chosen, 2, "chosen").await;
    append_numbered(&mut imap, &server_bin, 1, "server bin").await;
    rig.sync_tree(&mut imap).await;
    let (chosen_id, _) = rig.local(&chosen).await.expect("row");

    let account_id = rig.account_id;
    rig.db
        .write(move |tx| {
            folders::use_as(tx, chosen_id, Role::Trash).expect("use as");
            Ok(())
        })
        .await
        .expect("write");

    // A whole listing later — the server still calls its own folder the Bin — the choice holds.
    rig.sync_tree(&mut imap).await;
    let role_of = |path: String| {
        rig.db.read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT role FROM mailbox WHERE account_id = ?1 AND remote_path = ?2",
                    rusqlite::params![account_id, path],
                    |row| row.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten())
        })
    };
    assert_eq!(
        role_of(chosen.clone()).await.expect("read").as_deref(),
        Some("trash")
    );
    assert_eq!(
        role_of(server_bin.clone()).await.expect("read"),
        None,
        "one Bin per account, and the user said which"
    );

    // And Erase Deleted Items empties the chosen one, not the server's.
    rig.db
        .write(move |tx| {
            folders::erase(tx, account_id, Role::Trash).expect("erase");
            Ok(())
        })
        .await
        .expect("write");
    assert_eq!(rig.drain(&mut imap, caps, &events).await, 1);

    assert_eq!(exists(&mut imap, &chosen).await, 0);
    assert!(
        exists(&mut imap, &server_bin).await >= 1,
        "the server's own Bin was left alone"
    );

    // Deleted in webmail, the choice goes with the mailbox and the server's Bin is the Bin again.
    imap.delete(&chosen).await.expect("DELETE");
    rig.sync_tree(&mut imap).await;

    assert_eq!(
        role_of(server_bin.clone()).await.expect("read").as_deref(),
        Some("trash")
    );
    let choices: i64 = rig
        .db
        .read(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM mailbox_role", [], |row| row.get(0))?))
        .await
        .expect("count");
    assert_eq!(choices, 0);

    // Tidy the message this test put in the server's own Bin.
    empty_on_server(&mut imap, &server_bin).await;
    let _ = imap.logout().await;
    rig.tidy().await;
}

/// Rebuild: the mailbox is read again from the server, and what only this computer knows stays.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn a_rebuild_puts_a_damaged_copy_right_and_keeps_what_is_only_here() {
    let rig = Rig::open().await;
    let events: std::sync::Arc<Recorder> = std::sync::Arc::new(Recorder::default());
    let (mut imap, _) = rig.connect().await;
    rig.sync_tree(&mut imap).await;

    let folder = rig.name("Rebuild");
    imap.create(&folder).await.expect("CREATE");
    append_numbered(&mut imap, &folder, 5, "rebuild").await;
    rig.sync_tree(&mut imap).await;
    let (folder_id, _) = rig.local(&folder).await.expect("row");
    let _ = imap.logout().await;

    let engine = halcyon_lib::sync::engine::SyncEngine::new();
    let account_id = rig.account_id;
    engine
        .sync_mailboxes(events.as_ref(), &rig.db, account_id, &[folder_id])
        .await
        .expect("first pass");

    let held = |folder_id: i64| {
        rig.db.read(move |conn| {
            let mut statement = conn
                .prepare("SELECT uid, subject FROM message WHERE mailbox_id = ?1 ORDER BY uid")?;
            let rows = statement.query_map(rusqlite::params![folder_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    };
    assert_eq!(held(folder_id).await.expect("read").len(), 5);

    // Damage the copy every way a broken mailbox is broken: a wrong subject, a message missing,
    // one here that the server does not have, and a cached body that is wrong.
    let ids: (i64, i64) = rig
        .db
        .write(move |tx| {
            tx.execute(
                "UPDATE message SET subject = 'Wrong' WHERE mailbox_id = ?1 AND uid = 1",
                rusqlite::params![folder_id],
            )?;
            tx.execute(
                "DELETE FROM message WHERE mailbox_id = ?1 AND uid = 2",
                rusqlite::params![folder_id],
            )?;
            tx.execute(
                "INSERT INTO message (account_id, mailbox_id, uid, subject, date_sent,
                     date_received, size, from_all, to_all, body_text, has_attachment, flag_seen,
                     flag_flagged, is_junk)
                 VALUES (?1, ?2, 99999, 'Ghost', 0, 0, 1, '', '', '', 0, 0, 0, 0)",
                rusqlite::params![account_id, folder_id],
            )?;
            // What only this computer knows, on the message the rebuild will read again.
            tx.execute(
                "UPDATE message SET flag_color = 'blue', snooze_until = 4102444800,
                                    junk_by_user = 1
                  WHERE mailbox_id = ?1 AND uid = 3",
                rusqlite::params![folder_id],
            )?;
            tx.execute(
                "UPDATE message SET body_state = 'full', body_text = 'CORRUPT', raw_path = NULL
                  WHERE mailbox_id = ?1 AND uid = 4",
                rusqlite::params![folder_id],
            )?;

            let kept: i64 = tx.query_row(
                "SELECT id FROM message WHERE mailbox_id = ?1 AND uid = 3",
                rusqlite::params![folder_id],
                |row| row.get(0),
            )?;
            let cached: i64 = tx.query_row(
                "SELECT id FROM message WHERE mailbox_id = ?1 AND uid = 4",
                rusqlite::params![folder_id],
                |row| row.get(0),
            )?;
            Ok((kept, cached))
        })
        .await
        .expect("damage");

    rig.db
        .write(move |tx| {
            folders::request_rebuild(tx, folder_id).expect("rebuild");
            Ok(())
        })
        .await
        .expect("write");

    engine
        .sync_mailboxes(events.as_ref(), &rig.db, account_id, &[folder_id])
        .await
        .expect("rebuild pass");

    assert_eq!(
        held(folder_id).await.expect("read"),
        (1..=5)
            .map(|uid| (uid, format!("rebuild {uid}")))
            .collect::<Vec<_>>(),
        "every message read again, the missing one back, and the one the server never had gone"
    );

    let (colour, snooze, junk): (Option<String>, Option<i64>, bool) = rig
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                "SELECT flag_color, snooze_until, junk_by_user FROM message WHERE id = ?1",
                rusqlite::params![ids.0],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?)
        })
        .await
        .expect("read");
    assert_eq!(
        (colour.as_deref(), snooze, junk),
        (Some("blue"), Some(4_102_444_800), true),
        "a flag colour, a snooze and a junk verdict are this computer's, and stay"
    );

    let (body, raw): (Option<String>, Option<String>) = rig
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                "SELECT body_text, raw_path FROM message WHERE id = ?1",
                rusqlite::params![ids.1],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .await
        .expect("read");
    assert!(
        body.as_deref()
            .is_some_and(|text| text.contains("folders_gate, 4")),
        "the cached body was downloaded again: {body:?}"
    );
    assert!(
        raw.as_deref()
            .is_some_and(|path| std::path::Path::new(path).starts_with(rig.dir.path())),
        "cached beside this store, never in the user's: {raw:?}"
    );

    let rebuilt: bool = rig
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT rebuild_requested FROM mailbox WHERE id = ?1",
                    rusqlite::params![folder_id],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap_or(true))
        })
        .await
        .expect("read");
    assert!(!rebuilt, "the rebuild was asked for once, and is done");

    let finished = events.rebuilt();
    assert_eq!(finished, vec![(folder_id, 5)], "the window was told");

    let (mut imap, _) = rig.connect().await;
    let _ = imap.logout().await;
    rig.tidy().await;
}

/// A folder emptied on another device is emptied here — which an empty `UID SEARCH` cannot say.
#[tokio::test]
#[ignore = "needs the Dovecot rig in test/dovecot"]
async fn a_folder_emptied_on_the_server_is_emptied_here() {
    let rig = Rig::open().await;
    let events: std::sync::Arc<Recorder> = std::sync::Arc::new(Recorder::default());
    let (mut imap, _) = rig.connect().await;
    rig.sync_tree(&mut imap).await;

    let folder = rig.name("Emptied");
    imap.create(&folder).await.expect("CREATE");
    append_numbered(&mut imap, &folder, 3, "emptied").await;
    rig.sync_tree(&mut imap).await;
    let (folder_id, _) = rig.local(&folder).await.expect("row");

    let engine = halcyon_lib::sync::engine::SyncEngine::new();
    let account_id = rig.account_id;
    engine
        .sync_mailboxes(events.as_ref(), &rig.db, account_id, &[folder_id])
        .await
        .expect("first pass");

    let count = |folder_id: i64| {
        rig.db.read(move |conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM message WHERE mailbox_id = ?1",
                rusqlite::params![folder_id],
                |row| row.get::<_, i64>(0),
            )?)
        })
    };
    assert_eq!(count(folder_id).await.expect("read"), 3);

    empty_on_server(&mut imap, &folder).await;
    let _ = imap.logout().await;

    engine
        .sync_mailboxes(events.as_ref(), &rig.db, account_id, &[folder_id])
        .await
        .expect("second pass");

    assert_eq!(
        count(folder_id).await.expect("read"),
        0,
        "the copy here kept mail the server had thrown away"
    );

    rig.tidy().await;
}
