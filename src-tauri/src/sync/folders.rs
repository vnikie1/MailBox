//! Mailboxes the user makes, renames, deletes, empties, rebuilds and gives roles to, and the
//! order of Favourites. The mailbox context menu.
//!
//! Every function here is the local half of a change and runs inside the command's
//! transaction: it rewrites the tree the sidebar shows and queues the server half with
//! `ops::enqueue`. That is standing rule 10's whole model — the sidebar changes when the
//! transaction commits, and the server hears about it at the next sync.
//!
//! ## When the server says no
//!
//! A server can refuse a name — a reserved word, a character it will not store, a folder limit —
//! and it says so only when the operation reaches it. `ops::drain` gives up on a refused change
//! at once and calls back into this module (`abandon_created`, `abandon_rename`) to put the local
//! tree back the way the server has it, so the sidebar does not go on showing a folder that is
//! never going to exist.
//!
//! ## Where a new mailbox goes
//!
//! At the top of the account, or inside a mailbox that can hold others (`can_contain`). The
//! sidebar nests by path (`db::query::mailboxes_tree`), so a folder made inside another shows
//! there at once.

use rusqlite::{params, OptionalExtension, Transaction};

use crate::db::{write, DbError};

use super::mailboxes::{self, Role};
use super::ops::{self, Op};
use super::utf7;

/// The longest name accepted, in characters.
///
/// RFC 3501 sets no limit. Servers do — Exchange allows 255 characters for a whole path — and a
/// name refused there is refused only when the operation reaches it, long after the sheet that
/// could have said so has closed.
pub const MAX_NAME: usize = 200;

#[derive(Debug, thiserror::Error)]
pub enum FolderError {
    /// Something the user can put right, as a sentence they can read.
    #[error("{0}")]
    Invalid(String),

