//! The mailbox context menu's local half, and what happens when the server refuses it.

use rusqlite::{params, Connection};

use super::*;
use crate::sync::mailboxes::{self, Discovered};
use crate::sync::ops::{self, Op};

/// An account with a server (1), an imported archive with none (2), and a Gmail-shaped tree.
fn store() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open");
    conn.execute_batch("PRAGMA foreign_keys = ON;").expect("fk");
    crate::db::migrate::run(&mut conn).expect("migrate");

    conn.execute(
        "INSERT INTO account (id, display_name, email, provider, imap_host, imap_port,
                              imap_security, auth_kind, cred_ref)
         VALUES (1, 'Google', 'me@gmail.test', 'gmail', 'imap.gmail.test', 993, 'tls',
                 'password', 'halcyon:me')",
        [],
    )
    .expect("account");
    conn.execute(
        "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
         VALUES (2, 'Archive', 'local@localhost', 'local', 'none', 'local:none')",
        [],
    )
    .expect("local account");

    for (id, account, path, role) in [
        (1, 1, "INBOX", Some("inbox")),
        (2, 1, "[Gmail]/Trash", Some("trash")),
        (3, 1, "[Gmail]/Spam", Some("junk")),
        (4, 1, "Work", None),
        (5, 1, "Work/Clients", None),
        (6, 1, "[Gmail]/Starred", None),
        (7, 2, "Imported", None),
    ] {
        add_mailbox(&conn, id, account, path, role, Some("/"));
    }

    conn
}

fn add_mailbox(
    conn: &Connection,
    id: i64,
    account: i64,
    path: &str,
    role: Option<&str>,
    delimiter: Option<&str>,
) {
    conn.execute(
        "INSERT INTO mailbox (id, account_id, remote_path, display_name, role, delimiter)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            account,
            path,
            mailboxes::display_name(path, delimiter),
            role,
            delimiter
        ],
    )
    .expect("mailbox");
}

fn add_message(conn: &Connection, id: i64, mailbox: i64, seen: bool, snooze: Option<i64>) {
    let account: i64 = conn
        .query_row(
            "SELECT account_id FROM mailbox WHERE id = ?1",
            [mailbox],
            |row| row.get(0),
        )
        .expect("account of mailbox");

    conn.execute(
        "INSERT INTO message (
             id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
             from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged, is_junk,
             snooze_until, message_id
         ) VALUES (?1, ?2, ?3, ?1, 'S', 0, 0, 10, 'a@b.test', '', 'body', 0, ?4, 0, 0, ?5, ?6)",
        params![
            id,
            account,
            mailbox,
            i64::from(seen),
            snooze,
            format!("<{id}@test>")
        ],
    )
    .expect("message");
}

fn recount(conn: &mut Connection, mailboxes: &[i64]) {
    let tx = conn.transaction().expect("tx");
    write::recount_mailboxes(&tx, mailboxes).expect("recount");
    tx.commit().expect("commit");
}

