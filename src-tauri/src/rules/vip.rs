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
    tx.execute(
        "UPDATE message SET is_junk = 1, junk_by_user = 1
          WHERE LOWER(from_all) LIKE '%' || ?1 || '%'",
        params![address],
    )?;

    Ok(())
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
