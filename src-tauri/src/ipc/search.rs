//! The search command surface. docs/03 §4, docs/06 Phase 9.

use serde::Serialize;
use tauri::State;
use ts_rs::TS;

use crate::db::{model::MessageRow, Db};
use crate::search::{query, run, suggest};

use super::mail::AppError;

type Response<T> = Result<T, AppError>;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// One result, with why it is where it is.
///
/// The score and its parts travel to the UI rather than being discarded, because a ranking
/// nobody can inspect is a ranking nobody can correct. It is what the exit gate's "justify the
/// ordering" asks for, and it is what makes a complaint about search actionable rather than a
/// matter of opinion.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub row: MessageRow,
    pub score: f64,
    /// The terms that matched, for highlighting in the reader.
    pub terms: Vec<String>,
    pub from_vip: bool,
    pub participated: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    /// What the parser made of the text, so the UI can draw the tokens back as chips.
    pub parsed: query::Query,
}

#[tauri::command]
pub async fn search_run(
    db: State<'_, Db>,
    text: String,
    mailbox_ids: Vec<i64>,
    limit: u32,
) -> Response<SearchResults> {
    let stamp = now();
    let parsed = query::parse(&text, stamp);
    let terms = parsed.terms.clone();

    let hits = db
        .read({
            let parsed = parsed.clone();
            move |conn| run::run_parsed(conn, &parsed, &mailbox_ids, limit, stamp)
        })
        .await?;

    Ok(SearchResults {
        hits: hits
            .into_iter()
            .map(|hit| SearchHit {
                row: hit.row,
                score: hit.score,
                terms: terms.clone(),
                from_vip: hit.signals.from_vip,
                participated: hit.signals.participated,
            })
            .collect(),
        parsed,
    })
}

/// Candidates for the dropdown. Runs on every keystroke, so it has a 30ms budget.
#[tauri::command]
pub async fn search_suggest(
    db: State<'_, Db>,
    text: String,
    limit: u32,
) -> Response<Vec<suggest::Suggestion>> {
    let stamp = now();

    Ok(db
        .read(move |conn| {
            let parsed = query::parse(&text, stamp);
            suggest::suggest(conn, &text, &parsed, limit as usize)
        })
        .await?)
}

/// The searches this user has run, most recent first.
#[tauri::command]
pub async fn search_history(db: State<'_, Db>, limit: u32) -> Response<Vec<String>> {
    Ok(db
        .read(move |conn| {
            let mut statement =
                conn.prepare("SELECT text FROM search_history ORDER BY last_used DESC LIMIT ?1")?;

            let rows = statement
                .query_map(rusqlite::params![limit], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            Ok(rows)
        })
        .await?)
}

/// Records a search the user actually ran.
///
/// Called when a search is *committed* — Enter, or a suggestion chosen — and never on a
/// keystroke. Recording every prefix would fill the history with the twelve fragments typed on
/// the way to one query, which is the opposite of useful.
#[tauri::command]
pub async fn search_remember(db: State<'_, Db>, text: String) -> Response<()> {
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Ok(());
    }

    let stamp = now();

    db.write(move |tx| {
        tx.execute(
            "INSERT INTO search_history (text, last_used, times_used) VALUES (?1, ?2, 1)
             ON CONFLICT(text) DO UPDATE SET
                 last_used = excluded.last_used,
                 times_used = times_used + 1",
            rusqlite::params![trimmed, stamp],
        )?;

        // Bounded. A history that grows without limit is a list nobody scrolls and a table
        // that only ever gets bigger.
        tx.execute(
            "DELETE FROM search_history
              WHERE text NOT IN (
                SELECT text FROM search_history ORDER BY last_used DESC LIMIT 50
              )",
            [],
        )?;

        Ok(())
    })
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn search_history_clear(db: State<'_, Db>) -> Response<()> {
    db.write(|tx| {
        tx.execute("DELETE FROM search_history", [])?;
        Ok(())
    })
    .await?;

    Ok(())
}

/// Saves the current search as a smart mailbox. docs/06 Phase 9.
///
/// The search is stored as the *predicate* it corresponds to, not as its text. A smart mailbox
/// built from a search has to keep working when the search language changes, and a saved string
/// re-parsed by a later version could quietly come to mean something else.
#[tauri::command]
pub async fn search_save_as_smart(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    name: String,
    text: String,
) -> Response<i64> {
    let parsed = query::parse(&text, now());
    let predicate = crate::search::as_predicate(&parsed);

    let id = db
        .write(move |tx| {
            crate::rules::engine::smart_save(tx, None, &name, Some("search"), &predicate)
        })
        .await?;

    let _ = tauri::Emitter::emit(&app, "smart:changed", ());
    Ok(id)
}