fn row(conn: &Connection, id: i64) -> Option<(String, String, Option<String>)> {
    conn.query_row(
        "SELECT remote_path, display_name, delimiter FROM mailbox WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .ok()
}

fn queued(tx: &Transaction<'_>, account: i64) -> Vec<Op> {
    ops::queued(tx, account)
        .expect("queued")
        .into_iter()
        .map(|(_, op)| op)
        .collect()
}

fn invalid(result: Result<impl std::fmt::Debug, FolderError>) -> String {
    match result {
        Err(FolderError::Invalid(message)) => message,
        other => panic!("expected a refusal the user can read, got {other:?}"),
    }
}

// --------------------------------------------------------------------------- names

#[test]
fn a_name_is_trimmed_and_otherwise_kept() {
    assert_eq!(
        validate_name("  Receipts 2026 ", Some("/")).as_deref(),
        Ok("Receipts 2026")
    );
    assert_eq!(validate_name("Été", Some("/")).as_deref(), Ok("Été"));
    assert_eq!(validate_name("a.b", Some("/")).as_deref(), Ok("a.b"));
}

#[test]
fn a_name_that_cannot_be_a_single_mailbox_is_refused_in_words() {
    for (name, expected) in [
        ("", "Enter a name"),
        ("   ", "Enter a name"),
        ("Tab\there", "tabs or line breaks"),
        ("Two\nlines", "tabs or line breaks"),
        ("Q3/Q4", "can’t contain “/”"),
        ("100%", "“%” or “*”"),
        ("*", "“%” or “*”"),
    ] {
        let message = validate_name(name, Some("/")).expect_err(name);
        assert!(message.contains(expected), "{name:?}: {message}");
    }

    let long = "x".repeat(MAX_NAME + 1);
    assert!(validate_name(&long, Some("/")).is_err());
    assert!(validate_name(&"x".repeat(MAX_NAME), Some("/")).is_ok());
}

#[test]
fn the_separator_that_matters_is_the_servers() {
    // On a server that separates with ".", a slash is an ordinary character and a dot is not.
    assert!(validate_name("Q3/Q4", Some(".")).is_ok());
    assert!(validate_name("v1.2", Some(".")).is_err());
}

#[test]
fn only_folders_the_user_made_can_be_renamed_or_deleted() {
    assert!(!editable(Some("inbox"), "INBOX"));
    assert!(!editable(None, "inbox"));
    assert!(!editable(Some("trash"), "Trash"));
    assert!(!editable(None, "[Gmail]/Starred"));
    assert!(!editable(None, "[Google Mail]/Important"));
    assert!(editable(None, "Work"));
    assert!(editable(None, "Work/Clients"));
    assert!(editable(None, "INBOX.Receipts"));
}

fn folder_list(entries: &[(&str, Option<&str>)]) -> Vec<Folder> {
    entries
        .iter()
        .enumerate()
        .map(|(index, (path, delimiter))| Folder {
            id: index as i64 + 1,
            path: (*path).to_string(),
            delimiter: delimiter.map(str::to_string),
            role: None,
        })
        .collect()
}

#[test]
fn a_server_that_keeps_every_folder_inside_the_inbox_gets_the_next_one_there_too() {
    let courier = folder_list(&[
        ("INBOX", Some(".")),
        ("INBOX.Drafts", Some(".")),
        ("INBOX.Sent", Some(".")),
    ]);
    assert_eq!(personal_prefix(&courier), "INBOX.");

    let gmail = folder_list(&[("INBOX", Some("/")), ("[Gmail]/Sent Mail", Some("/"))]);
    assert_eq!(personal_prefix(&gmail), "");

    // One folder outside is enough to say the server does not require it.
    let mixed = folder_list(&[
        ("INBOX", Some(".")),
        ("INBOX.Sent", Some(".")),
        ("Archive", Some(".")),
    ]);
    assert_eq!(personal_prefix(&mixed), "");

    // Nothing to go on: the root.
    assert_eq!(personal_prefix(&folder_list(&[("INBOX", Some("."))])), "");
    assert_eq!(
        personal_prefix(&folder_list(&[("INBOX", None), ("INBOX.Sent", None)])),
        ""
    );
}

#[test]
fn a_renamed_folder_stays_where_it_was() {
    assert_eq!(parent_of("Work/Clients", Some("/")), "Work/");
    assert_eq!(parent_of("INBOX.Receipts", Some(".")), "INBOX.");
    assert_eq!(parent_of("Work", Some("/")), "");
    assert_eq!(parent_of("Work/Clients", None), "");
}

// -------------------------------------------------------------------------- create

#[test]
fn a_new_mailbox_is_in_the_sidebar_at_once_and_queued_for_the_server() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    let id = create(&tx, 1, "  Receipts ").expect("create");

    let (path, name, delimiter): (String, String, Option<String>) = tx
        .query_row(
            "SELECT remote_path, display_name, delimiter FROM mailbox WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("row");

    assert_eq!(path, "Receipts");
    assert_eq!(name, "Receipts");
    assert_eq!(
        delimiter.as_deref(),
        Some("/"),
        "the account's separator is recorded"
    );

    assert_eq!(
        queued(&tx, 1),
        vec![Op::CreateMailbox {
            path: "Receipts".into()
        }]
    );
}

#[test]
fn a_name_with_accents_is_stored_as_typed_and_sent_as_the_server_spells_it() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    let id = create(&tx, 1, "Reçus 2026").expect("create");

    let (path, name): (String, String) = tx
        .query_row(
            "SELECT remote_path, display_name FROM mailbox WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("row");

    assert_eq!(name, "Reçus 2026");
    assert_eq!(path, "Re&AOc-us 2026");
    assert_eq!(
        queued(&tx, 1),
        vec![Op::CreateMailbox {
            path: "Re&AOc-us 2026".into()
        }]
    );
}

#[test]
fn a_name_already_taken_is_refused_however_it_is_written() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    assert!(invalid(create(&tx, 1, "work")).contains("already a mailbox called “work”"));
    assert!(invalid(create(&tx, 1, "WORK")).contains("already"));
    assert!(invalid(create(&tx, 1, "inbox")).contains("inbox"));
    assert!(invalid(create(&tx, 1, "Work/New")).contains("“/”"));

    // A taken name with accents, compared as the user reads it.
    create(&tx, 1, "Été").expect("first");
    assert!(invalid(create(&tx, 1, "été")).contains("already"));

    assert_eq!(
        queued(&tx, 1).len(),
        1,
        "only the one real create is queued"
    );
}

#[test]
fn a_new_mailbox_on_a_server_that_nests_everything_goes_inside_the_inbox() {
    let mut conn = store();
    conn.execute(
        "INSERT INTO account (id, display_name, email, provider, imap_host, auth_kind, cred_ref)
         VALUES (3, 'Host', 'me@host.test', 'imap', 'mail.host.test', 'password', 'halcyon:h')",
        [],
    )
    .expect("account");
    add_mailbox(&conn, 30, 3, "INBOX", Some("inbox"), Some("."));
    add_mailbox(&conn, 31, 3, "INBOX.Sent", Some("sent"), Some("."));

    let tx = conn.transaction().expect("tx");
    let id = create(&tx, 3, "Projects").expect("create");

    let path: String = tx
        .query_row(
            "SELECT remote_path FROM mailbox WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .expect("row");
    assert_eq!(path, "INBOX.Projects");
    assert!(invalid(create(&tx, 3, "v1.2")).contains("“.”"));
}

#[test]
fn a_folder_in_an_imported_archive_is_made_here_and_nowhere_else() {
    // There is no server to tell. A queued CREATE would fail at every sync for ever.
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    create(&tx, 2, "Old Projects").expect("create");

    assert!(queued(&tx, 2).is_empty());
}

#[test]
fn an_account_that_has_gone_is_reported_as_gone() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    assert!(matches!(
        create(&tx, 99, "Anything"),
        Err(FolderError::Gone)
    ));
}

#[test]
fn a_mailbox_made_inside_another_is_named_by_its_parent_and_the_separator() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    let id = create_in(&tx, 1, Some(5), "Été 2026").expect("create");

    assert_eq!(
        row(&tx, id),
        Some((
            "Work/Clients/&AMk-t&AOk- 2026".to_string(),
            "Été 2026".to_string(),
            Some("/".to_string())
        ))
    );
    assert_eq!(
        queued(&tx, 1),
        vec![Op::CreateMailbox {
            path: "Work/Clients/&AMk-t&AOk- 2026".into()
        }]
    );

    // Taken is taken inside the same parent, and says which parent.
    assert!(invalid(create_in(&tx, 1, Some(4), "clients"))
        .contains("already a mailbox called “clients” in “Work”"));
    // The same name elsewhere is a different mailbox.
    create_in(&tx, 1, None, "Clients").expect("the same name at the top");
}

