//! A small, presentable mail store for the Microsoft Store screenshots. docs/07 §2.7.
//!
//! The screenshots are published, so they cannot come from anybody's real mail. Nor can the
//! browser build stand in for the app: it renders no message bodies — "Message bodies are only
//! available in the desktop app" — and its compose window says composing is unavailable. So this
//! writes two accounts of invented correspondence into a store of its own, and the real app is
//! pointed at that store instead of the user's.
//!
//! Every message goes in through `transfer::import::write_message`, which is the path an imported
//! `.eml` takes and the same path a synced message takes. Bodies therefore render through the
//! sanitiser, search finds them, and the conversation threads exactly as real mail would. Nothing
//! here writes a message row by hand.
//!
//! Both accounts have syncing off. The app never tries to reach the invented servers, so no
//! connection error ends up in a screenshot.
//!
//! ## It refuses any store it did not make
//!
//! The obvious mistake with a tool like this is running it against the real `halcyon.db`. So it
//! writes only to a file that does not exist yet, or to one carrying its own marker in `setting`,
//! and `--reset` deletes only such a file. Somebody's mail can never be the target.
//!
//! ```text
//! cargo run --manifest-path src-tauri/Cargo.toml --features devtools --bin storedemo -- ^
//!     --path <scratch>\com.uniki.halcyon\halcyon.db --reset
//! ```
//!
//! Dates are relative to now, so a run just before the screenshots puts the newest mail under
//! "Today" with sensible times.

use std::path::{Path, PathBuf};

use base64::Engine;
use rusqlite::{params, Connection, Transaction};

use halcyon_lib::accounts::provider::{AuthKind, Provider, Security, ServerSettings};
use halcyon_lib::accounts::store::{self, NewAccount};
use halcyon_lib::db;
use halcyon_lib::transfer::import;

/// The `setting` row that marks a store as this tool's. Its presence is the only thing that lets
/// the tool write to, or delete, an existing file.
const MARKER: &str = "storedemo.made_by";

const MAYA: Person = Person {
    name: "Maya Chen",
    address: "maya@lumenstudio.example",
};
const MAYA_HOME: Person = Person {
    name: "Maya Chen",
    address: "maya.chen@mailbox.example",
};

#[derive(Clone, Copy)]
struct Person {
    name: &'static str,
    address: &'static str,
}

impl Person {
    fn header(self) -> String {
        format!("{} <{}>", encode_words(self.name), self.address)
    }
}

const fn person(name: &'static str, address: &'static str) -> Person {
    Person { name, address }
}

/// One invented message, and everything about it a screenshot can show.
struct Demo {
    /// `work` or `home`.
    account: &'static str,
    mailbox: &'static str,
    from: Person,
    to: Person,
    subject: &'static str,
    minutes_ago: i64,
    seen: bool,
    answered: bool,
    flag: Option<&'static str>,
    /// Unique within the store, and what `in_reply_to` names.
    id: &'static str,
    in_reply_to: Option<&'static str>,
    references: &'static [&'static str],
    /// Paragraphs. A line starting "• " becomes a list item in the HTML part.
    body: &'static str,
    /// A full HTML part, for mail that is designed rather than typed.
    html: Option<&'static str>,
    attachment: Option<(&'static str, usize)>,
}

const HOUR: i64 = 60;
const DAY: i64 = 24 * HOUR;

/// The base every invented message starts from: read, unflagged, no attachment, not a reply.
#[allow(clippy::too_many_arguments)]
const fn plain(
    account: &'static str,
    mailbox: &'static str,
    from: Person,
    to: Person,
    subject: &'static str,
    minutes_ago: i64,
    id: &'static str,
    body: &'static str,
) -> Demo {
    Demo {
        account,
        mailbox,
        from,
        to,
        subject,
        minutes_ago,
        seen: true,
        answered: false,
        flag: None,
        id,
        in_reply_to: None,
        references: &[],
        body,
        html: None,
        attachment: None,
    }
}

