//! Running a search: parse, compile, rank. docs/06 Phase 9.
//!
//! The signals ranking needs are not all on the message row. Whether the sender is a VIP and
//! whether the user took part in the thread are both joins, and doing them per candidate would
//! be two round trips per row. Both are fetched once for the whole result set instead — a
//! search returning 200 candidates does two extra queries, not four hundred.

use std::collections::HashSet;

use rusqlite::Connection;

use crate::db::{model::MessageRow, DbError};

use super::compile;
use super::query::{self, Query};
use super::rank::{self, Signals};

/// One search result, with the score that put it where it is.
///
/// The score is carried out of here rather than discarded because the exit gate asks for the
/// ranking of five real queries to be shown and justified, and a number nobody can see is a
/// number nobody can check.
#[derive(Debug, Clone)]
pub struct Hit {
    pub row: MessageRow,
    pub score: f64,
    pub signals: Signals,
}

fn row_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<(MessageRow, f64)> {
    Ok((
        MessageRow {
            id: row.get(0)?,
            thread_id: row.get(1)?,
            mailbox_id: row.get(2)?,
            account_id: row.get(3)?,
            subject: row.get(4)?,
            from_name: row.get(5)?,
            from_addr: row.get(6)?,
            date_received: row.get(7)?,
            preview: row.get(8)?,
            size: row.get(9)?,
            seen: row.get::<_, i64>(10)? != 0,
            answered: row.get::<_, i64>(11)? != 0,
            flagged: row.get::<_, i64>(12)? != 0,
            flag_color: row.get(13)?,
            has_attachment: row.get::<_, i64>(14)? != 0,
            muted: row.get::<_, i64>(15)? != 0,
        },
        // 16, not 15: the muted column was added ahead of it.
        row.get::<_, f64>(16)?,
    ))
}

/// The set of VIP addresses, lower-cased.
fn vip_addresses(conn: &Connection) -> Result<HashSet<String>, DbError> {
    let mut statement = conn.prepare("SELECT address FROM vip")?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows.into_iter().collect())
}