#[test]
fn a_mailbox_cannot_be_made_where_the_server_will_not_have_one() {
    let mut conn = store();
    add_mailbox(&conn, 8, 1, "Unlisted", None, None);
    let tx = conn.transaction().expect("tx");

    // The Inbox, Gmail's own folders, and a folder whose separator is not known yet.
    assert!(invalid(create_in(&tx, 1, Some(1), "Sub")).contains("inside “Inbox”"));
    assert!(invalid(create_in(&tx, 1, Some(6), "Sub")).contains("inside “Starred”"));
    assert!(invalid(create_in(&tx, 1, Some(8), "Sub")).contains("inside “Unlisted”"));

    // A parent in another account is, to this one, a mailbox that is not there.
    assert!(matches!(
        create_in(&tx, 1, Some(7), "Sub"),
        Err(FolderError::Gone)
    ));
    assert!(matches!(
        create_in(&tx, 1, Some(99), "Sub"),
        Err(FolderError::Gone)
    ));

    assert!(queued(&tx, 1).is_empty(), "nothing refused was queued");
}

#[test]
fn which_mailboxes_can_hold_others() {
    assert!(can_contain("Work", Some("/")));
    assert!(can_contain("Archive", Some("/")), "roles included");
    assert!(can_contain("INBOX.Projects", Some(".")));
    assert!(!can_contain("INBOX", Some("/")));
    assert!(!can_contain("inbox", Some(".")));
    assert!(!can_contain("[Gmail]/Starred", Some("/")));
    assert!(!can_contain("[Google Mail]/Bin", Some("/")));
    assert!(!can_contain("Work", None), "no separator, no path to write");
    assert!(!can_contain("Work", Some("")));
}

// -------------------------------------------------------------------------- rename

#[test]
fn a_renamed_folder_keeps_its_row_its_mail_and_its_children() {
    let mut conn = store();
    add_message(&conn, 1, 4, false, None);
    add_message(&conn, 2, 5, true, None);
    recount(&mut conn, &[4, 5]);

    let tx = conn.transaction().expect("tx");
    rename(&tx, 4, "Projects").expect("rename");

    assert_eq!(
        row(&tx, 4).map(|(path, name, _)| (path, name)),
        Some(("Projects".into(), "Projects".into()))
    );
    // The child moves with its parent and keeps its own name.
    assert_eq!(
        row(&tx, 5).map(|(path, name, _)| (path, name)),
        Some(("Projects/Clients".into(), "Clients".into()))
    );

    let messages: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM message WHERE mailbox_id IN (4, 5)",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(messages, 2, "a rename is not a delete and a create");

    assert_eq!(
        queued(&tx, 1),
        vec![Op::RenameMailbox {
            from: "Work".into(),
            to: "Projects".into(),
            delimiter: Some("/".into()),
        }]
    );
}

#[test]
fn a_child_is_renamed_where_it_is() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    rename(&tx, 5, "Customers").expect("rename");

    assert_eq!(
        row(&tx, 5).map(|(path, ..)| path).as_deref(),
        Some("Work/Customers")
    );
    assert_eq!(row(&tx, 4).map(|(path, ..)| path).as_deref(), Some("Work"));
}

#[test]
fn renaming_to_the_same_name_changes_nothing() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    rename(&tx, 4, " Work ").expect("rename");
    assert!(queued(&tx, 1).is_empty());

    // A change of case is a change.
    rename(&tx, 4, "WORK").expect("rename");
    assert_eq!(queued(&tx, 1).len(), 1);
}

#[test]
fn a_folder_the_account_depends_on_cannot_be_renamed() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    assert!(invalid(rename(&tx, 1, "Mail")).contains("can’t be renamed"));
    assert!(invalid(rename(&tx, 2, "Rubbish")).contains("can’t be renamed"));
    assert!(invalid(rename(&tx, 6, "Stars")).contains("can’t be renamed"));
    assert!(matches!(rename(&tx, 99, "X"), Err(FolderError::Gone)));
    assert!(queued(&tx, 1).is_empty());
}

#[test]
fn a_rename_onto_a_siblings_name_is_refused() {
    let mut conn = store();
    add_mailbox(&conn, 45, 1, "Work/Other", None, Some("/"));
    let tx = conn.transaction().expect("tx");

    assert!(invalid(rename(&tx, 5, "other")).contains("already a mailbox called “other”"));
    // The same leaf in a different place is a different folder.
    rename(&tx, 4, "Clients").expect("a top-level Clients beside Work/Clients");
}

// -------------------------------------------------------------------------- delete

#[test]
fn deleting_a_folder_takes_everything_inside_it() {
    let mut conn = store();
    add_message(&conn, 1, 4, false, None);
    add_message(&conn, 2, 5, false, None);
    add_message(&conn, 3, 5, true, Some(i64::MAX / 2));
    add_message(&conn, 4, 1, false, None);
    recount(&mut conn, &[1, 4, 5]);

    let tx = conn.transaction().expect("tx");
    let deleted = delete(&tx, 4).expect("delete");

    assert_eq!(deleted.account_id, 1);
    assert_eq!(deleted.messages, 3, "snoozed mail goes too");
    assert_eq!(deleted.mailbox_ids, vec![5, 4], "children first");
    assert!(row(&tx, 4).is_none());
    assert!(row(&tx, 5).is_none());

    let left: i64 = tx
        .query_row("SELECT COUNT(*) FROM message", [], |row| row.get(0))
        .expect("count");
    assert_eq!(left, 1, "the Inbox is untouched");

    assert_eq!(
        queued(&tx, 1),
        vec![Op::DeleteMailbox {
            paths: vec!["Work/Clients".into(), "Work".into()],
            delimiter: Some("/".into()),
        }]
    );
}

#[test]
fn a_folder_the_account_depends_on_cannot_be_deleted() {
    let mut conn = store();
    add_mailbox(&conn, 40, 1, "Projects", None, Some("/"));
    add_mailbox(&conn, 41, 1, "Projects/Archive", Some("archive"), Some("/"));

    let tx = conn.transaction().expect("tx");

    assert!(invalid(delete(&tx, 1)).contains("can’t be deleted"));
    assert!(invalid(delete(&tx, 3)).contains("can’t be deleted"));
    assert!(invalid(delete(&tx, 6)).contains("can’t be deleted"));
    // The folder is the user's, but the Archive inside it is the account's.
    assert!(invalid(delete(&tx, 40)).contains("contains the account’s Archive mailbox"));

    assert!(row(&tx, 40).is_some());
    assert!(queued(&tx, 1).is_empty());
}