const JONAS: Person = person("Jonas Weber", "jonas@northwind.example");
const PRIYA: Person = person("Priya Raman", "priya@lumenstudio.example");
const DANIEL: Person = person("Daniel Okafor", "daniel@lumenstudio.example");
const HANNAH: Person = person("Hannah Reid", "hannah@lumenstudio.example");
const MARCUS: Person = person("Marcus Lee", "marcus@lumenstudio.example");
const SOFIA: Person = person("Sofia Alvarez", "sofia@alvarezarchitects.example");
const AIKO: Person = person("Aiko Tanaka", "aiko@paperplanepress.example");
const LEO: Person = person("Leo Martins", "leo@brightside.example");
const OLIVIA: Person = person("Olivia Brooks", "olivia@greenleaf.example");
const RAVI: Person = person("Ravi Kapoor", "ravi@kapoorlegal.example");
const CLOUD: Person = person("Northwind Cloud", "billing@northwind-cloud.example");
const BRIEF: Person = person("The Weekly Brief", "hello@weeklybrief.example");
const AIRLINE: Person = person("Northwind Air", "bookings@northwind-air.example");
const MUM: Person = person("Mum", "ling.chen@mailbox.example");
const SAM: Person = person("Sam Patel", "sam.patel@mailbox.example");
const RUNNERS: Person = person("Northside Runners", "club@northsiderunners.example");
const BOOKS: Person = person("Kestrel Books", "orders@kestrelbooks.example");
const LIBRARY: Person = person("City Library", "notices@citylibrary.example");
const ENERGY: Person = person("Greenhouse Energy", "hello@greenhouse-energy.example");