    /// The mailbox or account went away between the click and the write.
    #[error("that mailbox no longer exists")]
    Gone,

    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<rusqlite::Error> for FolderError {
    fn from(error: rusqlite::Error) -> Self {
        FolderError::Db(error.into())
    }
}

/// One mailbox, as this module needs to see it.
#[derive(Debug, Clone)]
struct Folder {
    id: i64,
    path: String,
    delimiter: Option<String>,
    role: Option<String>,
}

fn folders_of(tx: &Transaction<'_>, account_id: i64) -> Result<Vec<Folder>, DbError> {
    let mut statement = tx.prepare(
        "SELECT id, remote_path, delimiter, role FROM mailbox WHERE account_id = ?1 ORDER BY id",
    )?;

    let rows = statement.query_map(params![account_id], |row| {
        Ok(Folder {
            id: row.get(0)?,
            path: row.get(1)?,
            delimiter: row.get(2)?,
            role: row.get(3)?,
        })
    })?;

    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// A mailbox and the account it belongs to.
fn folder(tx: &Transaction<'_>, mailbox_id: i64) -> Result<Option<(i64, Folder)>, DbError> {
    Ok(tx
        .query_row(
            "SELECT account_id, id, remote_path, delimiter, role FROM mailbox WHERE id = ?1",
            params![mailbox_id],
            |row| {
                Ok((
                    row.get(0)?,
                    Folder {
                        id: row.get(1)?,
                        path: row.get(2)?,
                        delimiter: row.get(3)?,
                        role: row.get(4)?,
                    },
                ))
            },
        )
        .optional()?)
}

/// Whether the account has a server to tell, or `None` if there is no such account.
///
/// An imported archive has no server: its folders exist only here, so changing them queues
/// nothing — there would be nobody to send it to, and the drain would fail on it for ever.
fn has_server(tx: &Transaction<'_>, account_id: i64) -> Result<Option<bool>, DbError> {
    Ok(tx
        .query_row(
            "SELECT imap_host IS NOT NULL AND imap_host <> '' FROM account WHERE id = ?1",
            params![account_id],
            |row| row.get::<_, bool>(0),
        )
        .optional()?)
}

/// The hierarchy separator the account's server uses.
///
/// The Inbox's, where it has one, because it is the one folder every server lists; otherwise
/// whatever any folder reported. `None` until the first sync after the column existed.
fn account_delimiter(folders: &[Folder]) -> Option<String> {
    folders
        .iter()
        .find(|folder| folder.path.eq_ignore_ascii_case("INBOX") && folder.delimiter.is_some())
        .or_else(|| folders.iter().find(|folder| folder.delimiter.is_some()))
        .and_then(|folder| folder.delimiter.clone())
}

/// Whether the user may rename or delete this mailbox.
///
/// Not the Inbox, and not a folder with a role: Drafts, Sent, Junk, the Bin and Archive are where
/// the app itself files mail, and renaming one out from under it would send deleted mail
/// somewhere the user never looks. Nor Gmail's own `[Gmail]` folders, which Gmail will not rename
/// or delete whatever a client asks.
pub fn editable(role: Option<&str>, path: &str) -> bool {
    if role.is_some() || path.eq_ignore_ascii_case("INBOX") {
        return false;
    }

    let upper = path.to_ascii_uppercase();
    !(upper.starts_with("[GMAIL]") || upper.starts_with("[GOOGLE MAIL]"))
}

/// Whether New Mailbox may make a mailbox inside this one.
///
/// Not the Inbox. Servers that keep every folder inside it (Courier, cPanel) have the sidebar
/// show those folders at the top of the account, so a folder made "inside the Inbox" would appear
/// beside it — and Gmail refuses one outright. Not Gmail's own `[Gmail]` folders either, which
/// Gmail will not put labels under. Every other folder can, the ones with roles included:
/// "Archive/2024" is a common way to file.
///
/// A mailbox whose separator is not known yet cannot: without it there is no way to write the
/// path of something inside it.
pub fn can_contain(path: &str, delimiter: Option<&str>) -> bool {
    if !delimiter.is_some_and(|separator| !separator.is_empty())
        || path.eq_ignore_ascii_case("INBOX")
    {
        return false;
    }

    let upper = path.to_ascii_uppercase();
    !(upper.starts_with("[GMAIL]") || upper.starts_with("[GOOGLE MAIL]"))
}

/// A name the user typed, trimmed, or the sentence that says what is wrong with it.
///
/// The window checks the same rules as the user types (`src/features/sidebar/mailboxName.ts`);
/// these are the ones that count.
pub fn validate_name(name: &str, delimiter: Option<&str>) -> Result<String, String> {
    let name = name.trim();

    if name.is_empty() {
        return Err("Enter a name for the mailbox.".into());
    }

    if name.chars().any(char::is_control) {
        return Err("A mailbox name can’t contain tabs or line breaks.".into());
    }

    // A separator in a name makes a folder inside a folder, which is not what was asked for.
    if let Some(separator) = delimiter.filter(|separator| !separator.is_empty()) {
        if name.contains(separator) {
            return Err(format!("A mailbox name can’t contain “{separator}”."));
        }
    }

    // LIST's wildcards. RFC 3501 does not forbid them in a name, but a folder called "*" is one
    // no client can list on its own.
    if name.contains('%') || name.contains('*') {
        return Err("A mailbox name can’t contain “%” or “*”.".into());
    }

    if name.chars().count() > MAX_NAME {
        return Err(format!(
            "A mailbox name can be at most {MAX_NAME} characters."
        ));
    }

    Ok(name.to_string())
}

/// Where a new top-level mailbox goes: at the root, or inside `INBOX` on the servers that keep
/// every folder there and refuse to create one anywhere else — Courier, and cPanel's Dovecot.
///
/// Read from the folders the account already has rather than asked of the server, because
/// `async-imap` has no `NAMESPACE` command and the evidence is already here: if every folder but
/// the Inbox is inside it, the next one belongs there too.
fn personal_prefix(folders: &[Folder]) -> String {
    let mut prefix: Option<String> = None;

    for folder in folders {
        if folder.path.eq_ignore_ascii_case("INBOX") {
            continue;
        }

        let Some(separator) = folder.delimiter.as_deref().filter(|d| !d.is_empty()) else {
            return String::new();
        };

        let wanted = format!("INBOX{separator}");
        let head = folder.path.get(..wanted.len());

        match head {
            Some(head) if head.eq_ignore_ascii_case(&wanted) => match &prefix {
                None => prefix = Some(head.to_string()),
                Some(existing) if existing == head => {}
                Some(_) => return String::new(),
            },
            _ => return String::new(),
        }
    }

    prefix.unwrap_or_default()
}

/// Everything up to and including the last separator: where a renamed folder stays.
fn parent_of<'a>(path: &'a str, delimiter: Option<&str>) -> &'a str {
    match delimiter
        .filter(|separator| !separator.is_empty())
        .and_then(|separator| path.rfind(separator).map(|at| at + separator.len()))
    {
        Some(end) => &path[..end],
        None => "",
    }
}

