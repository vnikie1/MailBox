//! Smart mailboxes, rules, flags, VIPs, junk, snooze and undo. docs/03 §4, docs/06 Phase 8.
//!
//! Every mutation here goes through one transaction that does three things together: capture
//! the undo state, make the change, and queue whatever the server needs to be told. They are
//! one unit deliberately — a change the user can see but cannot undo, or one that never
//! reaches the server, is worse than one that failed outright and said so.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use ts_rs::TS;

use crate::db::{Db, DbError};
use crate::rules::engine::{self, Action, Rule, RunReport, SmartMailbox};
use crate::rules::junk::{self, Classifier};
use crate::rules::predicate::Predicate;
use crate::rules::vip::{self, FlagName, Refused, Vip};
use crate::sync::ops;
use crate::undo::{self, Available, Field, Stack};

use super::mail::AppError;

type Response<T> = Result<T, AppError>;

impl From<Refused> for AppError {
    fn from(refused: Refused) -> Self {
        // These are the one class of failure the user can do something about, so unlike a
        // database error they say exactly what went wrong.
        let (code, message) = match refused {
            Refused::TooManyVips => (
                "tooManyVips",
                format!("You can have at most {} VIPs.", vip::MAX_VIPS),
            ),
            Refused::NotAnAddress => (
                "notAnAddress",
                "That does not look like an email address.".to_string(),
            ),
            Refused::UnknownColour => (
                "unknownColour",
                "That is not one of the seven flag colours.".to_string(),
            ),
        };

        Self {
            code: code.into(),
            message,
        }
    }
}

/// Seconds since the epoch.
///
/// Taken once per command rather than per row: a batch whose rows carry timestamps a
/// millisecond apart sorts in an order nobody intended.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// Tells the UI something changed, so it can invalidate rather than poll (standing rule 14).
fn announce(app: &AppHandle, what: &str) {
    let _ = app.emit(what, ());
}

/* ---------------------------------------------------------------------- smart mailboxes */

#[tauri::command]
pub async fn smart_list(db: State<'_, Db>) -> Response<Vec<SmartMailbox>> {
    Ok(db.read(engine::smart_list).await?)
}

#[tauri::command]
pub async fn smart_save(
    app: AppHandle,
    db: State<'_, Db>,
    id: Option<i64>,
    name: String,
    icon: Option<String>,
    predicate: Predicate,
) -> Response<i64> {
    let saved = db
        .write(move |tx| engine::smart_save(tx, id, &name, icon.as_deref(), &predicate))
        .await?;

    announce(&app, "smart:changed");
    Ok(saved)
}

#[tauri::command]
pub async fn smart_delete(app: AppHandle, db: State<'_, Db>, id: i64) -> Response<()> {
    db.write(move |tx| engine::smart_delete(tx, id)).await?;
    announce(&app, "smart:changed");
    Ok(())
}

/// The rows a smart mailbox matches.
///
/// The predicate is compiled to SQL rather than evaluated over a full table scan in Rust: a
/// smart mailbox over 50,000 messages is opened from the sidebar and has to draw immediately.
#[tauri::command]
pub async fn smart_messages(
    db: State<'_, Db>,
    predicate: Predicate,
    limit: u32,
    offset: u32,
) -> Response<Vec<crate::db::model::MessageRow>> {
    Ok(db
        .read(move |conn| crate::db::query::messages_matching(conn, &predicate, limit, offset))
        .await?)
}

/* ------------------------------------------------------------------------------- rules */

#[tauri::command]
pub async fn rules_list(db: State<'_, Db>) -> Response<Vec<Rule>> {
    Ok(db.read(engine::rules_list).await?)
}

#[tauri::command]
pub async fn rule_save(
    app: AppHandle,
    db: State<'_, Db>,
    id: Option<i64>,
    name: String,
    enabled: bool,
    predicate: Predicate,
    actions: Vec<Action>,
) -> Response<i64> {
    let saved = db
        .write(move |tx| engine::rule_save(tx, id, &name, enabled, &predicate, &actions))
        .await?;

    announce(&app, "rules:changed");
    Ok(saved)
}