fn messages() -> Vec<Demo> {
    vec![
        // ---- the conversation: a request, Maya's reply, and the answer that arrived just now
        Demo {
            answered: true,
            ..plain(
                "work",
                "Inbox",
                JONAS,
                MAYA,
                "Spring campaign \u{2014} final artwork",
                2 * DAY + 3 * HOUR,
                "spring-1@northwind.example",
                "Hi Maya,\n\n\
                 We're nearly there. Could you send the final artwork for the spring campaign \
                 by Wednesday? We need the poster, the two social crops and the bus-shelter \
                 version.\n\n\
                 If there's still a choice between the two layouts, send both and we'll \
                 decide here.\n\n\
                 Thanks,\nJonas",
            )
        },
        Demo {
            answered: false,
            in_reply_to: Some("spring-1@northwind.example"),
            references: &["spring-1@northwind.example"],
            attachment: Some(("Spring campaign layouts.pdf", 2_412_000)),
            ..plain(
                "work",
                "Sent",
                MAYA,
                JONAS,
                "Re: Spring campaign \u{2014} final artwork",
                DAY + 4 * HOUR,
                "spring-2@lumenstudio.example",
                "Hi Jonas,\n\n\
                 Both layouts are attached, at full size and in the two social crops. The \
                 bus-shelter version is the last page.\n\n\
                 My vote is the second one: the headline has more room, and it reads from \
                 across the street.\n\n\
                 Maya",
            )
        },
        Demo {
            seen: false,
            in_reply_to: Some("spring-2@lumenstudio.example"),
            references: &["spring-1@northwind.example", "spring-2@lumenstudio.example"],
            ..plain(
                "work",
                "Inbox",
                JONAS,
                MAYA,
                "Re: Spring campaign \u{2014} final artwork",
                9,
                "spring-3@northwind.example",
                "Hi Maya,\n\n\
                 These are lovely. The second layout is the one \u{2014} the type finally has \
                 room to breathe, and the colour holds up in print.\n\n\
                 Two small things before we sign off:\n\
                 \u{2022} Could the headline sit a touch lower on the poster, so it clears the \
                 fold?\n\
                 \u{2022} The legal line needs to be 8pt at least for the bus shelters.\n\n\
                 If you can turn those around by Thursday we'll still make the print slot.\n\n\
                 Thank you \u{2014} this is the best work we've had on the account.\n\n\
                 Jonas",
            )
        },
        // ---- the rest of the work inbox, newest first
        Demo {
            seen: false,
            attachment: Some(("Venue shortlist.pdf", 1_184_000)),
            ..plain(
                "work",
                "Inbox",
                PRIYA,
                MAYA,
                "Studio offsite: venue shortlist",
                38,
                "offsite@lumenstudio.example",
                "Morning all,\n\n\
                 I've narrowed the offsite down to three places, all within an hour of the \
                 studio:\n\
                 \u{2022} The Granary \u{2014} a big open room, good light, and a garden for \
                 lunch.\n\
                 \u{2022} Harbour House \u{2014} on the water; smaller and quieter.\n\
                 \u{2022} Fieldwork \u{2014} a converted print works, which feels very on \
                 brand.\n\n\
                 Details and prices are in the attached PDF. Could you add your vote to the \
                 sheet by Friday?\n\n\
                 Priya",
            )
        },
        Demo {
            html: Some(INVOICE_HTML),
            attachment: Some(("Invoice 2026-09.pdf", 86_000)),
            ..plain(
                "work",
                "Inbox",
                CLOUD,
                MAYA,
                "Your invoice for September",
                HOUR + 25,
                "invoice-0926@northwind-cloud.example",
                "Hello Maya,\n\n\
                 Your invoice for September is attached.\n\n\
                 Plan: Studio, 5 seats\nAmount: \u{a3}84.00\n\
                 Paid on 1 October with the card ending 4242.\n\n\
                 Thank you for being a customer.\n\nThe Northwind Cloud team",
            )
        },
        Demo {
            flag: Some("orange"),
            ..plain(
                "work",
                "Inbox",
                SOFIA,
                MAYA,
                "Kick-off call \u{2014} notes and next steps",
                3 * HOUR + 10,
                "kickoff@alvarezarchitects.example",
                "Hi Maya,\n\n\
                 Thanks for your time this morning. My notes, so we're working from the same \
                 list:\n\
                 \u{2022} The brand refresh launches alongside the new website in March.\n\
                 \u{2022} Three directions for the first review \u{2014} rough rather than \
                 polished.\n\
                 \u{2022} Our team will send the photography archive by Monday.\n\n\
                 Next check-in is Wednesday at 10. I'll send an invitation.\n\n\
                 Best wishes,\nSofia",
            )
        },
        Demo {
            html: Some(NEWSLETTER_HTML),
            ..plain(
                "work",
                "Inbox",
                BRIEF,
                MAYA,
                "Ten tools worth keeping this year",
                5 * HOUR + 40,
                "issue-142@weeklybrief.example",
                "The Weekly Brief, issue 142.\n\n\
                 Ten tools worth keeping this year: the ones our readers still open every \
                 day, a year after they first wrote in about them.",
            )
        },
        Demo {
            answered: true,
            ..plain(
                "work",
                "Inbox",
                DANIEL,
                MAYA,
                "Lunch on Thursday?",
                DAY + 2 * HOUR,
                "lunch@lumenstudio.example",
                "Fancy lunch on Thursday? The new place on Harbour Street does a very good \
                 ramen. 12:30?\n\nD",
            )
        },
        Demo {
            in_reply_to: Some("lunch@lumenstudio.example"),
            references: &["lunch@lumenstudio.example"],
            ..plain(
                "work",
                "Sent",
                MAYA,
                DANIEL,
                "Re: Lunch on Thursday?",
                DAY + HOUR + 30,
                "lunch-reply@lumenstudio.example",
                "Yes please \u{2014} 12:30 works. I'll book a table.\n\nM",
            )
        },
        Demo {
            attachment: Some(("Proofs v3.pdf", 3_906_000)),
            ..plain(
                "work",
                "Inbox",
                AIKO,
                MAYA,
                "Print proofs are ready",
                DAY + 6 * HOUR,
                "proofs-v3@paperplanepress.example",
                "Hi Maya,\n\n\
                 The third round of proofs is attached. We've corrected the paper stock and \
                 the spot colour on the cover.\n\n\
                 If you're happy, reply \u{201c}approved\u{201d} and we'll start the run on \
                 Monday.\n\n\
                 Aiko\nPaper Plane Press",
            )
        },
        plain(
            "work",
            "Inbox",
            HANNAH,
            MAYA,
            "Timesheets due Friday",
            DAY + 9 * HOUR,
            "timesheets@lumenstudio.example",
            "Hi everyone,\n\n\
             A reminder that September's timesheets are due on Friday. If you worked on the \
             Northwind account, please split your hours between the campaign and the \
             retainer.\n\nThank you!\nHannah",
        ),
        plain(
            "work",
            "Inbox",
            LEO,
            MAYA,
            "Introduction: Leo \u{2194} Maya",
            2 * DAY + 8 * HOUR,
            "intro@brightside.example",
            "Hi Maya,\n\n\
             Olivia suggested I get in touch. We're a small team building a reading app for \
             schools, and we're looking for a studio to help with the identity.\n\n\
             Would you have time for a call next week?\n\nBest,\nLeo",
        ),
        Demo {
            flag: Some("red"),
            ..plain(
                "work",
                "Inbox",
                OLIVIA,
                MAYA,
                "Proposal accepted \u{1f389}",
                3 * DAY + 4 * HOUR,
                "accepted@greenleaf.example",
                "Maya,\n\n\
                 Great news \u{2014} the board signed off the proposal this afternoon, budget \
                 and timeline as written.\n\n\
                 Could we start the week of the 20th? I'll send the purchase order \
                 separately.\n\nOlivia",
            )
        },
        Demo {
            html: Some(BOOKING_HTML),
            ..plain(
                "work",
                "Inbox",
                AIRLINE,
                MAYA,
                "Booking confirmed: London to Lisbon, 14 November",
                4 * DAY + 2 * HOUR,
                "booking-NW4821@northwind-air.example",
                "Your booking is confirmed.\n\n\
                 London (LHR) to Lisbon (LIS), Friday 14 November, departing 08:40.\n\
                 Booking reference: NW4821.",
            )
        },
        plain(
            "work",
            "Inbox",
            MARCUS,
            MAYA,
            "Brand guidelines v2 \u{2014} feedback welcome",
            5 * DAY + 3 * HOUR,
            "guidelines@lumenstudio.example",
            "Hi all,\n\n\
             The second draft of the guidelines is up in the shared drive. The biggest \
             change is the type scale, which is now built from one ratio instead of three.\n\n\
             Comments by the end of next week, please.\n\nMarcus",
        ),
        plain(
            "work",
            "Inbox",
            RAVI,
            MAYA,
            "Contract countersigned",
            8 * DAY,
            "contract@kapoorlegal.example",
            "Dear Maya,\n\n\
             Please find confirmation that the Greenleaf agreement has been countersigned. \
             A copy is on file with us.\n\nKind regards,\nRavi Kapoor",
        ),
        plain(
            "work",
            "Archive",
            SOFIA,
            MAYA,
            "Photography archive",
            12 * DAY,
            "archive@alvarezarchitects.example",
            "Hi Maya, the archive link is in our shared folder. Sofia",
        ),
        // ---- personal
        Demo {
            seen: false,
            ..plain(
                "home",
                "Inbox",
                MUM,
                MAYA_HOME,
                "Sunday lunch",
                2 * HOUR + 15,
                "sunday@mailbox.example",
                "Hello love,\n\n\
                 Are you still coming on Sunday? Dad is making dumplings, so come hungry.\n\n\
                 Bring a jumper \u{2014} it's getting cold in the garden.\n\n\
                 Lots of love,\nMum x",
            )
        },
        plain(
            "home",
            "Inbox",
            RUNNERS,
            MAYA_HOME,
            "Saturday 10k: route and start time",
            6 * HOUR,
            "saturday@northsiderunners.example",
            "Morning runners,\n\n\
             This Saturday's 10k starts from the boathouse at 8:30. The route follows the \
             river out to the old mill and back along the canal.\n\n\
             Coffee afterwards, as ever.\n\nSee you there!",
        ),
        Demo {
            html: Some(SHIPPED_HTML),
            ..plain(
                "home",
                "Inbox",
                BOOKS,
                MAYA_HOME,
                "Your order has shipped",
                DAY + 5 * HOUR,
                "order-31847@kestrelbooks.example",
                "Your order is on its way.\n\n\
                 Order 31847: The Shape of Design; A Pattern Language.\n\
                 Expected delivery: Friday.",
            )
        },
        plain(
            "home",
            "Inbox",
            SAM,
            MAYA_HOME,
            "Photos from Lisbon",
            2 * DAY + 6 * HOUR,
            "lisbon@mailbox.example",
            "Hey!\n\n\
             Finally sorted the photos from the trip \u{2014} the album's in the shared \
             folder. The one of the tram at sunset might be my favourite photo I've ever \
             taken.\n\nSam",
        ),
        plain(
            "home",
            "Inbox",
            LIBRARY,
            MAYA_HOME,
            "Reminder: two books due next week",
            3 * DAY + 2 * HOUR,
            "due@citylibrary.example",
            "Hello,\n\n\
             Two of your books are due back on Tuesday. You can renew them online or at any \
             branch.\n\nCity Library",
        ),
        plain(
            "home",
            "Inbox",
            ENERGY,
            MAYA_HOME,
            "Your October statement",
            6 * DAY,
            "statement@greenhouse-energy.example",
            "Hello Maya,\n\n\
             Your October statement is ready. You used 11% less electricity than last \
             October.\n\nGreenhouse Energy",
        ),
    ]
}

