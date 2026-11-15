//! Local changes on their way to the server. docs/03 §5's `pending_op` drain.
//!
//! Flagging a message, moving it, deleting it — all of those are written to the local database
//! immediately, because the user is looking at the result and a client that waits for a round
//! trip before redrawing feels broken. The server still has to be told, and it might not be
//! reachable when the change is made.
//!
//! So every mutation records its intent in `pending_op` **inside the same transaction as the
//! local write**. That is the whole design: the local change and the obligation to push it are
//! one atomic unit, so there is no window in which the screen says one thing, the server
//! another, and nothing remembers the difference. Offline mode is not a mode — it is what this
//! queue does when the drain cannot connect.
//!
//! The drain runs at the *start* of a sync, before anything is fetched. The other order loses
//! data: pulling the server's state first would overwrite the local change with the stale
//! value the server still holds, and the queued op would then push a value the user had
//! already seen reverted.

use rusqlite::{params, Transaction};
use serde::{Deserialize, Serialize};

use crate::db::{Db, DbError};

use super::events::{payload, Events};
use super::session::{ImapSession, SyncError};

/// How many times to retry one operation before dropping it.
///
/// Dropping is not silent — it logs and clears — but it has to happen. An operation that can
/// never succeed (a message the server has already expunged, a mailbox that has been deleted)
/// would otherwise sit at the head of the queue and block every change made after it.
const MAX_ATTEMPTS: i64 = 5;

/// One queued change, as stored in `pending_op.payload_json`.
///
/// UIDs rather than local row ids: the row id means nothing to the server, and by the time the
/// drain runs the local row may have moved mailbox or been replaced. The mailbox is carried as
/// its remote path for the same reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Op {
    /// `UID STORE` — set or clear `\Seen` and `\Flagged`.
    Flag {
        mailbox: String,
        uids: Vec<u32>,
        seen: Option<bool>,
        flagged: Option<bool>,
    },

    /// `UID MOVE`, falling back to COPY + STORE + EXPUNGE where the server has no MOVE.
    Move {
        from: String,
        to: String,
        uids: Vec<u32>,
    },

    /// `UID MOVE` of a message whose UID in `from` is not known yet, found by its `Message-ID`.
    ///
    /// A message this app moved has a new UID in its new mailbox, and the sync learns it only
    /// when it next reads that mailbox. Moving it again before then has nothing else to name it
    /// by — and Undo, a few seconds after Archive and after the archive has reached the server,
    /// is the ordinary way to get there. A `Move` then named the UID the message *used* to have
    /// in the Inbox, which the server ignores, so the undo was reversed by the next sync (H7,
    /// 2026-11-14 audit).
    MoveByMessageId {
        from: String,
        to: String,
        message_id: String,
    },

    /// `UID STORE \Deleted` then expunge. Only ever a *permanent* delete; moving to Trash is
    /// a `Move`, which is what the delete command does unless the user asked otherwise.
    Delete { mailbox: String, uids: Vec<u32> },

    /// `Flag`, for a message in `mailbox` whose UID there is not known yet, found by its
    /// `Message-ID`.
    ///
    /// A message moved here has a new UID that the sync learns only when it next reads this
    /// mailbox. A flag change made before then was sent to the mailbox and UID it had *before*
    /// the move, which the server accepted as a change to nothing; it was then forgotten as sent.
    /// On Gmail, where Archive is a move, a star or a read mark on freshly archived mail never
    /// reached the server (M4, 2026-11-14 audit).
    FlagByMessageId {
        mailbox: String,
        message_id: String,
        seen: Option<bool>,
        flagged: Option<bool>,
    },

    /// `Delete`, for a message known only by its `Message-ID`. See `FlagByMessageId`.
    DeleteByMessageId { mailbox: String, message_id: String },

    /// `APPEND` a draft to the Drafts mailbox, replacing the copy it supersedes.
    ///
    /// Queued rather than sent inline because it happens every thirty seconds while someone is
    /// typing, and a save that opened a connection each time would make the act of writing a
    /// message a stream of network activity. It also means a draft written on a train is
    /// appended when the train leaves the tunnel, rather than lost.
    AppendDraft {
        mailbox: String,
        /// Where the bytes are. Read at drain time, so a draft saved five times appends only
        /// whatever the last save wrote.
        eml_path: String,
        /// The UID of the copy this replaces, when one is known.
        replaces: Option<u32>,
        /// The draft's stable `Message-ID`.
        ///
        /// Carried so a copy written by *another* device can be recognised: the server is
        /// asked for every message with this id, and anything that is not the copy we are
        /// replacing was put there by somebody else.
        message_id: String,
    },

    /// Every copy of a draft, found by its `Message-ID`, flagged `\Deleted` and expunged.
    ///
    /// For a draft that has been sent or thrown away. Nothing removed the server copy before, so
    /// a sent message stayed in Drafts on every device and could be sent a second time from one
    /// (H4, 2026-11-14 audit). By id rather than by UID because the UID of the last copy may not
    /// be known yet, and because every copy with the draft's own id is this draft.
    DeleteDraft { mailbox: String, message_id: String },

    /// `CREATE`, then `SUBSCRIBE`. New Mailbox.
    ///
    /// Queued like every other change, per standing rule 10: the folder is in the sidebar the
    /// moment it is named, and on the server when the server can be reached.
    CreateMailbox { path: String },

    /// `RENAME`. Rename Mailbox.
    ///
    /// The server renames the children with the parent, and the separator is carried so the
    /// sync can recognise those children while the rename is still on its way.
    RenameMailbox {
        from: String,
        to: String,
        delimiter: Option<String>,
    },

    /// `DELETE` for a mailbox and everything inside it. Delete Mailbox.
    ///
    /// Listed deepest first. IMAP deletes one name at a time, and some servers refuse to delete a
    /// mailbox that still has children. The separator says what "inside" means, for a child
    /// another device adds before this reaches the server.
    DeleteMailbox {
        paths: Vec<String>,
        delimiter: Option<String>,
    },

    /// Every message in a mailbox, permanently: `\Deleted` on `1:*`, then expunge. Erase Deleted
    /// Items and Erase Junk Mail.
    ///
    /// The whole mailbox rather than a UID list, because the local store holds only the newest
    /// messages of any folder but the Inbox — an erase of the UIDs known here would leave the
    /// older mail on the server, and it would never come back to be seen and erased. `1:*` is
    /// read when the operation runs, after every operation queued before it: a message deleted
    /// just before the erase is moved to the Bin first, and erased with the rest.
    EraseMailbox { mailbox: String },
}

impl Op {
    fn kind(&self) -> &'static str {
        match self {
            Op::Flag { .. } => "flag",
            Op::Move { .. } => "move",
            Op::MoveByMessageId { .. } => "moveById",
            Op::Delete { .. } => "delete",
            Op::FlagByMessageId { .. } => "flagById",
            Op::DeleteByMessageId { .. } => "deleteById",
            Op::AppendDraft { .. } => "append",
            Op::DeleteDraft { .. } => "deleteDraft",
            Op::CreateMailbox { .. } => "mailboxCreate",
            Op::RenameMailbox { .. } => "mailboxRename",
            Op::DeleteMailbox { .. } => "mailboxDelete",
            Op::EraseMailbox { .. } => "mailboxErase",
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Op::Flag {
                uids,
                seen,
                flagged,
                ..
            } => uids.is_empty() || (seen.is_none() && flagged.is_none()),
            Op::Move { uids, .. } | Op::Delete { uids, .. } => uids.is_empty(),
            Op::MoveByMessageId {
                from,
                to,
                message_id,
            } => from == to || message_id.trim().is_empty(),
            Op::DeleteMailbox { paths, .. } => paths.is_empty(),
            Op::RenameMailbox { from, to, .. } => from == to,
            Op::FlagByMessageId {
                message_id,
                seen,
                flagged,
                ..
            } => message_id.trim().is_empty() || (seen.is_none() && flagged.is_none()),
            Op::DeleteByMessageId { message_id, .. } => message_id.trim().is_empty(),
            // Always worth doing: the point is the bytes, not a UID list.
            Op::AppendDraft { .. } => false,
            Op::DeleteDraft { message_id, .. } => message_id.trim().is_empty(),
            Op::CreateMailbox { .. } | Op::EraseMailbox { .. } => false,
        }
    }

    /// Whether this changes the mailbox tree rather than the mail in it.
    ///
    /// A server refusing one of these refuses it for good — a name it will not accept does not
    /// become acceptable on the fifth attempt — so the drain gives up on it at once and says why,
    /// instead of failing five syncs in a row first.
    fn is_structural(&self) -> bool {
        matches!(
            self,
            Op::CreateMailbox { .. }
                | Op::RenameMailbox { .. }
                | Op::DeleteMailbox { .. }
                | Op::EraseMailbox { .. }
        )
    }

    /// The mailbox whose UIDs this operation names, for the kinds that name messages by UID.
    ///
    /// Those UIDs hold only while the mailbox keeps its `UIDVALIDITY`, so this is the mailbox
    /// whose value is recorded beside the operation when it is queued (M3, 2026-11-14 audit).
    fn uid_mailbox(&self) -> Option<&str> {
        match self {
            Op::Flag { mailbox, .. } | Op::Delete { mailbox, .. } => Some(mailbox),
            Op::Move { from, .. } => Some(from),
            _ => None,
        }
    }

    /// Every mailbox path this operation names.
    pub(crate) fn paths(&self) -> Vec<&str> {
        match self {
            Op::Flag { mailbox, .. }
            | Op::Delete { mailbox, .. }
            | Op::FlagByMessageId { mailbox, .. }
            | Op::DeleteByMessageId { mailbox, .. }
            | Op::AppendDraft { mailbox, .. }
            | Op::DeleteDraft { mailbox, .. }
            | Op::EraseMailbox { mailbox } => vec![mailbox],
            Op::Move { from, to, .. } | Op::MoveByMessageId { from, to, .. } => vec![from, to],
            Op::CreateMailbox { path } => vec![path],
            Op::RenameMailbox { from, to, .. } => vec![from, to],
            Op::DeleteMailbox { paths, .. } => paths.iter().map(String::as_str).collect(),
        }
    }

    /// The same, for rewriting.
    pub(crate) fn paths_mut(&mut self) -> Vec<&mut String> {
        match self {
            Op::Flag { mailbox, .. }
            | Op::Delete { mailbox, .. }
            | Op::FlagByMessageId { mailbox, .. }
            | Op::DeleteByMessageId { mailbox, .. }
            | Op::AppendDraft { mailbox, .. }
            | Op::DeleteDraft { mailbox, .. }
            | Op::EraseMailbox { mailbox } => vec![mailbox],
            Op::Move { from, to, .. } | Op::MoveByMessageId { from, to, .. } => vec![from, to],
            Op::CreateMailbox { path } => vec![path],
            Op::RenameMailbox { from, to, .. } => vec![from, to],
            Op::DeleteMailbox { paths, .. } => paths.iter_mut().collect(),
        }
    }
}

/// Which of a message's flags the user has changed here and the server has not heard about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Unsent {
    pub seen: bool,
    pub flagged: bool,
}

