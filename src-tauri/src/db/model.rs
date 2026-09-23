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
    /// The colour the user picked for this account in Settings, or `None` for the accent.
    ///
    /// Here rather than only on `AccountDetail` because the settings pane is not where a
    /// per-account colour is *for*. It was stored, and offered, and read back by the pane
    /// that set it — and the mailbox window never received it, so picking a colour changed
    /// nothing anyone could see. One of the seven names in `COLORS`
    /// (`src/features/accounts/AccountsSettings.tsx`), which are the flag palette.
    pub color: Option<String>,
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
    /// The mailbox this one is inside, as the sidebar nests it. Null at the top of the account.
    ///
    /// Worked out from the paths (`db::query::mailboxes_tree`) rather than stored: a folder
    /// renamed or made on another device moves in the tree with nothing to keep in step. Never
    /// the Inbox — servers that keep every folder inside it would otherwise show the whole
    /// account as the Inbox's children.
    #[ts(type = "number | null")]
    pub parent_id: Option<i64>,
    pub role: Option<String>,
    #[ts(type = "number")]
    pub unread_count: i64,
    #[ts(type = "number")]
    pub total_count: i64,
    /// Where it sits in Favourites, or null when it is not a favourite. A position among every
    /// favourite, built-in rows included; `favourites_list` has the whole order.
    #[ts(type = "number | null")]
    pub favourite_order: Option<i64>,
    /// Whether the user chose this mailbox's role with Use This Mailbox As, rather than the
    /// server naming it.
    pub role_chosen: bool,
    /// The server's hierarchy separator, for checking a name before it is sent. Null until the
    /// mailbox has been listed by a sync that stored it.
    pub delimiter: Option<String>,
    /// Whether Rename and Delete are offered. See `sync::folders::editable`.
    pub editable: bool,
    /// How many mailboxes are inside this one, at any depth — the ones Delete Mailbox takes
    /// with it, which its confirmation has to name.
    #[ts(type = "number")]
    pub descendants: i64,
    /// Whether New Mailbox may put a mailbox inside this one. See `sync::folders::can_contain`.
    pub can_contain: bool,
}

/// One of the rows every sidebar's Favourites starts with.
///
/// Serialised as the key the table stores (`allInboxes`), so the stored and the sent spelling
/// cannot drift apart. The sidebar maps each to the row it builds (`buildSidebar`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum BuiltinFavourite {
    AllInboxes,
    Vips,
    Flagged,
    AllDrafts,
    AllSent,
}

impl BuiltinFavourite {
    pub const ALL: [BuiltinFavourite; 5] = [
        BuiltinFavourite::AllInboxes,
        BuiltinFavourite::Vips,
        BuiltinFavourite::Flagged,
        BuiltinFavourite::AllDrafts,
        BuiltinFavourite::AllSent,
    ];

    pub fn key(self) -> &'static str {
        match self {
            BuiltinFavourite::AllInboxes => "allInboxes",
            BuiltinFavourite::Vips => "vips",
            BuiltinFavourite::Flagged => "flagged",
            BuiltinFavourite::AllDrafts => "allDrafts",
            BuiltinFavourite::AllSent => "allSent",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|builtin| builtin.key() == key)
    }
}

/// One entry of Favourites. `favourites_list` returns them in the order the sidebar shows.
///
/// Exactly one of `builtin` and `mailbox_id` is set; the table's CHECK says so too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct FavouriteRow {
    /// What `favourite_move` names it by.
    #[ts(type = "number")]
    pub id: i64,
    pub builtin: Option<BuiltinFavourite>,
    #[ts(type = "number | null")]
    pub mailbox_id: Option<i64>,
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
    /// Whether the conversation this belongs to is muted. From `thread`, not `message`.
    pub muted: bool,
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
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub flag_color: Option<String>,
    /// Whether the message is filed as junk.
    pub is_junk: bool,
    /// True when a person made that call, rather than the filter.
    ///
    /// The reader needs the difference: "we think this is junk" invites a correction, while
    /// "you marked this as junk" is a statement, and a banner that argued with the user about
    /// their own decision would be the fastest way to make them turn the filter off.
    pub junk_by_user: bool,
    /// The filter's confidence, when it had an opinion. `null` before it has been scored.
    pub junk_score: Option<f64>,
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