#[test]
fn a_message_moved_out_of_a_deleted_folder_forgets_where_it_came_from() {
    // Its origin pointer would otherwise name a row that is gone — or, once SQLite reuses the
    // id, a different folder, and `locate` would send its UID against that one.
    let mut conn = store();
    add_message(&conn, 1, 4, false, None);

    let tx = conn.transaction().expect("tx");
    write::move_to(&tx, &[1], 1).expect("park in the Inbox");

    let origin: Option<i64> = tx
        .query_row(
            "SELECT origin_mailbox_id FROM message WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("origin");
    assert_eq!(origin, Some(4));

    delete(&tx, 4).expect("delete");

    let (origin, origin_uid): (Option<i64>, Option<i64>) = tx
        .query_row(
            "SELECT origin_mailbox_id, origin_uid FROM message WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("origin");
    assert_eq!((origin, origin_uid), (None, None));
    assert!(
        ops::locate(&tx, &[1]).expect("locate").is_empty(),
        "nothing can be sent for it"
    );
}

#[test]
fn deleting_in_an_imported_archive_queues_nothing() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    delete(&tx, 7).expect("delete");

    assert!(row(&tx, 7).is_none());
    assert!(queued(&tx, 2).is_empty());
}

// --------------------------------------------------------------------------- erase

#[test]
fn erasing_the_bin_removes_every_message_in_it_and_nothing_else() {
    let mut conn = store();
    // More than one batch, snoozed mail included, and mail elsewhere that must survive.
    for id in 1..=1_203 {
        add_message(
            &conn,
            id,
            2,
            id % 3 == 0,
            (id % 7 == 0).then_some(i64::MAX / 2),
        );
    }
    add_message(&conn, 5_000, 1, false, None);
    add_message(&conn, 5_001, 3, false, None);
    recount(&mut conn, &[1, 2, 3]);

    let tx = conn.transaction().expect("tx");
    let erased = erase(&tx, 1, Role::Trash).expect("erase");

    assert_eq!(erased.mailbox_id, 2);
    assert_eq!(erased.messages, 1_203);

    let (unread, total): (i64, i64) = tx
        .query_row(
            "SELECT unread_count, total_count FROM mailbox WHERE id = 2",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("counts");
    assert_eq!((unread, total), (0, 0), "the badge goes to nought");

    let survivors: i64 = tx
        .query_row("SELECT COUNT(*) FROM message", [], |row| row.get(0))
        .expect("count");
    assert_eq!(survivors, 2);

    assert_eq!(
        queued(&tx, 1),
        vec![Op::EraseMailbox {
            mailbox: "[Gmail]/Trash".into()
        }]
    );
}

#[test]
fn erasing_junk_empties_the_junk_mailbox() {
    let mut conn = store();
    add_message(&conn, 1, 3, false, None);
    add_message(&conn, 2, 2, false, None);

    let tx = conn.transaction().expect("tx");
    let erased = erase(&tx, 1, Role::Junk).expect("erase");

    assert_eq!((erased.mailbox_id, erased.messages), (3, 1));
    assert_eq!(
        queued(&tx, 1),
        vec![Op::EraseMailbox {
            mailbox: "[Gmail]/Spam".into()
        }]
    );
}

#[test]
fn an_empty_bin_is_still_erased_on_the_server() {
    // The local store holds only the newest mail of a folder. An empty Bin here says nothing
    // about the older messages the server still has in it.
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    assert_eq!(erase(&tx, 1, Role::Trash).expect("erase").messages, 0);
    assert_eq!(queued(&tx, 1).len(), 1);
}

#[test]
fn an_account_without_the_mailbox_says_so() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    assert!(invalid(erase(&tx, 2, Role::Trash)).contains("no Bin"));
    assert!(invalid(erase(&tx, 2, Role::Junk)).contains("no Junk"));
    assert!(matches!(
        erase(&tx, 99, Role::Trash),
        Err(FolderError::Gone)
    ));
}

// ------------------------------------------------------------------------ favourites

fn favourite_order(conn: &Connection, id: i64) -> Option<i64> {
    conn.query_row(
        "SELECT position FROM favourite WHERE mailbox_id = ?1",
        [id],
        |row| row.get(0),
    )
    .ok()
}

/// Favourites as the sidebar lists them: a built-in row by its key, a mailbox by its id.
fn favourites(conn: &Connection) -> Vec<String> {
    crate::db::query::favourites_list(conn)
        .expect("list")
        .into_iter()
        .map(|row| match (row.builtin, row.mailbox_id) {
            (Some(builtin), _) => builtin.key().to_string(),
            (None, Some(id)) => id.to_string(),
            (None, None) => unreachable!("the table's CHECK forbids it"),
        })
        .collect()
}

fn favourite_id(conn: &Connection, key: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM favourite WHERE builtin = ?1 OR CAST(mailbox_id AS TEXT) = ?1",
        [key],
        |row| row.get(0),
    )
    .expect("favourite")
}

#[test]
fn favourites_are_added_at_the_end_and_removed_without_moving_the_rest() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    assert_eq!(set_favourite(&tx, 4, true).expect("add"), 1);
    set_favourite(&tx, 7, true).expect("add another account's");
    set_favourite(&tx, 5, true).expect("add");

    // After the five rows every sidebar starts with.
    assert_eq!(favourite_order(&tx, 4), Some(6));
    assert_eq!(favourite_order(&tx, 7), Some(7));
    assert_eq!(favourite_order(&tx, 5), Some(8));

    // Adding again is not a move to the end.
    set_favourite(&tx, 4, true).expect("again");
    assert_eq!(favourite_order(&tx, 4), Some(6));

    set_favourite(&tx, 7, false).expect("remove");
    assert_eq!(favourite_order(&tx, 7), None);
    assert_eq!(
        favourite_order(&tx, 5),
        Some(8),
        "the others stay where they were"
    );

    assert!(
        queued(&tx, 1).is_empty(),
        "favourites never reach the server"
    );
    assert!(matches!(
        set_favourite(&tx, 99, true),
        Err(FolderError::Gone)
    ));
}