/// Refuses a path that is the Inbox's, or that another mailbox already has.
///
/// Compared as the user reads them — decoded, and without regard to case. Gmail treats labels
/// that differ only in case as one label, and two folders a person cannot tell apart are a
/// mistake on any server.
fn check_free(
    folders: &[Folder],
    path: &str,
    except: Option<i64>,
    name: &str,
    parent: Option<&str>,
) -> Result<(), FolderError> {
    if path.eq_ignore_ascii_case("INBOX") {
        return Err(FolderError::Invalid(
            "“Inbox” is the name of the account’s inbox.".into(),
        ));
    }

    let wanted = utf7::display(path).to_lowercase();
    let taken = folders
        .iter()
        .filter(|folder| Some(folder.id) != except)
        .any(|folder| utf7::display(&folder.path).to_lowercase() == wanted);

    if taken {
        return Err(FolderError::Invalid(match parent {
            Some(parent) => format!("There’s already a mailbox called “{name}” in “{parent}”."),
            None => format!("There’s already a mailbox called “{name}”."),
        }));
    }

    Ok(())
}

/// New Mailbox, at the top of the account. Returns the new mailbox's id.
pub fn create(tx: &Transaction<'_>, account_id: i64, name: &str) -> Result<i64, FolderError> {
    create_in(tx, account_id, None, name)
}

/// New Mailbox, at the top of the account or inside `parent_id`. Returns the new mailbox's id.
pub fn create_in(
    tx: &Transaction<'_>,
    account_id: i64,
    parent_id: Option<i64>,
    name: &str,
) -> Result<i64, FolderError> {
    let server = has_server(tx, account_id)?.ok_or(FolderError::Gone)?;
    let folders = folders_of(tx, account_id)?;

    // Where the name goes, the separator it is checked against, and the parent's name for a
    // refusal. A parent in another account is, as far as this one knows, a mailbox that is gone.
    let (prefix, delimiter, parent_name) = match parent_id {
        None => (personal_prefix(&folders), account_delimiter(&folders), None),
        Some(parent_id) => {
            let parent = folders
                .iter()
                .find(|folder| folder.id == parent_id)
                .ok_or(FolderError::Gone)?;
            let parent_name = mailboxes::display_name(&parent.path, parent.delimiter.as_deref());

            let separator = match parent.delimiter.as_deref() {
                Some(separator) if can_contain(&parent.path, Some(separator)) => separator,
                _ => {
                    return Err(FolderError::Invalid(format!(
                        "A mailbox can’t be made inside “{parent_name}”."
                    )))
                }
            };

            (
                format!("{}{separator}", parent.path),
                Some(separator.to_string()),
                Some(parent_name),
            )
        }
    };

    // "/" stands in until the server's separator is known. It is the commonest one, and a name
    // refused for it on a server that uses "." has lost nothing but a slash.
    let name =
        validate_name(name, delimiter.as_deref().or(Some("/"))).map_err(FolderError::Invalid)?;
    let path = format!("{prefix}{}", utf7::encode(&name));
    check_free(&folders, &path, None, &name, parent_name.as_deref())?;

    tx.execute(
        "INSERT INTO mailbox
             (account_id, remote_path, display_name, role, sort_order, subscribed, delimiter)
         VALUES (?1, ?2, ?3, NULL, 100, 1, ?4)",
        params![account_id, path, name, delimiter],
    )?;
    let mailbox_id = tx.last_insert_rowid();

    if server {
        ops::enqueue(tx, account_id, &Op::CreateMailbox { path })?;
    }

    Ok(mailbox_id)
}