const INVOICE_HTML: &str = r#"<div style="font-family:Segoe UI,Arial,sans-serif;color:#1d1d1f;max-width:560px">
<p style="font-size:15px">Hello Maya,</p>
<p style="font-size:15px">Your invoice for September is attached. Here is a summary.</p>
<table style="width:100%;border-collapse:collapse;font-size:15px;margin:18px 0">
<tr><td style="padding:10px 0;border-bottom:1px solid #e5e5ea;color:#6e6e73">Plan</td><td style="padding:10px 0;border-bottom:1px solid #e5e5ea;text-align:right">Studio, 5 seats</td></tr>
<tr><td style="padding:10px 0;border-bottom:1px solid #e5e5ea;color:#6e6e73">Period</td><td style="padding:10px 0;border-bottom:1px solid #e5e5ea;text-align:right">1&ndash;30 September 2026</td></tr>
<tr><td style="padding:10px 0;border-bottom:1px solid #e5e5ea;color:#6e6e73">Paid</td><td style="padding:10px 0;border-bottom:1px solid #e5e5ea;text-align:right">1 October, card ending 4242</td></tr>
<tr><td style="padding:14px 0;font-weight:600">Total</td><td style="padding:14px 0;text-align:right;font-weight:600">&pound;84.00</td></tr>
</table>
<p style="font-size:15px">Thank you for being a customer.</p>
<p style="font-size:15px;color:#6e6e73">The Northwind Cloud team</p>
</div>"#;