/// The `Message-ID` of each of these rows, for spotting the same message under two labels.
///
/// One query for the whole candidate set, for the same reason `participated_threads` is: asked
/// per row it would be a round trip through the reader pool for one string, a couple of hundred
/// times.
/// Which of `ids` sit in a mailbox that has a `role` — inbox, sent, archive and the rest.
///
/// A Gmail label folder has no role, which is exactly what makes it the copy to drop when the
/// same message is also filed somewhere real.
fn filed_in_real_mailbox(
    conn: &Connection,
    ids: &[i64],
) -> Result<std::collections::HashSet<i64>, DbError> {
    if ids.is_empty() {
        return Ok(std::collections::HashSet::new());
    }

    let list = (0..ids.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let mut statement = conn.prepare(&format!(
        "SELECT message.id
           FROM message
           JOIN mailbox ON mailbox.id = message.mailbox_id
          WHERE message.id IN ({list}) AND mailbox.role IS NOT NULL"
    ))?;

    let params: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

    let rows = statement
        .query_map(params.as_slice(), |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows.into_iter().collect())
}

/// Keeps one candidate per `(account, Message-Id)`, preferring one filed in a real mailbox.
///
/// A candidate with no `Message-Id` is always kept: an absent value is not evidence that two
/// messages are the same one, and collapsing on it would hide real mail.
///
/// Order is preserved, so whatever the query planner returned first still ranks first among
/// equals — this only removes, it never reorders.
fn dedupe_by_identity(
    candidates: Vec<(MessageRow, f64)>,
    identities: &std::collections::HashMap<i64, String>,
    filed: &std::collections::HashSet<i64>,
) -> Vec<(MessageRow, f64)> {
    let mut best: std::collections::HashMap<(i64, &str), usize> = std::collections::HashMap::new();
    let mut drop = vec![false; candidates.len()];

    for (index, (row, _)) in candidates.iter().enumerate() {
        let Some(message_id) = identities.get(&row.id).filter(|id| !id.is_empty()) else {
            continue;
        };

        match best.get(&(row.account_id, message_id.as_str())) {
            None => {
                best.insert((row.account_id, message_id.as_str()), index);
            }
            Some(&kept) => {
                // The one in a real mailbox wins. If neither is, or both are, the first stays —
                // which keeps the ranking the search already produced.
                let challenger_is_filed = filed.contains(&row.id);
                let kept_is_filed = candidates
                    .get(kept)
                    .is_some_and(|(kept_row, _)| filed.contains(&kept_row.id));

                if challenger_is_filed && !kept_is_filed {
                    drop[kept] = true;
                    best.insert((row.account_id, message_id.as_str()), index);
                } else {
                    drop[index] = true;
                }
            }
        }
    }

    candidates
        .into_iter()
        .zip(drop)
        .filter_map(|(candidate, dropped)| (!dropped).then_some(candidate))
        .collect()
}

fn message_identities(
    conn: &Connection,
    ids: &[i64],
) -> Result<std::collections::HashMap<i64, String>, DbError> {
    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let list = (0..ids.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let mut statement = conn.prepare(&format!(
        "SELECT id, message_id FROM message WHERE id IN ({list}) AND message_id IS NOT NULL"
    ))?;

    let params: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

    let rows = statement
        .query_map(params.as_slice(), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows.into_iter().collect())
}

/// Which of these threads the user has sent a message in.
///
/// One query for the whole candidate set. Asked per row it would be a join per result, which at
/// 200 candidates is 200 round trips through the reader pool for a boolean.
fn participated_threads(conn: &Connection, thread_ids: &[i64]) -> Result<HashSet<i64>, DbError> {
    if thread_ids.is_empty() {
        return Ok(HashSet::new());
    }

    let list = (0..thread_ids.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "SELECT DISTINCT message.thread_id
           FROM message
           JOIN mailbox ON mailbox.id = message.mailbox_id
          WHERE message.thread_id IN ({list})
            AND mailbox.role = 'sent'"
    );

    let params: Vec<&dyn rusqlite::ToSql> = thread_ids
        .iter()
        .map(|id| id as &dyn rusqlite::ToSql)
        .collect();

    let mut statement = conn.prepare(&sql)?;
    let rows = statement
        .query_map(params.as_slice(), |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows.into_iter().collect())
}

/// Runs a search and returns the ranked results.
pub fn run(
    conn: &Connection,
    text: &str,
    scope: &[i64],
    limit: u32,
    now: i64,
) -> Result<Vec<Hit>, DbError> {
    let parsed = query::parse(text, now);
    run_parsed(conn, &parsed, scope, limit, now)
}

/// The same, given an already-parsed query.
///
/// Separate so the suggestion path — which parses as the user types — does not have to parse
/// twice, and so tests can hand in a query without going through the text form.
pub fn run_parsed(
    conn: &Connection,
    parsed: &Query,
    scope: &[i64],
    limit: u32,
    now: i64,
) -> Result<Vec<Hit>, DbError> {
    if parsed.is_empty() {
        return Ok(Vec::new());
    }

    let compiled = compile::compile(parsed, scope, limit, now);

    let candidates = {
        let params: Vec<&dyn rusqlite::ToSql> = compiled
            .params
            .iter()
            .map(|value| value as &dyn rusqlite::ToSql)
            .collect();

        let mut statement = conn.prepare(&compiled.sql)?;
        let rows = statement
            .query_map(params.as_slice(), row_from)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        rows
    };

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let vips = vip_addresses(conn)?;

    let candidate_ids: Vec<i64> = candidates.iter().map(|(row, _)| row.id).collect();
    let identities = message_identities(conn, &candidate_ids)?;

    // One hit per email, not one per folder it happens to sit in. Gmail exposes every label as
    // an IMAP folder, so a labelled message is stored once per label and a search across
    // mailboxes returned the same mail several times over — on the account this was reported
    // from, 107 Message-IDs were duplicated inside one account.
    //
    // It is arguable that two hits in two folders is informative. It is not, in practice: the
    // result list shows a subject and a sender, so the repeats read as "this arrived twice"
    // rather than "this is filed in two places", and they push genuinely different results off
    // the end of a limited list.
    let filed = filed_in_real_mailbox(conn, &candidate_ids)?;
    let candidates = dedupe_by_identity(candidates, &identities, &filed);

    let thread_ids: Vec<i64> = candidates
        .iter()
        .filter_map(|(row, _)| row.thread_id)
        .collect();
    let participated = participated_threads(conn, &thread_ids)?;

    let scored: Vec<(MessageRow, Signals)> = candidates
        .into_iter()
        .map(|(row, bm25)| {
            let from_vip = row
                .from_addr
                .as_deref()
                .map(|address| vips.contains(&address.trim().to_lowercase()))
                .unwrap_or(false);

            let signals = Signals {
                bm25,
                age_seconds: now - row.date_received,
                from_vip,
                participated: row.thread_id.is_some_and(|id| participated.contains(&id)),
                flagged: row.flagged,
            };

            (row, signals)
        })
        .collect();

    let mut hits: Vec<Hit> = scored
        .into_iter()
        .map(|(row, signals)| Hit {
            score: rank::score(&signals),
            row,
            signals,
        })
        .collect();

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Deterministic tiebreak, so the same search twice gives the same order rather
            // than whatever SQLite happened to return.
            .then_with(|| b.row.date_received.cmp(&a.row.date_received))
            .then_with(|| a.row.id.cmp(&b.row.id))
    });

    // One row per message, applied after sorting and before the limit.
    //
    // After sorting, so the copy kept is the best-ranked one. Before the limit, so removing a
    // duplicate cannot leave the caller with fewer results than it asked for -- the query
    // already over-fetches for exactly this kind of pruning.
    //
    // Gmail exposes a labelled message once per label, so the store holds several rows with the
    // same Message-ID: 88 of 1,432 on the account this was found on. Search showed the same
    // email two and three times, which reads as separate emails rather than as one filed twice.
    let mut seen = std::collections::HashSet::new();
    hits.retain(|hit| match identities.get(&hit.row.id) {
        // No Message-ID means nothing proves this is the same message as any other, and
        // dropping mail on a guess is the worse mistake of the two.
        Some(id) if !id.is_empty() => seen.insert(id.clone()),
        _ => true,
    });

    hits.truncate(limit as usize);
    Ok(hits)
}

