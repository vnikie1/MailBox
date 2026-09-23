//! Mailbox tree discovery and role inference. docs/03 §5, docs/06 Phase 5 §1.
//!
//! *`LIST`/`LSUB` → build the mailbox tree; infer roles from `SPECIAL-USE`, falling back to
//! name heuristics per provider.*
//!
//! Roles are what let the sidebar say "Sent" instead of `[Gmail]/Sent Mail`, and what let
//! Archive and Delete know where to put things. Getting one wrong is not cosmetic: a Trash
//! role pointed at the wrong folder deletes mail into a folder the user never looks in.
//!
//! `SPECIAL-USE` (RFC 6154) is authoritative where a server offers it. The name heuristics
//! exist because plenty of servers do not, and because Gmail's names are localised — a French
//! Gmail account has `[Gmail]/Messages envoyés`, which no English word list will ever match.
//! For Gmail the *attributes* are always present even without SPECIAL-USE advertised, which
//! is why the attribute path is tried first for every provider rather than gated on the
//! capability.

use futures::StreamExt;

use crate::db::DbError;

use super::session::{ImapSession, SyncError};

/// A mailbox as the server describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// The raw IMAP path, exactly as the server spells it. Never shown to the user.
    pub remote_path: String,
    /// The leaf name, for display.
    pub display_name: String,
    /// The hierarchy separator this server uses — "/" for Gmail, "." for many Dovecot setups.
    pub delimiter: Option<String>,
    pub role: Option<Role>,
    /// `\Noselect`: a container that holds children but no messages, like Gmail's `[Gmail]`.
    /// Listed so the tree has a parent to hang children from, never synced.
    pub selectable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Inbox,
    Drafts,
    Sent,
    Junk,
    Trash,
    Archive,
    /// Gmail's `All Mail`. Every message appears here as well as in its labels, so it is
    /// deliberately *not* Archive — counting both would double every message in the app.
    All,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Inbox => "inbox",
            Role::Drafts => "drafts",
            Role::Sent => "sent",
            Role::Junk => "junk",
            Role::Trash => "trash",
            Role::Archive => "archive",
            Role::All => "all",
        }
    }

    /// The role a stored string names. The inverse of `as_str`.
    pub fn parse(value: &str) -> Option<Role> {
        [
            Role::Inbox,
            Role::Drafts,
            Role::Sent,
            Role::Junk,
            Role::Trash,
            Role::Archive,
            Role::All,
        ]
        .into_iter()
        .find(|role| role.as_str() == value)
    }

    /// The order roles appear in the sidebar. docs/01 §3.
    pub fn sort_order(self) -> i64 {
        match self {
            Role::Inbox => 0,
            Role::Drafts => 1,
            Role::Sent => 2,
            Role::Junk => 3,
            Role::Trash => 4,
            Role::Archive => 5,
            Role::All => 6,
        }
    }
}

/// Reads a role from RFC 6154 attributes.
///
/// The attribute strings arrive from `imap-proto` in a few shapes depending on the server, so
/// this matches on the backslash-prefixed name case-insensitively rather than on an enum.
fn role_from_attributes(attributes: &[String]) -> Option<Role> {
    for attribute in attributes {
        let name = attribute.trim_start_matches('\\').to_ascii_lowercase();

        let role = match name.as_str() {
            "inbox" => Role::Inbox,
            "drafts" => Role::Drafts,
            "sent" => Role::Sent,
            "junk" | "spam" => Role::Junk,
            "trash" => Role::Trash,
            "archive" => Role::Archive,
            "all" | "allmail" => Role::All,
            _ => continue,
        };

        return Some(role);
    }

    None
}