#[test]
fn a_renamed_favourite_is_still_a_favourite_and_a_deleted_one_is_not() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    set_favourite(&tx, 4, true).expect("add");
    set_favourite(&tx, 5, true).expect("add");
    rename(&tx, 4, "Projects").expect("rename");

    assert_eq!(favourite_order(&tx, 4), Some(6));

    // Deleting Work takes Work/Clients with it, and both favourites.
    delete(&tx, 4).expect("delete");
    assert_eq!(
        favourites(&tx),
        vec!["allInboxes", "vips", "flagged", "allDrafts", "allSent"]
    );
}

#[test]
fn a_favourite_moves_among_the_built_in_rows_and_nothing_else_moves() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    set_favourite(&tx, 4, true).expect("add");
    set_favourite(&tx, 5, true).expect("add");

    // Work, dragged above Flagged.
    let work = favourite_id(&tx, "4");
    move_favourite(&tx, work, Some(favourite_id(&tx, "flagged"))).expect("move");
    assert_eq!(
        favourites(&tx),
        vec![
            "allInboxes",
            "vips",
            "4",
            "flagged",
            "allDrafts",
            "allSent",
            "5"
        ]
    );

    // All Inboxes, dragged to the end.
    move_favourite(&tx, favourite_id(&tx, "allInboxes"), None).expect("to the end");
    assert_eq!(
        favourites(&tx),
        vec![
            "vips",
            "4",
            "flagged",
            "allDrafts",
            "allSent",
            "5",
            "allInboxes"
        ]
    );

    // Onto itself: nothing happens.
    move_favourite(&tx, work, Some(work)).expect("onto itself");
    assert_eq!(favourites(&tx)[1], "4");

    // Positions are dense again, so no run of moves can run out of room between two rows.
    let positions: Vec<i64> = tx
        .prepare("SELECT position FROM favourite ORDER BY position")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(positions, (1..=7).collect::<Vec<i64>>());

    // A mailbox dragged in lands where it was dropped; one already there moves there.
    add_favourite_at(&tx, 7, Some(favourite_id(&tx, "flagged"))).expect("dropped in");
    add_favourite_at(&tx, 5, Some(favourite_id(&tx, "vips"))).expect("dropped again");
    assert_eq!(
        favourites(&tx),
        vec![
            "5",
            "vips",
            "4",
            "7",
            "flagged",
            "allDrafts",
            "allSent",
            "allInboxes"
        ]
    );
    assert!(matches!(
        add_favourite_at(&tx, 99, None),
        Err(FolderError::Gone)
    ));

    assert!(matches!(
        move_favourite(&tx, 999, None),
        Err(FolderError::Gone)
    ));
    assert!(
        matches!(move_favourite(&tx, work, Some(999)), Err(FolderError::Gone)),
        "the row it was dropped against has gone"
    );
    assert!(queued(&tx, 1).is_empty());
}

// ------------------------------------------------------------- Use This Mailbox As

/// An account on a server that is not Gmail, with a Bin the server named and two folders.
fn plain_account(conn: &Connection) {
    conn.execute(
        "INSERT INTO account (id, display_name, email, provider, imap_host, auth_kind, cred_ref)
         VALUES (3, 'Host', 'me@host.test', 'imap', 'mail.host.test', 'password', 'halcyon:h')",
        [],
    )
    .expect("account");
    add_mailbox(conn, 30, 3, "INBOX", Some("inbox"), Some("/"));
    add_mailbox(conn, 31, 3, "Trash", Some("trash"), Some("/"));
    add_mailbox(conn, 32, 3, "Deleted Messages", None, Some("/"));
    add_mailbox(conn, 33, 3, "Old Sent", Some("sent"), Some("/"));
}

fn role_of(conn: &Connection, id: i64) -> Option<String> {
    conn.query_row("SELECT role FROM mailbox WHERE id = ?1", [id], |row| {
        row.get(0)
    })
    .expect("row")
}

#[test]
fn a_mailbox_used_as_the_bin_takes_the_role_from_the_one_that_had_it() {
    let mut conn = store();
    plain_account(&conn);
    let tx = conn.transaction().expect("tx");

    assert_eq!(use_as(&tx, 32, Role::Trash).expect("use as"), 3);
    assert_eq!(role_of(&tx, 32).as_deref(), Some("trash"));
    assert_eq!(role_of(&tx, 31), None, "one Bin per account");

    // And a mailbox has one role: Old Sent, made the Archive, is no longer Sent.
    use_as(&tx, 33, Role::Archive).expect("use as");
    assert_eq!(role_of(&tx, 33).as_deref(), Some("archive"));

    // Choosing again moves the choice rather than adding a second.
    use_as(&tx, 32, Role::Archive).expect("use as");
    assert_eq!(role_of(&tx, 32).as_deref(), Some("archive"));
    assert_eq!(role_of(&tx, 33), None);

    let choices: Vec<(String, i64)> = tx
        .prepare("SELECT role, mailbox_id FROM mailbox_role ORDER BY role")
        .expect("prepare")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(choices, vec![("archive".to_string(), 32)]);

    assert!(queued(&tx, 3).is_empty(), "the server is not told");
}

#[test]
fn a_role_is_not_offered_where_it_would_mean_nothing_or_harm() {
    let mut conn = store();
    plain_account(&conn);
    let tx = conn.transaction().expect("tx");

    assert!(invalid(use_as(&tx, 30, Role::Archive)).contains("Inbox"));
    assert!(invalid(use_as(&tx, 32, Role::Inbox)).contains("only be used as"));
    assert!(invalid(use_as(&tx, 32, Role::All)).contains("only be used as"));
    // Gmail, by provider.
    assert!(invalid(use_as(&tx, 4, Role::Trash)).contains("Gmail decides"));
    // An imported archive files nothing.
    assert!(invalid(use_as(&tx, 7, Role::Trash)).contains("isn’t on a server"));
    assert!(matches!(
        use_as(&tx, 99, Role::Trash),
        Err(FolderError::Gone)
    ));

    assert_eq!(role_of(&tx, 31).as_deref(), Some("trash"), "nothing moved");
}

