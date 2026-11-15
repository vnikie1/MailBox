//! VIPs, flag names, blocked senders, Remind Me and Follow Up. docs/01 §8.
//!
//! These are small, but they share a property worth stating once: every one of them keys off an
//! **address**, and an address compared the wrong way is a feature that silently does nothing.
//! `Ada@Example.com` and `ada@example.com` are the same mailbox to every provider in existence,
//! so everything here normalises before it stores and before it compares.

use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::DbError;

/// Mail's cap. Enforced here rather than as a table constraint so that hitting it produces a
/// message the user understands instead of a failed write.
pub const MAX_VIPS: usize = 100;

/// Lower-cases and strips a display name, so `"Ada" <Ada@Example.COM>` and `ada@example.com`
/// are one key.
///
/// The local part is case-sensitive per RFC 5321 §2.4 and nothing on earth treats it that way.
/// Honouring the RFC here would mean a VIP that stops working when the sender's client changes
/// how it capitalises their own name.
pub fn normalise(address: &str) -> String {
    let inner = match (address.rfind('<'), address.rfind('>')) {
        (Some(open), Some(close)) if close > open => &address[open + 1..close],
        _ => address,
    };

    inner.trim().to_lowercase()
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Vip {
    pub address: String,
    #[ts(type = "number")]
    pub added_at: i64,
}

/// Why an operation could not be done, in terms the UI can put in front of a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    TooManyVips,
    NotAnAddress,
    UnknownColour,
}

