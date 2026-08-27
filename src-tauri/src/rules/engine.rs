//! Rules and Smart Mailboxes, built on the one predicate engine. docs/01 §8, docs/06 Phase 8.
//!
//! A Smart Mailbox is a saved predicate the list queries through. A Rule is the same predicate
//! plus actions, run when mail arrives and on demand. Neither has a matching implementation of
//! its own — see `predicate.rs` for why that matters.
//!
//! ## What is deliberately not here
//!
//! docs/01 §8 lists Mail's rule actions, and one of them is **run script**. It is not
//! implemented and should not be: a rule action that executes an arbitrary program, triggered
//! by mail *from anyone who knows the user's address*, is a remote code execution primitive
//! with a friendly editor in front of it. Mail can offer it because AppleScript runs inside a
//! sandbox the OS arbitrates; there is no equivalent here, and "the user configured it" is not
//! a defence when the trigger is attacker-controlled.
//!
//! **Play sound** is absent for a duller reason: it belongs with notifications in Phase 10, and
//! implementing it here would mean two sound paths.

use rusqlite::{params, Transaction};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::{Db, DbError};

use super::predicate::{Predicate, Subject};

/// What a rule does to a message it matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum Action {
    /// Move to a mailbox, by id.
    ///
    /// Annotated on the inner field rather than the variant: without it ts-rs emits `bigint`,
    /// and a mailbox id that arrives as a `bigint` will not compare equal to the `number` the
    /// rest of the UI holds.
    MoveTo(#[ts(type = "number")] i64),
    MarkRead,
    MarkUnread,
    Flag,
    Unflag,
    /// One of the seven flag colours. docs/01 §8.
    SetColour(String),
    MarkJunk,
    /// Move to Trash. Never a permanent delete — a rule that destroys mail outright, on a
    /// predicate the user wrote in thirty seconds, is not a feature anyone recovers from.
    Delete,
    /// Stop evaluating further rules for this message.
    StopEvaluating,
}

/// A stored rule.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub predicate: Predicate,
    pub actions: Vec<Action>,
    #[ts(type = "number")]
    pub sort_order: i64,
}

/// A stored smart mailbox.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SmartMailbox {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub icon: Option<String>,
    pub predicate: Predicate,
    #[ts(type = "number")]
    pub sort_order: i64,
}

/// The seven colours docs/01 §8 names, and nothing else.
///
/// Validated rather than trusted: the colour reaches a CSS custom property name, and an
/// unrecognised value would be a token that does not resolve — an invisible flag.
pub const FLAG_COLOURS: &[&str] = &["red", "orange", "yellow", "green", "blue", "purple", "gray"];

pub fn is_flag_colour(value: &str) -> bool {
    FLAG_COLOURS.contains(&value)
}

/* ------------------------------------------------------------------------ smart mailboxes */