/// Rename Mailbox. Returns the account, for the event.
///
/// The folder keeps its row, so its messages, its unread count and its place in Favourites all
/// stay with it — a rename is not a delete and a create.
pub fn rename(tx: &Transaction<'_>, mailbox_id: i64, name: &str) -> Result<i64, FolderError> {
    let (account_id, folder) = folder(tx, mailbox_id)?.ok_or(FolderError::Gone)?;

    if !editable(folder.role.as_deref(), &folder.path) {
        return Err(FolderError::Invalid(
            "This mailbox belongs to the account and can’t be renamed.".into(),
        ));
    }

    let delimiter = folder.delimiter.clone();
    let name =
        validate_name(name, delimiter.as_deref().or(Some("/"))).map_err(FolderError::Invalid)?;
    let path = format!(
        "{}{}",
        parent_of(&folder.path, delimiter.as_deref()),
        utf7::encode(&name)
    );

    if path == folder.path {
        return Ok(account_id);
    }

    let folders = folders_of(tx, account_id)?;
    let parent = parent_of(&folder.path, delimiter.as_deref());
    let parent_name = folders
        .iter()
        .find(|each| {
            delimiter
                .as_deref()
                .is_some_and(|separator| parent.strip_suffix(separator) == Some(&each.path))
        })
        .map(|each| mailboxes::display_name(&each.path, each.delimiter.as_deref()));
    check_free(
        &folders,
        &path,
        Some(folder.id),
        &name,
        parent_name.as_deref(),
    )?;

    move_subtree(tx, &folders, &folder.path, &path, delimiter.as_deref())?;
    tx.execute(
        "UPDATE mailbox SET display_name = ?2 WHERE id = ?1",
        params![mailbox_id, name],
    )?;

    if has_server(tx, account_id)?.unwrap_or(false) {
        ops::enqueue(
            tx,
            account_id,
            &Op::RenameMailbox {
                from: folder.path,
                to: path,
                delimiter,
            },
        )?;
    }

    Ok(account_id)
}

/// Rewrites the path of a mailbox and of every mailbox inside it.
fn move_subtree(
    tx: &Transaction<'_>,
    folders: &[Folder],
    from: &str,
    to: &str,
    delimiter: Option<&str>,
) -> Result<(), DbError> {
    for folder in folders {
        if let Some(path) = ops::repath(&folder.path, from, to, delimiter) {
            tx.execute(
                "UPDATE mailbox SET remote_path = ?2 WHERE id = ?1",
                params![folder.id, path],
            )?;
        }
    }

    Ok(())
}

/// What Delete Mailbox removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deleted {
    pub account_id: i64,
    /// The mailbox and every mailbox that was inside it.
    pub mailbox_ids: Vec<i64>,
    pub messages: usize,
}

/// Delete Mailbox: the mailbox, everything inside it, and every message in any of them.
pub fn delete(tx: &Transaction<'_>, mailbox_id: i64) -> Result<Deleted, FolderError> {
    let (account_id, folder) = folder(tx, mailbox_id)?.ok_or(FolderError::Gone)?;

    if !editable(folder.role.as_deref(), &folder.path) {
        return Err(FolderError::Invalid(
            "This mailbox belongs to the account and can’t be deleted.".into(),
        ));
    }

    let delimiter = folder.delimiter.clone();
    let folders = folders_of(tx, account_id)?;

    let mut subtree: Vec<&Folder> = folders
        .iter()
        .filter(|each| ops::within(&each.path, &folder.path, delimiter.as_deref()))
        .collect();

    // A folder the app files into can sit inside one the user made — "Projects/Archive" given
    // the Archive role by the server. Deleting the parent would take it too.
    if let Some(special) = subtree.iter().find(|each| each.role.is_some()) {
        return Err(FolderError::Invalid(format!(
            "This mailbox contains the account’s {} mailbox, so it can’t be deleted.",
            mailboxes::display_name(&special.path, special.delimiter.as_deref())
        )));
    }

    // Deepest first. Inside one subtree a child's path always extends its parent's, so longer
    // means deeper.
    subtree.sort_by(|a, b| b.path.len().cmp(&a.path.len()).then(a.path.cmp(&b.path)));

    let mailbox_ids: Vec<i64> = subtree.iter().map(|each| each.id).collect();
    let paths: Vec<String> = subtree.iter().map(|each| each.path.clone()).collect();

    let messages = remove_mailboxes(tx, &mailbox_ids)?;

    if has_server(tx, account_id)?.unwrap_or(false) {
        ops::enqueue(tx, account_id, &Op::DeleteMailbox { paths, delimiter })?;
    }

    Ok(Deleted {
        account_id,
        mailbox_ids,
        messages,
    })
}

