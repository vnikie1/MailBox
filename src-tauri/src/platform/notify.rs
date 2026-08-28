//! New-mail notifications. docs/06 Phase 10.
//!
//! ## What decides whether anything is shown
//!
//! Four gates, in order, and every one of them exists because the alternative is an app people
//! turn notifications off for entirely:
//!
//! 1. **Notifications are enabled**, per account. Someone with a work account and a newsletter
//!    account does not want to hear about both.
//! 2. **VIP-only, if set.** The setting docs/06 asks for, and the one that makes a busy mailbox
//!    bearable.
//! 3. **Not junk.** Being notified about spam is worse than not being notified at all.
//! 4. **Not muted, not snoozed.** Both are the user saying "not now" in as many words, and a
//!    notification would be overruling them.
//!
//! ## Why it summarises rather than showing one per message
//!
//! A sync that arrives after a night away can bring forty messages. Forty toasts is not
//! information, it is a denial of service against the person's own desktop — and Windows will
//! collapse them into the Action Center anyway, where they are read as a wall of noise. Above a
//! threshold this shows one notification that says how many.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use ts_rs::TS;

use crate::db::{Db, DbError};

/// Above this many at once, one summary instead.
const SUMMARISE_ABOVE: usize = 3;

/// What the user has chosen. Stored per account in `setting`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct NotifyPrefs {
    pub enabled: bool,
    /// Only mail from a VIP raises a notification.
    pub vip_only: bool,
    /// A sound with the notification. Off by default, per docs/06.
    pub sound: bool,
}

impl Default for NotifyPrefs {
    fn default() -> Self {
        // On, everything, silent. A mail client that told you nothing until you found the
        // setting would be a worse default than one that is briefly too chatty — but a *sound*
        // by default is the thing people actually resent, so that one starts off.
        Self {
            enabled: true,
            vip_only: false,
            sound: false,
        }
    }
}

fn key(account_id: i64, name: &str) -> String {
    format!("notify.{account_id}.{name}")
}

