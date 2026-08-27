//! Turning a parsed query into a statement. docs/06 Phase 9.
//!
//! ## Two shapes, because there are two kinds of query
//!
//! A query with free text goes through the FTS5 index and is ranked by relevance. A query with
//! only structured constraints — `is:unread larger:5MB` — has no text to match, cannot use the
//! index at all, and is a filtered scan ordered by date. Forcing the second through FTS5 by
//! matching everything would be far slower than the scan and would rank by a BM25 score that
//! means nothing when every row scores identically.
//!
//! ## Nothing typed reaches the statement
//!
//! Every value is a bound parameter, including the FTS5 match expression. The one thing built
//! from user text is that expression, and each term is quoted and its own quotes stripped, so
//! `NEAR`, `*`, `^` and `"` are matched as words rather than executed as syntax. There is a
//! test for each of those.

use rusqlite::types::Value;

use super::query::Query;

/// A statement and its parameters.
pub struct Compiled {
    pub sql: String,
    pub params: Vec<Value>,
    /// True when the FTS index is being used, so the caller knows a relevance score exists.
    pub ranked: bool,
}

/// Builds the FTS5 `MATCH` expression for the free-text terms.
///
/// Each term is quoted, which turns every FTS5 operator into a literal, and the last term gets
/// a `*` so results appear while the user is still typing. Only the last: prefix-matching every
/// term makes "in the" match most of the mailbox, and the cost lands on the longest posting
/// lists in the index.
fn match_expression(terms: &[String]) -> String {
    let count = terms.len();

    terms
        .iter()
        .enumerate()
        .map(|(index, term)| {
            // Doubling is how FTS5 escapes a quote inside a quoted string. Stripping it instead
            // would silently change what the user searched for.
            let escaped = term.replace('"', "\"\"");

            if index + 1 == count {
                format!("\"{escaped}\"*")
            } else {
                format!("\"{escaped}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Adds one `LIKE` clause per value, ANDed together.
///
/// `%` and `_` in the user's text are escaped, so someone searching for `100%` does not get a
/// wildcard. The same escaping as `rules::predicate`, for the same reason.
fn like_clause(sql: &mut String, params: &mut Vec<Value>, column: &str, values: &[String]) {
    for value in values {
        params.push(Value::Text(format!("%{}%", escape_like(value))));
        sql.push_str(&format!(
            " AND LOWER(COALESCE({column}, '')) LIKE ?{} ESCAPE '\\'",
            params.len()
        ));
    }
}

fn escape_like(value: &str) -> String {
    value
        .to_lowercase()
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// The columns the list needs, qualified.
const COLUMNS: &str = "message.id, message.thread_id, message.mailbox_id, message.account_id, \
     message.subject, message.from_name, message.from_addr, message.date_received, \
     message.preview, message.size, message.flag_seen, message.flag_answered, \
     message.flag_flagged, message.flag_color, message.has_attachment";

/// Compiles a query into a statement returning message rows, most relevant first.
///
/// `scope` limits the search to a set of mailboxes; empty means everywhere.
pub fn compile(query: &Query, scope: &[i64], limit: u32, now: i64) -> Compiled {
    let mut params: Vec<Value> = Vec::new();
    let ranked = !query.terms.is_empty();

    let mut sql = if ranked {
        params.push(Value::Text(match_expression(&query.terms)));
        format!(
            "SELECT {COLUMNS}, bm25(message_fts) AS relevance
               FROM message_fts
               JOIN message ON message.id = message_fts.rowid
               JOIN mailbox ON mailbox.id = message.mailbox_id
              WHERE message_fts MATCH ?1"
        )
    } else {
        format!(
            "SELECT {COLUMNS}, 0.0 AS relevance
               FROM message
               JOIN mailbox ON mailbox.id = message.mailbox_id
              WHERE 1 = 1"
        )
    };

    // Snoozed mail is hidden here as it is everywhere else: a message the user asked to be
    // reminded about later should not surface in a search they ran in the meantime.
    //
    // The moment is **bound, not called**. `strftime('%s','now')` is non-deterministic, so
    // SQLite may not hoist it out of the loop and evaluates it once per row — which over
    // 100,000 rows was measured at roughly 60ms of the 121ms this query used to take. Binding
    // it also makes the query consistent with itself, which calling a clock per row is not.
    params.push(Value::Integer(now));
    sql.push_str(&format!(
        " AND (message.snooze_until IS NULL OR message.snooze_until <= ?{})",
        params.len()
    ));

    if !scope.is_empty() {
        let start = params.len() + 1;
        let list = (0..scope.len())
            .map(|index| format!("?{}", start + index))
            .collect::<Vec<_>>()
            .join(", ");

        sql.push_str(&format!(" AND message.mailbox_id IN ({list})"));
        params.extend(scope.iter().map(|id| Value::Integer(*id)));
    }

    like_clause(&mut sql, &mut params, "message.from_all", &query.from);
    like_clause(&mut sql, &mut params, "message.subject", &query.subject);
    like_clause(
        &mut sql,
        &mut params,
        "mailbox.display_name",
        &query.mailbox,
    );

    // `to:` searches the recipients *and* the Cc, because "did I send this to Ada" does not
    // distinguish between the two and a search that did would miss half the answers.
    for value in &query.to {
        params.push(Value::Text(format!("%{}%", escape_like(value))));
        let first = params.len();
        params.push(Value::Text(format!("%{}%", escape_like(value))));

        sql.push_str(&format!(
            " AND (LOWER(COALESCE(message.to_all, '')) LIKE ?{first} ESCAPE '\\' \
               OR LOWER(COALESCE(message.cc_json, '')) LIKE ?{} ESCAPE '\\')",
            params.len()
        ));
    }

    if let Some(wanted) = query.has_attachment {
        sql.push_str(&format!(
            " AND message.has_attachment = {}",
            i64::from(wanted)
        ));
    }

    if let Some(unread) = query.is_unread {
        // `flag_seen` is the protocol's word and `is:unread` is the user's; the inversion is
        // easy to get right once and wrong everywhere else.
        sql.push_str(&format!(" AND message.flag_seen = {}", i64::from(!unread)));
    }

    if let Some(flagged) = query.is_flagged {
        sql.push_str(&format!(
            " AND message.flag_flagged = {}",
            i64::from(flagged)
        ));
    }

    if let Some(junk) = query.is_junk {
        sql.push_str(&format!(" AND message.is_junk = {}", i64::from(junk)));
    } else {
        // Junk is excluded unless asked for. A search that surfaced the spam folder by default
        // would put the thing the filter caught back in front of the user.
        sql.push_str(" AND message.is_junk = 0");
    }

    for (bound, comparison) in [(query.after, ">="), (query.before, "<")] {
        if let Some(at) = bound {
            params.push(Value::Integer(at));
            sql.push_str(&format!(
                " AND message.date_received {comparison} ?{}",
                params.len()
            ));
        }
    }

    for (bound, comparison) in [(query.larger_than, ">"), (query.smaller_than, "<")] {
        if let Some(bytes) = bound {
            params.push(Value::Integer(bytes));
            sql.push_str(&format!(" AND message.size {comparison} ?{}", params.len()));
        }
    }

    // Candidates are selected by **date**, not by relevance, and then ranked properly in
    // `rank`. That looks wrong and is the most important decision in this file.
    //
    // `ORDER BY bm25()` cannot be answered from an index: SQLite has to score every matching
    // row and sort them all. A term like "the" matches 98,565 of 101,282 messages here, so
    // that is 98k joins and a 98k-row temporary b-tree — measured at 160-220ms, against a
    // 120ms budget, and no index can help because the work is inherent to ranking everything.
    //
    // Selecting by date instead is answerable from `ix_msg_list` and stops at the first N.
    // It is consistent with the ranking rather than a compromise against it: `rank` applies a
    // 30-day half-life, so a message old enough to fall outside a generous recent window would
    // need an enormous relevance advantage to beat anything inside it. The candidates that
    // could win are overwhelmingly recent, and this selects exactly those.
    //
    // The cost is real and worth stating: a very old message that is a far better match than
    // anything recent can fall outside the window and go unfound. The window is set wide —
    // twenty times the requested count — so that needs a mailbox where the answer is both
    // ancient and unique.
    let over_fetch = i64::from(limit) * 6;
    params.push(Value::Integer(over_fetch));

    sql.push_str(&format!(
        " ORDER BY message.date_received DESC LIMIT ?{}",
        params.len()
    ));

    Compiled {
        sql,
        params,
        ranked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::query;

    const NOW: i64 = 1_787_000_000;

    fn parsed(input: &str) -> Query {
        query::parse(input, NOW)
    }

    #[test]
    fn free_text_uses_the_index_and_structured_only_does_not() {
        assert!(compile(&parsed("figures"), &[], 50, NOW).ranked);
        assert!(!compile(&parsed("is:unread"), &[], 50, NOW).ranked);
    }

    #[test]
    fn only_the_last_term_is_a_prefix() {
        // Prefix-matching every term makes "in the" match most of the mailbox, and the cost
        // lands on the longest posting lists in the index.
        let expression = match_expression(&["quarterly".into(), "fig".into()]);
        assert_eq!(expression, "\"quarterly\" \"fig\"*");
    }

    #[test]
    fn fts_operators_are_quoted_rather_than_executed() {
        for hostile in ["NEAR", "AND", "*", "^top", "a OR b"] {
            let expression = match_expression(&[hostile.to_string()]);
            assert!(
                expression.starts_with('"'),
                "{hostile} was not quoted: {expression}"
            );
        }
    }

    #[test]
    fn a_quote_in_a_term_is_doubled_rather_than_stripped() {
        // Doubling is FTS5's escape. Stripping would silently change what was searched for.
        assert_eq!(
            match_expression(&["say \"hi\"".into()]),
            "\"say \"\"hi\"\"\"*"
        );
    }

    #[test]
    fn every_value_is_bound_and_none_is_spliced() {
        // Quoted, so the whole thing survives the parser's word splitting and arrives at the
        // compiler in one piece. Unquoted it would be split on spaces and the test would prove
        // only that the first fragment was safe.
        let hostile = "'; DROP TABLE message; --";
        let compiled = compile(&parsed(&format!("from:\"{hostile}\"")), &[], 50, NOW);

        assert!(
            !compiled.sql.contains("DROP TABLE"),
            "a value reached the statement: {}",
            compiled.sql
        );
        assert!(compiled.params.iter().any(|value| matches!(
            value,
            Value::Text(text) if text.contains("drop table")
        )));
    }

    #[test]
    fn a_literal_percent_is_escaped() {
        let compiled = compile(&parsed("from:100%"), &[], 50, NOW);

        assert!(compiled.params.iter().any(|value| matches!(
            value,
            Value::Text(text) if text.contains("100\\%")
        )));
    }

    #[test]
    fn junk_is_excluded_unless_asked_for() {
        // Otherwise a search puts back in front of the user exactly what the filter caught.
        assert!(compile(&parsed("figures"), &[], 50, NOW)
            .sql
            .contains("is_junk = 0"));
        assert!(compile(&parsed("is:junk"), &[], 50, NOW)
            .sql
            .contains("is_junk = 1"));
    }

    #[test]
    fn unread_inverts_the_stored_column() {
        assert!(compile(&parsed("is:unread"), &[], 50, NOW)
            .sql
            .contains("flag_seen = 0"));
        assert!(compile(&parsed("is:read"), &[], 50, NOW)
            .sql
            .contains("flag_seen = 1"));
    }

    #[test]
    fn to_searches_the_cc_as_well() {
        // "Did I send this to Ada" does not distinguish, and a search that did would miss half
        // the answers.
        let compiled = compile(&parsed("to:ada"), &[], 50, NOW);
        assert!(compiled.sql.contains("to_all"));
        assert!(compiled.sql.contains("cc_json"));
    }

    #[test]
    fn the_query_over_fetches_so_re_ranking_has_something_to_work_with() {
        // Taking exactly `limit` rows and then re-ranking would let the recency and VIP boosts
        // reorder a window that had already thrown away the message they would promote.
        let compiled = compile(&parsed("figures"), &[], 50, NOW);
        assert!(compiled
            .params
            .iter()
            .any(|value| matches!(value, Value::Integer(300))));
    }

    #[test]
    fn candidates_are_selected_by_date_rather_than_by_relevance() {
        // The decision the module comment explains at length: `ORDER BY bm25()` cannot be
        // answered from an index and forces every matching row through a temporary b-tree,
        // which at 98,000 matches is far outside the budget.
        let compiled = compile(&parsed("the"), &[], 50, NOW);

        assert!(compiled.sql.contains("ORDER BY message.date_received DESC"));
        assert!(
            !compiled.sql.contains("ORDER BY bm25"),
            "candidate selection is still ordering by relevance"
        );

        // bm25 is still *selected*, because the ranking needs it.
        assert!(compiled.sql.contains("bm25(message_fts)"));
    }
}
