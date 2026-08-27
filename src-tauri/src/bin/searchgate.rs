//! Phase 9's exit gate. docs/06 Phase 9.
//!
//! > < 120ms for single-term and multi-token queries at 100k messages; suggestions within 30ms
//! > of a keystroke; show the ranked results for 5 real queries and justify the ordering.
//!
//! Run against a copy of the live store, which holds ~101,000 messages:
//!
//! ```text
//! cargo run --release --bin searchgate
//! ```
//!
//! **Release, not debug.** A debug build measures rustc's unoptimised output, not the app, and
//! the difference is routinely five to ten times — a budget met in debug is meaningless and one
//! missed in debug says nothing either. The gate refuses to report a pass from a debug build
//! rather than quietly flattering itself.
//!
//! Each query is run several times and the **worst** run is reported, not the mean. A budget is
//! a promise about what the user experiences, and they experience the slow one.

use std::time::Instant;

use halcyon_lib::search::{query, run};
use rusqlite::Connection;

/// docs/06's budget for a search.
const SEARCH_BUDGET_MS: u128 = 120;

/// docs/06's budget for a suggestion, measured from the keystroke.
const SUGGEST_BUDGET_MS: u128 = 30;

/// How many times each query runs. The first is cold and the rest are warm; both matter, and
/// reporting only the warm one would describe a state the user rarely starts from.
const RUNS: usize = 5;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let live = halcyon_lib::db::default_path();

    if !live.exists() {
        eprintln!("no store at {}", live.display());
        eprintln!("the gate needs a seeded database; run the app and let a sync finish");
        std::process::exit(2);
    }

    // Copied, never opened in place: the app may be running against this file.
    let scratch = std::env::temp_dir().join("halcyon-searchgate.db");
    for stale in ["db", "db-wal", "db-shm"] {
        let _ = std::fs::remove_file(scratch.with_extension(stale));
    }
    std::fs::copy(&live, &scratch)?;
    for suffix in ["-wal", "-shm"] {
        let from = live.with_extension(format!("db{suffix}"));
        if from.exists() {
            let _ = std::fs::copy(&from, scratch.with_extension(format!("db{suffix}")));
        }
    }

    let conn = Connection::open(&scratch)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;

    let total: i64 = conn.query_row("SELECT COUNT(*) FROM message", [], |row| row.get(0))?;
    let indexed: i64 = conn
        .query_row("SELECT COUNT(*) FROM message_fts", [], |row| row.get(0))
        .unwrap_or(0);

    println!("store: {total} messages, {indexed} indexed");

    if cfg!(debug_assertions) {
        println!("\n!! DEBUG BUILD — timings below are not a verdict. Re-run with --release.");
    }

    if total < 50_000 {
        eprintln!(
            "\nonly {total} messages; the gate is about behaviour at 100k and this is not that"
        );
        std::process::exit(3);
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);

    /* ------------------------------------------------------------------ timings */

    let queries: &[(&str, &str)] = &[
        ("single term", "invoice"),
        ("single term, common", "the"),
        ("two terms", "quarterly figures"),
        ("multi-token", "from:ada subject:figures is:unread"),
        ("text and token", "invoice from:bank larger:100kb"),
        ("structured only", "is:unread has:attachment"),
        ("dated", "invoice last week"),
    ];

    println!(
        "\n  {:<24} {:>9} {:>9} {:>7}",
        "query", "worst", "best", "hits"
    );

    let mut worst_overall: u128 = 0;

    for (label, text) in queries {
        let mut timings = Vec::with_capacity(RUNS);
        let mut hits = 0;

        for _ in 0..RUNS {
            let started = Instant::now();
            let found = run::run(&conn, text, &[], 50, now)?;
            timings.push(started.elapsed().as_millis());
            hits = found.len();
        }

        let worst = timings.iter().copied().max().unwrap_or(0);
        let best = timings.iter().copied().min().unwrap_or(0);
        worst_overall = worst_overall.max(worst);

        println!("  {label:<24} {worst:>7}ms {best:>7}ms {hits:>7}");
    }

    // The plan for the slowest query, because a timing without one says what but never why.
    {
        let parsed = query::parse("the", now);
        let compiled = halcyon_lib::search::compile::compile(&parsed, &[], 50, now);
        let params: Vec<&dyn rusqlite::ToSql> = compiled
            .params
            .iter()
            .map(|value| value as &dyn rusqlite::ToSql)
            .collect();

        println!(
            "
  plan for the slowest query:"
        );
        let mut statement = conn.prepare(&format!("EXPLAIN QUERY PLAN {}", compiled.sql))?;
        let rows = statement.query_map(params.as_slice(), |row| row.get::<_, String>(3))?;
        for row in rows {
            println!("    {}", row?);
        }

        let matches: i64 = conn.query_row(
            "SELECT COUNT(*) FROM message_fts WHERE message_fts MATCH ?1",
            rusqlite::params!["\"the\"*"],
            |row| row.get(0),
        )?;
        println!("    (\"the\"* matches {matches} of {total} messages)");
    }

    /* --------------------------------------------------------------- suggestions */
    //
    // A suggestion is what happens on a keystroke, so it is measured as the whole path from
    // the text to the candidates: parse, then the lookups that resolve a prefix into typed
    // tokens. Measuring only the parse would be measuring the cheap half.

    let prefixes = ["q", "qu", "quar", "from:a", "is:"];
    let mut worst_suggest: u128 = 0;

    println!("\n  {:<24} {:>9}", "keystroke", "worst");

    for prefix in prefixes {
        let mut worst = 0u128;

        for _ in 0..RUNS {
            let started = Instant::now();
            let parsed = query::parse(prefix, now);
            let _ = halcyon_lib::search::suggest::suggest(&conn, prefix, &parsed, 8)?;
            worst = worst.max(started.elapsed().as_millis());
        }

        worst_suggest = worst_suggest.max(worst);
        println!("  {prefix:<24} {worst:>7}ms");
    }

    /* ------------------------------------------------- five queries, ranked and explained */

    println!("\n=== ranked results for five real queries ===");

    let samples: &[&str] = &[
        "invoice",
        "from:bank statement",
        "meeting last week",
        "has:attachment invoice",
        "payment",
    ];

    for text in samples {
        println!("\n  query: {text}");
        let hits = run::run(&conn, text, &[], 5, now)?;

        if hits.is_empty() {
            println!("    (no results)");
            continue;
        }

        for (place, hit) in hits.iter().enumerate() {
            let age_days = hit.signals.age_seconds / 86_400;
            let why = [
                (hit.signals.from_vip, "VIP"),
                (hit.signals.participated, "replied"),
                (hit.signals.flagged, "flagged"),
            ]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, label)| *label)
            .collect::<Vec<_>>()
            .join(", ");

            println!(
                "    {}. score {:>8.3}  bm25 {:>7.2}  {:>5}d  {:<28} {}",
                place + 1,
                hit.score,
                hit.signals.bm25,
                age_days,
                hit.row
                    .subject
                    .as_deref()
                    .unwrap_or("(no subject)")
                    .chars()
                    .take(28)
                    .collect::<String>(),
                if why.is_empty() { "-".into() } else { why },
            );
        }
    }

    /* --------------------------------------------------------------------- verdict */

    println!("\n=== verdict ===");
    println!("  worst search      {worst_overall:>4}ms   budget {SEARCH_BUDGET_MS}ms");
    println!("  worst suggestion  {worst_suggest:>4}ms   budget {SUGGEST_BUDGET_MS}ms");

    let _ = std::fs::remove_file(&scratch);

    if cfg!(debug_assertions) {
        println!("\nNOT A VERDICT: debug build. Re-run with --release.");
        std::process::exit(4);
    }

    if worst_overall <= SEARCH_BUDGET_MS && worst_suggest <= SUGGEST_BUDGET_MS {
        println!("\nGATE PASSED");
        Ok(())
    } else {
        println!("\nGATE FAILED");
        std::process::exit(1);
    }
}