const NEWSLETTER_HTML: &str = r#"<div style="font-family:Georgia,serif;color:#1d1d1f;max-width:600px;margin:0 auto">
<div style="background:#14325c;color:#ffffff;padding:28px 32px;border-radius:12px 12px 0 0">
<div style="font-family:Segoe UI,Arial,sans-serif;font-size:12px;letter-spacing:1.5px;text-transform:uppercase;opacity:.75">The Weekly Brief &middot; Issue 142</div>
<h1 style="font-size:30px;line-height:1.2;margin:10px 0 0;font-weight:normal">Ten tools worth keeping this year</h1>
</div>
<div style="background:#f5f5f7;padding:28px 32px;border-radius:0 0 12px 12px;font-size:17px;line-height:1.6">
<p style="margin-top:0">A year ago we asked readers which tools they could not do without. This week we went back to ask which ones they still open every day.</p>
<h2 style="font-family:Segoe UI,Arial,sans-serif;font-size:19px;margin:26px 0 6px">1. A notebook that syncs nowhere</h2>
<p style="margin:0 0 14px">The most-mentioned tool was not software at all. Several of you wrote that the best way to think is somewhere nothing can interrupt you.</p>
<h2 style="font-family:Segoe UI,Arial,sans-serif;font-size:19px;margin:26px 0 6px">2. A mail client that stays out of the way</h2>
<p style="margin:0 0 14px">Three panes, quick search, and nothing asking to be noticed. Readers who switched said the difference was calm.</p>
<p style="margin:26px 0 0"><a href="https://weeklybrief.example/142" style="display:inline-block;background:#14325c;color:#ffffff;text-decoration:none;font-family:Segoe UI,Arial,sans-serif;font-size:15px;padding:11px 20px;border-radius:8px">Read the full list</a></p>
</div>
<p style="font-family:Segoe UI,Arial,sans-serif;font-size:12px;color:#86868b;text-align:center;margin:18px 0">You receive this because you subscribed at weeklybrief.example.</p>
</div>"#;