/// Name heuristics, for servers that offer no `SPECIAL-USE` attributes.
///
/// Matched on the **leaf** name, lowercased, after stripping a provider's container prefix.
/// Deliberately conservative: an unrecognised folder becomes a plain folder, which is
/// harmless, whereas a wrong guess sends deleted mail somewhere the user will not look.
fn role_from_name(path: &str, delimiter: Option<&str>) -> Option<Role> {
    if path.eq_ignore_ascii_case("INBOX") {
        return Some(Role::Inbox);
    }

    let separator = delimiter.unwrap_or("/");
    let leaf = path.rsplit(separator).next().unwrap_or(path);
    // Decoded first. The list below has words like "envoyés" in it, and the wire form of that is
    // `envoy&AOk-s` — which no amount of lowercasing will ever make equal.
    let lowered = super::utf7::display(leaf).trim().to_lowercase();

    // English, plus the handful of spellings that appear on servers configured in other
    // languages and are unambiguous. Anything more speculative belongs in SPECIAL-USE.
    let role = match lowered.as_str() {
        "drafts" | "draft" | "entwürfe" | "brouillons" | "borradores" => Role::Drafts,
        "sent" | "sent mail" | "sent items" | "sent messages" | "gesendet" | "envoyés" => {
            Role::Sent
        }
        "junk" | "junk e-mail" | "junk email" | "spam" | "bulk mail" => Role::Junk,
        "trash" | "deleted" | "deleted items" | "deleted messages" | "bin" | "papierkorb" => {
            Role::Trash
        }
        "archive" | "archives" | "archiv" => Role::Archive,
        "all mail" | "all" => Role::All,
        _ => return None,
    };

    Some(role)
}

/// The role for one listed mailbox: attributes first, then the name.
pub fn infer_role(path: &str, attributes: &[String], delimiter: Option<&str>) -> Option<Role> {
    // INBOX is defined by RFC 3501 and is case-insensitive; no server needs an attribute for
    // it, and some label it `\Noinferiors` and nothing else.
    if path.eq_ignore_ascii_case("INBOX") {
        return Some(Role::Inbox);
    }

    role_from_attributes(attributes).or_else(|| role_from_name(path, delimiter))
}

/// The display name for a mailbox: the leaf, with a provider container stripped, decoded from
/// the modified UTF-7 the server sends.
///
/// `[Gmail]/Sent Mail` shows as "Sent Mail", not as the whole path — the sidebar nests it
/// under its parent, so repeating the parent in the label is noise. And `Re&AOc-us` shows as
/// "Reçus", which it did not until the codec existed.
pub fn display_name(path: &str, delimiter: Option<&str>) -> String {
    let separator = delimiter.filter(|d| !d.is_empty()).unwrap_or("/");

    if path.eq_ignore_ascii_case("INBOX") {
        return "Inbox".to_string();
    }

    let leaf = path.rsplit(separator).next().unwrap_or(path);
    super::utf7::display(leaf).trim().to_string()
}

/// Lists every mailbox on the server.
///
/// `LIST "" "*"` rather than `LSUB`: docs/03 §5 mentions both, but a folder the user has not
/// subscribed to still exists and still receives mail, and a client that hides it makes mail
/// disappear. Subscription is recorded and used for sidebar defaults instead.
pub async fn discover(session: &mut ImapSession) -> Result<Vec<Discovered>, SyncError> {
    let mut listed = Vec::new();

    {
        let mut stream = session.list(Some(""), Some("*")).await?;

        while let Some(item) = stream.next().await {
            let name = item?;

            let attributes: Vec<String> = name
                .attributes()
                .iter()
                .map(|attribute| format!("{attribute:?}"))
                .collect();

            let path = name.name().to_string();
            let delimiter = name.delimiter().map(str::to_string);

            // `\Noselect` marks a container with no messages of its own — Gmail's `[Gmail]`
            // is the common case. It has to be listed so its children have a parent, but
            // SELECTing it is a protocol error.
            let selectable = !attributes
                .iter()
                .any(|attribute| attribute.to_ascii_lowercase().contains("noselect"));

            listed.push(Discovered {
                role: infer_role(&path, &attributes, delimiter.as_deref()),
                display_name: display_name(&path, delimiter.as_deref()),
                remote_path: path,
                delimiter,
                selectable,
            });
        }
    }

    Ok(resolve_duplicate_roles(listed))
}

