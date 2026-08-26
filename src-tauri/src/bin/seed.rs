//! Development seed tool. docs/06 Phase 3 — "a `seed` dev binary generating 100,000
//! realistic messages across 3 accounts and 40 mailboxes".
//!
//! Not part of the shipped app: Tauri bundles only the binary named by `productName`, so
//! this is built in development and never installed.
//!
//! Run it with:
//!
//! ```text
//! cargo run --manifest-path src-tauri/Cargo.toml --bin seed -- --reset
//! cargo run --manifest-path src-tauri/Cargo.toml --bin seed -- --messages 5000 --path .\dev.db
//! ```
//!
//! It finishes by printing the `EXPLAIN QUERY PLAN` output and timings the exit gate asks
//! to see, against whatever it just generated.

use std::path::PathBuf;
use std::time::Instant;

use halcyon_lib::db::{
    self,
    model::{Cursor, ListQuery, SearchQuery},
    query,
};
use rusqlite::Connection;

/// Deterministic PRNG. The same seed gives the same hundred thousand messages, so a
/// measurement taken today can be compared with one taken next week.
struct Rng(u64);

impl Rng {
    fn next_u32(&mut self) -> u32 {
        // xorshift64*: small, fast, and good enough for picking words out of a list.
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }

    fn below(&mut self, bound: u32) -> u32 {
        if bound == 0 {
            0
        } else {
            self.next_u32() % bound
        }
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u32) as usize]
    }

    fn chance(&mut self, percent: u32) -> bool {
        self.below(100) < percent
    }
}

const FIRST_NAMES: &[&str] = &[
    "Ada", "Marcus", "Priya", "Tomas", "Hannah", "Kenji", "Rosalind", "Danny", "Isabel", "Sam",
    "Greta", "Owen", "Nadia", "Bill", "Yuki", "Farah", "Diego", "Mei", "Oskar", "Leila",
];

const LAST_NAMES: &[&str] = &[
    "Whitfield",
    "Oyelaran",
    "Ramanathan",
    "Bergqvist",
    "Wexler",
    "Nakamura",
    "Achebe",
    "Feld",
    "Moreau",
    "Okonkwo",
    "Lindqvist",
    "Trelawney",
    "Farouk",
    "Hutchings",
    "Tanabe",
    "Baptiste",
];

const DOMAINS: &[&str] = &[
    "northgate.example",
    "lantern.example",
    "driftwood.example",
    "meridian.example",
    "quayside.example",
    "harborline.example",
    "sablewood.example",
];

const SUBJECT_HEADS: &[&str] = &[
    "Draft agenda for",
    "Notes from",
    "Follow-up on",
    "Re: the",
    "Questions about",
    "Revised",
    "Budget approval for",
    "Handover:",
    "Feedback on",
    "Invoice for",
    "Reminder:",
    "Weekly digest:",
    "Payment received for",
    "Your statement for",
];

const SUBJECT_TAILS: &[&str] = &[
    "Thursday's review",
    "the Fenwick contract",
    "the warehouse audit",
    "the onboarding flow",
    "quarterly figures",
    "the site visit",
    "next year's pricing",
    "the supplier change",
    "the offsite venue",
    "the staging outage",
    "the design role panel",
    "the annual accounts",
];

const SENTENCES: &[&str] = &[
    "Just wanted to check in before the meeting so we are not caught out by the numbers again.",
    "I have attached the revised version with the changes we discussed on the call.",
    "Let me know if this works and I will get it booked in.",
    "The short answer is yes, but there are a couple of caveats worth walking through.",
    "Can we push this to next week? Something has come up on my end.",
    "Everything looks good from here. One small thing on page four.",
    "This is the third time it has happened this month, so it is worth investigating.",
    "No action needed, just keeping you in the loop.",
    "I spoke to them this morning and they are happy to proceed on the original terms.",
    "The invoice has been raised and should reach you by the end of the day.",
    "Following up on this as I have not heard back.",
    "That timing works. I will send an invite across shortly.",
];