const BOOKING_HTML: &str = r#"<div style="font-family:Segoe UI,Arial,sans-serif;color:#1d1d1f;max-width:560px">
<p style="font-size:15px">Hello Maya, your booking is confirmed.</p>
<div style="border:1px solid #e5e5ea;border-radius:12px;padding:20px 24px;margin:18px 0">
<div style="font-size:13px;color:#6e6e73;text-transform:uppercase;letter-spacing:1px">Friday 14 November</div>
<table style="width:100%;margin-top:10px;font-size:15px"><tr>
<td><div style="font-size:30px;font-weight:600">LHR</div><div style="color:#6e6e73">London 08:40</div></td>
<td style="text-align:center;color:#6e6e73">&rarr; 2h 45m</td>
<td style="text-align:right"><div style="font-size:30px;font-weight:600">LIS</div><div style="color:#6e6e73">Lisbon 11:25</div></td>
</tr></table>
<div style="margin-top:14px;font-size:14px;color:#6e6e73">Booking reference <b style="color:#1d1d1f">NW4821</b> &middot; Seat 14A</div>
</div>
<p style="font-size:15px">Online check-in opens 24 hours before departure.</p>
</div>"#;

const SHIPPED_HTML: &str = r#"<div style="font-family:Segoe UI,Arial,sans-serif;color:#1d1d1f;max-width:560px">
<p style="font-size:15px">Good news, Maya &mdash; your order is on its way.</p>
<table style="width:100%;border-collapse:collapse;font-size:15px;margin:16px 0">
<tr><td style="padding:10px 0;border-bottom:1px solid #e5e5ea">The Shape of Design</td><td style="padding:10px 0;border-bottom:1px solid #e5e5ea;text-align:right">&pound;14.99</td></tr>
<tr><td style="padding:10px 0;border-bottom:1px solid #e5e5ea">A Pattern Language</td><td style="padding:10px 0;border-bottom:1px solid #e5e5ea;text-align:right">&pound;32.00</td></tr>
</table>
<p style="font-size:15px">Expected delivery: <b>Friday</b>. Order 31847.</p>
<p style="font-size:15px;color:#6e6e73">Kestrel Books</p>
</div>"#;

struct Options {
    path: PathBuf,
    reset: bool,
}

fn parse_args() -> Result<Options, String> {
    let mut path = None;
    let mut reset = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--path" => path = args.next().map(PathBuf::from),
            "--reset" => reset = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }

    // Required, with no default. `db::default_path()` is the user's own mail.
    let path = path.ok_or("--path is required: the store to create, never the real one")?;
    Ok(Options { path, reset })
}

