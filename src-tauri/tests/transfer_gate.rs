//! Import and export against a real file. docs/06 Phase 11.
//!
//! The unit tests in `transfer::` each check one decision. This checks the whole path on a
//! file with the things that actually break importers in it: an RFC 2047 encoded subject, a
//! body line beginning `From `, an mboxrd-quoted line, Thunderbird's status flags, and a reply
//! that has to thread against the message above it.
//!
//! Written as a gate rather than a unit test because it is the claim the phase makes — *import
//! from a Thunderbird profile and mbox; export to mbox and an .eml tree* — and a claim of that
//! shape deserves one test that fails if any part of it stops being true.

use halcyon_lib::transfer::{export, import, mbox};
use rusqlite::{params, Connection};

/// The sample. Every line of it is here to break something.
const SAMPLE: &str = "From ada@example.test Thu Aug 27 09:34:00 2026\n\
X-Mozilla-Status: 0001\n\
Message-ID: <first@example.test>\n\
From: Ada Lovelace <ada@example.test>\n\
To: Vishal Singh <vishal@example.test>\n\
Subject: =?UTF-8?B?VGhlIGFuYWx5dGljYWwgZW5naW5l?=\n\
Date: Thu, 27 Aug 2026 09:34:00 +0000\n\
Content-Type: text/plain; charset=utf-8\n\
\n\
It weaves algebraic patterns.\n\
\n\
>From the notes of Menabrea.\n\
\n\
From here on it is my own work.\n\
\n\
From ada@example.test Fri Aug 28 11:00:00 2026\n\
X-Mozilla-Status: 0005\n\
Message-ID: <second@example.test>\n\
In-Reply-To: <first@example.test>\n\
References: <first@example.test>\n\
From: Charles Babbage <charles@example.test>\n\
To: ada@example.test\n\
Subject: Re: The analytical engine\n\
Date: Fri, 28 Aug 2026 11:00:00 +0000\n\
Content-Type: text/plain; charset=utf-8\n\
\n\
Quite so.\n";

struct Fixture {
    conn: Connection,
    cache: std::path::PathBuf,
    account: i64,
    mailbox: i64,
}

fn import_sample(name: &str) -> Fixture {
    let cache = std::env::temp_dir().join(format!("halcyon-gate-{name}"));
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&cache).expect("cache");

    let mut conn = Connection::open_in_memory().expect("open");
    halcyon_lib::db::migrate::run(&mut conn).expect("migrate");

    let tx = conn.transaction().expect("tx");
    let account = import::local_account(&tx, "On My PC").expect("account");
    let mailbox = import::mailbox_for(&tx, account, "Local Folders/Inbox").expect("mailbox");

    let mut uid = import::next_uid(&tx, mailbox).expect("uid");
    mbox::read(SAMPLE.as_bytes(), |raw| {
        if import::write_message(&tx, &cache, account, mailbox, uid, raw)
            .expect("write")
            .is_some()
        {
            uid += 1;
        }
        Ok(())
    })
    .expect("read");

    import::finish(&tx, account, &[mailbox]).expect("finish");
    tx.commit().expect("commit");

    Fixture {
        conn,
        cache,
        account,
        mailbox,
    }
}