/// Keeps at most one mailbox per role.
///
/// Servers do produce duplicates — a `Sent` folder alongside `[Gmail]/Sent Mail`, or two
/// folders both carrying `\Trash` after a migration. Two mailboxes claiming the same role
/// means "move to Trash" has no single answer, so the first by path order wins and the rest
/// become ordinary folders. They are still listed and still synced; they simply stop being
/// special.
fn resolve_duplicate_roles(mut listed: Vec<Discovered>) -> Vec<Discovered> {
    listed.sort_by(|a, b| a.remote_path.cmp(&b.remote_path));

    let mut claimed: Vec<Role> = Vec::new();

    for mailbox in &mut listed {
        let Some(role) = mailbox.role else {
            continue;
        };

        if claimed.contains(&role) {
            tracing::debug!(
                path = %mailbox.remote_path,
                role = role.as_str(),
                "a second mailbox claimed this role; treating it as an ordinary folder"
            );
            mailbox.role = None;
        } else {
            claimed.push(role);
        }
    }

    listed
}

/// Writes the discovered tree into the database, preserving ids for mailboxes we already had.
///
/// Matching on `remote_path` rather than replacing wholesale: the `message` table references
/// `mailbox.id`, so recreating rows would orphan every message in the account. A mailbox that
/// has genuinely gone from the server is left in place here and removed by the caller only
/// once it is sure — a `LIST` that fails halfway must not delete half the mailbox tree.
///
/// A listed mailbox the user has renamed or deleted since the `LIST` was taken is skipped: the
/// server still has the old name only because the change has not reached it yet
/// (`ops::PendingTree`). Returns the id and path of each mailbox it did write, so the caller
/// syncs exactly those.
///
/// Roles are the server's, except where the user chose one with Use This Mailbox As
/// (`chosen_roles`, `effective_roles`).
pub fn persist(
    tx: &rusqlite::Transaction<'_>,
    account_id: i64,
    discovered: &[Discovered],
) -> Result<Vec<(i64, String)>, DbError> {
    let pending = super::ops::pending_tree(tx, account_id)?;
    let chosen = chosen_roles(tx, account_id)?;
    let roles = effective_roles(discovered, &chosen, &pending);
    let mut ids = Vec::with_capacity(discovered.len());

    for ((index, mailbox), role) in discovered.iter().enumerate().zip(roles) {
        if pending.hides(&mailbox.remote_path) {
            tracing::debug!(
                path = %mailbox.remote_path,
                "listed mailbox has a rename or delete on its way; not recreating it"
            );
            continue;
        }

        let sort_order = role.map(Role::sort_order).unwrap_or(100 + index as i64);

        tx.execute(
            "INSERT INTO mailbox
                 (account_id, remote_path, display_name, role, sort_order, subscribed, delimiter)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
             ON CONFLICT(account_id, remote_path) DO UPDATE SET
                 display_name = excluded.display_name,
                 role         = excluded.role,
                 sort_order   = excluded.sort_order,
                 delimiter    = excluded.delimiter",
            rusqlite::params![
                account_id,
                mailbox.remote_path,
                mailbox.display_name,
                role.map(Role::as_str),
                sort_order,
                mailbox.delimiter,
            ],
        )?;

        let id: i64 = tx.query_row(
            "SELECT id FROM mailbox WHERE account_id = ?1 AND remote_path = ?2",
            rusqlite::params![account_id, mailbox.remote_path],
            |row| row.get(0),
        )?;

        ids.push((id, mailbox.remote_path.clone()));
    }

    Ok(ids)
}

