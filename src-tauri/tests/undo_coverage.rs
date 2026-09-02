//! Every command that changes a message records an undo step. docs/06 Phase 8.
//!
//! ## Why this is a source-level test
//!
//! The Phase 8 gate proves the undo *machinery* works: `phase8_gate.rs` calls `undo::capture`
//! and `undo::undo` directly and checks eleven action types restore exact prior state. All of it
//! passes, and all of it passed while undo was unreachable for the three actions a person
//! performs most often.
//!
//! The gap is that nothing checked whether the commands the UI actually calls *use* the
//! machinery. `msg_archive`, `msg_move` and `msg_delete` did. `msg_toggle_flag`,
//! `msg_toggle_read` and `msg_set_flags` did not, so flagging a message and marking it read
//! could not be undone — and because `Stack::push_step` silently drops an empty step and
//! `useUndo` shows no toast when there is nothing to undo, Ctrl+Z did nothing at all, with no
//! error anywhere to notice.
//!
//! A behavioural test would need a Tauri `State` and an `AppHandle`. This reads the source
//! instead, which is cruder and catches the whole class: a new mutating command that forgets.

const MAIL: &str = include_str!("../src/ipc/mail.rs");
const ORGANISE: &str = include_str!("../src/ipc/organise.rs");

/// Commands that change a message and must therefore be undoable.
///
/// Listed by name rather than detected, so adding a command is a deliberate decision about
/// whether it belongs here — a heuristic over `fn` names would quietly exempt anything named
/// unusually, which is the failure this test exists to prevent.
const MUST_RECORD: &[(&str, &str)] = &[
    (
        "msg_toggle_flag",
        "flagging is the most repeated action in the app",
    ),
    (
        "msg_toggle_read",
        "marking read is the second most repeated",
    ),
    ("msg_set_flags", "the general form of both"),
    (
        "msg_archive",
        "archive was the first of these to be caught, during Phase 10",
    ),
    ("msg_move", "moving to the wrong folder is what undo is for"),
    (
        "msg_delete",
        "deleting is the one nobody can afford to have be final",
    ),
];

/// The body of a function, from its signature to the start of the next top-level item.
fn body_of<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let signature = format!("pub async fn {name}(");
    let start = source.find(&signature)?;
    let rest = &source[start..];

    // The next top-level `#[tauri::command]` or doc block ends this one. Falling back to the
    // remainder of the file is fine for the last command in it.
    let end = rest[1..]
        .find("\n#[tauri::command]")
        .map(|at| at + 1)
        .unwrap_or(rest.len());

    Some(&rest[..end])
}

#[test]
fn every_mutating_command_captures_an_undo_step() {
    let mut missing = Vec::new();

    for (name, why) in MUST_RECORD {
        let body = body_of(MAIL, name)
            .or_else(|| body_of(ORGANISE, name))
            .unwrap_or_else(|| panic!("no command named {name}; has it been renamed?"));

        let captures = body.contains("undo::capture");
        let records = body.contains("stack.record");

        if !captures || !records {
            missing.push(format!(
                "  {name}: capture={captures} record={records} -- {why}"
            ));
        }
    }

    assert!(
        missing.is_empty(),
        "these commands change a message and cannot be undone:\n{}\n\n\
         Capturing without recording is just as broken as neither: `Stack::push_step` drops a \
         step it is never given, `undo_perform` then returns None, and `useUndo` shows no toast \
         for None -- so Ctrl+Z does nothing and says nothing.",
        missing.join("\n")
    );
}

/// How many times a thing appears in code, ignoring comments.
///
/// Counting raw occurrences reported a false failure the first time this ran: the doc comment
/// above `msg_toggle_flag` mentions `undo::capture` while explaining why the Phase 8 gate missed
/// this bug, and prose about a call is not a call. A test whose first result is a false alarm
/// teaches whoever sees the second one to ignore it.
fn occurrences_in_code(source: &str, needle: &str) -> usize {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .map(|line| line.matches(needle).count())
        .sum()
}

#[test]
fn a_command_that_captures_also_records() {
    // The halfway mistake. Capturing builds the step; recording is what puts it on the stack,
    // and a command that does the first without the second pays the cost and gets nothing.
    for (name, source) in [("mail.rs", MAIL), ("organise.rs", ORGANISE)] {
        let captures = occurrences_in_code(source, "undo::capture");
        let records = occurrences_in_code(source, "stack.record");

        assert!(
            records >= captures,
            "{name} captures {captures} undo steps but records only {records}; \
             at least one is built and thrown away"
        );
    }
}

#[test]
fn the_check_can_actually_fail() {
    // The body scanner is the part that could silently pass everything by returning a body that
    // happens to contain the strings, or by matching the wrong function.
    let body = body_of(MAIL, "msg_archive").expect("msg_archive exists");
    assert!(
        body.contains("undo::capture"),
        "the scanner did not find the capture in a command that certainly has one"
    );

    let read_only = body_of(MAIL, "messages_page").expect("messages_page exists");
    assert!(
        !read_only.contains("undo::capture"),
        "the scanner ran past the end of a function and into another one"
    );
}