#[test]
fn a_google_address_added_by_hand_is_gmail_too() {
    let mut conn = store();
    conn.execute(
        "INSERT INTO account (id, display_name, email, provider, imap_host, auth_kind, cred_ref)
         VALUES (4, 'Mine', 'other@gmail.test', 'imap', 'IMAP.Gmail.com.', 'password', 'halcyon:g')",
        [],
    )
    .expect("account");
    add_mailbox(&conn, 40, 4, "Receipts", None, Some("/"));
    let tx = conn.transaction().expect("tx");

    assert!(invalid(use_as(&tx, 40, Role::Archive)).contains("Gmail decides"));
}

#[test]
fn a_chosen_role_outlasts_the_sync_that_says_otherwise() {
    let mut conn = store();
    plain_account(&conn);
    let tx = conn.transaction().expect("tx");

    use_as(&tx, 32, Role::Trash).expect("use as");

    // The server goes on calling "Trash" the Bin.
    let mut listing = listed(&["INBOX", "Trash", "Deleted Messages", "Old Sent"]);
    listing[0].role = Some(Role::Inbox);
    listing[1].role = Some(Role::Trash);
    listing[3].role = Some(Role::Sent);
    mailboxes::persist(&tx, 3, &listing).expect("persist");

    assert_eq!(role_of(&tx, 32).as_deref(), Some("trash"));
    assert_eq!(role_of(&tx, 31), None);
    assert_eq!(
        role_of(&tx, 33).as_deref(),
        Some("sent"),
        "the rest as the server says"
    );
    assert_eq!(role_of(&tx, 30).as_deref(), Some("inbox"));

    // Deleted in webmail: the choice goes with the mailbox, and the server's Bin is the Bin again.
    let without = listed(&["INBOX", "Trash", "Old Sent"]);
    let mut without = without;
    without[0].role = Some(Role::Inbox);
    without[1].role = Some(Role::Trash);
    without[2].role = Some(Role::Sent);
    mailboxes::persist(&tx, 3, &without).expect("persist");
    assert_eq!(
        role_of(&tx, 31).as_deref(),
        Some("trash"),
        "a choice of a mailbox that has gone does not leave the account without a Bin"
    );
    let keep: Vec<String> = without.iter().map(|m| m.remote_path.clone()).collect();
    mailboxes::prune(&tx, 3, &keep).expect("prune");

    let choices: i64 = tx
        .query_row("SELECT COUNT(*) FROM mailbox_role", [], |row| row.get(0))
        .expect("count");
    assert_eq!(choices, 0);
}

#[test]
fn a_renamed_mailbox_keeps_the_role_it_was_given() {
    let mut conn = store();
    plain_account(&conn);
    let tx = conn.transaction().expect("tx");

    // Old Sent is a role folder, so not editable; Deleted Messages is, until it is chosen.
    rename(&tx, 32, "Binned").expect("rename");
    use_as(&tx, 32, Role::Trash).expect("use as");

    // The rename is on its way; a sync meanwhile still lists the old name.
    let mut listing = listed(&["INBOX", "Trash", "Deleted Messages"]);
    listing[1].role = Some(Role::Trash);
    mailboxes::persist(&tx, 3, &listing).expect("persist");

    assert_eq!(
        role_of(&tx, 32).as_deref(),
        Some("trash"),
        "the renamed row keeps the role while its new name is on its way"
    );
    assert_eq!(role_of(&tx, 31), None);
}

// ------------------------------------------------------------------------- Rebuild

#[test]
fn rebuild_is_asked_for_and_left_to_the_sync() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    assert_eq!(request_rebuild(&tx, 4).expect("rebuild"), 1);
    let requested: bool = tx
        .query_row(
            "SELECT rebuild_requested FROM mailbox WHERE id = 4",
            [],
            |row| row.get(0),
        )
        .expect("row");
    assert!(requested);
    assert!(
        queued(&tx, 1).is_empty(),
        "nothing to send: the sync reads the mailbox"
    );

    assert!(invalid(request_rebuild(&tx, 7)).contains("isn’t on a server"));
    assert!(matches!(request_rebuild(&tx, 99), Err(FolderError::Gone)));
}

// ------------------------------------------------------- the sync, while changes wait

fn listed(paths: &[&str]) -> Vec<Discovered> {
    paths
        .iter()
        .map(|path| Discovered {
            remote_path: (*path).to_string(),
            display_name: mailboxes::display_name(path, Some("/")),
            delimiter: Some("/".into()),
            role: None,
            selectable: true,
        })
        .collect()
}

fn paths(conn: &Connection, account: i64) -> Vec<String> {
    let mut statement = conn
        .prepare("SELECT remote_path FROM mailbox WHERE account_id = ?1 ORDER BY remote_path")
        .expect("prepare");
    let rows = statement
        .query_map([account], |row| row.get(0))
        .expect("query");
    rows.collect::<rusqlite::Result<Vec<_>>>().expect("rows")
}

const SERVER: [&str; 6] = [
    "INBOX",
    "[Gmail]/Trash",
    "[Gmail]/Spam",
    "Work",
    "Work/Clients",
    "[Gmail]/Starred",
];

#[test]
fn a_new_folder_survives_a_sync_that_ran_before_the_server_had_it() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    create(&tx, 1, "Receipts").expect("create");

    mailboxes::persist(&tx, 1, &listed(&SERVER)).expect("persist");
    let keep: Vec<String> = SERVER.iter().map(|p| (*p).to_string()).collect();
    let removed = mailboxes::prune(&tx, 1, &keep).expect("prune");

    assert!(removed.is_empty(), "pruned {removed:?}");
    assert!(paths(&tx, 1).contains(&"Receipts".to_string()));
}