pub fn prefs_for(conn: &rusqlite::Connection, account_id: i64) -> Result<NotifyPrefs, DbError> {
    let read = |name: &str, fallback: bool| -> bool {
        conn.query_row(
            "SELECT value FROM setting WHERE key = ?1",
            rusqlite::params![key(account_id, name)],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .map_or(fallback, |value| {
            value == "1" || value.eq_ignore_ascii_case("true")
        })
    };

    let default = NotifyPrefs::default();

    Ok(NotifyPrefs {
        enabled: read("enabled", default.enabled),
        vip_only: read("vipOnly", default.vip_only),
        sound: read("sound", default.sound),
    })
}

pub fn set_prefs(
    tx: &rusqlite::Transaction<'_>,
    account_id: i64,
    prefs: NotifyPrefs,
) -> Result<(), DbError> {
    for (name, value) in [
        ("enabled", prefs.enabled),
        ("vipOnly", prefs.vip_only),
        ("sound", prefs.sound),
    ] {
        tx.execute(
            "INSERT INTO setting (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key(account_id, name), if value { "1" } else { "0" }],
        )?;
    }

    Ok(())
}

/// One message worth telling the user about.
#[derive(Debug, Clone)]
pub struct Arrival {
    pub id: i64,
    pub from: String,
    pub subject: String,
}

/// Which of these arrivals should raise a notification.
///
/// Pure, and separated from the showing so the *policy* can be tested. Whether a toast appears
/// is a decision with four inputs and several ways to get it wrong; whether Windows draws it is
/// not something a test can see.
pub fn worth_showing(
    conn: &rusqlite::Connection,
    account_id: i64,
    ids: &[i64],
) -> Result<Vec<Arrival>, DbError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let prefs = prefs_for(conn, account_id)?;
    if !prefs.enabled {
        return Ok(Vec::new());
    }

    let list = (0..ids.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");

    // Junk, muted threads and snoozed messages are excluded in the query rather than after it.
    // Each is the user having already said "not this" — about the sender, the conversation, or
    // the timing — and a notification would be overruling them.
    let sql = format!(
        "SELECT message.id, COALESCE(message.from_all, ''), COALESCE(message.subject, '')
           FROM message
           LEFT JOIN thread ON thread.id = message.thread_id
          WHERE message.id IN ({list})
            AND message.is_junk = 0
            AND message.flag_seen = 0
            AND message.snooze_until IS NULL
            AND COALESCE(thread.muted, 0) = 0
          ORDER BY message.date_received DESC"
    );

    let params: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

    let mut statement = conn.prepare(&sql)?;
    let rows = statement
        .query_map(params.as_slice(), |row| {
            Ok(Arrival {
                id: row.get(0)?,
                from: row.get(1)?,
                subject: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    if !prefs.vip_only {
        return Ok(rows);
    }

    let mut kept = Vec::new();
    for arrival in rows {
        if crate::rules::vip::is_vip(conn, &arrival.from)? {
            kept.push(arrival);
        }
    }

    Ok(kept)
}

/// Shows what `worth_showing` selected.
pub async fn announce(app: &AppHandle, db: &Db, account_id: i64, ids: Vec<i64>) {
    let Ok(arrivals) = db
        .read(move |conn| worth_showing(conn, account_id, &ids))
        .await
    else {
        return;
    };

    if arrivals.is_empty() {
        return;
    }

    let shown = if arrivals.len() > SUMMARISE_ABOVE {
        // One summary. Forty toasts is not information; it is a denial of service against the
        // user's own desktop.
        super::toast::show_summary(app, arrivals.len())
    } else {
        let mut last = Ok(());

        for arrival in &arrivals {
            last = super::toast::show_message(
                app,
                arrival.id,
                &display_name(&arrival.from),
                if arrival.subject.trim().is_empty() {
                    "(no subject)"
                } else {
                    &arrival.subject
                },
            );
        }

        last
    };

    if let Err(error) = shown {
        // A notification that could not be shown is not worth failing a sync over. Two of the
        // three reasons it fails are not errors at all: the user turned notifications off at the
        // OS level, or this is a dev build whose AUMID Windows has never been told about. See
        // the note at the top of toast.rs.
        tracing::debug!(%error, "could not show a notification");
    }
}

/// The readable half of a `From` header.
///
/// The address is not what someone wants to read on a toast, and a header like
/// `"Ada Lovelace" <ada@example.test>` shown raw is mostly punctuation.
fn display_name(from: &str) -> String {
    let first = from.split(',').next().unwrap_or(from).trim();

    let name = match (first.find('<'), first.rfind('>')) {
        (Some(open), Some(close)) if close > open => first[..open].trim(),
        _ => first,
    };

    let cleaned = name.trim_matches('"').trim();

    if cleaned.is_empty() {
        first.trim_matches(['<', '>']).to_string()
    } else {
        cleaned.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn store() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open");
        crate::db::migrate::run(&mut conn).expect("migrate");

        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'T', 'me@t.test', 'other', 'password', 'halcyon:me')",
            [],
        )
        .expect("account");

        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (1, 1, 'INBOX', 'Inbox', 'inbox')",
            [],
        )
        .expect("mailbox");

        conn.execute(
            "INSERT INTO thread (id, account_id, subject_base, last_date, message_count, muted)
             VALUES (1, 1, 'S', 0, 1, 0)",
            [],
        )
        .expect("thread");

        conn
    }

    #[allow(clippy::too_many_arguments)]
    fn add(conn: &Connection, id: i64, from: &str, junk: bool, snooze: Option<i64>, seen: bool) {
        conn.execute(
            "INSERT INTO message (
                 id, account_id, mailbox_id, thread_id, uid, subject, date_sent, date_received,
                 size, from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged,
                 is_junk, snooze_until
             ) VALUES (?1, 1, 1, 1, ?1, 'Subject', 0, 0, 10, ?2, '', '', 0, ?5, 0, ?3, ?4)",
            rusqlite::params![id, from, i64::from(junk), snooze, i64::from(seen)],
        )
        .expect("message");
    }

    #[test]
    fn an_ordinary_arrival_is_announced() {
        let conn = store();
        add(&conn, 1, "ada@example.test", false, None, false);

        assert_eq!(worth_showing(&conn, 1, &[1]).expect("check").len(), 1);
    }

    #[test]
    fn junk_is_never_announced() {
        // Being notified about spam is worse than not being notified at all.
        let conn = store();
        add(&conn, 1, "spam@example.test", true, None, false);

        assert!(worth_showing(&conn, 1, &[1]).expect("check").is_empty());
    }

    #[test]
    fn a_snoozed_message_is_not_announced() {
        // The user has said "not now" in as many words.
        let conn = store();
        add(
            &conn,
            1,
            "ada@example.test",
            false,
            Some(i64::MAX / 2),
            false,
        );

        assert!(worth_showing(&conn, 1, &[1]).expect("check").is_empty());
    }

    #[test]
    fn a_muted_thread_is_not_announced() {
        let conn = store();
        add(&conn, 1, "ada@example.test", false, None, false);
        conn.execute("UPDATE thread SET muted = 1 WHERE id = 1", [])
            .expect("mute");

        assert!(worth_showing(&conn, 1, &[1]).expect("check").is_empty());
    }

    #[test]
    fn a_message_already_read_is_not_announced() {
        // Read elsewhere between arriving and this running — on a phone, most likely. A toast
        // for mail the user has already dealt with is the most annoying kind.
        let conn = store();
        add(&conn, 1, "ada@example.test", false, None, true);

        assert!(worth_showing(&conn, 1, &[1]).expect("check").is_empty());
    }

    #[test]
    fn notifications_can_be_turned_off_per_account() {
        let mut conn = store();
        add(&conn, 1, "ada@example.test", false, None, false);

        let tx = conn.transaction().expect("tx");
        set_prefs(
            &tx,
            1,
            NotifyPrefs {
                enabled: false,
                ..NotifyPrefs::default()
            },
        )
        .expect("set");
        tx.commit().expect("commit");

        assert!(worth_showing(&conn, 1, &[1]).expect("check").is_empty());
    }

    #[test]
    fn vip_only_keeps_only_vips() {
        let mut conn = store();
        add(&conn, 1, "ada@example.test", false, None, false);
        add(&conn, 2, "stranger@example.test", false, None, false);

        conn.execute(
            "INSERT INTO vip (address, added_at) VALUES ('ada@example.test', 0)",
            [],
        )
        .expect("vip");

        let tx = conn.transaction().expect("tx");
        set_prefs(
            &tx,
            1,
            NotifyPrefs {
                vip_only: true,
                ..NotifyPrefs::default()
            },
        )
        .expect("set");
        tx.commit().expect("commit");

        let kept = worth_showing(&conn, 1, &[1, 2]).expect("check");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, 1);
    }

    #[test]
    fn the_default_is_on_and_silent() {
        // A sound by default is the thing people resent; silence is not.
        let prefs = NotifyPrefs::default();
        assert!(prefs.enabled);
        assert!(!prefs.sound);
        assert!(!prefs.vip_only);
    }

    #[test]
    fn a_from_header_becomes_a_readable_name() {
        assert_eq!(
            display_name("\"Ada Lovelace\" <ada@example.test>"),
            "Ada Lovelace"
        );
        assert_eq!(
            display_name("Ada Lovelace <ada@example.test>"),
            "Ada Lovelace"
        );
        assert_eq!(display_name("ada@example.test"), "ada@example.test");
        // Only the first, when a header carries several.
        assert_eq!(display_name("Ada <a@x.test>, Grace <g@x.test>"), "Ada");
    }

    #[test]
    fn an_address_only_header_shows_the_address_rather_than_nothing() {
        assert_eq!(display_name("<ada@example.test>"), "ada@example.test");
    }
}