#[cfg(test)]
mod dedupe_tests {
    use super::*;

    fn row(id: i64) -> MessageRow {
        MessageRow {
            id,
            thread_id: Some(1),
            mailbox_id: 1,
            account_id: 1,
            subject: Some("Credit Advice".into()),
            from_name: None,
            from_addr: Some("bank@example.test".into()),
            date_received: 0,
            preview: None,
            size: 0,
            seen: false,
            answered: false,
            flagged: false,
            flag_color: None,
            has_attachment: false,
            muted: false,
        }
    }

    fn identities(pairs: &[(i64, &str)]) -> std::collections::HashMap<i64, String> {
        pairs
            .iter()
            .map(|(id, mid)| (*id, (*mid).to_string()))
            .collect()
    }

    fn ids(result: &[(MessageRow, f64)]) -> Vec<i64> {
        result.iter().map(|(row, _)| row.id).collect()
    }

    #[test]
    fn the_same_mail_under_two_labels_is_one_hit() {
        // Gmail stores a labelled message once per label. Searching across mailboxes returned
        // it once per folder, which reads as "this arrived three times".
        let candidates = vec![(row(1), 1.0), (row(2), 1.0), (row(3), 1.0)];
        let identities = identities(&[(1, "abc@x"), (2, "abc@x"), (3, "abc@x")]);
        let filed = std::collections::HashSet::from([2]);

        let kept = dedupe_by_identity(candidates, &identities, &filed);

        assert_eq!(
            ids(&kept),
            vec![2],
            "kept a label's copy over the filed one"
        );
    }

    #[test]
    fn different_messages_are_all_kept() {
        let candidates = vec![(row(1), 1.0), (row(2), 1.0)];
        let identities = identities(&[(1, "abc@x"), (2, "def@x")]);

        let kept = dedupe_by_identity(candidates, &identities, &std::collections::HashSet::new());

        assert_eq!(ids(&kept), vec![1, 2]);
    }

    #[test]
    fn messages_without_an_identity_are_never_collapsed() {
        // Two different emails that both lack a Message-Id are still two emails. Folding them
        // on the absent value would hide one.
        let candidates = vec![(row(1), 1.0), (row(2), 1.0)];

        let kept = dedupe_by_identity(
            candidates,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        );

        assert_eq!(ids(&kept), vec![1, 2]);
    }

    #[test]
    fn the_order_the_ranking_produced_survives() {
        // This removes; it must never reorder. A search that shuffled its own ranking while
        // deduplicating would be a worse bug than the one being fixed.
        let candidates = vec![(row(5), 3.0), (row(1), 2.0), (row(9), 1.0)];
        let identities = identities(&[(5, "a@x"), (1, "b@x"), (9, "c@x")]);

        let kept = dedupe_by_identity(candidates, &identities, &std::collections::HashSet::new());

        assert_eq!(ids(&kept), vec![5, 1, 9]);
    }

    #[test]
    fn two_accounts_holding_the_same_message_id_are_both_kept() {
        // The same newsletter to two addresses is two emails the user actually received, and
        // Message-Ids are only unique per sender, not per mailbox.
        let mut second = row(2);
        second.account_id = 2;

        let candidates = vec![(row(1), 1.0), (second, 1.0)];
        let identities = identities(&[(1, "abc@x"), (2, "abc@x")]);

        let kept = dedupe_by_identity(candidates, &identities, &std::collections::HashSet::new());

        assert_eq!(ids(&kept), vec![1, 2]);
    }
}