#[test]
fn a_renamed_folder_is_not_recreated_or_pruned_while_the_rename_is_on_its_way() {
    // The failure this prevents: the LIST still says "Work", so the sync would insert a fresh
    // empty "Work" beside the renamed folder — and then delete "Projects", with its mail, for
    // not being on the server.
    let mut conn = store();
    add_message(&conn, 1, 4, false, None);
    let tx = conn.transaction().expect("tx");

    rename(&tx, 4, "Projects").expect("rename");

    let written = mailboxes::persist(&tx, 1, &listed(&SERVER)).expect("persist");
    let keep: Vec<String> = SERVER.iter().map(|p| (*p).to_string()).collect();
    mailboxes::prune(&tx, 1, &keep).expect("prune");

    assert!(
        !written
            .iter()
            .any(|(_, path)| path == "Work" || path == "Work/Clients"),
        "the old names were written back: {written:?}"
    );

    let now = paths(&tx, 1);
    assert!(now.contains(&"Projects".to_string()), "{now:?}");
    assert!(now.contains(&"Projects/Clients".to_string()), "{now:?}");
    assert!(!now.contains(&"Work".to_string()), "{now:?}");

    let mail: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM message WHERE mailbox_id = 4",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(mail, 1);
}

#[test]
fn a_deleted_folder_is_not_brought_back_by_a_sync_that_still_lists_it() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    delete(&tx, 4).expect("delete");
    mailboxes::persist(&tx, 1, &listed(&SERVER)).expect("persist");

    let now = paths(&tx, 1);
    assert!(!now.contains(&"Work".to_string()), "{now:?}");
    assert!(!now.contains(&"Work/Clients".to_string()), "{now:?}");
}

#[test]
fn once_the_server_has_the_change_the_listing_is_believed_again() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    create(&tx, 1, "Receipts").expect("create");
    for (id, _) in ops::queued(&tx, 1).expect("queued") {
        ops::forget(&tx, id).expect("drained");
    }

    // The server never made it — say another client deleted it straight away. With nothing
    // pending, its absence is evidence again.
    let keep: Vec<String> = SERVER.iter().map(|p| (*p).to_string()).collect();
    let removed = mailboxes::prune(&tx, 1, &keep).expect("prune");
    assert_eq!(removed, vec!["Receipts".to_string()]);
}

#[test]
fn the_separator_is_learnt_from_the_listing() {
    let mut conn = store();
    conn.execute("UPDATE mailbox SET delimiter = NULL", [])
        .expect("forget");
    let tx = conn.transaction().expect("tx");

    mailboxes::persist(&tx, 1, &listed(&SERVER)).expect("persist");

    assert_eq!(row(&tx, 4).and_then(|(_, _, d)| d).as_deref(), Some("/"));
}

// --------------------------------------------------------- when the server says no

fn enqueue(tx: &Transaction<'_>, account: i64, op: Op) {
    ops::enqueue(tx, account, &op).expect("enqueue");
}

/// The id of the queued operation that is the given kind of change, for `abandon_*`.
fn id_of(tx: &Transaction<'_>, account: i64, matches: impl Fn(&Op) -> bool) -> i64 {
    ops::queued(tx, account)
        .expect("queued")
        .into_iter()
        .find(|(_, op)| matches(op))
        .map(|(id, _)| id)
        .expect("queued op")
}

#[test]
fn a_folder_the_server_refused_goes_and_the_mail_moved_into_it_goes_back() {
    let mut conn = store();
    add_message(&conn, 1, 1, false, None);
    add_message(&conn, 2, 1, true, None);
    recount(&mut conn, &[1]);

    let tx = conn.transaction().expect("tx");
    let folder = create(&tx, 1, "Receipts").expect("create");

    // What `msg_move` does: locate first, queue the move, then park the rows.
    for group in ops::locate(&tx, &[1, 2]).expect("locate") {
        enqueue(
            &tx,
            1,
            Op::Move {
                from: group.mailbox,
                to: "Receipts".into(),
                uids: group.uids,
            },
        );
    }
    write::move_to(&tx, &[1, 2], folder).expect("move");
    // And something unrelated, which must be left alone.
    enqueue(
        &tx,
        1,
        Op::Flag {
            mailbox: "INBOX".into(),
            uids: vec![9],
            seen: Some(true),
            flagged: None,
        },
    );

    let refused = id_of(&tx, 1, |op| matches!(op, Op::CreateMailbox { .. }));
    ops::forget(&tx, refused).expect("the drain forgets it first");
    let name = abandon_created(&tx, 1, refused, "Receipts").expect("abandon");

    assert_eq!(name, "Receipts");
    assert!(!paths(&tx, 1).contains(&"Receipts".to_string()));

    let back: Vec<(i64, i64, i64)> = {
        let mut statement = tx
            .prepare("SELECT id, mailbox_id, uid FROM message ORDER BY id")
            .expect("prepare");
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("query");
        rows.collect::<rusqlite::Result<Vec<_>>>().expect("rows")
    };
    assert_eq!(
        back,
        vec![(1, 1, 1), (2, 1, 2)],
        "back in the Inbox under their own UIDs"
    );

    let (unread, total): (i64, i64) = tx
        .query_row(
            "SELECT unread_count, total_count FROM mailbox WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("counts");
    assert_eq!((unread, total), (1, 2));

    assert_eq!(
        queued(&tx, 1),
        vec![Op::Flag {
            mailbox: "INBOX".into(),
            uids: vec![9],
            seen: Some(true),
            flagged: None,
        }],
        "the move into the folder went; the unrelated flag stayed"
    );
}

#[test]
fn a_refused_folder_is_followed_through_its_renames() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    let folder = create(&tx, 1, "Receipts").expect("create");
    rename(&tx, folder, "Bills").expect("rename");
    rename(&tx, folder, "Invoices").expect("rename again");

    let refused = id_of(&tx, 1, |op| matches!(op, Op::CreateMailbox { .. }));
    ops::forget(&tx, refused).expect("forget");
    let name = abandon_created(&tx, 1, refused, "Receipts").expect("abandon");

    assert_eq!(name, "Invoices", "named as the sidebar last showed it");
    assert!(row(&tx, folder).is_none());
    assert!(queued(&tx, 1).is_empty(), "both renames went with it");
}