/// Whether an existing file is a store this tool made.
fn made_here(path: &Path) -> bool {
    let Ok(conn) = Connection::open(path) else {
        return false;
    };
    conn.query_row(
        "SELECT 1 FROM setting WHERE key = ?1",
        params![MARKER],
        |row| row.get::<_, i64>(0),
    )
    .is_ok()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = parse_args()?;
    let folder = options
        .path
        .parent()
        .ok_or("--path needs a folder")?
        .to_path_buf();

    if options.path.exists() {
        if !made_here(&options.path) {
            return Err(format!(
                "{} exists and was not made by storedemo; refusing to touch it",
                options.path.display()
            )
            .into());
        }
        if !options.reset {
            return Err("that demo store already exists; pass --reset to rebuild it".into());
        }

        for suffix in ["", "-wal", "-shm"] {
            let mut file = options.path.clone().into_os_string();
            file.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(file));
        }
        let _ = std::fs::remove_dir_all(folder.join("bodies"));
        println!("removed the previous demo store");
    }

    std::fs::create_dir_all(&folder)?;

    let mut conn = Connection::open(&options.path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    db::migrate::run(&mut conn)?;

    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO setting (key, value) VALUES (?1, 'storedemo')",
        params![MARKER],
    )?;

    let work = account(&tx, "Lumen Studio", MAYA.address, "blue")?;
    let home = account(&tx, "Personal", MAYA_HOME.address, "green")?;

    let now = chrono::Utc::now().timestamp();
    let mut mailboxes: Vec<(i64, i64)> = Vec::new();
    let mut written = 0usize;

    for demo in messages() {
        let account_id = if demo.account == "work" { work } else { home };

        let mailbox_id = import::mailbox_for(&tx, account_id, demo.mailbox)?;
        if !mailboxes.contains(&(account_id, mailbox_id)) {
            mailboxes.push((account_id, mailbox_id));
        }

        let uid = import::next_uid(&tx, mailbox_id)?;
        let raw = render(&demo, now);

        let Some(message_id) =
            import::write_message(&tx, &folder, account_id, mailbox_id, uid, &raw, None)?
        else {
            return Err(format!("{} did not parse", demo.id).into());
        };

        if let Some(colour) = demo.flag {
            tx.execute(
                "UPDATE message SET flag_flagged = 1, flag_color = ?2 WHERE id = ?1",
                params![message_id, colour],
            )?;
        }

        written += 1;
    }

    // The standard mailboxes an account shows even when they are empty, so the sidebar reads like
    // a real account's rather than a list of whatever happened to receive mail.
    for account_id in [work, home] {
        for path in ["Drafts", "Sent", "Junk", "Trash", "Archive"] {
            let mailbox_id = import::mailbox_for(&tx, account_id, path)?;
            if !mailboxes.contains(&(account_id, mailbox_id)) {
                mailboxes.push((account_id, mailbox_id));
            }
        }
    }
    for path in ["Clients", "Invoices", "Travel"] {
        let mailbox_id = import::mailbox_for(&tx, work, path)?;
        if !mailboxes.contains(&(work, mailbox_id)) {
            mailboxes.push((work, mailbox_id));
        }
    }

    for account_id in [work, home] {
        let ids: Vec<i64> = mailboxes
            .iter()
            .filter(|(owner, _)| *owner == account_id)
            .map(|(_, id)| *id)
            .collect();
        import::finish(&tx, account_id, &ids)?;
    }

    // Jonas is the client the conversation is with; a VIP shows how the list marks one.
    tx.execute(
        "INSERT INTO vip (address, added_at) VALUES (?1, ?2)",
        params![JONAS.address, now],
    )?;

    tx.commit()?;

    println!(
        "{written} messages in {} mailboxes across 2 accounts",
        mailboxes.len()
    );
    println!("store: {}", options.path.display());
    Ok(())
}

fn account(
    tx: &Transaction<'_>,
    display_name: &str,
    email: &str,
    colour: &str,
) -> Result<i64, Box<dyn std::error::Error>> {
    let server = |host: &str, port: u16| ServerSettings {
        host: host.into(),
        port,
        security: Security::Tls,
    };

    let id = store::insert(
        tx,
        &NewAccount {
            display_name: display_name.into(),
            email: email.into(),
            provider: Provider::Other,
            imap: server("imap.mail.example", 993),
            smtp: server("smtp.mail.example", 465),
            auth_kind: AuthKind::Password,
            color: Some(colour.into()),
        },
    )?;

    // Off, so the app never reaches for the invented server and no error lands in a screenshot.
    store::update(tx, id, None, None, Some(false))?;
    Ok(id)
}