const FOLDERS: &[&str] = &[
    "Clients",
    "Contracts",
    "Receipts",
    "Travel",
    "Family",
    "Bills",
    "Shopping",
    "Newsletters",
];

const STANDARD_ROLES: &[(&str, &str)] = &[
    ("inbox", "Inbox"),
    ("drafts", "Drafts"),
    ("sent", "Sent"),
    ("junk", "Junk"),
    ("trash", "Bin"),
    ("archive", "Archive"),
];

const ACCOUNTS: &[(&str, &str, &str)] = &[
    ("Northgate", "vishal@northgate.example", "imap"),
    ("iCloud", "vishal@icloud.example", "icloud"),
    ("Gmail", "vishal.singh@gmail.example", "gmail"),
];

struct Options {
    path: PathBuf,
    messages: usize,
    reset: bool,
}

fn parse_args() -> Options {
    let mut options = Options {
        path: db::default_path(),
        messages: 100_000,
        reset: false,
    };

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--path" => {
                if let Some(value) = args.next() {
                    options.path = PathBuf::from(value);
                }
            }
            "--messages" => {
                if let Some(value) = args.next() {
                    if let Ok(parsed) = value.parse() {
                        options.messages = parsed;
                    }
                }
            }
            "--reset" => options.reset = true,
            other => eprintln!("ignoring unknown argument {other}"),
        }
    }

    options
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = parse_args();

    if options.reset && options.path.exists() {
        // WAL leaves two siblings behind; removing only the .db would resurrect the old
        // contents from the write-ahead log on next open.
        for suffix in ["", "-wal", "-shm"] {
            let mut path = options.path.clone().into_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(path));
        }
        println!("removed existing database");
    }

    if let Some(parent) = options.path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut conn = Connection::open(&options.path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    db::migrate::run(&mut conn)?;

    let existing: i64 = conn.query_row("SELECT COUNT(*) FROM message", [], |row| row.get(0))?;
    if existing > 0 {
        println!("database already holds {existing} messages; pass --reset to rebuild");
    } else {
        seed(&mut conn, options.messages)?;
    }

    report(&conn, &options.path)?;
    println!("\ndatabase: {}", options.path.display());

    Ok(())
}