/// The flags queued for the server in one mailbox, by UID.
///
/// A sync that writes the server's flags over these puts back what the user has just changed.
/// The drain at the start of a pass protects a change made *before* the pass; one made *during*
/// it — Mark All Messages as Read while the account is syncing — used to be overwritten by the
/// flags the pass fetched a moment later, which the server still held. Found driving the built
/// app: three messages of six went back to unread and stayed so until the next sync.
pub fn unsent_flags(
    tx: &Transaction<'_>,
    account_id: i64,
    mailbox: &str,
) -> Result<std::collections::HashMap<u32, Unsent>, DbError> {
    let payloads: Vec<String> = {
        let mut statement = tx.prepare(
            "SELECT payload_json FROM pending_op WHERE account_id = ?1 AND kind = 'flag'",
        )?;
        let rows = statement.query_map(params![account_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut unsent: std::collections::HashMap<u32, Unsent> = std::collections::HashMap::new();

    for payload in payloads {
        let Ok(Op::Flag {
            mailbox: path,
            uids,
            seen,
            flagged,
        }) = serde_json::from_str::<Op>(&payload)
        else {
            continue;
        };

        if path != mailbox {
            continue;
        }

        for uid in uids {
            let entry = unsent.entry(uid).or_default();
            entry.seen |= seen.is_some();
            entry.flagged |= flagged.is_some();
        }
    }

    Ok(unsent)
}

/// The messages in one mailbox that the user has moved or deleted here and the server still
/// holds, by UID — or every message, while an erase of the mailbox is on its way.
///
/// The removal's counterpart of `unsent_flags`. A sync reads the mailbox from the server, which
/// still lists these, and wrote each one back as a new row: a message deleted a moment ago came
/// back, and a moved one showed in both folders, until the change reached the server and the
/// sync after it tidied up. Rebuild, which reads a whole mailbox again, made that easy to see.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnsentRemovals {
    pub uids: std::collections::HashSet<u32>,
    /// Messages a queued `MoveByMessageId` takes out, which have no UID to be listed by.
    pub message_ids: std::collections::HashSet<String>,
    /// An erase of the whole mailbox is queued.
    pub everything: bool,
}

impl UnsentRemovals {
    pub fn contains(&self, uid: u32) -> bool {
        self.everything || self.uids.contains(&uid)
    }

    /// `contains`, also matching a message by its `Message-ID`.
    pub fn contains_message(&self, uid: u32, message_id: Option<&str>) -> bool {
        // Compared without angle brackets: the envelope stores ids bare, and a draft's carries
        // them, as the builder writes it.
        self.contains(uid)
            || message_id
                .map(super::drafts::bare)
                .is_some_and(|id| !id.is_empty() && self.message_ids.contains(id))
    }
}

pub fn unsent_removals(
    tx: &Transaction<'_>,
    account_id: i64,
    mailbox: &str,
) -> Result<UnsentRemovals, DbError> {
    let payloads: Vec<String> = {
        let mut statement = tx.prepare(
            "SELECT payload_json FROM pending_op
              WHERE account_id = ?1
                AND kind IN ('move', 'moveById', 'delete', 'deleteById', 'deleteDraft',
                             'mailboxErase')",
        )?;
        let rows = statement.query_map(params![account_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut removals = UnsentRemovals::default();

    for payload in payloads {
        match serde_json::from_str::<Op>(&payload) {
            Ok(Op::Move { from, uids, .. }) if from == mailbox => removals.uids.extend(uids),
            Ok(Op::MoveByMessageId {
                from, message_id, ..
            }) if from == mailbox => {
                removals
                    .message_ids
                    .insert(super::drafts::bare(&message_id).to_string());
            }
            Ok(
                Op::DeleteDraft {
                    mailbox: path,
                    message_id,
                }
                | Op::DeleteByMessageId {
                    mailbox: path,
                    message_id,
                },
            ) if path == mailbox => {
                removals
                    .message_ids
                    .insert(super::drafts::bare(&message_id).to_string());
            }
            Ok(Op::Delete {
                mailbox: path,
                uids,
            }) if path == mailbox => {
                removals.uids.extend(uids);
            }
            Ok(Op::EraseMailbox { mailbox: path }) if path == mailbox => {
                removals.everything = true;
            }
            _ => {}
        }
    }

    Ok(removals)
}

/// The mailbox a row names: its account and its path.
fn account_and_path(
    tx: &Transaction<'_>,
    mailbox_id: i64,
) -> Result<Option<(i64, String)>, DbError> {
    use rusqlite::OptionalExtension;

    Ok(tx
        .query_row(
            "SELECT account_id, remote_path FROM mailbox WHERE id = ?1",
            params![mailbox_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

/// `unsent_removals`, for a mailbox named by its row.
pub fn unsent_removals_in(
    tx: &Transaction<'_>,
    mailbox_id: i64,
) -> Result<UnsentRemovals, DbError> {
    match account_and_path(tx, mailbox_id)? {
        Some((account_id, path)) => unsent_removals(tx, account_id, &path),
        None => Ok(UnsentRemovals::default()),
    }
}

/// Whether anything queued for the account names this mailbox.
///
/// Rebuild waits while it does: reading the mailbox again before those changes reach the server
/// would read back what they are about to change.
pub fn names_mailbox(
    tx: &Transaction<'_>,
    account_id: i64,
    mailbox: &str,
) -> Result<bool, DbError> {
    Ok(queued(tx, account_id)?
        .iter()
        .any(|(_, op)| op.paths().contains(&mailbox)))
}

/// `unsent_flags`, for a mailbox named by its row.
pub fn unsent_flags_in(
    tx: &Transaction<'_>,
    mailbox_id: i64,
) -> Result<std::collections::HashMap<u32, Unsent>, DbError> {
    match account_and_path(tx, mailbox_id)? {
        Some((account_id, path)) => unsent_flags(tx, account_id, &path),
        None => Ok(std::collections::HashMap::new()),
    }
}

/// The mailbox changes an account has queued and the server has not made yet.
///
/// The sync reads this before it believes a `LIST`. A folder renamed a moment ago is still
/// listed under its old name until the rename is sent, and a sync that took the listing at its
/// word would recreate the old folder beside the renamed one — and then delete the renamed one,
/// with its mail, for not being on the server.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PendingTree {
    /// Here and not yet there: created, or the new name of a rename.
    keep: Vec<(String, Option<String>)>,
    /// There and no longer here: the old name of a rename, or deleted.
    hide: Vec<(String, Option<String>)>,
}

impl PendingTree {
    /// A local mailbox the server does not have yet, which must not be pruned for its absence.
    pub fn keeps(&self, path: &str) -> bool {
        self.keep
            .iter()
            .any(|(root, delimiter)| within(path, root, delimiter.as_deref()))
    }

    /// A listed mailbox the user has already renamed or deleted, which must not be recreated.
    pub fn hides(&self, path: &str) -> bool {
        self.hide
            .iter()
            .any(|(root, delimiter)| within(path, root, delimiter.as_deref()))
    }
}

pub fn pending_tree(tx: &Transaction<'_>, account_id: i64) -> Result<PendingTree, DbError> {
    let payloads: Vec<String> = {
        let mut statement = tx.prepare(
            "SELECT payload_json FROM pending_op
              WHERE account_id = ?1
                AND kind IN ('mailboxCreate', 'mailboxRename', 'mailboxDelete')
              ORDER BY id",
        )?;

        let rows = statement.query_map(params![account_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut tree = PendingTree::default();

    for payload in payloads {
        match serde_json::from_str::<Op>(&payload) {
            Ok(Op::CreateMailbox { path }) => tree.keep.push((path, None)),
            Ok(Op::RenameMailbox {
                from,
                to,
                delimiter,
            }) => {
                tree.hide.push((from, delimiter.clone()));
                tree.keep.push((to, delimiter));
            }
            Ok(Op::DeleteMailbox { paths, delimiter }) => {
                for path in paths {
                    tree.hide.push((path, delimiter.clone()));
                }
            }
            _ => {}
        }
    }

    Ok(tree)
}

/// Whether `path` is `root` or somewhere inside it.
///
/// Without a separator a mailbox has no inside, so only the exact name matches.
pub fn within(path: &str, root: &str, delimiter: Option<&str>) -> bool {
    if path == root {
        return true;
    }

    match delimiter {
        Some(separator) if !separator.is_empty() => path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with(separator)),
        _ => false,
    }
}

/// `path` with its `from` prefix replaced by `to`, when it is inside `from`.
pub fn repath(path: &str, from: &str, to: &str, delimiter: Option<&str>) -> Option<String> {
    if !within(path, from, delimiter) {
        return None;
    }

    Some(format!("{to}{}", &path[from.len()..]))
}

/// The longest sequence set put in one command.
///
/// RFC 7162 §4 asks clients to keep a command line to about 8,000 octets, because servers set
/// limits of their own and do not agree on them. Every UID used to go in one command, however
/// many there were: Mark All Messages as Read on a mailbox of fifty thousand scattered UIDs
/// wrote a line of about 290 KB. The Dovecot rig took a 114 KB line without complaint when this
/// was tried, so no refusal has been seen — but a line whose length depends on the size of the
/// user's mailbox is a limit waiting to be found, and a refused flag change is retried for five
/// syncs and then dropped, leaving the mail unread everywhere but here.
const MAX_SET_LEN: usize = 7_000;

/// Formats a UID list as IMAP sequence sets, each short enough for one command.
///
/// Consecutive UIDs become a range — `4:7` for 4, 5, 6 and 7 — and nothing else does. The UIDs
/// in one operation are whatever the user happened to select, and a range spanning a gap would
/// touch messages they did not choose. That is a correctness point, not a performance one.
fn sequence_sets(uids: &[u32]) -> Vec<String> {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut runs: Vec<String> = Vec::new();
    let mut remaining = sorted.into_iter().peekable();

    while let Some(start) = remaining.next() {
        let mut end = start;
        while let Some(next) = end.checked_add(1) {
            if remaining.peek() != Some(&next) {
                break;
            }
            remaining.next();
            end = next;
        }

        runs.push(if start == end {
            start.to_string()
        } else {
            format!("{start}:{end}")
        });
    }

    let mut sets = Vec::new();
    let mut current = String::new();

    for run in runs {
        if !current.is_empty() && current.len() + 1 + run.len() > MAX_SET_LEN {
            sets.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(',');
        }
        current.push_str(&run);
    }

    if !current.is_empty() {
        sets.push(current);
    }

    sets
}

/// Where a set of local rows lives on the server, grouped so one command covers each group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub account_id: i64,
    pub mailbox: String,
    pub uids: Vec<u32>,
}

/// Resolves local message ids to the account, mailbox path and UIDs the server knows.
///
/// **Call before the local write, never after.** A move rewrites `message.mailbox_id`, so
/// afterwards this returns the destination and the queued operation would ask the server to
/// move messages out of the mailbox they were already moved to.
///
/// Grouped by mailbox because IMAP is: one `SELECT` and one command per mailbox, rather than a
/// round trip per message. A selection spanning two accounts is normal — "mark all as read"
/// across All Inboxes — and produces one group per account per mailbox.
pub fn locate(tx: &Transaction<'_>, ids: &[i64]) -> Result<Vec<Located>, DbError> {
    Ok(resolve(tx, ids)?.0)
}

/// A message this app can name to the server only by its `Message-ID`: one moved here whose
/// UID here the sync has not learned yet. See [`resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unnumbered {
    pub account_id: i64,
    pub mailbox: String,
    pub message_id: String,
}

/// [`locate`], and the messages it cannot name by UID.
///
/// A parked row — moved here, with the move queued or already sent — is named by its
/// `Message-ID` in the mailbox it is in now, wherever that move has got to. A change to it is
/// queued *behind* the move, so by the time the change runs the move has run too, and the
/// message is here under a UID nobody has told us. Naming it by its origin and old UID, as
/// `locate` did, sent the change to where the message no longer was: the server accepted it as
/// a change to nothing, and the queue forgot it as sent (M4, 2026-11-14 audit). A parked row
/// with no `Message-ID` is still named by its origin, as before; there is nothing else to name
/// it by.
pub fn resolve(
    tx: &Transaction<'_>,
    ids: &[i64],
) -> Result<(Vec<Located>, Vec<Unnumbered>), DbError> {
    if ids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    // In pieces, as `db::write` does (M7): SQLite takes 32,766 parameters in a statement, and a
    // Mark All as Read can name more messages than that. Each piece comes back grouped; the
    // groups are merged by account and mailbox, so a mailbox is still one command.
    if ids.len() > crate::db::write::CHUNK {
        let mut merged: Vec<Located> = Vec::new();
        let mut unnumbered: Vec<Unnumbered> = Vec::new();

        for chunk in ids.chunks(crate::db::write::CHUNK) {
            let (groups, more) = resolve(tx, chunk)?;
            unnumbered.extend(more);

            for group in groups {
                match merged.iter_mut().find(|known| {
                    known.account_id == group.account_id && known.mailbox == group.mailbox
                }) {
                    Some(known) => known.uids.extend(group.uids),
                    None => merged.push(group),
                }
            }
        }

        for group in &mut merged {
            group.uids.sort_unstable();
        }
        return Ok((merged, unnumbered));
    }

    let placeholders = (0..ids.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");

    // A parked row is named by where the *server* still has it, not by where it has been moved
    // to locally. `move_to` records that origin precisely so this query can find it.
    //
    // Without the COALESCE this selected the placeholder UID, which the loop below then threw
    // away — so every command issued against a message with an unfinished move queued nothing
    // at all, silently. Moving three messages where one was already parked queued an operation
    // naming two of them; flagging a parked message did nothing on the server. The local half
    // succeeded in both cases, which is what made it look like it had worked.
    //
    // The origin's path and UID are taken together or not at all. They used to be two separate
    // COALESCEs, so a row whose origin mailbox had since been deleted paired the *origin* UID
    // with its *current* mailbox — a UID from one folder sent against another, which names some
    // other message entirely. With no origin row the row's own UID is used, which for a parked
    // row is negative and skipped below: nothing is sent, which is the safe kind of wrong.
    let sql = format!(
        "SELECT message.account_id,
                CASE WHEN origin.id IS NOT NULL AND message.origin_uid IS NOT NULL
                     THEN origin.remote_path ELSE mailbox.remote_path END,
                CASE WHEN origin.id IS NOT NULL AND message.origin_uid IS NOT NULL
                     THEN message.origin_uid ELSE message.uid END,
                CASE WHEN message.uid < 0 THEN mailbox.remote_path END,
                CASE WHEN message.uid < 0 THEN message.message_id END
           FROM message
           JOIN mailbox ON mailbox.id = message.mailbox_id
           LEFT JOIN mailbox AS origin ON origin.id = message.origin_mailbox_id
          WHERE message.id IN ({placeholders})
          ORDER BY 1, 2, 3"
    );

    let params: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

    let rows = tx
        .prepare(&sql)?
        .query_map(params.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut groups: Vec<Located> = Vec::new();
    let mut unnumbered: Vec<Unnumbered> = Vec::new();

    for (account_id, mailbox, uid, parked_in, message_id) in rows {
        // Parked: by its `Message-ID`, where it is now. See above.
        if let (Some(here), Some(message_id)) = (
            parked_in,
            message_id.filter(|value| !value.trim().is_empty()),
        ) {
            unnumbered.push(Unnumbered {
                account_id,
                mailbox: here,
                message_id,
            });
            continue;
        }

        // A UID of 0 is a message that exists locally and has never been on the server —
        // there is nothing to tell the server about, and "0" is not a valid UID to send.
        let Ok(uid) = u32::try_from(uid) else {
            continue;
        };
        if uid == 0 {
            continue;
        }

        match groups.last_mut() {
            Some(last) if last.account_id == account_id && last.mailbox == mailbox => {
                last.uids.push(uid);
            }
            _ => groups.push(Located {
                account_id,
                mailbox,
                uids: vec![uid],
            }),
        }
    }

    Ok((groups, unnumbered))
}

/// Queues the server side of a flag change to these messages. **Call before the local write**,
/// as with [`locate`].
///
/// One `Flag` per mailbox for the messages with a known UID, and one `FlagByMessageId` for each
/// that has none yet. Every flag change goes through here — the toolbar, the menus, Mark All as
/// Read, colours, rules and Undo — so none of them can miss the second half.
pub fn queue_flags(
    tx: &Transaction<'_>,
    ids: &[i64],
    seen: Option<bool>,
    flagged: Option<bool>,
) -> Result<(), DbError> {
    let (groups, unnumbered) = resolve(tx, ids)?;

    for group in groups {
        enqueue(
            tx,
            group.account_id,
            &Op::Flag {
                mailbox: group.mailbox,
                uids: group.uids,
                seen,
                flagged,
            },
        )?;
    }

    for message in unnumbered {
        enqueue(
            tx,
            message.account_id,
            &Op::FlagByMessageId {
                mailbox: message.mailbox,
                message_id: message.message_id,
                seen,
                flagged,
            },
        )?;
    }

    Ok(())
}

/// Flag changes to many messages, queued the way a command acting on a selection queues them:
/// one operation per mailbox and change, not one per message.
///
/// Undo and the rules engine work a message at a time, and each message queued its own `Flag`.
/// Undoing a Mark All as Read over 5,000 messages queued 5,000 STOREs, sent one after another,
/// and the account synced nothing else until the last had gone (L4).
///
/// Per message, the last change to each flag is the one queued, as it is the one the row ends with.
/// **Queue the batch before queueing any move**: a flag change queued after a message's move names
/// a UID its old mailbox no longer has, and the server ignores it without a word.
#[derive(Debug, Default)]
pub struct FlagBatch {
    seen: std::collections::BTreeMap<i64, bool>,
    flagged: std::collections::BTreeMap<i64, bool>,
}

impl FlagBatch {
    pub fn add(&mut self, id: i64, seen: Option<bool>, flagged: Option<bool>) {
        if let Some(seen) = seen {
            self.seen.insert(id, seen);
        }
        if let Some(flagged) = flagged {
            self.flagged.insert(id, flagged);
        }
    }

    /// Queues everything added so far, and empties the batch.
    pub fn queue(&mut self, tx: &Transaction<'_>) -> Result<(), DbError> {
        let split = |changes: std::collections::BTreeMap<i64, bool>| {
            let (on, off): (Vec<_>, Vec<_>) = changes.into_iter().partition(|(_, value)| *value);
            let ids = |pairs: Vec<(i64, bool)>| pairs.into_iter().map(|(id, _)| id).collect();
            (ids(on), ids(off))
        };

        let (read, unread): (Vec<i64>, Vec<i64>) = split(std::mem::take(&mut self.seen));
        let (flag, unflag): (Vec<i64>, Vec<i64>) = split(std::mem::take(&mut self.flagged));

        for (ids, seen, flagged) in [
            (read, Some(true), None),
            (unread, Some(false), None),
            (flag, None, Some(true)),
            (unflag, None, Some(false)),
        ] {
            if !ids.is_empty() {
                queue_flags(tx, &ids, seen, flagged)?;
            }
        }

        Ok(())
    }
}

/// Queues the server side of permanently deleting these messages. **Call before the local
/// write.** See [`queue_flags`].
pub fn queue_delete(tx: &Transaction<'_>, ids: &[i64]) -> Result<(), DbError> {
    let (groups, unnumbered) = resolve(tx, ids)?;

    for group in groups {
        enqueue(
            tx,
            group.account_id,
            &Op::Delete {
                mailbox: group.mailbox,
                uids: group.uids,
            },
        )?;
    }

    for message in unnumbered {
        enqueue(
            tx,
            message.account_id,
            &Op::DeleteByMessageId {
                mailbox: message.mailbox,
                message_id: message.message_id,
            },
        )?;
    }

    Ok(())
}

/// The remote path of one mailbox, for naming a move's destination.
pub fn mailbox_path(tx: &Transaction<'_>, mailbox_id: i64) -> Result<Option<String>, DbError> {
    Ok(tx
        .query_row(
            "SELECT remote_path FROM mailbox WHERE id = ?1",
            params![mailbox_id],
            |row| row.get(0),
        )
        .ok())
}

/// Queues one operation. Call inside the transaction that makes the local change.
/// Takes `uids` out of any queued move that would take them out of `from`.
///
/// A UID can leave a mailbox once. Moving a message that already has an unfinished move left
/// both operations queued, and they cannot both be right: the first ran, the UID changed, and
/// the second could no longer find the message — so the server performed the first move and
/// never the second. The returning message then failed to match the parked row, which had
/// since moved somewhere else, and was inserted beside it. One message, two rows, for good,
/// because `remove_missing` ignores `uid <= 0`.
///
/// Rewriting rather than appending is what makes the second move *replace* the first instead
/// of racing it. An operation left with no UIDs is deleted; a batch that still has other
/// messages in it keeps them.
fn supersede_move(
    tx: &Transaction<'_>,
    account_id: i64,
    from: &str,
    uids: &[u32],
) -> Result<(), DbError> {
    // Collected before writing: the statement borrows the transaction, and the rewrite below
    // writes to the same table it is reading.
    let queued: Vec<(i64, String)> = {
        let mut statement = tx.prepare(
            "SELECT id, payload_json FROM pending_op WHERE account_id = ?1 AND kind = 'move'",
        )?;

        let rows = statement.query_map(params![account_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;

        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    for (id, payload) in queued {
        // A payload this version cannot read is left alone rather than dropped. Standing rule
        // 13: an operation we do not understand is not an operation we may throw away.
        let Ok(Op::Move {
            from: queued_from,
            to,
            uids: queued_uids,
        }) = serde_json::from_str::<Op>(&payload)
        else {
            continue;
        };

        if queued_from != from {
            continue;
        }

        let kept: Vec<u32> = queued_uids
            .iter()
            .copied()
            .filter(|uid| !uids.contains(uid))
            .collect();

        if kept.len() == queued_uids.len() {
            continue;
        }

        if kept.is_empty() {
            tx.execute("DELETE FROM pending_op WHERE id = ?1", params![id])?;
            continue;
        }

        let rewritten = Op::Move {
            from: queued_from,
            to,
            uids: kept,
        };

        let payload = serde_json::to_string(&rewritten).map_err(|error| DbError::Encode {
            what: "pending_op payload",
            detail: error.to_string(),
        })?;

        tx.execute(
            "UPDATE pending_op SET payload_json = ?2 WHERE id = ?1",
            params![id, payload],
        )?;
    }

    Ok(())
}

/// Takes any queued save of this draft off the queue. See `enqueue`.
fn supersede_draft(tx: &Transaction<'_>, account_id: i64, message_id: &str) -> Result<(), DbError> {
    for (id, op) in queued(tx, account_id)? {
        if let Op::AppendDraft {
            message_id: queued_id,
            ..
        } = op
        {
            if queued_id == message_id {
                tx.execute("DELETE FROM pending_op WHERE id = ?1", params![id])?;
            }
        }
    }

    Ok(())
}

pub fn enqueue(tx: &Transaction<'_>, account_id: i64, op: &Op) -> Result<(), DbError> {
    if op.is_empty() {
        return Ok(());
    }

    // Every path that moves mail goes through here — move, archive, delete-to-trash and the
    // rules engine — so this is the one place that has to know a second move replaces a first.
    if let Op::Move { from, uids, .. } = op {
        supersede_move(tx, account_id, from, uids)?;
    }

    // And a draft saved again replaces its queued save, as a delete of it does. Each autosave
    // used to queue its own append, every one with the `replaces` that was known when it was
    // queued — none, until the first had run — so a few saves between pushes each appended a
    // fresh copy, Drafts filled with duplicates, and the copies were taken for another device's
    // edits (H5, 2026-11-14 audit). Every save writes the same file, so only the newest matters.
    if let Op::AppendDraft { message_id, .. } | Op::DeleteDraft { message_id, .. } = op {
        supersede_draft(tx, account_id, message_id)?;
    }

    let payload = serde_json::to_string(op).map_err(|error| DbError::Encode {
        what: "pending_op payload",
        detail: error.to_string(),
    })?;

    // What the UIDs in it mean. See `uid_mailbox` and `stale`.
    let validity: Option<i64> = match op.uid_mailbox() {
        Some(path) => {
            use rusqlite::OptionalExtension;
            tx.query_row(
                "SELECT uid_validity FROM mailbox WHERE account_id = ?1 AND remote_path = ?2",
                params![account_id, path],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten()
        }
        None => None,
    };

    tx.execute(
        "INSERT INTO pending_op (account_id, kind, payload_json, created_at, uid_validity)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            account_id,
            op.kind(),
            payload,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            validity,
        ],
    )?;

    Ok(())
}

/// Whether a mailbox's `UIDVALIDITY` has changed since an operation naming its UIDs was queued.
///
/// Unknown on either side is not a change: an operation queued before the value was recorded,
/// or a server that sends none, is applied as it always was.
fn stale(queued: Option<u32>, selected: Option<u32>) -> bool {
    matches!((queued, selected), (Some(q), Some(s)) if q != 0 && s != 0 && q != s)
}

/// Where the server has one local message right now, as far as this app can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OnServer {
    account_id: i64,
    mailbox_id: i64,
    path: String,
    /// Its UID there, when known. Not known for a message moved there by an operation that has
    /// already run, until the sync reads that mailbox and adopts it.
    uid: Option<u32>,
    message_id: Option<String>,
}

/// Where the server has a message: the answer `locate` gives, and the case it cannot.
///
/// A row moved here is *parked*: a negative UID, with its origin recorded, because until the
/// queued move runs the server still has it in the origin under the origin's UID. Once the move
/// has run the origin is out of date — the message is in the mailbox the row is in, under a UID
/// the sync has not learned yet — and nothing said so. `locate` went on naming the origin, so
/// every later move of that message, Undo included, asked the server to move a UID that was no
/// longer there (H7, 2026-11-14 audit). Which of the two is true is read from the queue: if an
/// operation that moves the message out of its origin is still waiting, it has not happened.
fn on_server(tx: &Transaction<'_>, id: i64) -> Result<Option<OnServer>, DbError> {
    use rusqlite::OptionalExtension;

    struct Row {
        account_id: i64,
        mailbox_id: i64,
        path: String,
        uid: i64,
        origin_id: Option<i64>,
        origin_uid: Option<i64>,
        message_id: Option<String>,
    }

    let row = tx
        .query_row(
            "SELECT message.account_id, message.mailbox_id, mailbox.remote_path, message.uid,
                    message.origin_mailbox_id, message.origin_uid, message.message_id
               FROM message JOIN mailbox ON mailbox.id = message.mailbox_id
              WHERE message.id = ?1",
            params![id],
            |row| {
                Ok(Row {
                    account_id: row.get(0)?,
                    mailbox_id: row.get(1)?,
                    path: row.get(2)?,
                    uid: row.get(3)?,
                    origin_id: row.get(4)?,
                    origin_uid: row.get(5)?,
                    message_id: row.get(6)?,
                })
            },
        )
        .optional()?;

    let Some(Row {
        account_id,
        mailbox_id,
        path,
        uid,
        origin_id,
        origin_uid,
        message_id,
    }) = row
    else {
        return Ok(None);
    };
    let message_id = message_id.filter(|value| !value.trim().is_empty());

    // On the server where it is here, under its own UID: not parked.
    if let Ok(uid) = u32::try_from(uid) {
        if uid == 0 {
            // Here and never on the server: nothing to tell it.
            return Ok(None);
        }
        return Ok(Some(OnServer {
            account_id,
            mailbox_id,
            path,
            uid: Some(uid),
            message_id,
        }));
    }

    // Parked. With no origin there is nothing to go on, and sending nothing is the safe kind of
    // wrong — as `locate` has always done.
    let Some(origin_id) = origin_id else {
        return Ok(None);
    };
    let Some((origin_account, origin_path)) = account_and_path(tx, origin_id)? else {
        return Ok(None);
    };

    let origin_uid = origin_uid
        .and_then(|uid| u32::try_from(uid).ok())
        .filter(|uid| *uid > 0);

    let still_queued = queued(tx, origin_account)?.iter().any(|(_, op)| match op {
        Op::Move { from, uids, .. } => {
            *from == origin_path && origin_uid.is_some_and(|uid| uids.contains(&uid))
        }
        Op::MoveByMessageId {
            from,
            message_id: queued_id,
            ..
        } => *from == origin_path && message_id.as_deref() == Some(queued_id.as_str()),
        _ => false,
    });

    if still_queued {
        return Ok(Some(OnServer {
            account_id: origin_account,
            mailbox_id: origin_id,
            path: origin_path,
            uid: origin_uid,
            message_id,
        }));
    }

    // The move ran. The server has it here, under a UID the next sync of this mailbox will learn.
    Ok(Some(OnServer {
        account_id,
        mailbox_id,
        path,
        uid: None,
        message_id,
    }))
}

/// Takes a queued move-by-Message-ID of this message out of `from` off the queue.
fn cancel_move_by_id(
    tx: &Transaction<'_>,
    account_id: i64,
    from: &str,
    message_id: &str,
) -> Result<(), DbError> {
    for (id, op) in queued(tx, account_id)? {
        if let Op::MoveByMessageId {
            from: queued_from,
            message_id: queued_id,
            ..
        } = op
        {
            if queued_from == from && queued_id == message_id {
                tx.execute("DELETE FROM pending_op WHERE id = ?1", params![id])?;
            }
        }
    }

    Ok(())
}

/// Messages a move has put back exactly where the server already had them, with the UID they
/// have there. See [`settle`].
#[derive(Debug, Default)]
#[must_use = "pass this to `ops::settle` after the local move"]
pub struct Homecoming(Vec<(i64, u32)>);

/// Queues the server side of moving these messages to `destination`. **Call before the local
/// write**, then [`settle`] with what it returns.
///
/// The one place a move of mail decides what the server must do, for every caller — Move,
/// Archive, Delete to Trash, a rule, Undo and Redo. Each used to work it out alone from
/// `locate`, and all of them skipped a message whose server copy was already in the
/// destination. That was wrong both ways a message gets there:
///
/// * **Its move out has not run yet.** Moving it back has to *cancel* that move. Skipping left it
///   queued, so a message archived and then dragged straight back, or archived and undone within
///   two seconds, was archived on the server anyway and the next sync took it away again.
/// * **Its move out has run.** Then it is not in its origin at all, and the only name it has is
///   its `Message-ID` — see [`on_server`].
///
/// A message whose earlier move is replaced by this one keeps one operation, never two: the
/// queued one is rewritten or removed first.
pub fn queue_move(
    tx: &Transaction<'_>,
    ids: &[i64],
    destination: i64,
) -> Result<Homecoming, DbError> {
    let mut homecoming = Homecoming::default();

    let Some(to) = mailbox_path(tx, destination)? else {
        return Ok(homecoming);
    };

    // By UID, grouped so one command covers each source mailbox, as `locate` grouped them.
    let mut by_uid: Vec<(i64, String, Vec<u32>)> = Vec::new();

    for &id in ids {
        let Some(server) = on_server(tx, id)? else {
            continue;
        };

        // Whatever was going to move it is replaced by this.
        if let Some(uid) = server.uid {
            supersede_move(tx, server.account_id, &server.path, &[uid])?;
        }
        if let Some(message_id) = &server.message_id {
            cancel_move_by_id(tx, server.account_id, &server.path, message_id)?;
        }

        // A parked row records where the server has it, so the next move starts from there.
        // Written now because `write::move_to` keeps a parked row's origin as it is.
        tx.execute(
            "UPDATE message SET origin_mailbox_id = ?2, origin_uid = ?3
              WHERE id = ?1 AND uid < 0",
            params![id, server.mailbox_id, server.uid],
        )?;

        if server.path == to {
            // Back where the server has it: nothing to send. With the UID known the row can
            // stop being parked once it is there; without it, the next sync adopts it.
            if let Some(uid) = server.uid {
                homecoming.0.push((id, uid));
            }
            continue;
        }

        match (server.uid, &server.message_id) {
            (Some(uid), _) => {
                match by_uid.iter_mut().find(|(account, path, _)| {
                    *account == server.account_id && *path == server.path
                }) {
                    Some((_, _, uids)) => uids.push(uid),
                    None => by_uid.push((server.account_id, server.path.clone(), vec![uid])),
                }
            }
            (None, Some(message_id)) => enqueue(
                tx,
                server.account_id,
                &Op::MoveByMessageId {
                    from: server.path.clone(),
                    to: to.clone(),
                    message_id: message_id.clone(),
                },
            )?,
            (None, None) => tracing::debug!(
                id,
                "move: no UID and no Message-ID to name the message by; not sent"
            ),
        }
    }

    for (account_id, from, uids) in by_uid {
        enqueue(
            tx,
            account_id,
            &Op::Move {
                from,
                to: to.clone(),
                uids,
            },
        )?;
    }

    Ok(homecoming)
}

/// After the local move: a message back exactly where the server has it takes its UID there
/// again and stops being parked, as though it had never left.
///
/// Skipped if another row already holds that UID in the mailbox; the sync sorts that out, and
/// a unique-constraint failure here would take the whole move with it.
pub fn settle(tx: &Transaction<'_>, homecoming: &Homecoming) -> Result<(), DbError> {
    for (id, uid) in &homecoming.0 {
        tx.execute(
            "UPDATE message
                SET uid = ?2, origin_mailbox_id = NULL, origin_uid = NULL
              WHERE id = ?1
                AND NOT EXISTS (
                    SELECT 1 FROM message AS other
                     WHERE other.mailbox_id = message.mailbox_id AND other.uid = ?2
                       AND other.id <> message.id)",
            params![id, uid],
        )?;
    }

    Ok(())
}

/// Everything the account has queued, oldest first.
///
/// Order matters and is not merely tidiness: flagging a message and then moving it must reach
/// the server in that order, because after the move the UID in the first operation no longer
/// resolves in the mailbox it names.
pub fn queued(tx: &Transaction<'_>, account_id: i64) -> Result<Vec<(i64, Op)>, DbError> {
    let mut statement = tx
        .prepare("SELECT id, payload_json FROM pending_op WHERE account_id = ?1 ORDER BY id ASC")?;

    let rows = statement.query_map(params![account_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut ops = Vec::new();

    for row in rows {
        let (id, payload) = row?;

        // A payload that will not parse is from a version of this app that wrote a shape we
        // no longer understand. Skipping it keeps the queue moving; the cleanup below drops
        // it so it is not re-read on every sync forever.
        match serde_json::from_str::<Op>(&payload) {
            Ok(op) => ops.push((id, op)),
            Err(error) => {
                tracing::warn!(id, %error, "pending_op payload could not be read; dropping it");
                tx.execute("DELETE FROM pending_op WHERE id = ?1", params![id])?;
            }
        }
    }

    Ok(ops)
}

/// How many operations are waiting. Used to decide whether a drain is worth a connection.
pub fn pending_count(tx: &Transaction<'_>, account_id: i64) -> Result<i64, DbError> {
    Ok(tx.query_row(
        "SELECT COUNT(*) FROM pending_op WHERE account_id = ?1",
        params![account_id],
        |row| row.get(0),
    )?)
}

pub(crate) fn forget(tx: &Transaction<'_>, id: i64) -> Result<(), DbError> {
    tx.execute("DELETE FROM pending_op WHERE id = ?1", params![id])?;
    Ok(())
}

/// Replaces a queued operation with a corrected version of itself, keeping its place.
pub(crate) fn rewrite(tx: &Transaction<'_>, id: i64, op: &Op) -> Result<(), DbError> {
    let payload = serde_json::to_string(op).map_err(|error| DbError::Encode {
        what: "pending_op payload",
        detail: error.to_string(),
    })?;

    tx.execute(
        "UPDATE pending_op SET kind = ?2, payload_json = ?3 WHERE id = ?1",
        params![id, op.kind(), payload],
    )?;

    Ok(())
}

/// Everything the account queued after operation `after`, oldest first.
///
/// Payloads this version cannot read are skipped here rather than dropped: `queued` and the
/// drain own that decision.
pub(crate) fn queued_after(
    tx: &Transaction<'_>,
    account_id: i64,
    after: i64,
) -> Result<Vec<(i64, Op)>, DbError> {
    let mut statement = tx.prepare(
        "SELECT id, payload_json FROM pending_op
          WHERE account_id = ?1 AND id > ?2
          ORDER BY id ASC",
    )?;

    let rows = statement.query_map(params![account_id, after], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut ops = Vec::new();
    for row in rows {
        let (id, payload) = row?;
        if let Ok(op) = serde_json::from_str::<Op>(&payload) {
            ops.push((id, op));
        }
    }

    Ok(ops)
}

/// The oldest readable operation after `after`, dropping unreadable ones on the way.
///
/// With the `UIDVALIDITY` recorded when it was queued. See `stale`.
fn next_queued(
    tx: &Transaction<'_>,
    account_id: i64,
    mut after: i64,
) -> Result<Option<(i64, Op, Option<u32>)>, DbError> {
    use rusqlite::OptionalExtension;

    loop {
        let row: Option<(i64, String, Option<i64>)> = tx
            .query_row(
                "SELECT id, payload_json, uid_validity FROM pending_op
                  WHERE account_id = ?1 AND id > ?2
                  ORDER BY id ASC LIMIT 1",
                params![account_id, after],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;

        let Some((id, payload, validity)) = row else {
            return Ok(None);
        };

        match serde_json::from_str::<Op>(&payload) {
            Ok(op) => {
                let validity = validity.and_then(|v| u32::try_from(v).ok());
                return Ok(Some((id, op, validity)));
            }
            Err(error) => {
                tracing::warn!(id, %error, "pending_op payload could not be read; dropping it");
                forget(tx, id)?;
                after = id;
            }
        }
    }
}

/// A mailbox change the server turned down, as the window is told about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Refused {
    pub account_id: i64,
    /// What was refused, what happens now, and the server's own words.
    pub message: String,
}

/// Gives up on a refused mailbox change and puts the local tree back the way the server has it.
///
/// Returns the sentence the user is shown.
fn give_up(
    tx: &Transaction<'_>,
    account_id: i64,
    id: i64,
    op: &Op,
    words: &str,
) -> Result<String, DbError> {
    use super::folders;
    use super::mailboxes::display_name;

    forget(tx, id)?;

    let message = match op {
        Op::CreateMailbox { path } => {
            let name = folders::abandon_created(tx, account_id, id, path)?;
            format!("The server would not create the mailbox “{name}”, so it has been removed. The server said: {words}")
        }
        Op::RenameMailbox {
            from,
            to,
            delimiter,
        } => {
            folders::abandon_rename(tx, account_id, id, from, to, delimiter.as_deref())?;
            format!(
                "The server would not rename “{}” to “{}”, so it keeps its old name. The server said: {words}",
                display_name(from, delimiter.as_deref()),
                display_name(to, delimiter.as_deref()),
            )
        }
        Op::DeleteMailbox { paths, delimiter } => {
            // Deepest first, so the folder the user chose is the last one listed.
            let name = paths
                .last()
                .map(|path| display_name(path, delimiter.as_deref()))
                .unwrap_or_default();
            format!("The server would not delete the mailbox “{name}”. It will reappear the next time Halcyon checks for mail. The server said: {words}")
        }
        Op::EraseMailbox { mailbox } => {
            let name: String = tx
                .query_row(
                    "SELECT display_name FROM mailbox WHERE account_id = ?1 AND remote_path = ?2",
                    params![account_id, mailbox],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| display_name(mailbox, None));
            format!("The server would not erase the messages in “{name}”, so they may still be there. The server said: {words}")
        }
        _ => words.to_string(),
    };

    Ok(message)
}

/// Records a failure, and gives up once the operation has had its chances.
///
/// Returns true when the row was dropped rather than kept.
fn record_failure(tx: &Transaction<'_>, id: i64, error: &str) -> Result<bool, DbError> {
    let attempts: i64 = tx.query_row(
        "SELECT attempts + 1 FROM pending_op WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )?;

    if attempts >= MAX_ATTEMPTS {
        forget(tx, id)?;
        return Ok(true);
    }

    tx.execute(
        "UPDATE pending_op SET attempts = ?2, last_error = ?3 WHERE id = ?1",
        params![id, attempts, error],
    )?;

    Ok(false)
}

/// Issues one `UID STORE` and consumes its response.
///
/// A function rather than an inline loop so the response stream — which borrows the session —
/// is dropped before the caller needs the session again. `UID STORE` returns the new flags for
/// every message touched, and leaving those unread would desynchronise the connection: the
/// next command would read them as its own answer. `.SILENT` asks the server not to send them,
/// but a server is free to ignore that, so they are drained either way.
async fn store_flags(
    session: &mut ImapSession,
    set: &str,
    instruction: String,
) -> Result<(), SyncError> {
    use futures::StreamExt;

    let mut stream = session.uid_store(set, instruction).await?;
    while let Some(item) = stream.next().await {
        item?;
    }

    Ok(())
}

/// Sets and clears `\Seen` and `\Flagged` on a set of UIDs in the selected mailbox.
///
/// Set and clear are separate commands: IMAP's `+FLAGS` and `-FLAGS` cannot be combined, and
/// `FLAGS` without a sign would replace the whole set — wiping \Answered and every keyword the
/// user never touched.
async fn store_flag_changes(
    session: &mut ImapSession,
    set: &str,
    seen: Option<bool>,
    flagged: Option<bool>,
) -> Result<(), SyncError> {
    let mut add: Vec<&str> = Vec::new();
    let mut remove: Vec<&str> = Vec::new();

    for (value, name) in [(seen, "\\Seen"), (flagged, "\\Flagged")] {
        match value {
            Some(true) => add.push(name),
            Some(false) => remove.push(name),
            None => {}
        }
    }

    if !add.is_empty() {
        store_flags(session, set, format!("+FLAGS.SILENT ({})", add.join(" "))).await?;
    }
    if !remove.is_empty() {
        store_flags(
            session,
            set,
            format!("-FLAGS.SILENT ({})", remove.join(" ")),
        )
        .await?;
    }

    Ok(())
}

/// Applies one operation to the server.
/// What applying one operation turned up. Only drafts have anything to say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Set when the server held a copy of this draft that we did not put there.
    pub conflicting_draft: Option<String>,
    /// The `Message-ID` and UID of a draft just appended, so the next save can replace it.
    ///
    /// Returned rather than written here because `apply` has a session and no database. The
    /// caller records it, in the same place it records a conflict.
    pub appended_draft: Option<(String, u32)>,
}

/// Selects the mailbox an operation names by UID, and says whether its UIDs still mean what they
/// meant when it was queued.
///
/// False after the server has reset the mailbox's `UIDVALIDITY`: the same numbers now name
/// other messages, and a Delete queued before the reset expunged whichever of them had the number
/// (M3, 2026-11-14 audit). Such an operation is dropped instead. The change it carried is lost,
/// which is what a reset means for every client; the sync drops the mailbox's copy and reads it
/// again the next time it reaches it.
async fn select_for_uids(
    session: &mut ImapSession,
    mailbox: &str,
    validity: Option<u32>,
) -> Result<bool, SyncError> {
    let selected = session.select(mailbox).await?;

    if stale(validity, selected.uid_validity) {
        tracing::warn!(
            mailbox,
            queued = validity,
            found = selected.uid_validity,
            "the mailbox was reset after this change was queued; its UIDs name other mail now, so it is dropped"
        );
        return Ok(false);
    }

    Ok(true)
}

async fn apply(
    session: &mut ImapSession,
    op: &Op,
    validity: Option<u32>,
    has_move: bool,
) -> Result<Applied, SyncError> {
    match op {
        Op::Flag {
            mailbox,
            uids,
            seen,
            flagged,
        } => {
            if !select_for_uids(session, mailbox, validity).await? {
                return Ok(Applied::default());
            }

            // Several commands when the set is long, each one whole. A retry after a failure
            // part-way repeats the ones that landed, which for flags changes nothing.
            for set in sequence_sets(uids) {
                store_flag_changes(session, &set, *seen, *flagged).await?;
            }
        }

        Op::FlagByMessageId {
            mailbox,
            message_id,
            seen,
            flagged,
        } => {
            // The newest copy, as `MoveByMessageId` takes: it is the one the move put there.
            let found = other_copies(session, mailbox, message_id, None).await?;
            match found.iter().max() {
                Some(uid) => {
                    store_flag_changes(session, &uid.to_string(), *seen, *flagged).await?;
                }
                None => tracing::debug!(
                    mailbox,
                    "flag by Message-ID: not in the mailbox any more; nothing to change"
                ),
            }
        }

        Op::DeleteByMessageId {
            mailbox,
            message_id,
        } => {
            let found = match other_copies(session, mailbox, message_id, None).await {
                Ok(found) => found,
                Err(SyncError::Imap(error)) if nonexistent(&error) => Vec::new(),
                Err(error) => return Err(error),
            };
            if let Some(uid) = found.iter().max() {
                let set = uid.to_string();
                store_flags(session, &set, "+FLAGS.SILENT (\\Deleted)".into()).await?;
                expunge(session, &set).await?;
            }
        }

        Op::Move { from, to, uids } => {
            if !select_for_uids(session, from, validity).await? {
                return Ok(Applied::default());
            }
            move_selected(session, uids, to, has_move).await?;
        }

        Op::MoveByMessageId {
            from,
            to,
            message_id,
        } => {
            // `other_copies` selects `from` and searches it; a search the server refuses counts
            // as nothing found, and a move of nothing is done.
            let found = other_copies(session, from, message_id, None).await?;

            // The newest copy, if somehow there are several: it is the one this app put there.
            // Moving every copy would take along a duplicate the user filed there themselves.
            match found.iter().max() {
                Some(uid) => move_selected(session, &[*uid], to, has_move).await?,
                None => tracing::debug!(
                    from,
                    "move by Message-ID: not in the mailbox any more; nothing to move"
                ),
            }
        }

        Op::Delete { mailbox, uids } => {
            if !select_for_uids(session, mailbox, validity).await? {
                return Ok(Applied::default());
            }

            for set in sequence_sets(uids) {
                store_flags(session, &set, "+FLAGS.SILENT (\\Deleted)".into()).await?;
                expunge(session, &set).await?;
            }
        }

        Op::AppendDraft {
            mailbox,
            eml_path,
            replaces,
            message_id,
        } => {
            // A draft that has since been sent or discarded leaves its file behind for a
            // moment. Nothing to append is not a failure — it is a queue that has been
            // overtaken, and treating it as an error would block everything behind it.
            let Ok(raw) = std::fs::read(eml_path) else {
                tracing::debug!(eml_path, "draft file is gone; nothing to append");
                return Ok(Applied::default());
            };

            // Asked *before* anything is written: which copies of this draft does the server
            // already hold? Anything that is not the one we are replacing was appended by
            // another device, which means the same draft was edited in two places.
            //
            // Checked first rather than after the append, or our own new copy would be in the
            // answer and every single save would look like a conflict.
            let theirs = other_copies(session, mailbox, message_id, *replaces).await?;

            // The new copy first. If the append fails the old draft is still on the server,
            // whereas deleting first and failing to append loses whatever was typed.
            session
                .append(
                    super::session::append_name(mailbox),
                    Some("(\\Draft \\Seen)"),
                    None,
                    &raw,
                )
                .await?;

            // Our own previous copy is replaced. The other device's copy is **left alone**:
            // whichever one we deleted would be work somebody did, and the person who did it
            // is the only one who can say which version matters.
            if let Some(old) = replaces {
                session.select(mailbox).await?;
                let set = old.to_string();
                store_flags(session, &set, "+FLAGS.SILENT (\\Deleted)".into()).await?;
                expunge(session, &set).await?;
            }

            // Which UID the append landed on, so the *next* save can delete this copy.
            //
            // `draft.remote_uid` is what `replaces` is read from, and nothing anywhere wrote
            // it -- the column existed, the schema comment explained exactly what it was for
            // ("without it, thirty seconds of typing produces one draft per save in every
            // other client the user owns"), and the value was never stored. So `replaces` was
            // always None, the branch above never ran, and every autosave left another copy
            // on the server. A ten-minute message wrote twenty of them.
            //
            // It also broke the conflict check: `other_copies` excludes only `ours`, so with
            // `replaces` None every copy this app had appended looked like another device's
            // work, and from the second save onwards the draft was flagged as edited in two
            // places. The warning was about copies it had made itself.
            //
            // Found by search rather than APPENDUID: async-imap does not surface the UIDPLUS
            // response, and a search for the draft's own `Message-ID` needs no extension. The
            // new copy is whatever carries that id and was not there a moment ago.
            let appended = match other_copies(session, mailbox, message_id, None).await {
                Ok(found) => found
                    .into_iter()
                    .filter(|uid| !theirs.contains(uid))
                    .max()
                    .map(|uid| (message_id.clone(), uid)),
                Err(error) => {
                    // Not a failure of the append, which has already happened. The next save
                    // appends again without replacing, which is the old behaviour for one
                    // round rather than for ever.
                    tracing::debug!(%error, message_id, "could not learn the appended draft UID");
                    None
                }
            };

            if !theirs.is_empty() {
                tracing::info!(
                    message_id,
                    copies = theirs.len(),
                    "the same draft was edited elsewhere; both copies kept"
                );

                return Ok(Applied {
                    conflicting_draft: Some(message_id.clone()),
                    appended_draft: appended,
                });
            }

            return Ok(Applied {
                conflicting_draft: None,
                appended_draft: appended,
            });
        }

        Op::DeleteDraft {
            mailbox,
            message_id,
        } => {
            // Every copy with the draft's id: ours, and any another device saved, since a draft
            // that has been sent or thrown away is done with everywhere. A mailbox that is not
            // there, or a search the server refuses, finds nothing — and nothing is what there is
            // to delete then.
            let copies = match other_copies(session, mailbox, message_id, None).await {
                Ok(copies) => copies,
                Err(SyncError::Imap(error)) if nonexistent(&error) => Vec::new(),
                Err(error) => return Err(error),
            };

            for set in sequence_sets(&copies) {
                store_flags(session, &set, "+FLAGS.SILENT (\\Deleted)".into()).await?;
                expunge(session, &set).await?;
            }
        }

        // Each of the four below is written to be run twice. The drain forgets an operation
        // only after it has succeeded, so a crash or a dropped connection between the command
        // and the forgetting sends it again — and "that folder already exists" is then the
        // answer to a question already settled, not a failure.
        Op::CreateMailbox { path } => {
            if let Err(error) = session.create(path).await {
                if !already_exists(&error) {
                    return Err(error.into());
                }
            }

            // Subscribed too, so a client that shows only subscribed folders — Thunderbird's
            // default — shows this one. A server refusing the subscription still made the folder.
            if let Err(error) = session.subscribe(path).await {
                tracing::debug!(%error, "SUBSCRIBE refused for a new mailbox");
            }
        }

        Op::RenameMailbox { from, to, .. } => {
            leave_selected(session).await;

            if let Err(error) = session.rename(from, to).await {
                // The rename already happened: the old name is gone and the new one is there.
                let done = nonexistent(&error) && exists(session, to).await?;
                if !done {
                    return Err(error.into());
                }
            }

            // Subscriptions do not reliably follow a rename. Neither refusal changes the folder.
            if let Err(error) = session.unsubscribe(from).await {
                tracing::debug!(%error, "UNSUBSCRIBE refused for a renamed mailbox");
            }
            if let Err(error) = session.subscribe(to).await {
                tracing::debug!(%error, "SUBSCRIBE refused for a renamed mailbox");
            }
        }

        Op::DeleteMailbox { paths, .. } => {
            leave_selected(session).await;

            for path in paths {
                if let Err(error) = session.delete(path).await {
                    if !nonexistent(&error) {
                        return Err(error.into());
                    }
                }

                if let Err(error) = session.unsubscribe(path).await {
                    tracing::debug!(%error, "UNSUBSCRIBE refused for a deleted mailbox");
                }
            }
        }

        Op::EraseMailbox { mailbox } => {
            let selected = match session.select(mailbox).await {
                Ok(selected) => selected,
                // A mailbox that is not there has nothing in it to erase.
                Err(error) if nonexistent(&error) => return Ok(Applied::default()),
                Err(error) => return Err(error.into()),
            };

            // `1:*` in an empty mailbox is an error on some servers rather than a no-op.
            if selected.exists > 0 {
                store_flags(session, "1:*", "+FLAGS.SILENT (\\Deleted)".into()).await?;
                expunge(session, "1:*").await?;
            }
        }
    }

    Ok(Applied::default())
}

/// Whether a refusal says one of these things, in the server's own words or its RFC 5530 code.
///
/// `async-imap` does not parse codes it does not know — `ALREADYEXISTS`, `NONEXISTENT` — so they
/// arrive inside the text, which is where this looks.
fn says(error: &async_imap::error::Error, needles: &[&str]) -> bool {
    let text = match error {
        async_imap::error::Error::No(text) | async_imap::error::Error::Bad(text) => text,
        _ => return false,
    };

    let upper = text.to_ascii_uppercase();
    needles.iter().any(|needle| upper.contains(needle))
}

/// `NO [ALREADYEXISTS]`, or Gmail's "Duplicate folder name".
fn already_exists(error: &async_imap::error::Error) -> bool {
    says(
        error,
        &["ALREADYEXISTS", "ALREADY EXISTS", "DUPLICATE FOLDER"],
    )
}

/// `NO [NONEXISTENT]`, or one of the ways servers say it without the code.
fn nonexistent(error: &async_imap::error::Error) -> bool {
    says(
        error,
        &[
            "NONEXISTENT",
            "DOESN'T EXIST",
            "DOES NOT EXIST",
            "UNKNOWN MAILBOX",
            "NO SUCH MAILBOX",
            "MAILBOX NOT FOUND",
        ],
    )
}

/// Whether the server lists a mailbox by exactly this name.
async fn exists(session: &mut ImapSession, path: &str) -> Result<bool, SyncError> {
    use futures::StreamExt;

    let mut found = false;
    let mut stream = session.list(Some(""), Some(path)).await?;

    while let Some(item) = stream.next().await {
        if item?.name() == path {
            found = true;
        }
    }

    Ok(found)
}

/// Moves the connection off whatever mailbox it last selected.
///
/// Some servers refuse to rename or delete the mailbox a connection has open, and the drain may
/// have just selected it for an earlier operation. `EXAMINE INBOX` is harmless — read-only, and
/// every server has one. If even that fails the command that follows will say why.
async fn leave_selected(session: &mut ImapSession) {
    if let Err(error) = session.examine("INBOX").await {
        tracing::debug!(%error, "could not step off the selected mailbox");
    }
}

/// The server's own sentence from a refusal, without the library's wrapping.
///
/// `async-imap` renders a refusal as `code: None, info: Some("…")`. The sentence inside is what
/// the user can act on, and the wrapping is noise, so it is taken out — and the whole string is
/// kept if the shape is ever different.
pub fn server_words(error: &SyncError) -> String {
    let text = match error {
        SyncError::Imap(
            async_imap::error::Error::No(text) | async_imap::error::Error::Bad(text),
        ) => text.as_str(),
        other => return other.to_string(),
    };

    let Some(start) = text.find("info: Some(\"") else {
        return text.to_string();
    };
    let inner = &text[start + "info: Some(\"".len()..];
    let Some(end) = inner.rfind("\")") else {
        return text.to_string();
    };

    let words = inner[..end].replace("\\\"", "\"").replace("\\\\", "\\");
    without_timing(&words).to_string()
}

/// Drops the timing Dovecot appends to every reply — "… exists (0.001 + 0.000 secs)." — which
/// is for its administrators and reads as noise in a sentence meant for the user.
fn without_timing(words: &str) -> &str {
    let Some(open) = words.rfind(" (") else {
        return words;
    };

    let tail = &words[open + 2..];
    let Some(figures) = tail.strip_suffix(" secs).") else {
        return words;
    };

    if figures
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | '+' | ' '))
    {
        &words[..open]
    } else {
        words
    }
}

/// Whether the server turned an operation down, as opposed to not hearing it.
fn is_refusal(error: &SyncError) -> bool {
    matches!(
        error,
        SyncError::Imap(async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_))
    )
}

/// UIDs in `mailbox` carrying `message_id`, other than `ours`.
///
/// A search failure is reported as "no other copies" rather than as an error. Not every server
/// indexes `Message-ID` for `SEARCH HEADER`, and a draft that cannot be saved because its
/// conflict check failed would be a far worse outcome than a conflict that goes unnoticed.
async fn other_copies(
    session: &mut ImapSession,
    mailbox: &str,
    message_id: &str,
    ours: Option<u32>,
) -> Result<Vec<u32>, SyncError> {
    session.select(mailbox).await?;

    // Quotes and backslashes escaped: a `Message-ID` is generated by us, but this same path
    // will one day carry one that came from a server, and an unescaped quote would end the
    // search string early and change what is being asked.
    let escaped = message_id.replace('\\', "\\\\").replace('"', "\\\"");

    let found = match session
        .uid_search(format!("HEADER Message-ID \"{escaped}\""))
        .await
    {
        Ok(found) => found,
        Err(error) => {
            tracing::debug!(%error, "draft conflict search refused; assuming no other copies");
            return Ok(Vec::new());
        }
    };

    Ok(found.into_iter().filter(|uid| Some(*uid) != ours).collect())
}

/// Moves these UIDs out of the selected mailbox into `to`.
async fn move_selected(
    session: &mut ImapSession,
    uids: &[u32],
    to: &str,
    has_move: bool,
) -> Result<(), SyncError> {
    // A retry after a failure part-way names UIDs that have already left, and a UID command
    // ignores UIDs that are not there (RFC 3501 §6.4.8).
    for set in sequence_sets(uids) {
        if has_move {
            session.uid_mv(&set, to).await?;
        } else {
            // The three-step fallback docs/03 §5 describes. Copy first: if it fails the message
            // is still in one place, whereas deleting first and failing to copy loses it outright.
            session
                .uid_copy(&set, super::session::copy_name(to))
                .await?;
            store_flags(session, &set, "+FLAGS.SILENT (\\Deleted)".into()).await?;
            expunge(session, &set).await?;
        }
    }

    Ok(())
}

/// Expunges just the UIDs named, where the server allows it.
///
/// A bare `EXPUNGE` removes **every** message flagged `\Deleted` in the mailbox, including
/// ones another client flagged and has not expunged yet. `UID EXPUNGE` (UIDPLUS) is the
/// narrow version. Falling back to the broad one is still better than leaving the message
/// flagged-but-present, which is what the user sees as "delete did nothing".
async fn expunge(session: &mut ImapSession, set: &str) -> Result<(), SyncError> {
    use futures::StreamExt;

    let narrow = match session.uid_expunge(set).await {
        Ok(stream) => {
            // The expunge response is not `Unpin`, so it has to be pinned before it can be
            // polled. `pin_mut!` pins it on the stack, which is what a stream consumed here
            // and dropped here wants.
            futures::pin_mut!(stream);
            while let Some(item) = stream.next().await {
                item?;
            }
            true
        }
        Err(error) => {
            tracing::debug!(%error, "UID EXPUNGE refused; falling back to EXPUNGE");
            false
        }
    };

    if narrow {
        return Ok(());
    }

    let stream = session.expunge().await?;
    futures::pin_mut!(stream);
    while let Some(item) = stream.next().await {
        item?;
    }

    Ok(())
}

/// Pushes everything queued for an account, oldest first.
///
/// Returns how many operations were sent. Stops at the first operation that fails rather than
/// skipping past it, for the ordering reason in [`queued`] — with one exception: a mailbox
/// change the server *refused* is given up on at once, the local tree is put back, and the
/// window is told why (`mailbox:refused`). A refusal of a name is final, and holding the queue
/// behind it would stop every other change for five syncs before saying anything.
pub async fn drain(
    events: &dyn Events,
    db: &Db,
    session: &mut ImapSession,
    account_id: i64,
    has_move: bool,
) -> Result<usize, SyncError> {
    let waiting = db.write(move |tx| pending_count(tx, account_id)).await?;

    if waiting == 0 {
        return Ok(0);
    }

    tracing::debug!(account_id, queued = waiting, "draining pending operations");

    let mut sent = 0usize;
    let mut after = 0i64;

    // One at a time, not a list read up front: giving up on a refused mailbox change rewrites
    // the operations queued behind it, and a list taken at the start would send the versions
    // that no longer exist.
    loop {
        let Some((id, op, validity)) = db
            .write(move |tx| next_queued(tx, account_id, after))
            .await?
        else {
            break;
        };
        after = id;

        // A draft's previous copy is the one its row says *now*, not the one known when the save
        // was queued: an append that ran since has recorded a newer UID, and a draft opened from
        // the server has one from the start. Taken at enqueue time it was none, so the copy was
        // never replaced (H5, 2026-11-14 audit).
        let op = match op {
            Op::AppendDraft {
                mailbox,
                eml_path,
                replaces,
                message_id,
            } => {
                let lookup = message_id.clone();
                let current = db
                    .read(move |conn| {
                        Ok(conn
                            .query_row(
                                "SELECT remote_uid FROM draft WHERE message_id = ?1",
                                params![lookup],
                                |row| row.get::<_, Option<i64>>(0),
                            )
                            .ok()
                            .flatten()
                            .and_then(|uid| u32::try_from(uid).ok()))
                    })
                    .await?;

                Op::AppendDraft {
                    mailbox,
                    eml_path,
                    replaces: current.or(replaces),
                    message_id,
                }
            }
            other => other,
        };

        match apply(session, &op, validity, has_move).await {
            Ok(applied) => {
                // Recorded before the op is forgotten, so a crash between the two leaves the
                // op queued and the conflict found again rather than lost.
                if let Some(message_id) = applied.conflicting_draft {
                    db.write(move |tx| mark_draft_conflict(tx, &message_id))
                        .await?;
                }

                // Likewise before forgetting the op. Losing this leaves a copy on the server
                // that the next save will not replace, which is the bug this fixes.
                if let Some((message_id, uid)) = applied.appended_draft {
                    db.write(move |tx| record_draft_uid(tx, &message_id, uid))
                        .await?;
                }

                db.write(move |tx| forget(tx, id)).await?;
                sent += 1;
            }

            Err(error) if op.is_structural() && is_refusal(&error) => {
                tracing::warn!(id, %error, "the server refused a mailbox change; giving up on it");

                // Ended as a sentence, since it ends one.
                let mut words = server_words(&error);
                if !words.ends_with(['.', '!', '?']) {
                    words.push('.');
                }
                let message = db
                    .write(move |tx| give_up(tx, account_id, id, &op, &words))
                    .await?;

                events.emit(
                    "mailbox:refused",
                    payload(&Refused {
                        account_id,
                        message,
                    }),
                );
                events.emit("mailboxes:changed", payload(&account_id));
            }

            Err(error) => {
                let detail = error.to_string();
                let dropped = db.write(move |tx| record_failure(tx, id, &detail)).await?;

                if dropped {
                    tracing::warn!(
                        id,
                        %error,
                        "pending operation failed too many times; dropping it"
                    );
                    // Dropped, so it no longer blocks the queue — carry on with the rest.
                    continue;
                }

                tracing::warn!(id, %error, "pending operation failed; will retry");

                // Stop rather than skip. Later operations may depend on this one having
                // landed, and pushing them out of order can move a message the server still
                // believes is somewhere else.
                return Err(error);
            }
        }
    }

    tracing::debug!(account_id, sent, "pending operations drained");
    Ok(sent)
}

/// Flags a draft as having been edited in two places at once.
///
/// A timestamp rather than a boolean: the compose window shows the banner only for a conflict
/// newer than the copy it is holding, so an old conflict the user already dealt with does not
/// reappear every time they reopen the draft.
/// Remembers which UID a draft's last APPEND landed on.
///
/// Keyed on `message_id` rather than the row id because that is what the queued operation
/// carries, and `ix_draft_message_id` makes it unique. A draft deleted while its append was in
/// flight updates nothing, which is correct: there is no next save to replace anything.
fn record_draft_uid(
    tx: &rusqlite::Transaction<'_>,
    message_id: &str,
    uid: u32,
) -> Result<(), DbError> {
    tx.execute(
        "UPDATE draft SET remote_uid = ?2 WHERE message_id = ?1",
        rusqlite::params![message_id, i64::from(uid)],
    )?;

    Ok(())
}

fn mark_draft_conflict(tx: &rusqlite::Transaction<'_>, message_id: &str) -> Result<(), DbError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);

    tx.execute(
        "UPDATE draft SET conflict_at = ?2 WHERE message_id = ?1",
        rusqlite::params![message_id, now],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate;

    fn store() -> rusqlite::Connection {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragma");
        migrate::run(&mut conn).expect("migrate");

        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'Test', 'ada@example.test', 'other', 'password', 'halcyon:ada')",
            [],
        )
        .expect("account");

        conn
    }

    fn flag(uids: Vec<u32>, seen: Option<bool>) -> Op {
        Op::Flag {
            mailbox: "INBOX".into(),
            uids,
            seen,
            flagged: None,
        }
    }

    #[test]
    fn a_sequence_set_lists_uids_rather_than_spanning_them() {
        // A range would touch every UID between the ones selected. The user picked three
        // messages; "4:900" is a different instruction entirely.
        assert_eq!(sequence_sets(&[4, 17, 900]), vec!["4,17,900"]);
        assert_eq!(sequence_sets(&[1]), vec!["1"]);
        assert!(sequence_sets(&[]).is_empty());
    }

    #[test]
    fn consecutive_uids_become_a_range_and_nothing_else_does() {
        assert_eq!(
            sequence_sets(&[7, 4, 5, 6, 9, 11, 12, 5]),
            vec!["4:7,9,11:12"],
            "sorted, deduplicated, and ranged only where nothing is skipped"
        );
        assert_eq!(
            sequence_sets(&[u32::MAX - 1, u32::MAX]),
            vec![format!("{}:{}", u32::MAX - 1, u32::MAX)]
        );
    }

    #[test]
    fn a_change_queued_by_uid_carries_what_those_uids_meant() {
        // M3 in the 2026-11-14 audit: a Delete queued before the server reset a mailbox expunged
        // whichever message had the number afterwards. The queue now records the UIDVALIDITY
        // each UID-naming change was queued under, and the drain compares it with SELECT's.
        let mut conn = store();
        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, uid_validity)
             VALUES (10, 1, 'INBOX', 'Inbox', 777), (11, 1, 'Archive', 'Archive', 888)",
            [],
        )
        .expect("mailboxes");
        let tx = conn.transaction().expect("tx");

        enqueue(&tx, 1, &flag(vec![7], Some(true))).expect("flag");
        enqueue(
            &tx,
            1,
            &Op::Move {
                from: "Archive".into(),
                to: "INBOX".into(),
                uids: vec![4],
            },
        )
        .expect("move");
        enqueue(
            &tx,
            1,
            &Op::Delete {
                mailbox: "INBOX".into(),
                uids: vec![9],
            },
        )
        .expect("delete");
        enqueue(
            &tx,
            1,
            &Op::CreateMailbox {
                path: "Receipts".into(),
            },
        )
        .expect("create");
        // A mailbox this store has never seen has no value to record.
        enqueue(
            &tx,
            1,
            &Op::Flag {
                mailbox: "Unknown".into(),
                uids: vec![1],
                seen: Some(true),
                flagged: None,
            },
        )
        .expect("unknown");

        let mut recorded = Vec::new();
        let mut after = 0;
        while let Some((id, op, validity)) = next_queued(&tx, 1, after).expect("next") {
            recorded.push((op.kind(), validity));
            after = id;
        }

        assert_eq!(
            recorded,
            vec![
                ("flag", Some(777)),
                ("move", Some(888)),
                ("delete", Some(777)),
                ("mailboxCreate", None),
                ("flag", None),
            ],
            "a move is checked against the mailbox it moves out of"
        );
    }

    #[test]
    fn a_change_to_a_moved_message_follows_it_to_where_it_went() {
        // M4 in the 2026-11-14 audit. A message archived a moment ago is parked in Archive; the
        // server gives it a new UID there that the sync has not learned. A star on it was sent to
        // the Inbox under its old UID, which named nothing by the time it ran — the move is queued
        // ahead of it — so the server changed nothing and the queue forgot it as sent.
        let mut conn = store();
        conn.execute_batch(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name)
             VALUES (10, 1, 'INBOX', 'Inbox'), (11, 1, 'Archive', 'Archive');
             INSERT INTO message (id, account_id, mailbox_id, uid, date_sent, date_received,
                                  message_id, origin_mailbox_id, origin_uid)
             VALUES (1, 1, 11, -1, 0, 0, 'moved@example.test', 10, 5),
                    (2, 1, 10, 6, 0, 0, 'stayed@example.test', NULL, NULL),
                    (3, 1, 11, -3, 0, 0, NULL, 10, 7);",
        )
        .expect("rows");
        let tx = conn.transaction().expect("tx");

        let (groups, unnumbered) = resolve(&tx, &[1, 2, 3]).expect("resolve");
        assert_eq!(
            unnumbered,
            vec![Unnumbered {
                account_id: 1,
                mailbox: "Archive".into(),
                message_id: "moved@example.test".into(),
            }],
            "named by its Message-ID, where it is now"
        );
        assert_eq!(
            groups
                .iter()
                .map(|group| (group.mailbox.as_str(), group.uids.clone()))
                .collect::<Vec<_>>(),
            vec![("INBOX", vec![6, 7])],
            "a message that never moved keeps its UID, and one with no Message-ID has only its \
             origin to go by"
        );

        queue_flags(&tx, &[1, 2], None, Some(true)).expect("flag");
        queue_delete(&tx, &[1]).expect("delete");

        let queued: Vec<Op> = queued(&tx, 1)
            .expect("queued")
            .into_iter()
            .map(|(_, op)| op)
            .collect();
        assert_eq!(
            queued,
            vec![
                Op::Flag {
                    mailbox: "INBOX".into(),
                    uids: vec![6],
                    seen: None,
                    flagged: Some(true),
                },
                Op::FlagByMessageId {
                    mailbox: "Archive".into(),
                    message_id: "moved@example.test".into(),
                    seen: None,
                    flagged: Some(true),
                },
                Op::DeleteByMessageId {
                    mailbox: "Archive".into(),
                    message_id: "moved@example.test".into(),
                },
            ]
        );

        // And a sync of Archive meanwhile does not bring back what is being deleted.
        let removals = unsent_removals(&tx, 1, "Archive").expect("removals");
        assert!(removals.contains_message(999, Some("moved@example.test")));
    }

    #[test]
    fn only_a_known_change_of_uidvalidity_makes_a_queued_change_stale() {
        assert!(stale(Some(777), Some(778)), "a reset");
        assert!(!stale(Some(777), Some(777)));
        // Queued before the column existed, or on a server that sends none: as before.
        assert!(!stale(None, Some(777)));
        assert!(!stale(Some(777), None));
        assert!(!stale(Some(0), Some(777)));
    }

    #[test]
    fn what_is_queued_against_a_mailbox_is_known_by_its_path() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        enqueue(
            &tx,
            1,
            &Op::Move {
                from: "INBOX".into(),
                to: "Archive".into(),
                uids: vec![4, 5],
            },
        )
        .expect("queue");
        enqueue(
            &tx,
            1,
            &Op::Delete {
                mailbox: "INBOX".into(),
                uids: vec![9],
            },
        )
        .expect("queue");
        enqueue(&tx, 1, &flag(vec![7], Some(true))).expect("queue");

        let removals = unsent_removals(&tx, 1, "INBOX").expect("removals");
        assert_eq!(
            removals.uids,
            [4, 5, 9]
                .into_iter()
                .collect::<std::collections::HashSet<u32>>()
        );
        assert!(!removals.everything);
        assert!(!removals.contains(7), "a flag change is not a removal");
        assert_eq!(
            unsent_removals(&tx, 1, "Archive").expect("removals"),
            UnsentRemovals::default(),
            "arriving is not leaving"
        );

        assert!(names_mailbox(&tx, 1, "INBOX").expect("names"));
        assert!(names_mailbox(&tx, 1, "Archive").expect("names"));
        assert!(!names_mailbox(&tx, 1, "Receipts").expect("names"));
        assert!(!names_mailbox(&tx, 2, "INBOX").expect("another account's"));

        enqueue(
            &tx,
            1,
            &Op::EraseMailbox {
                mailbox: "Junk".into(),
            },
        )
        .expect("queue");
        let junk = unsent_removals(&tx, 1, "Junk").expect("removals");
        assert!(junk.everything && junk.contains(12345));
    }

    #[test]
    fn a_whole_mailbox_is_one_short_command() {
        // Mark All Messages as Read on fifty thousand messages: one range, not 290 KB.
        let all: Vec<u32> = (1..=50_000).collect();
        assert_eq!(sequence_sets(&all), vec!["1:50000"]);
    }

    #[test]
    fn a_long_scattered_selection_is_split_into_commands_of_a_safe_length() {
        // Every other UID: no ranges possible, and far longer than one line should be.
        let scattered: Vec<u32> = (1..=40_000).map(|n| n * 2).collect();
        let sets = sequence_sets(&scattered);

        assert!(sets.len() > 1, "one set of {} bytes", sets[0].len());
        assert!(sets.iter().all(|set| set.len() <= MAX_SET_LEN));

        // Nothing lost, nothing added, nothing repeated.
        let back: Vec<u32> = sets
            .iter()
            .flat_map(|set| set.split(','))
            .map(|uid| uid.parse().expect("a single uid"))
            .collect();
        assert_eq!(back, scattered);
    }

    #[test]
    fn operations_come_back_in_the_order_they_were_made() {
        // Flag-then-move must reach the server that way round: after the move the UID in the
        // flag operation no longer resolves in the mailbox it names.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        enqueue(&tx, 1, &flag(vec![7], Some(true))).expect("first");
        enqueue(
            &tx,
            1,
            &Op::Move {
                from: "INBOX".into(),
                to: "Archive".into(),
                uids: vec![7],
            },
        )
        .expect("second");

        let ops = queued(&tx, 1).expect("queued");

        assert_eq!(ops.len(), 2);
        assert!(matches!(ops[0].1, Op::Flag { .. }));
        assert!(matches!(ops[1].1, Op::Move { .. }));
    }

    #[test]
    fn an_operation_that_changes_nothing_is_not_queued() {
        // A patch with neither flag set, or an empty selection, would otherwise put a row in
        // the queue that the drain has to connect to the server to discover is a no-op.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        enqueue(&tx, 1, &flag(vec![], Some(true))).expect("no uids");
        enqueue(&tx, 1, &flag(vec![7], None)).expect("no change");

        assert_eq!(pending_count(&tx, 1).expect("count"), 0);
    }

    #[test]
    fn a_round_trip_through_json_keeps_every_field() {
        // The payload outlives the process, so a shape that serialises lossily loses a user's
        // change rather than merely a field.
        let ops = [
            flag(vec![1, 2, 3], Some(false)),
            Op::Flag {
                mailbox: "Archive".into(),
                uids: vec![9],
                seen: None,
                flagged: Some(true),
            },
            Op::Move {
                from: "INBOX".into(),
                to: "[Gmail]/Trash".into(),
                uids: vec![4, 5],
            },
            Op::Delete {
                mailbox: "INBOX".into(),
                uids: vec![6],
            },
            Op::CreateMailbox {
                path: "Re&AOc-us".into(),
            },
            Op::RenameMailbox {
                from: "Work".into(),
                to: "Projects".into(),
                delimiter: Some("/".into()),
            },
            Op::DeleteMailbox {
                paths: vec!["Work/Clients".into(), "Work".into()],
                delimiter: None,
            },
            Op::EraseMailbox {
                mailbox: "[Gmail]/Trash".into(),
            },
        ];

        for op in ops {
            let json = serde_json::to_string(&op).expect("encode");
            let back: Op = serde_json::from_str(&json).expect("decode");
            assert_eq!(back, op, "{json}");
        }
    }

    #[test]
    fn an_operation_is_dropped_once_it_has_had_its_chances() {
        // Otherwise one impossible operation — a message the server already expunged — sits
        // at the head of the queue and blocks every change made after it, forever.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        enqueue(&tx, 1, &flag(vec![7], Some(true))).expect("enqueue");
        let id = queued(&tx, 1).expect("queued")[0].0;

        for attempt in 1..MAX_ATTEMPTS {
            assert!(
                !record_failure(&tx, id, "nope").expect("failure"),
                "dropped early on attempt {attempt}"
            );
            assert_eq!(pending_count(&tx, 1).expect("count"), 1);
        }

        assert!(record_failure(&tx, id, "nope").expect("last"));
        assert_eq!(pending_count(&tx, 1).expect("count"), 0);
    }

    #[test]
    fn an_unreadable_payload_is_dropped_rather_than_read_forever() {
        // Written by a version that stored a different shape. Keeping it would mean parsing
        // and failing on it at the start of every sync for the life of the install.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        tx.execute(
            "INSERT INTO pending_op (account_id, kind, payload_json, created_at)
             VALUES (1, 'flag', '{\"kind\":\"somethingElse\"}', 0)",
            [],
        )
        .expect("insert");

        assert!(queued(&tx, 1).expect("queued").is_empty());
        assert_eq!(pending_count(&tx, 1).expect("count"), 0);
    }

    /// The draft its UID belongs to, so the next save can replace rather than add.
    ///
    /// `remote_uid` existed from the first drafts migration, was read in exactly one place
    /// to build `Op::AppendDraft { replaces }`, and was written nowhere. So `replaces` was
    /// always None, the delete-the-old-copy branch never ran, and every autosave left one
    /// more copy on the server -- one per thirty seconds of typing, in every client the
    /// user owns. The schema comment beside the column described that exact outcome.
    #[test]
    fn a_draft_remembers_the_uid_its_append_landed_on() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        tx.execute(
            "INSERT INTO draft (id, account_id, message_id, updated_at)
             VALUES (1, 1, ?1, 0)",
            rusqlite::params!["<draft-1@halcyon.test>"],
        )
        .expect("draft");

        let before: Option<i64> = tx
            .query_row("SELECT remote_uid FROM draft WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("read");
        assert_eq!(before, None, "a draft starts with no server copy");

        record_draft_uid(&tx, "<draft-1@halcyon.test>", 4321).expect("record");

        // Read back with the same query `compose_save_draft` uses to build `replaces`.
        let after: Option<i64> = tx
            .query_row("SELECT remote_uid FROM draft WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("read");

        assert_eq!(
            after,
            Some(4321),
            "the next save cannot replace a copy it does not know the UID of"
        );
    }

    #[test]
    fn inside_means_below_a_separator_not_merely_starting_with_the_name() {
        assert!(within("Work", "Work", Some("/")));
        assert!(within("Work/Clients", "Work", Some("/")));
        assert!(within("Work/Clients/2026", "Work", Some("/")));
        // "Workshop" starts with "Work" and is a different folder.
        assert!(!within("Workshop", "Work", Some("/")));
        assert!(!within("Work.Clients", "Work", Some("/")));
        // No separator known: only the name itself.
        assert!(!within("Work/Clients", "Work", None));
        assert!(within("Work", "Work", None));

        assert_eq!(
            repath("Work/Clients", "Work", "Projects", Some("/")).as_deref(),
            Some("Projects/Clients")
        );
        assert_eq!(repath("Workshop", "Work", "Projects", Some("/")), None);
        assert_eq!(
            repath("INBOX.Work", "INBOX.Work", "INBOX.Jobs", Some(".")).as_deref(),
            Some("INBOX.Jobs")
        );
    }

    #[test]
    fn a_rename_to_the_same_name_is_nothing_to_send() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        enqueue(
            &tx,
            1,
            &Op::RenameMailbox {
                from: "A".into(),
                to: "A".into(),
                delimiter: None,
            },
        )
        .expect("enqueue");
        enqueue(
            &tx,
            1,
            &Op::DeleteMailbox {
                paths: vec![],
                delimiter: None,
            },
        )
        .expect("enqueue");

        assert_eq!(pending_count(&tx, 1).expect("count"), 0);
    }

    #[test]
    fn the_sync_knows_which_folders_are_waiting_on_the_server() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        for op in [
            Op::CreateMailbox { path: "New".into() },
            Op::RenameMailbox {
                from: "Old".into(),
                to: "Renamed".into(),
                delimiter: Some("/".into()),
            },
            Op::DeleteMailbox {
                paths: vec!["Gone/Child".into(), "Gone".into()],
                delimiter: Some("/".into()),
            },
            // Not structural, so not part of the tree.
            flag(vec![1], Some(true)),
        ] {
            enqueue(&tx, 1, &op).expect("enqueue");
        }

        let tree = pending_tree(&tx, 1).expect("tree");

        assert!(tree.keeps("New"));
        assert!(tree.keeps("Renamed"));
        assert!(tree.keeps("Renamed/Child"));
        assert!(!tree.keeps("INBOX"));

        assert!(tree.hides("Old"));
        assert!(tree.hides("Old/Child"));
        assert!(tree.hides("Gone"));
        assert!(tree.hides("Gone/Other"), "a child made elsewhere meanwhile");
        assert!(!tree.hides("New"));
        assert!(!tree.hides("Oldest"));

        // Another account's queue is its own.
        assert_eq!(pending_tree(&tx, 2).expect("tree"), PendingTree::default());
    }

    #[test]
    fn a_refusal_is_read_the_way_servers_actually_word_it() {
        use async_imap::error::Error;

        // What async-imap makes of a tagged NO, codes it does not parse included.
        let exists =
            Error::No("code: None, info: Some(\"[ALREADYEXISTS] Mailbox already exists\")".into());
        let gmail_exists = Error::No(
            "code: None, info: Some(\"[ALREADYEXISTS] Duplicate folder name Work (Failure)\")"
                .into(),
        );
        let missing = Error::No(
            "code: None, info: Some(\"[NONEXISTENT] Mailbox doesn't exist: Work\")".into(),
        );
        let gmail_missing = Error::No(
            "code: None, info: Some(\"[NONEXISTENT] Unknown Mailbox: Work (Failure)\")".into(),
        );
        let refused = Error::No(
            "code: None, info: Some(\"[CANNOT] Invalid mailbox name: \\\"x\\\"\")".into(),
        );

        assert!(already_exists(&exists));
        assert!(already_exists(&gmail_exists));
        assert!(!already_exists(&missing));
        assert!(nonexistent(&missing));
        assert!(nonexistent(&gmail_missing));
        assert!(!nonexistent(&refused));
        assert!(!already_exists(&Error::ConnectionLost));

        // The sentence the user sees has the wrapping taken off and the escapes undone.
        assert_eq!(
            server_words(&SyncError::Imap(refused)),
            "[CANNOT] Invalid mailbox name: \"x\""
        );
        assert!(is_refusal(&SyncError::Imap(Error::Bad("x".into()))));
        assert!(!is_refusal(&SyncError::Imap(Error::ConnectionLost)));

        // Dovecot's timing trailer, as the rig really sends it, is left out of the sentence —
        // and a parenthesis that is not timing is left in.
        let dovecot = Error::No(
            "code: None, info: Some(\"[ALREADYEXISTS] Target mailbox already exists (0.001 + 0.000 secs).\")"
                .into(),
        );
        assert_eq!(
            server_words(&SyncError::Imap(dovecot)),
            "[ALREADYEXISTS] Target mailbox already exists"
        );
        let gmail = Error::No(
            "code: None, info: Some(\"[NONEXISTENT] Unknown Mailbox: Work (Failure)\")".into(),
        );
        assert_eq!(
            server_words(&SyncError::Imap(gmail)),
            "[NONEXISTENT] Unknown Mailbox: Work (Failure)"
        );
    }

    #[test]
    fn a_queued_operation_can_be_corrected_in_place() {
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        enqueue(&tx, 1, &flag(vec![1], Some(true))).expect("first");
        enqueue(&tx, 1, &Op::CreateMailbox { path: "A".into() }).expect("second");
        let ids: Vec<i64> = queued(&tx, 1)
            .expect("queued")
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        rewrite(&tx, ids[1], &Op::CreateMailbox { path: "B".into() }).expect("rewrite");

        let after = queued_after(&tx, 1, ids[0]).expect("after");
        assert_eq!(
            after,
            vec![(ids[1], Op::CreateMailbox { path: "B".into() })]
        );
        assert!(queued_after(&tx, 1, ids[1]).expect("after").is_empty());
    }

    #[test]
    fn recording_a_uid_for_a_draft_that_has_gone_is_not_an_error() {
        // The draft was sent or discarded while its append was in flight. There is no next
        // save to replace anything, so updating nothing is the right outcome -- and failing
        // here would leave the operation queued and retried for ever.
        let mut conn = store();
        let tx = conn.transaction().expect("tx");

        record_draft_uid(&tx, "<gone@halcyon.test>", 7).expect("record");
    }
}