#[test]
fn gate_1_an_mbox_imports_with_its_envelopes_intact() {
    let f = import_sample("envelopes");

    let count: i64 = f
        .conn
        .query_row(
            "SELECT COUNT(*) FROM message WHERE mailbox_id = ?1",
            params![f.mailbox],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(count, 2, "both messages should have arrived");

    // The encoded-word subject. A client that shows `=?UTF-8?B?...?=` is unusable for anyone
    // whose correspondents do not write in ASCII.
    let subject: String = f
        .conn
        .query_row(
            "SELECT subject FROM message WHERE message_id = 'first@example.test'",
            [],
            |row| row.get(0),
        )
        .expect("subject");
    assert_eq!(subject, "The analytical engine");

    let from: String = f
        .conn
        .query_row(
            "SELECT from_name FROM message WHERE message_id = 'first@example.test'",
            [],
            |row| row.get(0),
        )
        .expect("from");
    assert_eq!(from, "Ada Lovelace");

    let _ = std::fs::remove_dir_all(&f.cache);
}

#[test]
fn gate_2_a_from_line_in_the_body_does_not_split_a_message() {
    // The single most common way an mbox importer corrupts a mailbox: a body line beginning
    // `From ` is read as a separator, the message ends early, and the rest becomes a headerless
    // message of its own.
    let f = import_sample("fromline");

    let body: String = f
        .conn
        .query_row(
            "SELECT body_text FROM message WHERE message_id = 'first@example.test'",
            [],
            |row| row.get(0),
        )
        .expect("body");

    assert!(
        body.contains("From here on it is my own work."),
        "the message was split at a body line: {body}"
    );
    // And the mboxrd quoting was undone rather than left in the text.
    assert!(body.contains("From the notes of Menabrea."), "{body}");
    assert!(!body.contains(">From the notes"), "{body}");

    let _ = std::fs::remove_dir_all(&f.cache);
}

#[test]
fn gate_3_thunderbird_read_and_flag_state_survives() {
    // Without this an import of years of archives arrives as thousands of unread messages,
    // which makes the result unusable however correct the text is.
    let f = import_sample("flags");

    let (seen, flagged): (i64, i64) = f
        .conn
        .query_row(
            "SELECT flag_seen, flag_flagged FROM message WHERE message_id = 'second@example.test'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("flags");

    // X-Mozilla-Status: 0005 is Read | Marked.
    assert_eq!(seen, 1);
    assert_eq!(flagged, 1);

    let _ = std::fs::remove_dir_all(&f.cache);
}

#[test]
fn gate_4_a_reply_threads_with_the_message_it_answers() {
    // Imported mail has to thread against itself, and by the same rules a synced message uses —
    // that is why the import builds its envelope through `envelope::from_headers` rather than
    // its own parser.
    let f = import_sample("threading");

    let threads: i64 = f
        .conn
        .query_row(
            "SELECT COUNT(DISTINCT thread_id) FROM message WHERE mailbox_id = ?1",
            params![f.mailbox],
            |row| row.get(0),
        )
        .expect("threads");

    assert_eq!(threads, 1, "the reply landed in a thread of its own");

    let _ = std::fs::remove_dir_all(&f.cache);
}

#[test]
fn gate_5_imported_mail_is_searchable() {
    let f = import_sample("search");

    let hits: i64 = f
        .conn
        .query_row(
            "SELECT COUNT(*) FROM message_fts WHERE message_fts MATCH 'algebraic'",
            [],
            |row| row.get(0),
        )
        .expect("search");

    assert_eq!(hits, 1);

    let _ = std::fs::remove_dir_all(&f.cache);
}

#[test]
fn gate_6_the_local_account_is_never_synced() {
    // The safety property the whole design rests on. If this is ever true, the next sync finds
    // rows the server has never heard of and deletes every imported message.
    let f = import_sample("nosync");

    let enabled: i64 = f
        .conn
        .query_row(
            "SELECT sync_enabled FROM account WHERE id = ?1",
            params![f.account],
            |row| row.get(0),
        )
        .expect("read");

    assert_eq!(enabled, 0);

    let _ = std::fs::remove_dir_all(&f.cache);
}

#[test]
fn gate_7_export_to_mbox_round_trips_back_through_import() {
    // Export, re-import, and the messages must be the same messages. This is the check that
    // catches an escaping bug in either direction, because a writer and reader that are wrong
    // in the same way still pass the two halves separately.
    let f = import_sample("roundtrip");
    let out = std::env::temp_dir().join("halcyon-gate-roundtrip-out");
    let _ = std::fs::remove_dir_all(&out);

    let mut counts = export::Exported::default();
    let path = export::to_mbox(&f.conn, f.mailbox, &out, &mut counts).expect("export");
    assert_eq!(counts.messages, 2);

    // Back in, into a second mailbox in the same store.
    let cache2 = std::env::temp_dir().join("halcyon-gate-roundtrip-cache2");
    let _ = std::fs::remove_dir_all(&cache2);

    let mut conn = f.conn;
    let tx = conn.transaction().expect("tx");
    let second = import::mailbox_for(&tx, f.account, "Local Folders/Again").expect("mailbox");
    let mut uid = import::next_uid(&tx, second).expect("uid");

    let file = std::fs::File::open(&path).expect("open");
    mbox::read(std::io::BufReader::new(file), |raw| {
        if import::write_message(&tx, &cache2, f.account, second, uid, raw)
            .expect("write")
            .is_some()
        {
            uid += 1;
        }
        Ok(())
    })
    .expect("read back");
    tx.commit().expect("commit");

    let (count, subject, body): (i64, String, String) = (
        conn.query_row(
            "SELECT COUNT(*) FROM message WHERE mailbox_id = ?1",
            params![second],
            |row| row.get(0),
        )
        .expect("count"),
        conn.query_row(
            "SELECT subject FROM message WHERE mailbox_id = ?1 ORDER BY uid LIMIT 1",
            params![second],
            |row| row.get(0),
        )
        .expect("subject"),
        conn.query_row(
            "SELECT body_text FROM message WHERE mailbox_id = ?1 ORDER BY uid LIMIT 1",
            params![second],
            |row| row.get(0),
        )
        .expect("body"),
    );

    assert_eq!(count, 2, "the round trip lost or invented a message");
    assert_eq!(subject, "The analytical engine");
    assert!(body.contains("From here on it is my own work."), "{body}");
    assert!(body.contains("From the notes of Menabrea."), "{body}");
    assert!(
        !body.contains(">From the notes"),
        "quoting accumulated: {body}"
    );

    let _ = std::fs::remove_dir_all(&f.cache);
    let _ = std::fs::remove_dir_all(&cache2);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn gate_8_export_to_an_eml_tree_writes_one_readable_file_per_message() {
    let f = import_sample("emltree");
    let out = std::env::temp_dir().join("halcyon-gate-eml-out");
    let _ = std::fs::remove_dir_all(&out);

    let mut counts = export::Exported::default();
    let folder = export::to_eml_tree(&f.conn, f.mailbox, &out, &mut counts).expect("export");

    let mut files: Vec<_> = std::fs::read_dir(&folder)
        .expect("read dir")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    files.sort();

    assert_eq!(files.len(), 2);

    // Each one is a message a mail client can open, not a fragment.
    let first = std::fs::read(&files[0]).expect("read");
    let parsed = mailparse::parse_mail(&first).expect("the exported file is not a message");
    assert!(
        parsed
            .headers
            .iter()
            .any(|header| header.get_key_ref() == "Message-ID"),
        "the exported .eml lost its headers"
    );

    let _ = std::fs::remove_dir_all(&f.cache);
    let _ = std::fs::remove_dir_all(&out);
}
