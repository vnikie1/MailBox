//! The types that cross the IPC seam. docs/03-architecture.md §4.
//!
//! Every one derives `TS`, and `cargo test` writes the matching TypeScript into
//! `src/lib/generated/`. That is the whole point: the contract is declared once, in Rust,
//! and the frontend's copy is derived from it. A field renamed on one side and not the
//! other becomes a TypeScript error rather than an `undefined` at runtime.
//!
//! **`i64` is declared to TypeScript as `number`, not `bigint`.** ts-rs defaults to
//! `bigint` because 64-bit integers are not generally safe as JS numbers — but Tauri's IPC
//! is JSON, and `JSON.parse` produces `number` whatever the type says. Declaring `bigint`
//! would describe a value that never arrives. It is safe here because the only i64 fields
//! are row ids and epoch seconds: ids reach billions and seconds reach ten digits, both far
//! inside `Number.MAX_SAFE_INTEGER` at 9.0e15.
//!
//! **Times are epoch seconds, as `i64`.** SQLite has no date type and the schema already
//! stores seconds; converting to a richer type here would mean a conversion on the way out
//! of the database, another on the way into JSON, and a third in the browser. The frontend
//! turns them into `Date` at its own edge.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// An account, as the sidebar needs it. Credentials are not representable here — standing
/// rule 12 keeps them in the Credential Manager, and `cred_ref` is deliberately not a field.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AccountRow {
    #[ts(type = "number")]
    pub id: i64,
    pub display_name: String,
    pub email: String,
    pub provider: String,
}

/// A mailbox. `role` stays a plain string rather than an enum: the set is open — servers
/// invent folder roles — and standing rule 13's "parse leniently, degrade visibly" applies
/// to a role we do not recognise just as much as to broken MIME.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MailboxRow {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub account_id: i64,
    pub display_name: String,
    #[ts(type = "number | null")]
    pub parent_id: Option<i64>,
    pub role: Option<String>,
    #[ts(type = "number")]
    pub unread_count: i64,
    #[ts(type = "number")]
    pub total_count: i64,
}

/// One row of the message list. docs/02 §6.3 — everything a row draws and nothing else;
/// bodies are fetched on demand by `message_get`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MessageRow {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number | null")]
    pub thread_id: Option<i64>,
    #[ts(type = "number")]
    pub mailbox_id: i64,
    #[ts(type = "number")]
    pub account_id: i64,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    #[ts(type = "number")]
    pub date_received: i64,
    pub preview: Option<String>,
    #[ts(type = "number")]
    pub size: i64,
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub flag_color: Option<String>,
    pub has_attachment: bool,
}

/// A message with its body and recipients, for the reader.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MessageFull {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number | null")]
    pub thread_id: Option<i64>,
    #[ts(type = "number")]
    pub mailbox_id: i64,
    #[ts(type = "number")]
    pub account_id: i64,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    pub to_json: Option<String>,
    pub cc_json: Option<String>,
    #[ts(type = "number")]
    pub date_sent: i64,
    #[ts(type = "number")]
    pub date_received: i64,
    #[ts(type = "number")]
    pub size: i64,
    pub preview: Option<String>,
    pub body_text: Option<String>,
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub flag_color: Option<String>,
    pub attachments: Vec<AttachmentRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentRow {
    #[ts(type = "number")]
    pub id: i64,
    pub filename: Option<String>,
    pub mime: Option<String>,
    #[ts(type = "number | null")]
    pub size: Option<i64>,
    pub is_inline: bool,
}

/// Where the next page starts. docs/06 Phase 3: "Cursor on (date_received, id)" — never
/// `OFFSET`, which re-walks every skipped row and drifts when rows arrive mid-scroll.
///
/// The id is not decoration. Timestamps collide constantly in mail (a sync commits a
/// hundred messages with the same received time), and a cursor on the date alone either
/// repeats or skips that whole run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    #[ts(type = "number")]
    pub date_received: i64,
    #[ts(type = "number")]
    pub id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    /// One id for a mailbox, several for a unified row such as All Inboxes.
    #[ts(type = "number[]")]
    pub mailbox_ids: Vec<i64>,
    /// Where to resume. `None` starts at the newest.
    pub cursor: Option<Cursor>,
    pub limit: u32,
    /// Unread only — the list's filter button.
    pub unread_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    /// `None` when the end has been reached, so the caller stops asking.
    pub next_cursor: Option<Cursor>,
}

/// A partial flag change. `None` leaves a flag alone, which is what makes this safe to
/// apply to a multi-selection whose members disagree.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct FlagPatch {
    pub seen: Option<bool>,
    pub flagged: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub text: String,
    /// Empty searches everywhere.
    #[ts(type = "number[]")]
    pub mailbox_ids: Vec<i64>,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MailboxCounts {
    #[ts(type = "number")]
    pub mailbox_id: i64,
    #[ts(type = "number")]
    pub unread: i64,
    #[ts(type = "number")]
    pub total: i64,
}
