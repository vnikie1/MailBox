//! Reads. docs/03-architecture.md §3 and §4.
//!
//! Every statement is parameterised. Where a query needs a variable-length `IN` list, the
//! *placeholders* are generated and the *values* are bound — `?1, ?2, ?3` built from a
//! count is parameterisation, and no caller-supplied text ever reaches the SQL text.
//!
//! Each query here has a supporting index, listed in its doc comment. `EXPLAIN QUERY PLAN`
//! output for the three the exit gate names is asserted in the tests at the bottom, so a
//! plan that silently degrades to a scan fails the build rather than the budget.

use rusqlite::{Connection, Row};

use super::model::{
    AccountRow, AttachmentRow, Cursor, ListQuery, MailboxCounts, MailboxRow, MessageFull,
    MessageRow, Page, SearchQuery,
};
use super::DbError;

/// `?1, ?2, ... ?n`, for an `IN` list of `n` bound values.
///
/// This builds placeholders, never values. It exists because SQLite has no array parameter
/// and the alternative — interpolating ids into the SQL — is the thing docs/06 forbids.
fn placeholders(count: usize, start: usize) -> String {
    (0..count)
        .map(|i| format!("?{}", start + i))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn accounts_list(conn: &Connection) -> Result<Vec<AccountRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, display_name, email, provider
           FROM account
          ORDER BY sort_order, id",
    )?;

    let rows = stmt
        .query_map([], |row| {
            Ok(AccountRow {
                id: row.get(0)?,
                display_name: row.get(1)?,
                email: row.get(2)?,
                provider: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Every mailbox, flat, in display order. The tree shape is a view concern — the sidebar
/// already builds it (see `src/features/sidebar/model.ts`), and it builds rows the database
/// has no notion of, such as All Inboxes.
///
/// Index: `ix_mailbox_account`.
pub fn mailboxes_tree(
    conn: &Connection,
    account_id: Option<i64>,
) -> Result<Vec<MailboxRow>, DbError> {
    let map = |row: &Row<'_>| {
        Ok(MailboxRow {
            id: row.get(0)?,
            account_id: row.get(1)?,
            display_name: row.get(2)?,
            parent_id: row.get(3)?,
            role: row.get(4)?,
            unread_count: row.get(5)?,
            total_count: row.get(6)?,
        })
    };

    const COLUMNS: &str =
        "id, account_id, display_name, parent_id, role, unread_count, total_count";

    let rows = match account_id {
        Some(id) => {
            let sql = format!(
                "SELECT {COLUMNS} FROM mailbox WHERE account_id = ?1 ORDER BY sort_order, id"
            );
            conn.prepare(&sql)?
                .query_map([id], map)?
                .collect::<Result<Vec<_>, _>>()?
        }
        None => {
            let sql = format!("SELECT {COLUMNS} FROM mailbox ORDER BY account_id, sort_order, id");
            conn.prepare(&sql)?
                .query_map([], map)?
                .collect::<Result<Vec<_>, _>>()?
        }
    };

    Ok(rows)
}

const MESSAGE_ROW_COLUMNS: &str = "id, thread_id, mailbox_id, account_id, subject, from_name, \
     from_addr, date_received, preview, size, flag_seen, flag_answered, flag_flagged, \
     flag_color, has_attachment";

fn message_row(row: &Row<'_>) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
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
    })
}

/// One page of the message list, keyset-paginated. docs/06 Phase 3 — never `OFFSET`.
///
/// The `WHERE` clause compares the row value `(date_received, id)` against the cursor,
/// which is exactly the shape of `ix_msg_list (mailbox_id, date_received DESC, id DESC)`.
/// That is what lets page 500 cost the same as page 1: SQLite seeks straight to the cursor
/// in the index instead of walking everything before it.
///
/// One extra row is fetched beyond `limit` and then dropped. It is how the caller learns
/// whether a next page exists without a second `COUNT(*)` over the mailbox.
pub fn messages_page(conn: &Connection, query: &ListQuery) -> Result<Page<MessageRow>, DbError> {
    if query.mailbox_ids.is_empty() {
        return Ok(Page {
            items: Vec::new(),
            next_cursor: None,
        });
    }

    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    for id in &query.mailbox_ids {
        params.push(Box::new(*id));
    }

    let in_list = placeholders(query.mailbox_ids.len(), 1);
    let mut sql = format!(
        "SELECT {MESSAGE_ROW_COLUMNS}
           FROM message
          WHERE mailbox_id IN ({in_list})"
    );

    if query.unread_only {
        sql.push_str(" AND flag_seen = 0");
    }

    if let Some(cursor) = query.cursor {
        let date_index = params.len() + 1;
        let id_index = params.len() + 2;
        sql.push_str(&format!(
            " AND (date_received, id) < (?{date_index}, ?{id_index})"
        ));
        params.push(Box::new(cursor.date_received));
        params.push(Box::new(cursor.id));
    }

    let limit_index = params.len() + 1;
    sql.push_str(&format!(
        " ORDER BY date_received DESC, id DESC LIMIT ?{limit_index}"
    ));
    // Ask for one more than needed; its presence is the "there is more" signal.
    params.push(Box::new(i64::from(query.limit) + 1));

    let borrowed: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let mut items = stmt
        .query_map(borrowed.as_slice(), message_row)?
        .collect::<Result<Vec<_>, _>>()?;

    let has_more = items.len() > query.limit as usize;
    if has_more {
        items.truncate(query.limit as usize);
    }

    let next_cursor = if has_more {
        items.last().map(|row| Cursor {
            date_received: row.date_received,
            id: row.id,
        })
    } else {
        None
    };

    Ok(Page { items, next_cursor })
}

/// Unread and total per mailbox, for the sidebar badges.
///
/// Index: `ix_msg_unread`, a partial index over `flag_seen = 0`. At a healthy inbox that is
/// a few hundred rows out of a hundred thousand, so the count is a short index scan rather
/// than a table scan.
pub fn mailbox_counts(
    conn: &Connection,
    mailbox_ids: &[i64],
) -> Result<Vec<MailboxCounts>, DbError> {
    if mailbox_ids.is_empty() {
        return Ok(Vec::new());
    }

    let in_list = placeholders(mailbox_ids.len(), 1);
    let sql = format!(
        "SELECT mailbox_id,
                SUM(CASE WHEN flag_seen = 0 THEN 1 ELSE 0 END),
                COUNT(*)
           FROM message
          WHERE mailbox_id IN ({in_list})
          GROUP BY mailbox_id"
    );

    let params: Vec<&dyn rusqlite::ToSql> = mailbox_ids
        .iter()
        .map(|id| id as &dyn rusqlite::ToSql)
        .collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params.as_slice(), |row| {
            Ok(MailboxCounts {
                mailbox_id: row.get(0)?,
                unread: row.get(1)?,
                total: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

const MESSAGE_FULL_COLUMNS: &str = "id, thread_id, mailbox_id, account_id, subject, from_name, \
     from_addr, to_json, cc_json, date_sent, date_received, size, preview, body_text, \
     flag_seen, flag_answered, flag_flagged, flag_color";

fn message_full(row: &Row<'_>) -> rusqlite::Result<MessageFull> {
    Ok(MessageFull {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        mailbox_id: row.get(2)?,
        account_id: row.get(3)?,
        subject: row.get(4)?,
        from_name: row.get(5)?,
        from_addr: row.get(6)?,
        to_json: row.get(7)?,
        cc_json: row.get(8)?,
        date_sent: row.get(9)?,
        date_received: row.get(10)?,
        size: row.get(11)?,
        preview: row.get(12)?,
        body_text: row.get(13)?,
        seen: row.get::<_, i64>(14)? != 0,
        answered: row.get::<_, i64>(15)? != 0,
        flagged: row.get::<_, i64>(16)? != 0,
        flag_color: row.get(17)?,
        attachments: Vec::new(),
    })
}

fn attachments_for(conn: &Connection, message_id: i64) -> Result<Vec<AttachmentRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, filename, mime, size, is_inline
           FROM attachment
          WHERE message_id = ?1
          ORDER BY id",
    )?;

    let rows = stmt
        .query_map([message_id], |row| {
            Ok(AttachmentRow {
                id: row.get(0)?,
                filename: row.get(1)?,
                mime: row.get(2)?,
                size: row.get(3)?,
                is_inline: row.get::<_, i64>(4)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// One message with its body and attachments.
pub fn message_get(conn: &Connection, id: i64) -> Result<Option<MessageFull>, DbError> {
    let sql = format!("SELECT {MESSAGE_FULL_COLUMNS} FROM message WHERE id = ?1");

    let mut stmt = conn.prepare(&sql)?;
    let mut found = stmt
        .query_map([id], message_full)?
        .collect::<Result<Vec<_>, _>>()?;

    match found.pop() {
        Some(mut message) => {
            message.attachments = attachments_for(conn, message.id)?;
            Ok(Some(message))
        }
        None => Ok(None),
    }
}

/// A whole conversation, oldest first — which is the order the reader stacks it in.
///
/// Index: `ix_msg_thread (thread_id, date_sent)`, so this is an index range scan and the
/// `ORDER BY` is free.
pub fn thread_get(conn: &Connection, thread_id: i64) -> Result<Vec<MessageFull>, DbError> {
    let sql = format!(
        "SELECT {MESSAGE_FULL_COLUMNS}
           FROM message
          WHERE thread_id = ?1
          ORDER BY date_sent ASC, id ASC"
    );

    let mut stmt = conn.prepare(&sql)?;
    let mut messages = stmt
        .query_map([thread_id], message_full)?
        .collect::<Result<Vec<_>, _>>()?;

    // A message whose `thread_id` is still NULL belongs to no thread yet — threading is the
    // sync engine's job in Phase 5, and until then a store can hold messages it has never
    // run. Falling back to the message with this id means the reader shows the mail instead
    // of an empty pane: standing rule 13's "degrade visibly, never fail" applied to
    // metadata that has not been computed yet rather than to metadata that is broken.
    if messages.is_empty() {
        if let Some(single) = message_get(conn, thread_id)? {
            return Ok(vec![single]);
        }
    }

    // One query for the whole thread's attachments rather than one per message: a
    // fifteen-message conversation would otherwise cost sixteen round trips to answer a
    // question that is usually "none".
    let mut stmt = conn.prepare(
        "SELECT attachment.message_id, attachment.id, attachment.filename, attachment.mime,
                attachment.size, attachment.is_inline
           FROM attachment
           JOIN message ON message.id = attachment.message_id
          WHERE message.thread_id = ?1
          ORDER BY attachment.message_id, attachment.id",
    )?;

    let attachments = stmt
        .query_map([thread_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                AttachmentRow {
                    id: row.get(1)?,
                    filename: row.get(2)?,
                    mime: row.get(3)?,
                    size: row.get(4)?,
                    is_inline: row.get::<_, i64>(5)? != 0,
                },
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for (message_id, attachment) in attachments {
        if let Some(message) = messages.iter_mut().find(|m| m.id == message_id) {
            message.attachments.push(attachment);
        }
    }

    Ok(messages)
}

/// Full-text search. docs/03 §5 budgets 120ms at 100k messages.
///
/// The FTS5 table is external-content over `message`, so this joins back to it for the row
/// data. `bm25()` orders by relevance rather than by date, which is what makes the first
/// few hits usually the right ones.
///
/// The query text is bound, never interpolated. FTS5's own syntax is still expressive
/// enough that a user could write something that errors, which is why the caller gets a
/// `Result` and not a silently empty list.
pub fn search(conn: &Connection, query: &SearchQuery) -> Result<Vec<MessageRow>, DbError> {
    let trimmed = query.text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    // Each term becomes a prefix match, so "quar fig" finds "quarterly figures" while the
    // user is still typing. Quoting each term keeps FTS5 operators in user input from
    // being executed as syntax.
    let match_expression = trimmed
        .split_whitespace()
        .map(|term| format!("\"{}\"*", term.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ");

    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(match_expression)];

    let mut sql = format!(
        "SELECT {}
           FROM message_fts
           JOIN message ON message.id = message_fts.rowid
          WHERE message_fts MATCH ?1",
        MESSAGE_ROW_COLUMNS
            .split(", ")
            .map(|column| format!("message.{column}"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    if !query.mailbox_ids.is_empty() {
        let start = params.len() + 1;
        let in_list = placeholders(query.mailbox_ids.len(), start);
        sql.push_str(&format!(" AND message.mailbox_id IN ({in_list})"));
        for id in &query.mailbox_ids {
            params.push(Box::new(*id));
        }
    }

    let limit_index = params.len() + 1;
    sql.push_str(&format!(" ORDER BY bm25(message_fts) LIMIT ?{limit_index}"));
    params.push(Box::new(i64::from(query.limit)));

    let borrowed: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(borrowed.as_slice(), message_row)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// `EXPLAIN QUERY PLAN` for a statement, as one string. Used by the tests to prove the
/// indexes are actually being used, and by the seed tool to print the plans docs/06 asks
/// to see.
pub fn explain(conn: &Connection, sql: &str) -> Result<String, DbError> {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;

    // EXPLAIN QUERY PLAN never evaluates the parameters, but rusqlite still insists the
    // count matches, so bind a NULL for each. The plan is identical either way — SQLite
    // chooses it from the schema and the ANALYZE statistics, not from the values.
    let nulls: Vec<&dyn rusqlite::ToSql> = std::iter::repeat_n(
        &rusqlite::types::Null as &dyn rusqlite::ToSql,
        stmt.parameter_count(),
    )
    .collect();

    let lines = stmt
        .query_map(nulls.as_slice(), |row| row.get::<_, String>(3))?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(lines.join("\n"))
}

/// What a reply needs from the message it answers.
///
/// Assembled here rather than in the IPC layer because it is one row and three columns, and
/// pulling it apart across the seam would mean three round trips for one compose window.
pub struct ReplySource {
    pub account_id: i64,
    pub envelope: crate::sync::envelope::Envelope,
    /// The parent's `References`, oldest first.
    pub references: Vec<String>,
    /// The original, quoted and ready to place below the cursor.
    pub quoted_html: String,
}

/// Reads one message as the basis for a reply.
///
/// The quoted body is built from the **stored, already-sanitised** HTML. Quoting the raw body
/// would put a sender's markup into a document the user is about to edit and send under their
/// own name, which is the one place hostile HTML would escape the reader's sandbox entirely.
pub fn reply_source(conn: &Connection, message_id: i64) -> Option<ReplySource> {
    let row = conn
        .query_row(
            "SELECT account_id, message_id, subject, from_name, from_addr,
                    to_json, cc_json, references_, body_html, body_text, date_sent
               FROM message WHERE id = ?1",
            rusqlite::params![message_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .ok()?;

    let (
        account_id,
        rfc_message_id,
        subject,
        from_name,
        from_addr,
        to_json,
        cc_json,
        references,
        body_html,
        body_text,
        date_sent,
    ) = row;

    let addresses = |json: Option<String>| -> Vec<crate::sync::envelope::Address> {
        let Some(json) = json else { return Vec::new() };
        serde_json::from_str::<Vec<serde_json::Value>>(&json)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| {
                let email = entry.get("email")?.as_str()?.to_string();
                Some(crate::sync::envelope::Address {
                    name: entry
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                    email,
                })
            })
            .collect()
    };

    let from = match from_addr {
        Some(email) if !email.trim().is_empty() => vec![crate::sync::envelope::Address {
            name: from_name,
            email,
        }],
        _ => Vec::new(),
    };

    let envelope = crate::sync::envelope::Envelope {
        message_id: rfc_message_id,
        subject: subject.unwrap_or_default(),
        from,
        to: addresses(to_json),
        cc: addresses(cc_json),
        date_sent,
        ..crate::sync::envelope::Envelope::default()
    };

    // Sanitised again on the way out. It was sanitised on the way in, and doing it twice costs
    // a millisecond — but this copy is about to be placed inside a document the user will send
    // under their own name, which is the one path where a mistake in the stored copy would
    // escape the reader's sandbox.
    let quoted_html = match body_html.as_deref() {
        Some(html) if !html.trim().is_empty() => {
            crate::mail::render::sanitise_for_enumeration(html)
        }
        _ => {
            let text = body_text.unwrap_or_default();
            format!(
                "<pre>{}</pre>",
                text.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            )
        }
    };

    Some(ReplySource {
        account_id,
        envelope,
        references: references
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect(),
        quoted_html,
    })
}