fn seed(conn: &mut Connection, message_count: usize) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let mut rng = Rng(0x5EED_0000_0000_0001);

    // Bulk load with the FTS triggers dropped, then rebuild the index in one pass. Indexing
    // row by row through the triggers costs several times more, and 'rebuild' is FTS5's own
    // supported way to construct an external-content index from scratch.
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS message_fts_insert;
         DROP TRIGGER IF EXISTS message_fts_update;
         DROP TRIGGER IF EXISTS message_fts_delete;",
    )?;

    let mut mailbox_ids: Vec<(i64, i64)> = Vec::new(); // (mailbox_id, account_id)
    let mut inbox_ids: Vec<(i64, i64)> = Vec::new();

    {
        let tx = conn.transaction()?;

        for (index, (name, email, provider)) in ACCOUNTS.iter().enumerate() {
            let account_id = index as i64 + 1;
            tx.execute(
                "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref, sort_order)
                 VALUES (?1, ?2, ?3, ?4, 'password', ?5, ?1)",
                (account_id, name, email, provider, format!("halcyon/{email}")),
            )?;

            for (order, (role, display)) in STANDARD_ROLES.iter().enumerate() {
                tx.execute(
                    "INSERT INTO mailbox (account_id, remote_path, display_name, role, sort_order)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    (account_id, display, display, role, order as i64),
                )?;
                let mailbox_id = tx.last_insert_rowid();
                mailbox_ids.push((mailbox_id, account_id));
                if *role == "inbox" {
                    inbox_ids.push((mailbox_id, account_id));
                }
            }

            // Enough custom folders to reach the 40 mailboxes docs/06 asks for.
            for (order, folder) in FOLDERS.iter().enumerate() {
                tx.execute(
                    "INSERT INTO mailbox (account_id, remote_path, display_name, sort_order)
                     VALUES (?1, ?2, ?3, ?4)",
                    (
                        account_id,
                        folder,
                        folder,
                        (STANDARD_ROLES.len() + order) as i64,
                    ),
                )?;
                mailbox_ids.push((tx.last_insert_rowid(), account_id));
            }
        }

        tx.commit()?;
    }

    println!(
        "{} mailboxes across {} accounts",
        mailbox_ids.len(),
        ACCOUNTS.len()
    );

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;

    // Two years back, weighted toward the present, so the date headers at the top of the
    // list have something to group.
    const TWO_YEARS: i64 = 730 * 24 * 60 * 60;

    let mut written = 0usize;
    let batch_size = 5_000;

    // UIDs are per-mailbox and monotonic in IMAP, and the schema enforces that with
    // UNIQUE(mailbox_id, uid). The first version of this numbered them from the batch
    // counter, so every message in a batch shared a uid and the constraint rejected the
    // second one to land in any mailbox — the schema catching a seed bug, which is the
    // constraint doing its job.
    let mut next_uid: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();

    while written < message_count {
        let take = batch_size.min(message_count - written);
        let tx = conn.transaction()?;

        {
            let mut insert = tx.prepare(
                "INSERT INTO message
                   (id, account_id, mailbox_id, uid, message_id, thread_id, subject, subject_base,
                    from_name, from_addr, to_json, date_sent, date_received, size, preview,
                    flag_seen, flag_answered, flag_flagged, flag_color, has_attachment,
                    body_state, body_text, from_all, to_all, attachment_names)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                         ?15, ?16, ?17, ?18, ?19, 'full', ?20, ?21, ?22, ?23)",
            )?;

            for row in 0..take {
                // Most mail lands in an inbox; the rest is spread over every folder.
                let (mailbox_id, account_id) = if rng.chance(70) {
                    *rng.pick(&inbox_ids)
                } else {
                    *rng.pick(&mailbox_ids)
                };

                let first = rng.pick(FIRST_NAMES);
                let last = rng.pick(LAST_NAMES);
                let domain = rng.pick(DOMAINS);
                let from_name = format!("{first} {last}");
                let from_addr = format!(
                    "{}.{}@{}",
                    first.to_lowercase(),
                    last.to_lowercase(),
                    domain
                );

                let subject = format!("{} {}", rng.pick(SUBJECT_HEADS), rng.pick(SUBJECT_TAILS));

                let body = format!(
                    "{}\n\n{}\n\n{}",
                    rng.pick(SENTENCES),
                    rng.pick(SENTENCES),
                    rng.pick(SENTENCES)
                );
                let preview: String = body.chars().take(200).collect();

                // Squaring the roll bunches dates toward now without needing a real
                // distribution: most mail is recent, a long tail is not.
                //
                // The seconds-of-day jitter is not cosmetic. Without it the integer
                // arithmetic below collapses every small roll to the same instant, and the
                // list showed several hundred messages all stamped with the same minute —
                // which looks broken and hides the date-header grouping entirely.
                let roll = rng.below(1000) as i64;
                let age = TWO_YEARS * roll * roll / 1_000_000;
                let seconds_into_day = rng.below(86_400) as i64;
                let date = now - age - seconds_into_day;

                let seen = rng.chance(82);
                let flagged = rng.chance(5);
                let has_attachment = rng.chance(14);

                let uid = next_uid.entry(mailbox_id).or_insert(0);
                *uid += 1;
                let uid = *uid;

                // thread_id stays NULL. Threading is the sync engine's job in Phase 5, and
                // inventing a one-message thread row per message would be structure that
                // means nothing — the FOREIGN KEY refused it, correctly, when this tried.
                // query::thread_get falls back to the message itself, which is the same
                // path real unthreaded mail takes.
                let message_id = written as i64 + row as i64 + 1;

                insert.execute(rusqlite::params![
                    message_id,
                    account_id,
                    mailbox_id,
                    uid,
                    format!("<{uid}.{account_id}@halcyon.example>"),
                    subject,
                    subject.trim_start_matches("Re: "),
                    from_name,
                    from_addr,
                    r#"[{"name":"Vishal Singh","address":"vishal@northgate.example"}]"#,
                    date,
                    date,
                    rng.below(60_000) as i64 + 2_400,
                    preview,
                    i64::from(seen),
                    i64::from(rng.chance(18)),
                    i64::from(flagged),
                    if flagged { Some("orange") } else { None },
                    i64::from(has_attachment),
                    body,
                    format!("{from_name} {from_addr}"),
                    "Vishal Singh vishal@northgate.example",
                    if has_attachment {
                        "agenda.pdf report.xlsx"
                    } else {
                        ""
                    },
                ])?;
            }
        }

        tx.commit()?;
        written += take;

        if written % 25_000 == 0 || written == message_count {
            println!("  {written} / {message_count} messages");
        }
    }

    println!("building the search index");
    conn.execute_batch("INSERT INTO message_fts(message_fts) VALUES('rebuild')")?;

    // Recreate exactly what migration 0001 defines. Kept verbatim so a divergence is a
    // visible edit rather than a silent behaviour change.
    conn.execute_batch(
        "CREATE TRIGGER message_fts_insert AFTER INSERT ON message BEGIN
           INSERT INTO message_fts(rowid, subject, body_text, from_all, to_all, attachment_names)
           VALUES (new.id, new.subject, new.body_text, new.from_all, new.to_all, new.attachment_names);
         END;
         CREATE TRIGGER message_fts_delete AFTER DELETE ON message BEGIN
           INSERT INTO message_fts(message_fts, rowid, subject, body_text, from_all, to_all, attachment_names)
           VALUES ('delete', old.id, old.subject, old.body_text, old.from_all, old.to_all, old.attachment_names);
         END;
         CREATE TRIGGER message_fts_update AFTER UPDATE ON message BEGIN
           INSERT INTO message_fts(message_fts, rowid, subject, body_text, from_all, to_all, attachment_names)
           VALUES ('delete', old.id, old.subject, old.body_text, old.from_all, old.to_all, old.attachment_names);
           INSERT INTO message_fts(rowid, subject, body_text, from_all, to_all, attachment_names)
           VALUES (new.id, new.subject, new.body_text, new.from_all, new.to_all, new.attachment_names);
         END;",
    )?;

    println!("refreshing mailbox counts");
    conn.execute_batch(
        "UPDATE mailbox
            SET unread_count = (SELECT COUNT(*) FROM message
                                 WHERE message.mailbox_id = mailbox.id AND flag_seen = 0),
                total_count  = (SELECT COUNT(*) FROM message
                                 WHERE message.mailbox_id = mailbox.id)",
    )?;

    // ANALYZE gives the planner real statistics. Without it SQLite guesses, and a guess on
    // a partial index is exactly where a plan quietly turns into a scan.
    println!("analysing");
    conn.execute_batch("ANALYZE")?;

    println!(
        "seeded {message_count} messages in {:.1}s",
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Prints the plans and timings docs/06 Phase 3 asks to see.
fn report(conn: &Connection, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let messages: i64 = conn.query_row("SELECT COUNT(*) FROM message", [], |row| row.get(0))?;
    let mailboxes: i64 = conn.query_row("SELECT COUNT(*) FROM mailbox", [], |row| row.get(0))?;

    let inbox: i64 = conn.query_row(
        "SELECT id FROM mailbox WHERE role = 'inbox' ORDER BY id LIMIT 1",
        [],
        |row| row.get(0),
    )?;

    println!("\n{messages} messages in {mailboxes} mailboxes\n");

    let time = |label: &str, mut run: Box<dyn FnMut() -> Result<usize, db::DbError> + '_>| {
        let started = Instant::now();
        let count = run().unwrap_or(0);
        println!(
            "  {label:<28} {:>7.1} ms   ({count} rows)",
            started.elapsed().as_secs_f64() * 1000.0
        );
    };

    println!("timings");

    let first_page = ListQuery {
        mailbox_ids: vec![inbox],
        cursor: None,
        limit: 100,
        unread_only: false,
    };
    time(
        "messages_page (first)",
        Box::new(|| query::messages_page(conn, &first_page).map(|page| page.items.len())),
    );

    // Deep into the mailbox: the whole point of keyset pagination is that this costs the
    // same as the first page. With OFFSET it would not.
    let deep_cursor = conn.query_row(
        "SELECT date_received, id FROM message WHERE mailbox_id = ?1
          ORDER BY date_received DESC, id DESC LIMIT 1 OFFSET 20000",
        [inbox],
        |row| {
            Ok(Cursor {
                date_received: row.get(0)?,
                id: row.get(1)?,
            })
        },
    );

    if let Ok(cursor) = deep_cursor {
        let deep_page = ListQuery {
            mailbox_ids: vec![inbox],
            cursor: Some(cursor),
            limit: 100,
            unread_only: false,
        };
        time(
            "messages_page (20k deep)",
            Box::new(|| query::messages_page(conn, &deep_page).map(|page| page.items.len())),
        );
    }

    // What a mailbox switch actually costs: the sidebar reads the counts cached on
    // `mailbox`, so it never touches `message` at all.
    time(
        "mailboxes_tree (sidebar)",
        Box::new(|| query::mailboxes_tree(conn, None).map(|rows| rows.len())),
    );

    // The maintenance path, for contrast. Deliberately NOT on the mutation path — it was,
    // and at this cost every "mark as read" would have paid it. See db::write::apply_delta.
    time(
        "recount (maintenance only)",
        Box::new(|| query::mailbox_counts(conn, &[inbox]).map(|rows| rows.len())),
    );

    // Standing rule 10: the local write has to be instant. Fifty messages marked read,
    // including the count maintenance, inside one transaction.
    {
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM message WHERE mailbox_id = ?1 LIMIT 50")
            .and_then(|mut stmt| {
                stmt.query_map([inbox], |row| row.get(0))?
                    .collect::<Result<Vec<i64>, _>>()
            })
            .unwrap_or_default();

        let started = Instant::now();
        let mut conn = Connection::open(path)?;
        let tx = conn.transaction()?;
        let changed = db::write::set_flags(
            &tx,
            &ids,
            db::model::FlagPatch {
                seen: Some(true),
                flagged: None,
            },
        )
        .unwrap_or(0);
        tx.rollback()?;

        println!(
            "  {:<28} {:>7.1} ms   ({changed} rows)",
            "set_flags (50, incl. counts)",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }

    let search_query = SearchQuery {
        text: "quarterly figures".into(),
        mailbox_ids: Vec::new(),
        limit: 50,
    };
    time(
        "search",
        Box::new(|| query::search(conn, &search_query).map(|rows| rows.len())),
    );

    println!("\nEXPLAIN QUERY PLAN");

    let plans: &[(&str, &str)] = &[
        (
            "messages_page",
            "SELECT id FROM message WHERE mailbox_id IN (?1)
               AND (date_received, id) < (?2, ?3)
             ORDER BY date_received DESC, id DESC LIMIT ?4",
        ),
        (
            "unread count",
            "SELECT COUNT(*) FROM message WHERE mailbox_id = ?1 AND flag_seen = 0",
        ),
        (
            "search",
            "SELECT message.id FROM message_fts JOIN message ON message.id = message_fts.rowid
              WHERE message_fts MATCH ?1 ORDER BY bm25(message_fts) LIMIT ?2",
        ),
    ];

    for (label, sql) in plans {
        println!("\n  {label}:");
        for line in query::explain(conn, sql)?.lines() {
            println!("    {line}");
        }
    }

    Ok(())
}