/// The roles the user chose for this account's mailboxes, with the path each mailbox has now.
pub fn chosen_roles(
    tx: &rusqlite::Transaction<'_>,
    account_id: i64,
) -> Result<Vec<(Role, String)>, DbError> {
    let mut statement = tx.prepare(
        "SELECT chosen.role, mailbox.remote_path
           FROM mailbox_role AS chosen
           JOIN mailbox ON mailbox.id = chosen.mailbox_id
          WHERE chosen.account_id = ?1",
    )?;

    let rows = statement
        .query_map(rusqlite::params![account_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // A role this build does not know is a row from a newer one; it is left alone, not applied.
    Ok(rows
        .into_iter()
        .filter_map(|(role, path)| Role::parse(&role).map(|role| (role, path)))
        .collect())
}

/// The role each listed mailbox ends up with: the server's, except where the user chose.
///
/// A choice takes the role from whichever listed mailbox the server gave it to, and gives it to
/// the chosen one. It holds only while the chosen mailbox is still there — listed, or made here
/// and on its way — so a folder deleted in webmail does not leave the account with no Bin until
/// the sync after (the mailbox's row, and the choice with it, go when `prune` runs).
pub fn effective_roles(
    discovered: &[Discovered],
    chosen: &[(Role, String)],
    pending: &super::ops::PendingTree,
) -> Vec<Option<Role>> {
    let mut roles: Vec<Option<Role>> = discovered.iter().map(|mailbox| mailbox.role).collect();

    for (role, path) in chosen {
        let present = discovered
            .iter()
            .any(|mailbox| &mailbox.remote_path == path)
            || pending.keeps(path);
        if !present {
            continue;
        }

        for (mailbox, current) in discovered.iter().zip(roles.iter_mut()) {
            if &mailbox.remote_path == path {
                *current = Some(*role);
            } else if *current == Some(*role) {
                *current = None;
            }
        }
    }

    roles
}

/// Removes mailboxes this account no longer has on the server.
///
/// The counterpart `persist` promises and nobody wrote: its comment says a vanished mailbox is
/// "left in place here and removed by the caller only once it is sure", and no caller ever was.
/// So a folder deleted in webmail stayed in the sidebar for the life of the install, with its
/// messages still in it and its unread count still in the badge.
///
/// **This deletes mail**, through `message.mailbox_id ... ON DELETE CASCADE`, which is why the
/// caller has to be sure. Two conditions make it so, and both are the caller's to check:
///
/// * The `LIST` completed. `discover` propagates a mid-stream error rather than returning a
///   short list, so an `Ok` really is the whole tree — a half-read tree must never be treated
///   as evidence that the rest is gone.
/// * The list is not empty. A server that answers with nothing is having a bad day, not
///   reporting that the account has no folders.
///
/// Returns the paths removed, for the log. A folder full of mail disappearing is worth a line
/// even when it is correct.
///
/// A mailbox the user has just made or renamed is not on the server *yet*, and is kept: its
/// absence from the list is the change on its way, not evidence that it has gone
/// (`ops::PendingTree`).
pub fn prune(
    tx: &rusqlite::Transaction<'_>,
    account_id: i64,
    keep: &[String],
) -> Result<Vec<String>, DbError> {
    if keep.is_empty() {
        return Ok(Vec::new());
    }

    let pending = super::ops::pending_tree(tx, account_id)?;

    let existing: Vec<(i64, String)> = tx
        .prepare("SELECT id, remote_path FROM mailbox WHERE account_id = ?1")?
        .query_map(rusqlite::params![account_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut removed = Vec::new();

    for (id, path) in existing {
        if keep.iter().any(|kept| kept == &path) || pending.keeps(&path) {
            continue;
        }

        // Through the same path a deleted folder takes, which also clears the origin other
        // messages remember for an unfinished move out of this one.
        super::folders::remove_mailboxes(tx, &[id])?;
        removed.push(path);
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn inbox_is_recognised_however_it_is_spelled() {
        // RFC 3501 makes INBOX case-insensitive and special. A server need not attribute it.
        assert_eq!(infer_role("INBOX", &[], None), Some(Role::Inbox));
        assert_eq!(infer_role("inbox", &[], None), Some(Role::Inbox));
        assert_eq!(
            infer_role("Inbox", &attrs(&["\\Noinferiors"]), None),
            Some(Role::Inbox)
        );
    }

    #[test]
    fn special_use_attributes_win_over_the_name() {
        // The whole point of RFC 6154: a folder called "Archivio" with \Sent is Sent. A
        // client that trusted the name would file replies into the wrong folder forever.
        assert_eq!(
            infer_role("Archivio", &attrs(&["\\Sent"]), Some("/")),
            Some(Role::Sent)
        );
        assert_eq!(
            infer_role("Cestino", &attrs(&["\\Trash"]), Some("/")),
            Some(Role::Trash)
        );
    }

    #[test]
    fn gmail_paths_are_recognised_by_attribute() {
        // Gmail localises its folder names — a French account has [Gmail]/Messages envoyés,
        // which no English word list will ever match. The attributes are always there.
        assert_eq!(
            infer_role("[Gmail]/Messages envoyés", &attrs(&["\\Sent"]), Some("/")),
            Some(Role::Sent)
        );
        assert_eq!(
            infer_role("[Gmail]/Tous les messages", &attrs(&["\\All"]), Some("/")),
            Some(Role::All)
        );
    }

    #[test]
    fn gmail_all_mail_is_not_archive() {
        // Every Gmail message appears in All Mail as well as in its labels. Treating it as
        // Archive would double-count the entire mailbox — docs/03 §5 calls this out by name.
        assert_eq!(
            infer_role("[Gmail]/All Mail", &attrs(&["\\All"]), Some("/")),
            Some(Role::All)
        );
        assert_ne!(
            infer_role("[Gmail]/All Mail", &attrs(&["\\All"]), Some("/")),
            Some(Role::Archive)
        );
    }

    #[test]
    fn names_are_a_fallback_for_servers_with_no_special_use() {
        assert_eq!(infer_role("Sent Items", &[], Some("/")), Some(Role::Sent));
        assert_eq!(
            infer_role("Deleted Items", &[], Some("/")),
            Some(Role::Trash)
        );
        assert_eq!(infer_role("Junk E-mail", &[], Some("/")), Some(Role::Junk));
        assert_eq!(infer_role("Bin", &[], Some("/")), Some(Role::Trash));
    }

    #[test]
    fn the_name_fallback_reads_the_leaf_not_the_whole_path() {
        // A Dovecot server with "." as its separator: INBOX.Sent is Sent.
        assert_eq!(infer_role("INBOX.Sent", &[], Some(".")), Some(Role::Sent));
        assert_eq!(
            infer_role("Work/Archive", &[], Some("/")),
            Some(Role::Archive)
        );
    }

    #[test]
    fn an_unrecognised_folder_is_left_as_an_ordinary_folder() {
        // Conservative on purpose. A wrong guess sends deleted mail into a folder the user
        // never opens; no guess just shows a folder with its own name.
        assert_eq!(infer_role("Clients", &[], Some("/")), None);
        assert_eq!(infer_role("Receipts 2019", &[], Some("/")), None);
        assert_eq!(infer_role("Sentimental", &[], Some("/")), None);
        assert_eq!(infer_role("Archived Projects", &[], Some("/")), None);
    }

    #[test]
    fn display_names_drop_the_container_prefix() {
        assert_eq!(display_name("[Gmail]/Sent Mail", Some("/")), "Sent Mail");
        assert_eq!(display_name("INBOX.Work.Clients", Some(".")), "Clients");
        assert_eq!(display_name("INBOX", Some(".")), "Inbox");
        assert_eq!(display_name("Receipts", None), "Receipts");
    }

    #[test]
    fn display_names_are_decoded_from_the_wire() {
        // What LIST actually sends. The sidebar showed these raw until the codec existed.
        assert_eq!(
            display_name("[Gmail]/Messages envoy&AOk-s", Some("/")),
            "Messages envoyés"
        );
        assert_eq!(display_name("Re&AOc-us", Some("/")), "Reçus");
        assert_eq!(display_name("Tom &- Jerry", Some("/")), "Tom & Jerry");
        // Not valid modified UTF-7, and still a folder with mail in it.
        assert_eq!(display_name("Q&A", Some("/")), "Q&A");
    }

    #[test]
    fn the_name_fallback_reads_encoded_names_too() {
        // "Entwürfe" as a server without SPECIAL-USE sends it. Compared raw, it never matched
        // the word in the list — so German Drafts folders were ordinary folders.
        assert_eq!(
            infer_role("Entw&APw-rfe", &[], Some("/")),
            Some(Role::Drafts)
        );
        assert_eq!(
            infer_role("INBOX.Gel&APY-scht", &[], Some(".")),
            None,
            "an unlisted word stays an ordinary folder"
        );
    }

    #[test]
    fn two_mailboxes_claiming_one_role_do_not_both_keep_it() {
        // A migrated account really does end up with both "Sent" and "[Gmail]/Sent Mail".
        // If both stayed Sent, "where does a reply get filed?" has two answers and the code
        // picks whichever the query returned first — different on different runs.
        let listed = vec![
            Discovered {
                remote_path: "[Gmail]/Sent Mail".into(),
                display_name: "Sent Mail".into(),
                delimiter: Some("/".into()),
                role: Some(Role::Sent),
                selectable: true,
            },
            Discovered {
                remote_path: "Sent".into(),
                display_name: "Sent".into(),
                delimiter: Some("/".into()),
                role: Some(Role::Sent),
                selectable: true,
            },
        ];

        let resolved = resolve_duplicate_roles(listed);
        let with_role: Vec<_> = resolved
            .iter()
            .filter(|m| m.role == Some(Role::Sent))
            .collect();

        assert_eq!(with_role.len(), 1, "exactly one mailbox may hold a role");
        // Both are still present — the loser becomes an ordinary folder, it does not vanish.
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn duplicate_resolution_is_deterministic() {
        // Sorted by path first, so the same server produces the same answer on every sync.
        // Without that, which folder is "Trash" could change between launches.
        let build = |paths: &[&str]| {
            paths
                .iter()
                .map(|path| Discovered {
                    remote_path: (*path).to_string(),
                    display_name: (*path).to_string(),
                    delimiter: Some("/".into()),
                    role: Some(Role::Trash),
                    selectable: true,
                })
                .collect::<Vec<_>>()
        };

        let forwards = resolve_duplicate_roles(build(&["A/Trash", "B/Trash", "C/Trash"]));
        let backwards = resolve_duplicate_roles(build(&["C/Trash", "B/Trash", "A/Trash"]));

        let winner = |list: &[Discovered]| {
            list.iter()
                .find(|m| m.role.is_some())
                .map(|m| m.remote_path.clone())
        };

        assert_eq!(winner(&forwards), winner(&backwards));
        assert_eq!(winner(&forwards).as_deref(), Some("A/Trash"));
    }

    #[test]
    fn a_role_the_user_chose_wins_over_the_servers_while_the_mailbox_is_there() {
        let listing = vec![
            Discovered {
                remote_path: "INBOX".into(),
                display_name: "Inbox".into(),
                delimiter: Some("/".into()),
                role: Some(Role::Inbox),
                selectable: true,
            },
            Discovered {
                remote_path: "Trash".into(),
                display_name: "Trash".into(),
                delimiter: Some("/".into()),
                role: Some(Role::Trash),
                selectable: true,
            },
            Discovered {
                remote_path: "Old Sent".into(),
                display_name: "Old Sent".into(),
                delimiter: Some("/".into()),
                role: Some(Role::Sent),
                selectable: true,
            },
        ];
        let pending = super::super::ops::PendingTree::default();

        // Nothing chosen: the server's word.
        assert_eq!(
            effective_roles(&listing, &[], &pending),
            vec![Some(Role::Inbox), Some(Role::Trash), Some(Role::Sent)]
        );

        // Old Sent chosen as the Bin: it gives up Sent, and Trash gives up the Bin.
        assert_eq!(
            effective_roles(&listing, &[(Role::Trash, "Old Sent".into())], &pending),
            vec![Some(Role::Inbox), None, Some(Role::Trash)]
        );

        // A chosen mailbox the server no longer lists changes nothing.
        assert_eq!(
            effective_roles(&listing, &[(Role::Trash, "Gone".into())], &pending),
            vec![Some(Role::Inbox), Some(Role::Trash), Some(Role::Sent)]
        );
    }

    #[test]
    fn role_names_read_back_as_the_roles_that_wrote_them() {
        for role in [
            Role::Inbox,
            Role::Drafts,
            Role::Sent,
            Role::Junk,
            Role::Trash,
            Role::Archive,
            Role::All,
        ] {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse("flagged"), None);
    }

    #[test]
    fn roles_sort_into_the_order_the_sidebar_expects() {
        let mut roles = vec![Role::Trash, Role::Inbox, Role::Sent, Role::Drafts];
        roles.sort_by_key(|role| role.sort_order());

        assert_eq!(
            roles,
            vec![Role::Inbox, Role::Drafts, Role::Sent, Role::Trash]
        );
    }

    fn store_with_mailboxes() -> rusqlite::Connection {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open");
        conn.execute_batch("PRAGMA foreign_keys = ON;").expect("fk");
        crate::db::migrate::run(&mut conn).expect("migrate");

        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (1, 'T', 'me@t.test', 'other', 'password', 'halcyon:me')",
            [],
        )
        .expect("account");

        for (id, path) in [(1, "INBOX"), (2, "Archive"), (3, "Old Project")] {
            conn.execute(
                "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
                 VALUES (?1, 1, ?2, ?2, NULL)",
                rusqlite::params![id, path],
            )
            .expect("mailbox");
        }

        conn.execute(
            "INSERT INTO message (
                 id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
                 from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged, is_junk
             ) VALUES (1, 1, 3, 1, 'S', 0, 0, 10, 'a@b.test', '', '', 0, 0, 0, 0)",
            [],
        )
        .expect("message");

        conn
    }

    #[test]
    fn a_folder_the_server_no_longer_lists_is_removed() {
        // `persist` says a vanished mailbox is "removed by the caller only once it is sure", and
        // no caller ever was — so a folder deleted in webmail stayed in the sidebar for the life
        // of the install, with its messages in it and its unread count in the badge.
        let mut conn = store_with_mailboxes();
        let tx = conn.transaction().expect("tx");

        let removed = prune(&tx, 1, &["INBOX".to_string(), "Archive".to_string()]).expect("prune");

        assert_eq!(removed, vec!["Old Project".to_string()]);

        let left: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM mailbox WHERE account_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(left, 2);

        // Its mail goes with it, through the cascade. That is the point of requiring the caller
        // to be sure before calling this.
        let orphans: i64 = tx
            .query_row("SELECT COUNT(*) FROM message", [], |row| row.get(0))
            .expect("count");
        assert_eq!(orphans, 0, "the folder went and its messages did not");
    }

    #[test]
    fn an_empty_list_removes_nothing() {
        // A server answering with no folders at all is having a bad day, not reporting that the
        // account is empty. Treating it as evidence would delete the whole account's mail.
        let mut conn = store_with_mailboxes();
        let tx = conn.transaction().expect("tx");

        assert!(prune(&tx, 1, &[]).expect("prune").is_empty());

        let left: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM mailbox WHERE account_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(left, 3, "an empty list deleted mailboxes");
    }

    #[test]
    fn another_accounts_mailboxes_are_left_alone() {
        // The keep-list belongs to one account. Pruning across accounts would delete an
        // imported archive the moment any server account synced.
        let mut conn = store_with_mailboxes();

        conn.execute(
            "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
             VALUES (2, 'Local', 'local@localhost', 'local', 'none', 'local:none')",
            [],
        )
        .expect("account");
        conn.execute(
            "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
             VALUES (99, 2, 'Imported', 'Imported', NULL)",
            [],
        )
        .expect("mailbox");

        let tx = conn.transaction().expect("tx");
        prune(&tx, 1, &["INBOX".to_string()]).expect("prune");

        let survived: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM mailbox WHERE account_id = 2",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(survived, 1, "pruning one account touched another");
    }
}