/// Removes mailboxes, and every message in them, from the local store.
///
/// Also used when the server has deleted a folder (`mailboxes::prune`) and when a folder the
/// server refused to create is taken back out.
pub(crate) fn remove_mailboxes(
    tx: &Transaction<'_>,
    mailbox_ids: &[i64],
) -> Result<usize, DbError> {
    let mut messages = 0;

    for mailbox_id in mailbox_ids {
        // A message moved *out* of this folder and not yet moved on the server remembers the
        // folder as its origin, so the move can find it there. Once the folder row is gone that
        // pointer names nothing — or, when SQLite reuses the id, some other folder — so it is
        // cleared. The queued move still carries the path it needs.
        tx.execute(
            "UPDATE message SET origin_mailbox_id = NULL, origin_uid = NULL
              WHERE origin_mailbox_id = ?1",
            params![mailbox_id],
        )?;

        messages += tx.execute(
            "DELETE FROM message WHERE mailbox_id = ?1",
            params![mailbox_id],
        )?;
        tx.execute("DELETE FROM mailbox WHERE id = ?1", params![mailbox_id])?;
    }

    Ok(messages)
}

/// What an erase removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Erased {
    pub mailbox_id: i64,
    pub messages: usize,
}

/// Erase Deleted Items (`Role::Trash`) and Erase Junk Mail (`Role::Junk`).
///
/// Every message in the account's mailbox of that role, snoozed ones included: the list hides
/// those, which is why this is a command and not a loop over what the window can see.
pub fn erase(tx: &Transaction<'_>, account_id: i64, role: Role) -> Result<Erased, FolderError> {
    let server = has_server(tx, account_id)?.ok_or(FolderError::Gone)?;

    let found: Option<(i64, String)> = tx
        .query_row(
            "SELECT id, remote_path FROM mailbox
              WHERE account_id = ?1 AND role = ?2
              ORDER BY id LIMIT 1",
            params![account_id, role.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    let Some((mailbox_id, path)) = found else {
        return Err(FolderError::Invalid(match role {
            Role::Junk => "This account has no Junk mailbox.".into(),
            _ => "This account has no Bin.".into(),
        }));
    };

    let ids: Vec<i64> = {
        let mut statement = tx.prepare("SELECT id FROM message WHERE mailbox_id = ?1")?;
        let rows = statement.query_map(params![mailbox_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    // Batched to stay under SQLite's limit on bound parameters. A Bin nobody has emptied in
    // years holds more messages than one statement can name.
    let mut messages = 0;
    for batch in ids.chunks(500) {
        messages += write::delete(tx, batch, true, None)?;
    }

    if server {
        ops::enqueue(tx, account_id, &Op::EraseMailbox { mailbox: path })?;
    }

    Ok(Erased {
        mailbox_id,
        messages,
    })
}

/// Add to Favourites, and Remove from Favourites. Returns the account, for the event.
///
/// Local, as Mail's favourites are: nothing about them reaches the server. Keyed by the mailbox's
/// row, so a renamed folder stays a favourite and a deleted one stops being one without anything
/// having to remember to clean up (`favourite.mailbox_id ... ON DELETE CASCADE`).
pub fn set_favourite(
    tx: &Transaction<'_>,
    mailbox_id: i64,
    favourite: bool,
) -> Result<i64, FolderError> {
    let (account_id, _) = folder(tx, mailbox_id)?.ok_or(FolderError::Gone)?;

    if favourite {
        // Added at the end, so nothing already there moves — Ctrl+1 to Ctrl+9 walk this list,
        // and a shortcut that changed meaning when something was added would be a trap. Already
        // there is not a move to the end: `OR IGNORE` meets the unique mailbox and stops.
        tx.execute(
            "INSERT OR IGNORE INTO favourite (position, mailbox_id)
             SELECT COALESCE(MAX(position), 0) + 1, ?1 FROM favourite",
            params![mailbox_id],
        )?;
    } else {
        tx.execute(
            "DELETE FROM favourite WHERE mailbox_id = ?1",
            params![mailbox_id],
        )?;
    }

    Ok(account_id)
}

/// Add to Favourites by dragging: the mailbox goes where it was dropped, or moves there if it
/// was a favourite already. Returns the account, for the event.
pub fn add_favourite_at(
    tx: &Transaction<'_>,
    mailbox_id: i64,
    before: Option<i64>,
) -> Result<i64, FolderError> {
    let account_id = set_favourite(tx, mailbox_id, true)?;

    let favourite_id: i64 = tx.query_row(
        "SELECT id FROM favourite WHERE mailbox_id = ?1",
        params![mailbox_id],
        |row| row.get(0),
    )?;
    move_favourite(tx, favourite_id, before)?;

    Ok(account_id)
}

/// Moves a favourite to just before another, or to the end when `before` is `None`.
///
/// The one way Favourites are reordered: a drag in the sidebar, or Alt+Up and Alt+Down on a row.
/// Every position is written again rather than squeezed between two others, so no run of moves
/// can exhaust the numbers between two rows.
pub fn move_favourite(
    tx: &Transaction<'_>,
    favourite_id: i64,
    before: Option<i64>,
) -> Result<(), FolderError> {
    let mut order: Vec<i64> = {
        let mut statement = tx.prepare("SELECT id FROM favourite ORDER BY position, id")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    if !order.contains(&favourite_id) {
        return Err(FolderError::Gone);
    }
    if before == Some(favourite_id) {
        return Ok(());
    }

    order.retain(|id| *id != favourite_id);
    let at = match before {
        None => order.len(),
        // The row it was dropped against went away meanwhile — removed in another window.
        Some(before) => order
            .iter()
            .position(|id| *id == before)
            .ok_or(FolderError::Gone)?,
    };
    order.insert(at, favourite_id);

    for (index, id) in order.iter().enumerate() {
        tx.execute(
            "UPDATE favourite SET position = ?2 WHERE id = ?1 AND position <> ?2",
            params![id, index as i64 + 1],
        )?;
    }

    Ok(())
}

/// The roles Use This Mailbox As offers: the ones the app files mail into.
///
/// Not the Inbox, which a server names and every account has exactly one of, and not All Mail,
/// which is Gmail's — and Gmail is not offered the choice at all (`use_as`).
pub const CHOOSABLE_ROLES: [Role; 5] = [
    Role::Drafts,
    Role::Sent,
    Role::Junk,
    Role::Trash,
    Role::Archive,
];

/// Whether the account is a Gmail account, by provider or by server.
///
/// By server as well, because a Google address added through "Other" with an app password is
/// Gmail all the same — the same two tests the window's `isGmail` makes.
fn is_gmail(tx: &Transaction<'_>, account_id: i64) -> Result<bool, DbError> {
    let found: Option<(String, Option<String>)> = tx
        .query_row(
            "SELECT provider, imap_host FROM account WHERE id = ?1",
            params![account_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    Ok(found.is_some_and(|(provider, host)| {
        let host = host
            .unwrap_or_default()
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        provider.eq_ignore_ascii_case("google")
            || provider.eq_ignore_ascii_case("gmail")
            || host == "imap.gmail.com"
            || host == "imap.googlemail.com"
    }))
}

/// Use This Mailbox As. Returns the account, for the event.
///
/// The choice is kept apart from the row (`mailbox_role`), because every sync rewrites
/// `mailbox.role` from what the server says and would undo it; `mailboxes::persist` reads the
/// choice back over the server's answer. The rows change here too, so the sidebar, and
/// everything that files mail by role, follow at once rather than at the next sync.
///
/// Whichever mailbox had the role gives it up, and this mailbox gives up the role it had: one
/// mailbox per role, and one role per mailbox, as in Mail.
pub fn use_as(tx: &Transaction<'_>, mailbox_id: i64, role: Role) -> Result<i64, FolderError> {
    if !CHOOSABLE_ROLES.contains(&role) {
        return Err(FolderError::Invalid(
            "A mailbox can only be used as Drafts, Sent, Junk, Bin or Archive.".into(),
        ));
    }

    let (account_id, folder) = folder(tx, mailbox_id)?.ok_or(FolderError::Gone)?;

    if folder.path.eq_ignore_ascii_case("INBOX") {
        return Err(FolderError::Invalid(
            "The Inbox can’t be used as another mailbox.".into(),
        ));
    }

    // An imported archive files nothing anywhere, so a role there would mean nothing.
    if !has_server(tx, account_id)?.unwrap_or(false) {
        return Err(FolderError::Invalid(
            "This mailbox isn’t on a server, so it can’t be used as another mailbox.".into(),
        ));
    }

    // Gmail says which of its folders is which, always, and acts on it: a message "deleted" into
    // a label rather than into Gmail's Bin is not deleted at all.
    if is_gmail(tx, account_id)? {
        return Err(FolderError::Invalid(
            "Gmail decides which of its mailboxes are Drafts, Sent, Junk and Bin.".into(),
        ));
    }

    tx.execute(
        "DELETE FROM mailbox_role WHERE mailbox_id = ?1 OR (account_id = ?2 AND role = ?3)",
        params![mailbox_id, account_id, role.as_str()],
    )?;
    tx.execute(
        "INSERT INTO mailbox_role (account_id, role, mailbox_id) VALUES (?1, ?2, ?3)",
        params![account_id, role.as_str(), mailbox_id],
    )?;

    tx.execute(
        "UPDATE mailbox SET role = NULL, sort_order = 100
          WHERE account_id = ?1 AND role = ?2 AND id <> ?3",
        params![account_id, role.as_str(), mailbox_id],
    )?;
    tx.execute(
        "UPDATE mailbox SET role = ?2, sort_order = ?3 WHERE id = ?1",
        params![mailbox_id, role.as_str(), role.sort_order()],
    )?;

    Ok(account_id)
}

/// Rebuild. Returns the account, for the sync that does the work.
///
/// Only asks: the next sync of the account reads the whole mailbox from the server again, once
/// nothing queued still names it (`engine::sync_mailbox`). Doing the work here would read back a
/// message deleted a moment ago whose deletion has not reached the server, and show it again.
pub fn request_rebuild(tx: &Transaction<'_>, mailbox_id: i64) -> Result<i64, FolderError> {
    let (account_id, _) = folder(tx, mailbox_id)?.ok_or(FolderError::Gone)?;

    if !has_server(tx, account_id)?.unwrap_or(false) {
        return Err(FolderError::Invalid(
            "This mailbox isn’t on a server, so there is nothing to rebuild it from.".into(),
        ));
    }

    tx.execute(
        "UPDATE mailbox SET rebuild_requested = 1 WHERE id = ?1",
        params![mailbox_id],
    )?;

    Ok(account_id)
}

/// Takes back a mailbox the server would not create, and everything done to it since.
///
/// Called by the drain after it has forgotten the refused operation. Returns the folder's name
/// as the sidebar showed it, for the sentence that tells the user.
///
/// Every later operation that names the folder, or a folder made inside it, is about something
/// that will never exist on the server, so each goes: moves into it, folders made inside it, and
/// any rename or delete of it. A rename is followed to its new name, because that is the name the
/// local row has now. The messages that were moved in go back to the folders the server still has
/// them in, rather than down with the folder.
pub(crate) fn abandon_created(
    tx: &Transaction<'_>,
    account_id: i64,
    after: i64,
    path: &str,
) -> Result<String, DbError> {
    // The separator the folder was made with, which is what says what was made inside it. The
    // create operation does not carry one; the row it made does.
    let delimiter: Option<String> = tx
        .query_row(
            "SELECT delimiter FROM mailbox WHERE account_id = ?1 AND remote_path = ?2",
            params![account_id, path],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        .or_else(|| {
            folders_of(tx, account_id)
                .ok()
                .and_then(|folders| account_delimiter(&folders))
        });
    let delimiter = delimiter.as_deref();

    let inside =
        |each: &str, names: &[String]| names.iter().any(|name| ops::within(each, name, delimiter));

    let mut names = vec![path.to_string()];
    let mut deleted = false;

    for (id, mut op) in ops::queued_after(tx, account_id, after)? {
        if !op.paths().iter().any(|each| inside(each, &names)) {
            continue;
        }

        match &mut op {
            // The folder itself under a new name, which its later operations will use.
            Op::RenameMailbox { from, to, .. } if names.contains(from) => {
                names.push(to.clone());
                ops::forget(tx, id)?;
            }
            Op::DeleteMailbox { paths, .. } => {
                let ends_it = paths.iter().any(|each| names.contains(each));
                paths.retain(|each| !inside(each, &names));

                if paths.is_empty() {
                    ops::forget(tx, id)?;
                } else {
                    ops::rewrite(tx, id, &op)?;
                }

                // The folder's life ended here. A later folder of the same name is a new one,
                // and the operations about it are not this folder's to cancel.
                if ends_it {
                    deleted = true;
                    break;
                }
            }
            _ => ops::forget(tx, id)?,
        }
    }

    let fallback = mailboxes::display_name(path, delimiter);

    if deleted {
        return Ok(fallback);
    }

    let Some(current) = names.last() else {
        return Ok(fallback);
    };

    // The folder and whatever was made inside it, which the server has no more than the folder.
    let subtree: Vec<(i64, String, String)> = folders_of(tx, account_id)?
        .into_iter()
        .filter(|folder| ops::within(&folder.path, current, delimiter))
        .map(|folder| {
            let name: String = tx
                .query_row(
                    "SELECT display_name FROM mailbox WHERE id = ?1",
                    params![folder.id],
                    |row| row.get(0),
                )
                .unwrap_or_default();
            (folder.id, folder.path, name)
        })
        .collect();

    let name = subtree
        .iter()
        .find(|(_, each, _)| each == current)
        .map(|(_, _, name)| name.clone());

    let Some(name) = name else {
        return Ok(fallback);
    };

    for (mailbox_id, _, _) in &subtree {
        unpark(tx, *mailbox_id)?;
    }
    let ids: Vec<i64> = subtree.iter().map(|(id, _, _)| *id).collect();
    remove_mailboxes(tx, &ids)?;

    Ok(name)
}

/// Sends messages parked in a mailbox back to where the server still has them.
///
/// A parked row is one moved here locally whose move has not reached the server; it remembers
/// its origin mailbox and UID, and restoring those is exactly undoing the move. `OR IGNORE`
/// because the origin may already hold that UID again — a sync fetched the message back — and
/// then the parked copy is the duplicate, and goes with the folder.
fn unpark(tx: &Transaction<'_>, mailbox_id: i64) -> Result<(), DbError> {
    let origins: Vec<i64> = {
        let mut statement = tx.prepare(
            "SELECT DISTINCT origin_mailbox_id FROM message
              WHERE mailbox_id = ?1 AND origin_uid IS NOT NULL
                AND origin_mailbox_id IN (SELECT id FROM mailbox)",
        )?;
        let rows = statement.query_map(params![mailbox_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    tx.execute(
        "UPDATE OR IGNORE message
            SET mailbox_id = origin_mailbox_id, uid = origin_uid,
                origin_mailbox_id = NULL, origin_uid = NULL
          WHERE mailbox_id = ?1 AND origin_uid IS NOT NULL
            AND origin_mailbox_id IN (SELECT id FROM mailbox)",
        params![mailbox_id],
    )?;

    // A recount rather than deltas: this is the rare path, and the origins are few.
    write::recount_mailboxes(tx, &origins)
}

/// Puts a folder back under the name the server kept, after a refused rename.
///
/// Everything queued since the rename names the folder by the name it was never given, so each
/// of those is rewritten to the old name — including a second rename, which then becomes a
/// fresh attempt from the name the server has. The local rows follow, unless the old name has
/// been taken locally in the meantime; then the renamed copy is dropped and the next sync
/// brings the folder back from the server.
pub(crate) fn abandon_rename(
    tx: &Transaction<'_>,
    account_id: i64,
    after: i64,
    from: &str,
    to: &str,
    delimiter: Option<&str>,
) -> Result<(), DbError> {
    for (id, mut op) in ops::queued_after(tx, account_id, after)? {
        let mut changed = false;

        for path in op.paths_mut() {
            if let Some(back) = ops::repath(path, to, from, delimiter) {
                *path = back;
                changed = true;
            }
        }

        if !changed {
            continue;
        }

        if op.is_empty() {
            ops::forget(tx, id)?;
        } else {
            ops::rewrite(tx, id, &op)?;
        }
    }

    let folders = folders_of(tx, account_id)?;
    let moving: Vec<(i64, String)> = folders
        .iter()
        .filter_map(|folder| {
            ops::repath(&folder.path, to, from, delimiter).map(|path| (folder.id, path))
        })
        .collect();

    let clash = moving.iter().any(|(id, path)| {
        folders
            .iter()
            .any(|folder| folder.id != *id && &folder.path == path)
    });

    if clash {
        let ids: Vec<i64> = moving.iter().map(|(id, _)| *id).collect();
        remove_mailboxes(tx, &ids)?;
        return Ok(());
    }

    for (id, path) in &moving {
        tx.execute(
            "UPDATE mailbox SET remote_path = ?2 WHERE id = ?1",
            params![id, path],
        )?;

        if path == from {
            tx.execute(
                "UPDATE mailbox SET display_name = ?2 WHERE id = ?1",
                params![id, mailboxes::display_name(from, delimiter)],
            )?;
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "folders_tests.rs"]
mod tests;