/// The message as an RFC 5322 file, the same shape a server or another client would hand over.
fn render(demo: &Demo, now: i64) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp(now - demo.minutes_ago * 60, 0)
        .unwrap_or_default()
        .to_rfc2822();

    // Thunderbird's status bits, which the importer reads: 0x1 read, 0x2 replied, 0x4 flagged.
    let status = u32::from(demo.seen)
        | (u32::from(demo.answered) << 1)
        | (u32::from(demo.flag.is_some()) << 2);

    let mut head = String::new();
    head.push_str(&format!("From: {}\r\n", demo.from.header()));
    head.push_str(&format!("To: {}\r\n", demo.to.header()));
    head.push_str(&format!("Subject: {}\r\n", encode_words(demo.subject)));
    head.push_str(&format!("Date: {date}\r\n"));
    head.push_str(&format!("Message-ID: <{}>\r\n", demo.id));
    if let Some(parent) = demo.in_reply_to {
        head.push_str(&format!("In-Reply-To: <{parent}>\r\n"));
    }
    if !demo.references.is_empty() {
        let list: Vec<String> = demo.references.iter().map(|id| format!("<{id}>")).collect();
        head.push_str(&format!("References: {}\r\n", list.join(" ")));
    }
    head.push_str(&format!("X-Mozilla-Status: {status:04x}\r\n"));
    head.push_str("MIME-Version: 1.0\r\n");

    let html = demo
        .html
        .map(str::to_string)
        .unwrap_or_else(|| html_from(demo.body));

    let alternative = format!(
        "--alt\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{}\r\n\
         --alt\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{}\r\n\
         --alt--\r\n",
        demo.body.replace('\n', "\r\n"),
        html
    );

    let mut out = head;
    match demo.attachment {
        None => {
            out.push_str("Content-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n");
            out.push_str(&alternative);
        }
        Some((name, size)) => {
            out.push_str("Content-Type: multipart/mixed; boundary=\"mixed\"\r\n\r\n");
            out.push_str(
                "--mixed\r\nContent-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n",
            );
            out.push_str(&alternative);
            out.push_str(&format!(
                "--mixed\r\nContent-Type: application/pdf; name=\"{name}\"\r\n\
                 Content-Disposition: attachment; filename=\"{name}\"\r\n\
                 Content-Transfer-Encoding: base64\r\n\r\n"
            ));
            let encoded = base64::engine::general_purpose::STANDARD.encode(pdf(name, size));
            for line in encoded.as_bytes().chunks(76) {
                out.push_str(&String::from_utf8_lossy(line));
                out.push_str("\r\n");
            }
            out.push_str("--mixed--\r\n");
        }
    }

    out.into_bytes()
}

/// Paragraphs to HTML, the way a mail client's composer would write them.
fn html_from(body: &str) -> String {
    let mut html = String::from(
        "<div style=\"font-family:Segoe UI,Arial,sans-serif;font-size:15px;line-height:1.5\">",
    );

    for paragraph in body.split("\n\n") {
        let (items, prose): (Vec<&str>, Vec<&str>) = paragraph
            .lines()
            .partition(|line| line.starts_with('\u{2022}'));

        if !prose.is_empty() {
            html.push_str("<p>");
            html.push_str(
                &prose
                    .iter()
                    .map(|line| escape(line))
                    .collect::<Vec<_>>()
                    .join("<br>"),
            );
            html.push_str("</p>");
        }
        if !items.is_empty() {
            html.push_str("<ul>");
            for item in items {
                html.push_str("<li>");
                html.push_str(&escape(item.trim_start_matches('\u{2022}').trim()));
                html.push_str("</li>");
            }
            html.push_str("</ul>");
        }
    }

    html.push_str("</div>");
    html
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// RFC 2047, for a header that is not plain ASCII.
fn encode_words(text: &str) -> String {
    if text.is_ascii() {
        text.to_string()
    } else {
        format!(
            "=?UTF-8?B?{}?=",
            base64::engine::general_purpose::STANDARD.encode(text)
        )
    }
}

/// A one-page PDF of roughly the given size, so an attachment chip shows a believable size.
///
/// Padded with a PDF comment, which every reader skips. The page itself is blank: nothing in a
/// screenshot opens it.
fn pdf(title: &str, size: usize) -> Vec<u8> {
    let mut out = format!(
        "%PDF-1.4\n1 0 obj <</Type/Catalog/Pages 2 0 R>> endobj\n\
         2 0 obj <</Type/Pages/Kids[3 0 R]/Count 1>> endobj\n\
         3 0 obj <</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]>> endobj\n\
         4 0 obj <</Title({title})>> endobj\n\
         trailer <</Root 1 0 R/Info 4 0 R>>\n"
    )
    .into_bytes();

    let filler = b"% Halcyon demo document\n";
    while out.len() + filler.len() < size {
        out.extend_from_slice(filler);
    }
    out.extend_from_slice(b"%%EOF\n");
    out
}