#[test]
fn a_refused_folder_that_was_already_deleted_needs_nothing_more() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    let folder = create(&tx, 1, "Receipts").expect("create");
    delete(&tx, folder).expect("delete");
    // A new folder of the same name is a different folder, and its create must survive.
    create(&tx, 1, "Receipts").expect("again");

    let refused = id_of(&tx, 1, |op| matches!(op, Op::CreateMailbox { .. }));
    ops::forget(&tx, refused).expect("forget");
    abandon_created(&tx, 1, refused, "Receipts").expect("abandon");

    assert_eq!(
        queued(&tx, 1),
        vec![Op::CreateMailbox {
            path: "Receipts".into()
        }]
    );
    assert!(paths(&tx, 1).contains(&"Receipts".to_string()));
}

#[test]
fn a_refused_folder_takes_the_folders_made_inside_it_with_it() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    let parent = create(&tx, 1, "Receipts").expect("create");
    let child = create_in(&tx, 1, Some(parent), "2026").expect("inside it");
    // Beside it, however alike the name.
    let neighbour = create(&tx, 1, "Receipts 2026").expect("beside it");

    let refused = id_of(
        &tx,
        1,
        |op| matches!(op, Op::CreateMailbox { path } if path == "Receipts"),
    );
    ops::forget(&tx, refused).expect("forget");
    let name = abandon_created(&tx, 1, refused, "Receipts").expect("abandon");

    assert_eq!(name, "Receipts");
    assert!(row(&tx, parent).is_none());
    assert!(
        row(&tx, child).is_none(),
        "the server has the folder inside no more than the folder"
    );
    assert!(row(&tx, neighbour).is_some());
    assert_eq!(
        queued(&tx, 1),
        vec![Op::CreateMailbox {
            path: "Receipts 2026".into()
        }],
        "the create of the folder inside went too; the one beside it stayed"
    );
}

#[test]
fn a_refused_rename_puts_the_old_name_back_everywhere() {
    let mut conn = store();
    add_message(&conn, 1, 4, false, None);
    let tx = conn.transaction().expect("tx");

    rename(&tx, 4, "Projects").expect("rename");
    // Queued after the rename, so written with the new name.
    enqueue(
        &tx,
        1,
        Op::Flag {
            mailbox: "Projects".into(),
            uids: vec![1],
            seen: Some(true),
            flagged: None,
        },
    );
    enqueue(
        &tx,
        1,
        Op::Move {
            from: "INBOX".into(),
            to: "Projects/Clients".into(),
            uids: vec![3],
        },
    );

    let refused = id_of(&tx, 1, |op| matches!(op, Op::RenameMailbox { .. }));
    ops::forget(&tx, refused).expect("forget");
    abandon_rename(&tx, 1, refused, "Work", "Projects", Some("/")).expect("abandon");

    assert_eq!(
        row(&tx, 4).map(|(path, name, _)| (path, name)),
        Some(("Work".into(), "Work".into()))
    );
    assert_eq!(
        row(&tx, 5).map(|(path, ..)| path).as_deref(),
        Some("Work/Clients")
    );

    assert_eq!(
        queued(&tx, 1),
        vec![
            Op::Flag {
                mailbox: "Work".into(),
                uids: vec![1],
                seen: Some(true),
                flagged: None,
            },
            Op::Move {
                from: "INBOX".into(),
                to: "Work/Clients".into(),
                uids: vec![3],
            },
        ]
    );
}

#[test]
fn a_second_rename_after_a_refused_one_starts_from_the_servers_name() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    rename(&tx, 4, "Projects").expect("rename");
    rename(&tx, 4, "Clients Archive").expect("rename again");

    let refused = id_of(
        &tx,
        1,
        |op| matches!(op, Op::RenameMailbox { to, .. } if to == "Projects"),
    );
    ops::forget(&tx, refused).expect("forget");
    abandon_rename(&tx, 1, refused, "Work", "Projects", Some("/")).expect("abandon");

    assert_eq!(
        queued(&tx, 1),
        vec![Op::RenameMailbox {
            from: "Work".into(),
            to: "Clients Archive".into(),
            delimiter: Some("/".into()),
        }]
    );
    // The row already has the name the next attempt asks for.
    assert_eq!(
        row(&tx, 4).map(|(path, ..)| path).as_deref(),
        Some("Clients Archive")
    );
}

#[test]
fn a_refused_rename_whose_old_name_was_reused_drops_the_renamed_copy() {
    let mut conn = store();
    let tx = conn.transaction().expect("tx");

    rename(&tx, 4, "Projects").expect("rename");
    let fresh = create(&tx, 1, "Work").expect("a new Work");

    let refused = id_of(&tx, 1, |op| matches!(op, Op::RenameMailbox { .. }));
    ops::forget(&tx, refused).expect("forget");
    abandon_rename(&tx, 1, refused, "Work", "Projects", Some("/")).expect("abandon");

    assert!(row(&tx, 4).is_none(), "the renamed copy went");
    assert!(row(&tx, 5).is_none(), "with its child");
    assert!(row(&tx, fresh).is_some(), "the new folder stayed");
}

#[test]
fn a_parked_message_whose_origin_row_vanished_is_not_sent_against_another_folder() {
    // `locate` used to pair the origin UID with the message's *current* folder whenever the
    // origin row was missing. Here the origin is simply gone, as a raw DELETE leaves it.
    let mut conn = store();
    add_message(&conn, 1, 4, false, None);
    conn.execute("UPDATE message SET uid = 77 WHERE id = 1", [])
        .expect("uid");

    let tx = conn.transaction().expect("tx");
    write::move_to(&tx, &[1], 1).expect("park");
    // `origin_mailbox_id` has no foreign key, so nothing stops this.
    tx.execute(
        "UPDATE message SET origin_mailbox_id = 999 WHERE id = 1",
        [],
    )
    .expect("dangle");

    assert!(ops::locate(&tx, &[1]).expect("locate").is_empty());
}