pub fn vip_list(conn: &Connection) -> Result<Vec<Vip>, DbError> {
    let mut statement = conn.prepare("SELECT address, added_at FROM vip ORDER BY address ASC")?;

    let rows = statement
        .query_map([], |row| {
            Ok(Vip {
                address: row.get(0)?,
                added_at: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows)
}

pub fn vip_add(
    tx: &Transaction<'_>,
    address: &str,
    now: i64,
) -> Result<Result<String, Refused>, DbError> {
    let address = normalise(address);

    if address.is_empty() || !address.contains('@') {
        return Ok(Err(Refused::NotAnAddress));
    }

    let already: i64 = tx.query_row(
        "SELECT COUNT(*) FROM vip WHERE address = ?1",
        params![address],
        |row| row.get(0),
    )?;

    // Re-adding someone already on the list must not fail on the cap, or the hundredth VIP
    // becomes impossible to re-add after being removed and restored.
    if already == 0 {
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM vip", [], |row| row.get(0))?;

        if count as usize >= MAX_VIPS {
            return Ok(Err(Refused::TooManyVips));
        }
    }

    tx.execute(
        "INSERT INTO vip (address, added_at) VALUES (?1, ?2)
         ON CONFLICT(address) DO NOTHING",
        params![address, now],
    )?;

    Ok(Ok(address))
}

pub fn vip_remove(tx: &Transaction<'_>, address: &str) -> Result<(), DbError> {
    tx.execute(
        "DELETE FROM vip WHERE address = ?1",
        params![normalise(address)],
    )?;
    Ok(())
}

/// Whether any address in a `From` header is a VIP.
///
/// Takes the whole header rather than a parsed address because that is what the message row
/// holds, and because a header can carry more than one address.
pub fn is_vip(conn: &Connection, from_header: &str) -> Result<bool, DbError> {
    for candidate in from_header.split(',') {
        let address = normalise(candidate);
        if address.is_empty() {
            continue;
        }

        let found: i64 = conn.query_row(
            "SELECT COUNT(*) FROM vip WHERE address = ?1",
            params![address],
            |row| row.get(0),
        )?;

        if found > 0 {
            return Ok(true);
        }
    }

    Ok(false)
}

/* ------------------------------------------------------------------------- blocked senders */

pub fn block_sender(tx: &Transaction<'_>, address: &str, now: i64) -> Result<(), DbError> {
    let address = normalise(address);
    if address.is_empty() {
        return Ok(());
    }

    tx.execute(
        "INSERT INTO blocked_sender (address, blocked_at) VALUES (?1, ?2)
         ON CONFLICT(address) DO NOTHING",
        params![address, now],
    )?;

    // Blocking is retroactive, as it is in Mail: the reason someone blocks a sender is usually
    // the mail already sitting in the Inbox, and a block that only applies to future mail
    // leaves the user to clear the rest out by hand.
    //
    // `junk_by_user` rather than a bare `is_junk`, so the classifier can tell "the user said
    // so" from "the filter thought so" — see junk.rs.
    //
    // The sender's address, exactly, as `mark_blocked_arrivals` compares it. This was a substring
    // of `from_all`, so blocking ann@company.com also junked everything from joann@company.com,
    // and anything whose sender's *name* happened to contain the address (M17, 2026-11-14 audit).
    // A `_` in the address was a LIKE wildcard too.
    tx.execute(
        "UPDATE message SET is_junk = 1, junk_by_user = 1
          WHERE LOWER(TRIM(from_addr)) = ?1",
        params![address],
    )?;

    Ok(())
}

/// Marks the arrivals from a blocked sender as junk, and says how many.
///
/// The forward half of blocking, which did not exist: `block_sender` marked the mail already
/// here, and nothing looked at the list again, so the next message from that sender arrived in
/// the Inbox and raised a toast (H10, 2026-11-14 audit). Called for Inbox arrivals, before the
/// rules and the classifier.
///
/// Matched exactly on the sender's address, as the list is stored, and as the retroactive half
/// in `block_sender` now matches too — `notoffers@pest.example` is not `offers@pest.example`.
pub fn mark_blocked_arrivals(tx: &Transaction<'_>, ids: &[i64]) -> Result<usize, DbError> {
    if ids.is_empty() {
        return Ok(0);
    }

    // In pieces past SQLite's parameter limit (M7). See `db::write::CHUNK`.
    if ids.len() > crate::db::write::CHUNK {
        let mut marked = 0;
        for chunk in ids.chunks(crate::db::write::CHUNK) {
            marked += mark_blocked_arrivals(tx, chunk)?;
        }
        return Ok(marked);
    }

    let list = (1..=ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "UPDATE message SET is_junk = 1, junk_by_user = 1
          WHERE id IN ({list})
            AND LOWER(TRIM(from_addr)) IN (SELECT address FROM blocked_sender)"
    );

    let bound: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
    Ok(tx.execute(&sql, bound.as_slice())?)
}

pub fn unblock_sender(tx: &Transaction<'_>, address: &str) -> Result<(), DbError> {
    tx.execute(
        "DELETE FROM blocked_sender WHERE address = ?1",
        params![normalise(address)],
    )?;
    Ok(())
}

pub fn blocked_list(conn: &Connection) -> Result<Vec<String>, DbError> {
    let mut statement = conn.prepare("SELECT address FROM blocked_sender ORDER BY address ASC")?;

    let rows = statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;

    Ok(rows)
}

pub fn is_blocked(conn: &Connection, from_header: &str) -> Result<bool, DbError> {
    for candidate in from_header.split(',') {
        let address = normalise(candidate);
        if address.is_empty() {
            continue;
        }

        let found: i64 = conn.query_row(
            "SELECT COUNT(*) FROM blocked_sender WHERE address = ?1",
            params![address],
            |row| row.get(0),
        )?;

        if found > 0 {
            return Ok(true);
        }
    }

    Ok(false)
}

/* ---------------------------------------------------------------------------- flag names */

/// The default label for each colour, used when the user has not renamed it.
pub fn default_flag_name(colour: &str) -> &'static str {
    match colour {
        "red" => "Red",
        "orange" => "Orange",
        "yellow" => "Yellow",
        "green" => "Green",
        "blue" => "Blue",
        "purple" => "Purple",
        "gray" => "Gray",
        _ => "Flag",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct FlagName {
    pub color: String,
    pub name: String,
}

pub fn flag_names(conn: &Connection) -> Result<Vec<FlagName>, DbError> {
    let mut statement = conn.prepare("SELECT color, name FROM flag_name")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let custom: std::collections::HashMap<String, String> = rows.into_iter().collect();

    // Driven by the fixed colour list rather than by the table, so the seven always come back
    // in the same order whether or not any of them has been renamed.
    Ok(super::engine::FLAG_COLOURS
        .iter()
        .map(|colour| FlagName {
            color: (*colour).to_string(),
            name: custom
                .get(*colour)
                .cloned()
                .unwrap_or_else(|| default_flag_name(colour).to_string()),
        })
        .collect())
}

pub fn rename_flag(
    tx: &Transaction<'_>,
    colour: &str,
    name: &str,
) -> Result<Result<(), Refused>, DbError> {
    if !super::engine::is_flag_colour(colour) {
        return Ok(Err(Refused::UnknownColour));
    }

    let name = name.trim();

    // An empty name means "back to the default", which is the only way to undo a rename
    // without a separate control for it.
    if name.is_empty() {
        tx.execute("DELETE FROM flag_name WHERE color = ?1", params![colour])?;
        return Ok(Ok(()));
    }

    tx.execute(
        "INSERT INTO flag_name (color, name) VALUES (?1, ?2)
         ON CONFLICT(color) DO UPDATE SET name = excluded.name",
        params![colour, name],
    )?;

    Ok(Ok(()))
}

/* ------------------------------------------------------------------- Remind Me / Follow Up */

/// Hides a message from its list until `until`.
///
/// The message stays in its mailbox rather than moving to a holding folder. Moving it would
/// sync the move to every other client the user has, and a message that disappears from the
/// server's copy of the Inbox is one they cannot find on their phone.
pub fn snooze(tx: &Transaction<'_>, ids: &[i64], until: i64) -> Result<usize, DbError> {
    let mut changed = 0;

    for &id in ids {
        changed += tx.execute(
            "UPDATE message SET snooze_until = ?2 WHERE id = ?1",
            params![id, until],
        )?;
    }

    Ok(changed)
}

pub fn unsnooze(tx: &Transaction<'_>, ids: &[i64]) -> Result<usize, DbError> {
    let mut changed = 0;

    for &id in ids {
        changed += tx.execute(
            "UPDATE message SET snooze_until = NULL WHERE id = ?1",
            params![id],
        )?;
    }

    Ok(changed)
}

/// Returns the messages whose snooze has come due, and clears it.
///
/// Returning the ids as well as clearing them is what lets the caller notify about exactly the
/// messages that just reappeared; a second query would race the next tick and either miss some
/// or repeat them.
pub fn wake_due(tx: &Transaction<'_>, now: i64) -> Result<Vec<i64>, DbError> {
    let due = {
        let mut statement = tx.prepare(
            "SELECT id FROM message WHERE snooze_until IS NOT NULL AND snooze_until <= ?1",
        )?;

        let rows = statement
            .query_map(params![now], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;

        rows
    };

    if !due.is_empty() {
        tx.execute(
            "UPDATE message SET snooze_until = NULL
              WHERE snooze_until IS NOT NULL AND snooze_until <= ?1",
            params![now],
        )?;
    }

    Ok(due)
}

/// Mutes a thread: replies still arrive and still sync, but raise no notification.
///
/// Deliberately not a delete and not a filter. A muted thread the user later needs is still
/// there, which is the whole difference between muting and unsubscribing.
pub fn mute_thread(tx: &Transaction<'_>, thread_id: i64, muted: bool) -> Result<(), DbError> {
    tx.execute(
        "UPDATE thread SET muted = ?2 WHERE id = ?1",
        params![thread_id, i64::from(muted)],
    )?;
    Ok(())
}

/// How long a sent message waits for a reply before it counts as needing follow-up.
pub const FOLLOW_UP_AFTER: i64 = 3 * 24 * 60 * 60;

/// Marks sent messages that asked a question and have had no reply.
///
/// The signal is deliberately conservative — a question mark, no later inbound message in the
/// thread, and enough time elapsed. A Follow Up list that fills with everything the user has
/// ever sent is one they stop looking at, and a nag that is wrong is worse than no nag at all.
pub fn detect_follow_ups(tx: &Transaction<'_>, now: i64) -> Result<usize, DbError> {
    let cutoff = now - FOLLOW_UP_AFTER;

    let changed = tx.execute(
        "UPDATE message
            SET follow_up_at = ?1
          WHERE follow_up_at IS NULL
            AND date_sent <= ?2
            AND mailbox_id IN (SELECT id FROM mailbox WHERE role = 'sent')
            AND (COALESCE(subject, '') LIKE '%?%' OR COALESCE(body_text, '') LIKE '%?%')
            AND thread_id IS NOT NULL
            AND NOT EXISTS (
              SELECT 1 FROM message reply
               WHERE reply.thread_id = message.thread_id
                 AND reply.date_sent > message.date_sent
                 AND reply.mailbox_id NOT IN (SELECT id FROM mailbox WHERE role = 'sent')
            )",
        params![now, cutoff],
    )?;

    // A reply that arrived after the mark was set clears it. Without this the badge stays on a
    // conversation that has been answered, which teaches the user to ignore the badge.
    tx.execute(
        "UPDATE message
            SET follow_up_at = NULL
          WHERE follow_up_at IS NOT NULL
            AND EXISTS (
              SELECT 1 FROM message reply
               WHERE reply.thread_id = message.thread_id
                 AND reply.date_sent > message.date_sent
                 AND reply.mailbox_id NOT IN (SELECT id FROM mailbox WHERE role = 'sent')
            )",
        [],
    )?;

    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocking_a_sender_junks_their_mail_and_no_one_elses() {
        // M17. Blocking ann@company.com used to junk joann@company.com's mail as well.
        let mut conn = Connection::open_in_memory().expect("open");
        crate::db::unicode_lower(&conn).expect("lower");
        crate::db::migrate::run(&mut conn).expect("migrate");

        conn.execute_batch(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'T', 'me@t.test', 'other', 'password', 'halcyon:me');
             INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (1, 1, 'INBOX', 'Inbox', 'inbox');",
        )
        .expect("seed");

        for (id, name, address) in [
            (1, "Ann", "ann@company.com"),
            (2, "Jo Ann", "joann@company.com"),
            (3, "Ann", " ANN@Company.com "),
            // Named after the address without being sent from it.
            (4, "ann@company.com fan club", "fans@example.test"),
            // A `_` would have been a LIKE wildcard for this one.
            (5, "Aan", "aan@company.com"),
        ] {
            conn.execute(
                "INSERT INTO message (
                     id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
                     from_name, from_addr, from_all, to_all, body_text, has_attachment,
                     flag_seen, flag_flagged, is_junk
                 ) VALUES (?1, 1, 1, ?1, 'S', 0, 0, 10, ?2, ?3, ?2 || ' ' || ?3, '', '', 0,
                           0, 0, 0)",
                params![id, name, address],
            )
            .expect("message");
        }

        let tx = conn.transaction().expect("tx");
        block_sender(&tx, "Ann@Company.com", 0).expect("block");
        block_sender(&tx, "a_n@company.com", 0).expect("block");

        let mut statement = tx
            .prepare("SELECT id, is_junk FROM message ORDER BY id")
            .expect("prepare");
        let junk: Vec<(i64, i64)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("rows");

        assert_eq!(junk, vec![(1, 1), (2, 0), (3, 1), (4, 0), (5, 0)]);
    }
}