#[tauri::command]
pub async fn rule_delete(app: AppHandle, db: State<'_, Db>, id: i64) -> Response<()> {
    db.write(move |tx| engine::rule_delete(tx, id)).await?;
    announce(&app, "rules:changed");
    Ok(())
}

/// Runs the rules over a selection. Alt+Ctrl+L in Mail. docs/01 §8.
#[tauri::command]
pub async fn rules_run(
    app: AppHandle,
    db: State<'_, Db>,
    stack: State<'_, Arc<Stack>>,
    ids: Vec<i64>,
) -> Response<RunReport> {
    let rules = db.read(engine::rules_list).await?;
    let captured = ids.clone();

    let (report, step) = db
        .write(move |tx| {
            // Captured before the run, over every field a rule can touch. Running rules by hand
            // over a big selection is exactly when someone realises the rule was wrong.
            let step = undo::capture(
                tx,
                "Apply Rules",
                &captured,
                &[Field::Mailbox, Field::Seen, Field::Flagged, Field::Junk],
            )?;

            let report = engine::run_over(tx, &captured, &rules)?;
            Ok((report, step))
        })
        .await?;

    stack.record(step);
    announce(&app, "mailbox:changed");
    Ok(report)
}

/* -------------------------------------------------------------------------------- flags */

#[tauri::command]
pub async fn flag_names(db: State<'_, Db>) -> Response<Vec<FlagName>> {
    Ok(db.read(vip::flag_names).await?)
}

#[tauri::command]
pub async fn flag_rename(
    app: AppHandle,
    db: State<'_, Db>,
    color: String,
    name: String,
) -> Response<()> {
    db.write(move |tx| vip::rename_flag(tx, &color, &name))
        .await??;

    announce(&app, "flags:changed");
    Ok(())
}

/// Sets or clears a colour flag on a selection.
///
/// `None` clears the flag. The IMAP side rides along as a plain `\Flagged` change, because
/// colour is a local concept: no server stores it, and inventing a keyword for it would put
/// `$MailFlagBitN` in every other client the user owns.
#[tauri::command]
pub async fn flag_set(
    app: AppHandle,
    db: State<'_, Db>,
    stack: State<'_, Arc<Stack>>,
    ids: Vec<i64>,
    color: Option<String>,
) -> Response<usize> {
    if let Some(colour) = &color {
        if !engine::is_flag_colour(colour) {
            return Err(Refused::UnknownColour.into());
        }
    }

    let affected = ids.clone();
    let label = match &color {
        Some(colour) => format!("Flag {}", vip::default_flag_name(colour)),
        None => "Clear Flag".to_string(),
    };

    let (changed, step) = db
        .write(move |tx| {
            let step = undo::capture(tx, label, &affected, &[Field::Flagged])?;

            for group in ops::locate(tx, &affected)? {
                ops::enqueue(
                    tx,
                    group.account_id,
                    &ops::Op::Flag {
                        mailbox: group.mailbox,
                        uids: group.uids,
                        seen: None,
                        flagged: Some(color.is_some()),
                    },
                )?;
            }

            let mut changed = 0;
            for &id in &affected {
                changed += tx.execute(
                    "UPDATE message SET flag_flagged = ?2, flag_color = ?3 WHERE id = ?1",
                    rusqlite::params![id, i64::from(color.is_some()), color],
                )?;
            }

            Ok((changed, step))
        })
        .await?;

    stack.record(step);
    announce(&app, "mailbox:changed");
    Ok(changed)
}

/* ---------------------------------------------------------------------------------- VIPs */

#[tauri::command]
pub async fn vips_list(db: State<'_, Db>) -> Response<Vec<Vip>> {
    Ok(db.read(vip::vip_list).await?)
}