pub fn smart_list(conn: &rusqlite::Connection) -> Result<Vec<SmartMailbox>, DbError> {
    let mut statement = conn.prepare(
        "SELECT id, name, icon, predicate_json, sort_order
           FROM smart_mailbox ORDER BY sort_order ASC, id ASC",
    )?;

    let rows = statement
        .query_map([], |row| {
            let json: String = row.get(3)?;

            Ok(SmartMailbox {
                id: row.get(0)?,
                name: row.get(1)?,
                icon: row.get(2)?,
                // A predicate that will not parse was written by another version. Falling back
                // to "matches nothing" keeps the sidebar working and makes the breakage
                // obvious — an empty smart mailbox — rather than taking the list down.
                predicate: serde_json::from_str(&json)
                    .unwrap_or_else(|_| Predicate::Any(Vec::new())),
                sort_order: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows)
}

pub fn smart_save(
    tx: &Transaction<'_>,
    id: Option<i64>,
    name: &str,
    icon: Option<&str>,
    predicate: &Predicate,
) -> Result<i64, DbError> {
    let json = serde_json::to_string(predicate).map_err(|error| DbError::Encode {
        what: "smart mailbox predicate",
        detail: error.to_string(),
    })?;

    match id {
        Some(id) => {
            tx.execute(
                "UPDATE smart_mailbox SET name = ?2, icon = ?3, predicate_json = ?4 WHERE id = ?1",
                params![id, name, icon, json],
            )?;
            Ok(id)
        }
        None => {
            tx.execute(
                "INSERT INTO smart_mailbox (name, icon, match_all, predicate_json, sort_order)
                 VALUES (?1, ?2, 1, ?3,
                         (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM smart_mailbox))",
                params![name, icon, json],
            )?;
            Ok(tx.last_insert_rowid())
        }
    }
}

pub fn smart_delete(tx: &Transaction<'_>, id: i64) -> Result<(), DbError> {
    tx.execute("DELETE FROM smart_mailbox WHERE id = ?1", params![id])?;
    Ok(())
}

/* -------------------------------------------------------------------------------- rules */

pub fn rules_list(conn: &rusqlite::Connection) -> Result<Vec<Rule>, DbError> {
    let mut statement = conn.prepare(
        "SELECT id, name, enabled, predicate_json, actions_json, sort_order
           FROM rule ORDER BY sort_order ASC, id ASC",
    )?;

    let rows = statement
        .query_map([], |row| {
            let predicate_json: String = row.get(3)?;
            let actions_json: String = row.get(4)?;

            Ok(Rule {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get::<_, i64>(2)? != 0,
                // A rule that will not parse matches nothing and does nothing. Silently doing
                // *something* on a predicate we could not read would be far worse.
                predicate: serde_json::from_str(&predicate_json)
                    .unwrap_or_else(|_| Predicate::Any(Vec::new())),
                actions: serde_json::from_str(&actions_json).unwrap_or_default(),
                sort_order: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows)
}

pub fn rule_save(
    tx: &Transaction<'_>,
    id: Option<i64>,
    name: &str,
    enabled: bool,
    predicate: &Predicate,
    actions: &[Action],
) -> Result<i64, DbError> {
    let predicate_json = serde_json::to_string(predicate).map_err(|error| DbError::Encode {
        what: "rule predicate",
        detail: error.to_string(),
    })?;
    let actions_json = serde_json::to_string(actions).map_err(|error| DbError::Encode {
        what: "rule actions",
        detail: error.to_string(),
    })?;

    match id {
        Some(id) => {
            tx.execute(
                "UPDATE rule
                    SET name = ?2, enabled = ?3, predicate_json = ?4, actions_json = ?5
                  WHERE id = ?1",
                params![id, name, i64::from(enabled), predicate_json, actions_json],
            )?;
            Ok(id)
        }
        None => {
            tx.execute(
                "INSERT INTO rule (name, enabled, match_all, predicate_json, actions_json, sort_order)
                 VALUES (?1, ?2, 1, ?3, ?4,
                         (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM rule))",
                params![name, i64::from(enabled), predicate_json, actions_json],
            )?;
            Ok(tx.last_insert_rowid())
        }
    }
}

pub fn rule_delete(tx: &Transaction<'_>, id: i64) -> Result<(), DbError> {
    tx.execute("DELETE FROM rule WHERE id = ?1", params![id])?;
    Ok(())
}

/// Reads the fields a rule needs about one stored message.
fn subject_row(tx: &Transaction<'_>, message_id: i64) -> Result<Option<OwnedSubject>, DbError> {
    let found = tx
        .query_row(
            "SELECT COALESCE(message.from_all, ''), COALESCE(message.to_all, ''),
                    COALESCE(message.cc_json, ''), COALESCE(message.subject, ''),
                    COALESCE(message.body_text, ''), COALESCE(mailbox.display_name, ''),
                    COALESCE(message.attachment_names, ''),
                    message.date_received, message.size,
                    message.has_attachment, message.flag_seen, message.flag_flagged,
                    message.is_junk
               FROM message
               JOIN mailbox ON mailbox.id = message.mailbox_id
              WHERE message.id = ?1",
            params![message_id],
            |row| {
                Ok(OwnedSubject {
                    from: row.get(0)?,
                    to: row.get(1)?,
                    cc: row.get(2)?,
                    subject: row.get(3)?,
                    body: row.get(4)?,
                    mailbox: row.get(5)?,
                    attachment_names: row.get(6)?,
                    date_received: row.get(7)?,
                    size: row.get(8)?,
                    has_attachment: row.get::<_, i64>(9)? != 0,
                    is_unread: row.get::<_, i64>(10)? == 0,
                    is_flagged: row.get::<_, i64>(11)? != 0,
                    is_junk: row.get::<_, i64>(12)? != 0,
                })
            },
        )
        .ok();

    Ok(found)
}

/// An owned `Subject`, because the borrowed one cannot outlive a row.
#[derive(Debug, Clone, Default)]
struct OwnedSubject {
    from: String,
    to: String,
    cc: String,
    subject: String,
    body: String,
    mailbox: String,
    attachment_names: String,
    date_received: i64,
    size: i64,
    has_attachment: bool,
    is_unread: bool,
    is_flagged: bool,
    is_junk: bool,
}

impl OwnedSubject {
    fn borrow(&self) -> Subject<'_> {
        Subject {
            from: &self.from,
            to: &self.to,
            cc: &self.cc,
            subject: &self.subject,
            body: &self.body,
            mailbox: &self.mailbox,
            attachment_names: &self.attachment_names,
            date_received: self.date_received,
            size: self.size,
            has_attachment: self.has_attachment,
            is_unread: self.is_unread,
            is_flagged: self.is_flagged,
            is_junk: self.is_junk,
        }
    }
}

/// Applies one action to one message.
fn apply(tx: &Transaction<'_>, message_id: i64, action: &Action) -> Result<(), DbError> {
    match action {
        Action::MoveTo(mailbox_id) => {
            crate::db::write::move_to(tx, &[message_id], *mailbox_id)?;
        }
        Action::MarkRead => {
            tx.execute(
                "UPDATE message SET flag_seen = 1 WHERE id = ?1",
                params![message_id],
            )?;
        }
        Action::MarkUnread => {
            tx.execute(
                "UPDATE message SET flag_seen = 0 WHERE id = ?1",
                params![message_id],
            )?;
        }
        Action::Flag => {
            tx.execute(
                "UPDATE message SET flag_flagged = 1 WHERE id = ?1",
                params![message_id],
            )?;
        }
        Action::Unflag => {
            tx.execute(
                "UPDATE message SET flag_flagged = 0, flag_color = NULL WHERE id = ?1",
                params![message_id],
            )?;
        }
        Action::SetColour(colour) => {
            // Validated rather than trusted: the value ends up naming a CSS custom property,
            // and an unrecognised one is a token that does not resolve — an invisible flag.
            if !is_flag_colour(colour) {
                return Ok(());
            }
            tx.execute(
                "UPDATE message SET flag_flagged = 1, flag_color = ?2 WHERE id = ?1",
                params![message_id, colour],
            )?;
        }
        Action::MarkJunk => {
            tx.execute(
                "UPDATE message SET is_junk = 1 WHERE id = ?1",
                params![message_id],
            )?;
        }
        Action::Delete => {
            // To Trash, never permanently. A rule that destroys mail outright on a predicate
            // written in thirty seconds is not something anyone recovers from.
            let trash: Option<i64> = tx
                .query_row(
                    "SELECT m.id FROM mailbox m
                       JOIN message msg ON msg.account_id = m.account_id
                      WHERE msg.id = ?1 AND m.role = 'trash'",
                    params![message_id],
                    |row| row.get(0),
                )
                .ok();

            if let Some(trash) = trash {
                crate::db::write::move_to(tx, &[message_id], trash)?;
            }
        }
        Action::StopEvaluating => {}
    }

    Ok(())
}

/// What running the rules did, for the UI and for the log.
#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    #[ts(type = "number")]
    pub examined: i64,
    #[ts(type = "number")]
    pub matched: i64,
    #[ts(type = "number")]
    pub actions_applied: i64,
}

/// Runs every enabled rule over a set of messages.
///
/// Used for both triggers docs/06 names: on arrival, with the ids the sync just wrote, and on
/// demand, with whatever the user has selected. One code path, so "run rules now" cannot behave
/// differently from the automatic pass — which is the difference people actually test against.
pub fn run_over(
    tx: &Transaction<'_>,
    message_ids: &[i64],
    rules: &[Rule],
) -> Result<RunReport, DbError> {
    let mut report = RunReport::default();

    for &message_id in message_ids {
        report.examined += 1;

        let Some(subject) = subject_row(tx, message_id)? else {
            continue;
        };

        let mut matched_any = false;

        for rule in rules.iter().filter(|rule| rule.enabled) {
            // Re-read after a rule that changed the message, so a second rule sees what the
            // first did. Rules that depend on each other in order are how people build them,
            // and evaluating them all against the original state would quietly break that.
            let current = if matched_any {
                match subject_row(tx, message_id)? {
                    Some(current) => current,
                    None => break,
                }
            } else {
                subject.clone()
            };

            if !rule.predicate.matches(&current.borrow()) {
                continue;
            }

            matched_any = true;

            for action in &rule.actions {
                apply(tx, message_id, action)?;
                report.actions_applied += 1;

                if matches!(action, Action::StopEvaluating) {
                    break;
                }
            }

            if rule
                .actions
                .iter()
                .any(|a| matches!(a, Action::StopEvaluating))
            {
                break;
            }
        }

        if matched_any {
            report.matched += 1;
        }
    }

    Ok(report)
}

/// Runs the rules over messages that have just arrived.
pub async fn run_on_arrival(db: &Db, message_ids: Vec<i64>) -> Result<RunReport, DbError> {
    if message_ids.is_empty() {
        return Ok(RunReport::default());
    }

    let rules = db.read(rules_list).await?;
    if rules.iter().all(|rule| !rule.enabled) {
        return Ok(RunReport::default());
    }

    db.write(move |tx| run_over(tx, &message_ids, &rules)).await
}
