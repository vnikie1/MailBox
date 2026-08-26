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
    let lowered = leaf.trim().to_ascii_lowercase();

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

/// The display name for a mailbox: the leaf, with a provider container stripped.
///
/// `[Gmail]/Sent Mail` shows as "Sent Mail", not as the whole path — the sidebar nests it
/// under its parent, so repeating the parent in the label is noise.
pub fn display_name(path: &str, delimiter: Option<&str>) -> String {
    let separator = delimiter.unwrap_or("/");

    if path.eq_ignore_ascii_case("INBOX") {
        return "Inbox".to_string();
    }

    path.rsplit(separator)
        .next()
        .unwrap_or(path)
        .trim()
        .to_string()
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
pub fn persist(
    tx: &rusqlite::Transaction<'_>,
    account_id: i64,
    discovered: &[Discovered],
) -> Result<Vec<(i64, String)>, DbError> {
    let mut ids = Vec::with_capacity(discovered.len());

    for (index, mailbox) in discovered.iter().enumerate() {
        let sort_order = mailbox
            .role
            .map(Role::sort_order)
            .unwrap_or(100 + index as i64);

        tx.execute(
            "INSERT INTO mailbox (account_id, remote_path, display_name, role, sort_order, subscribed)
             VALUES (?1, ?2, ?3, ?4, ?5, 1)
             ON CONFLICT(account_id, remote_path) DO UPDATE SET
                 display_name = excluded.display_name,
                 role         = excluded.role,
                 sort_order   = excluded.sort_order",
            rusqlite::params![
                account_id,
                mailbox.remote_path,
                mailbox.display_name,
                mailbox.role.map(Role::as_str),
                sort_order,
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
    fn roles_sort_into_the_order_the_sidebar_expects() {
        let mut roles = vec![Role::Trash, Role::Inbox, Role::Sent, Role::Drafts];
        roles.sort_by_key(|role| role.sort_order());

        assert_eq!(
            roles,
            vec![Role::Inbox, Role::Drafts, Role::Sent, Role::Trash]
        );
    }
}