#[tauri::command]
pub async fn vip_add(app: AppHandle, db: State<'_, Db>, address: String) -> Response<String> {
    let stamp = now();
    let added = db
        .write(move |tx| vip::vip_add(tx, &address, stamp))
        .await??;

    announce(&app, "vips:changed");
    Ok(added)
}

#[tauri::command]
pub async fn vip_remove(app: AppHandle, db: State<'_, Db>, address: String) -> Response<()> {
    db.write(move |tx| vip::vip_remove(tx, &address)).await?;
    announce(&app, "vips:changed");
    Ok(())
}

/* --------------------------------------------------------------------------------- junk */

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct JunkStatus {
    /// Whether the filter has enough labelled mail to answer at all.
    pub ready: bool,
    #[ts(type = "number")]
    pub clean_examples: i64,
    #[ts(type = "number")]
    pub junk_examples: i64,
    #[ts(type = "number")]
    pub needed: i64,
}

#[tauri::command]
pub async fn junk_status(db: State<'_, Db>) -> Response<JunkStatus> {
    let (clean, junk_count) = db.read(junk::corpus_size).await?;

    Ok(JunkStatus {
        ready: clean >= junk::MIN_CORPUS && junk_count >= junk::MIN_CORPUS,
        clean_examples: clean,
        junk_examples: junk_count,
        needed: junk::MIN_CORPUS,
    })
}

/// Marks a selection as junk or not junk, and trains on it.
///
/// The training half is the point: this is the only path by which the classifier learns
/// anything, because it is the only one carrying a label a human applied.
#[tauri::command]
pub async fn junk_mark(
    app: AppHandle,
    db: State<'_, Db>,
    stack: State<'_, Arc<Stack>>,
    ids: Vec<i64>,
    is_junk: bool,
) -> Response<usize> {
    let affected = ids.clone();
    let label = if is_junk {
        "Mark as Junk"
    } else {
        "Mark as Not Junk"
    };

    let (changed, step) = db
        .write(move |tx| {
            let step = undo::capture(tx, label, &affected, &[Field::Junk, Field::Mailbox])?;
            let mut changed = 0;

            for &id in &affected {
                let row = tx.query_row(
                    "SELECT COALESCE(from_all, ''), COALESCE(subject, ''),
                            COALESCE(body_text, ''), is_junk, junk_by_user
                       FROM message WHERE id = ?1",
                    rusqlite::params![id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)? != 0,
                            row.get::<_, i64>(4)? != 0,
                        ))
                    },
                );

                let Ok((from, subject, body, was_junk, was_by_user)) = row else {
                    continue;
                };

                // A judgement the user is reversing has to be taken back out of the corpus, or
                // their correction is only half applied and the filter keeps the belief that
                // caused the mistake.
                if was_by_user && was_junk != is_junk {
                    junk::untrain(tx, &from, &subject, &body, was_junk)?;
                }

                if !was_by_user || was_junk != is_junk {
                    junk::train(tx, &from, &subject, &body, is_junk)?;
                }

                changed += tx.execute(
                    "UPDATE message SET is_junk = ?2, junk_by_user = 1 WHERE id = ?1",
                    rusqlite::params![id, i64::from(is_junk)],
                )?;
            }

            Ok((changed, step))
        })
        .await?;

    stack.record(step);
    announce(&app, "mailbox:changed");
    announce(&app, "junk:changed");
    Ok(changed)
}

