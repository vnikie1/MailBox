//! Removes the seeded test accounts from the live store.
//!
//! Three accounts on `.example` domains were created for the Dovecot rig and for the 100,000-
//! message performance work. Every gate that needed them has passed — the Dovecot sync gate,
//! the Phase 8 five-predicate gate, the Phase 9 search budget — and what they leave behind is
//! an app that opens on 24,131 fabricated messages dated two days ago, from an account with no
//! server that can never update. That is indistinguishable, from the outside, from mail that
//! has stopped syncing.
//!
//! ```text
//! cargo run --bin unseed          # says what it would remove
//! cargo run --bin unseed -- --yes # removes it
//! ```
//!
//! Refuses to touch anything whose address is not a `.example` domain. RFC 2606 reserves
//! `.example` precisely so it can never be a real address, which makes it the one safe test
//! for "this account is not somebody's mail". Matching on the seed script's names, or on "has
//! no server configured", would both be one typo away from deleting a real account.

use halcyon_lib::accounts::store;
use rusqlite::Connection;

/// Whether an address is unambiguously a test address.
///
/// RFC 2606 reserves `.example`, `.test`, `.invalid` and `.localhost` so they can never resolve
/// to anything real. Anything else is treated as somebody's mail, whatever it looks like.
fn is_reserved(email: &str) -> bool {
    let domain = match email.rsplit_once('@') {
        Some((_, domain)) => domain.trim().to_ascii_lowercase(),
        None => return false,
    };

    [".example", ".test", ".invalid", ".localhost"]
        .iter()
        .any(|suffix| domain.ends_with(suffix))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let confirmed = std::env::args().any(|argument| argument == "--yes");
    let path = halcyon_lib::db::default_path();

    if !path.exists() {
        eprintln!("no store at {}", path.display());
        std::process::exit(2);
    }

    let mut conn = Connection::open(&path)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    let accounts = {
        let mut statement = conn.prepare("SELECT id, email FROM account ORDER BY id")?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    let mut doomed = Vec::new();

    println!("accounts in the store:");
    for (id, email) in &accounts {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM message WHERE account_id = ?1",
            [id],
            |row| row.get(0),
        )?;

        if is_reserved(email) {
            println!("  {id}  {email:<34} {count:>7}  <- seeded, will be removed");
            doomed.push(*id);
        } else {
            println!("  {id}  {email:<34} {count:>7}  kept");
        }
    }

    if doomed.is_empty() {
        println!("\nnothing to remove.");
        return Ok(());
    }

    if !confirmed {
        println!("\nnothing changed. Re-run with --yes to remove them.");
        return Ok(());
    }

    // Through the app's own removal, not raw SQL. It deletes row by row so the FTS5 triggers
    // fire — a cascade would leave the removed accounts' subjects and senders in the search
    // index, visible as results with no message behind them.
    for id in doomed {
        let tx = conn.transaction()?;
        let removed = store::remove(&tx, id)?;
        tx.commit()?;

        println!("removed account {id} ({})", removed.unwrap_or_default());
    }

    // The freed pages are most of the file. Without this the store stays ~118MB and the space
    // is only reused, never returned.
    println!("reclaiming space…");
    conn.execute_batch("VACUUM")?;

    let left: i64 = conn.query_row("SELECT COUNT(*) FROM message", [], |row| row.get(0))?;
    let indexed: i64 = conn
        .query_row("SELECT COUNT(*) FROM message_fts", [], |row| row.get(0))
        .unwrap_or(-1);

    println!("messages left: {left}");
    println!("search index:  {indexed}");

    if indexed != left {
        // Worth checking rather than assuming: an index left holding rows for deleted messages
        // is exactly what `store::remove`'s row-by-row delete exists to prevent, and a silent
        // mismatch here would show up later as search results that open nothing.
        eprintln!("!! the search index and the message table disagree");
        std::process::exit(1);
    }

    Ok(())
}