/// Scores unjudged mail in a mailbox and files what the filter is sure about.
///
/// Only ever touches messages the user has not judged: re-scoring something they already
/// decided on would overrule them, which is the fastest way to make someone stop trusting a
/// filter entirely.
#[tauri::command]
pub async fn junk_scan(app: AppHandle, db: State<'_, Db>, mailbox_id: i64) -> Response<usize> {
    let classifier = db.read(Classifier::load).await?;

    if !classifier.ready() {
        return Ok(0);
    }

    let filed = db
        .write(move |tx| {
            let candidates = {
                let mut statement = tx.prepare(
                    "SELECT id, COALESCE(from_all, ''), COALESCE(subject, ''),
                            COALESCE(body_text, '')
                       FROM message
                      WHERE mailbox_id = ?1 AND junk_by_user = 0 AND is_junk = 0",
                )?;

                let rows = statement
                    .query_map(rusqlite::params![mailbox_id], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;

                rows
            };

            let mut filed = 0;

            for (id, from, subject, body) in candidates {
                let verdict = classifier.score(&from, &subject, &body);

                let Some(score) = verdict.probability() else {
                    continue;
                };

                // The score is stored even when it is below the threshold. It is what lets
                // someone see *why* a message was filed, and what makes a later change of
                // threshold explicable rather than mysterious.
                tx.execute(
                    "UPDATE message SET junk_score = ?2, is_junk = ?3 WHERE id = ?1",
                    rusqlite::params![id, score, i64::from(verdict.is_junk())],
                )?;

                if verdict.is_junk() {
                    filed += 1;
                }
            }

            Ok::<_, DbError>(filed)
        })
        .await?;

    announce(&app, "mailbox:changed");
    Ok(filed)
}

/// Whether the filter only marks and never files. docs/06 Phase 8.
#[tauri::command]
pub async fn junk_training_mode(db: State<'_, Db>) -> Response<bool> {
    let stored: Option<String> = db
        .read(|conn| {
            Ok(conn
                .query_row(
                    "SELECT value FROM setting WHERE key = 'junk.trainingMode'",
                    [],
                    |row| row.get(0),
                )
                .ok())
        })
        .await?;

    Ok(stored.is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true")))
}

#[tauri::command]
pub async fn junk_set_training_mode(db: State<'_, Db>, enabled: bool) -> Response<()> {
    db.write(move |tx| {
        tx.execute(
            "INSERT INTO setting (key, value) VALUES ('junk.trainingMode', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![if enabled { "1" } else { "0" }],
        )?;
        Ok(())
    })
    .await?;

    Ok(())
}

/* ---------------------------------------------------------------------- blocked senders */

#[tauri::command]
pub async fn blocked_list(db: State<'_, Db>) -> Response<Vec<String>> {
    Ok(db.read(vip::blocked_list).await?)
}

#[tauri::command]
pub async fn block_sender(app: AppHandle, db: State<'_, Db>, address: String) -> Response<()> {
    let stamp = now();
    db.write(move |tx| vip::block_sender(tx, &address, stamp))
        .await?;

    announce(&app, "mailbox:changed");
    announce(&app, "blocked:changed");
    Ok(())
}

#[tauri::command]
pub async fn unblock_sender(app: AppHandle, db: State<'_, Db>, address: String) -> Response<()> {
    db.write(move |tx| vip::unblock_sender(tx, &address))
        .await?;
    announce(&app, "blocked:changed");
    Ok(())
}

/* -------------------------------------------------------------- Remind Me and muting */

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SnoozeRequest {
    #[ts(type = "number[]")]
    pub ids: Vec<i64>,
    /// Absolute, in seconds since the epoch. Computed in the UI against the *user's* clock and
    /// calendar, because "tomorrow morning" is a question about their timezone and their idea
    /// of morning, neither of which the core knows.
    #[ts(type = "number")]
    pub until: i64,
}

#[tauri::command]
pub async fn snooze(
    app: AppHandle,
    db: State<'_, Db>,
    stack: State<'_, Arc<Stack>>,
    request: SnoozeRequest,
) -> Response<usize> {
    let SnoozeRequest { ids, until } = request;

    let (changed, step) = db
        .write(move |tx| {
            let step = undo::capture(tx, "Remind Me", &ids, &[Field::Snooze])?;
            let changed = vip::snooze(tx, &ids, until)?;
            Ok((changed, step))
        })
        .await?;

    stack.record(step);
    announce(&app, "mailbox:changed");
    Ok(changed)
}

#[tauri::command]
pub async fn unsnooze(
    app: AppHandle,
    db: State<'_, Db>,
    stack: State<'_, Arc<Stack>>,
    ids: Vec<i64>,
) -> Response<usize> {
    let (changed, step) = db
        .write(move |tx| {
            let step = undo::capture(tx, "Cancel Reminder", &ids, &[Field::Snooze])?;
            let changed = vip::unsnooze(tx, &ids)?;
            Ok((changed, step))
        })
        .await?;

    stack.record(step);
    announce(&app, "mailbox:changed");
    Ok(changed)
}

#[tauri::command]
pub async fn mute_thread(
    app: AppHandle,
    db: State<'_, Db>,
    thread_id: i64,
    muted: bool,
) -> Response<()> {
    db.write(move |tx| vip::mute_thread(tx, thread_id, muted))
        .await?;

    announce(&app, "mailbox:changed");
    Ok(())
}

/// Refreshes the Follow Up marks. Cheap enough to run on every sync.
#[tauri::command]
pub async fn follow_ups_detect(app: AppHandle, db: State<'_, Db>) -> Response<usize> {
    let stamp = now();
    let marked = db
        .write(move |tx| vip::detect_follow_ups(tx, stamp))
        .await?;

    if marked > 0 {
        announce(&app, "mailbox:changed");
    }

    Ok(marked)
}

/* ------------------------------------------------------------------- notifications */

#[tauri::command]
pub async fn notify_prefs(
    db: State<'_, Db>,
    account_id: i64,
) -> Response<crate::platform::notify::NotifyPrefs> {
    Ok(db
        .read(move |conn| crate::platform::notify::prefs_for(conn, account_id))
        .await?)
}

#[tauri::command]
pub async fn notify_set_prefs(
    db: State<'_, Db>,
    account_id: i64,
    prefs: crate::platform::notify::NotifyPrefs,
) -> Response<()> {
    db.write(move |tx| crate::platform::notify::set_prefs(tx, account_id, prefs))
        .await?;
    Ok(())
}

/// Whether Halcyon starts with Windows.
///
/// Read from the OS rather than from a setting of our own, so the answer stays right when the
/// user removes it from Startup themselves — which they can, and a stored flag would then be a
/// lie the settings panel repeats back at them.
#[tauri::command]
pub async fn run_at_login(app: tauri::AppHandle) -> Response<bool> {
    use tauri_plugin_autostart::ManagerExt;

    Ok(app.autolaunch().is_enabled().unwrap_or(false))
}

#[tauri::command]
pub async fn set_run_at_login(app: tauri::AppHandle, enabled: bool) -> Response<()> {
    use tauri_plugin_autostart::ManagerExt;

    let result = if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };

    result.map_err(|error| AppError {
        code: "autostart".into(),
        message: format!("Windows would not change the startup setting: {error}"),
    })
}

/* --------------------------------------------------------------------------------- undo */

#[tauri::command]
pub async fn undo_available(stack: State<'_, Arc<Stack>>) -> Response<Available> {
    Ok(stack.available())
}

#[tauri::command]
pub async fn undo_perform(
    app: AppHandle,
    db: State<'_, Db>,
    stack: State<'_, Arc<Stack>>,
) -> Response<Option<String>> {
    // `db.write` moves its closure to the writer thread, so the closure has to own everything
    // it touches. An `Arc` clone is what carries the shared stack across that boundary.
    let stack = Arc::clone(&stack);
    let label = db.write(move |tx| undo::undo(&stack, tx)).await?;

    announce(&app, "mailbox:changed");
    Ok(label)
}

#[tauri::command]
pub async fn redo_perform(
    app: AppHandle,
    db: State<'_, Db>,
    stack: State<'_, Arc<Stack>>,
) -> Response<Option<String>> {
    let stack = Arc::clone(&stack);
    let label = db.write(move |tx| undo::redo(&stack, tx)).await?;

    announce(&app, "mailbox:changed");
    Ok(label)
}
