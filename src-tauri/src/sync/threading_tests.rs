//! Threading tests, written **before** the threading code. docs/06 Phase 5, verbatim:
//! *"Write the threading tests before the threading code."*
//!
//! They are in their own file rather than a `#[cfg(test)] mod` inside `threading.rs` so that
//! the order is visible in the repository rather than only claimed here: this file existed,
//! and failed to compile, before `threading.rs` did.
//!
//! What is being pinned is JWZ's algorithm (docs/03 §5) and the four things about it that
//! actually decide whether a mailbox reads correctly:
//!
//! * a reply joins its parent's thread even when the subject was rewritten;
//! * a message that arrives late and *bridges* two existing threads merges them;
//! * a shared subject alone never merges threads that have no reference link — otherwise
//!   every "Re: lunch?" in a decade lands in one conversation;
//! * a `References` cycle, which real mail contains, terminates.

use super::threading::{subject_base, thread_messages, Threadable};

fn message(id: i64, message_id: &str, subject: &str) -> Threadable {
    Threadable {
        id,
        message_id: Some(message_id.to_string()),
        in_reply_to: None,
        references: Vec::new(),
        subject: subject.to_string(),
        date: id * 1000,
        gm_thrid: None,
        thread_id: None,
    }
}

fn reply(id: i64, message_id: &str, subject: &str, parent: &str) -> Threadable {
    Threadable {
        in_reply_to: Some(parent.to_string()),
        references: vec![parent.to_string()],
        ..message(id, message_id, subject)
    }
}

/// The thread each message landed in, as a set of id-groups, so assertions do not depend on
/// which arbitrary key the implementation chose.
fn groups(messages: &[Threadable]) -> Vec<Vec<i64>> {
    let assignments = thread_messages(messages);

    let mut by_thread: std::collections::BTreeMap<i64, Vec<i64>> =
        std::collections::BTreeMap::new();
    for assignment in assignments {
        by_thread
            .entry(assignment.thread_key)
            .or_default()
            .push(assignment.message_id);
    }

    let mut out: Vec<Vec<i64>> = by_thread
        .into_values()
        .map(|mut ids| {
            ids.sort_unstable();
            ids
        })
        .collect();

    out.sort();
    out
}

/* ------------------------------------------------------------------ subject normalisation */

#[test]
fn subject_base_strips_the_prefixes_people_actually_send() {
    // docs/03 §5 names Re:, Fwd:, AW: and [list] — case- and whitespace-insensitive.
    assert_eq!(subject_base("Re: lunch"), "lunch");
    assert_eq!(subject_base("RE : lunch"), "lunch");
    assert_eq!(subject_base("re:lunch"), "lunch");
    assert_eq!(subject_base("Fwd: lunch"), "lunch");
    assert_eq!(subject_base("FW: lunch"), "lunch");
    assert_eq!(subject_base("AW: lunch"), "lunch");
    assert_eq!(subject_base("[mailing-list] lunch"), "lunch");
}

#[test]
fn subject_base_strips_repeated_prefixes() {
    // Six rounds of a forwarded reply is not unusual.
    assert_eq!(subject_base("Re: Re: Fwd: Re: lunch"), "lunch");
    assert_eq!(subject_base("[list] Re: [list] Fwd: lunch"), "lunch");
}

#[test]
fn subject_base_does_not_eat_a_subject_that_merely_starts_with_those_letters() {
    // "Reminder" begins with "Re" and is not a reply. Stripping it would silently merge
    // unrelated conversations, which is the failure mode people notice and never forgive.
    assert_eq!(subject_base("Reminder: standup"), "reminder: standup");
    assert_eq!(subject_base("Reference request"), "reference request");
    assert_eq!(subject_base("AWS bill"), "aws bill");
    assert_eq!(subject_base("Forward planning"), "forward planning");
}

#[test]
fn subject_base_is_empty_for_a_subject_that_is_only_prefixes() {
    assert_eq!(subject_base("Re:"), "");
    assert_eq!(subject_base("   "), "");
}

/* ---------------------------------------------------------------------------- the algorithm */

#[test]
fn a_lone_message_is_its_own_thread() {
    let messages = vec![message(1, "<a@x>", "hello")];

    assert_eq!(groups(&messages), vec![vec![1]]);
}

#[test]
fn a_reply_joins_its_parent() {
    let messages = vec![
        message(1, "<a@x>", "hello"),
        reply(2, "<b@x>", "Re: hello", "<a@x>"),
    ];

    assert_eq!(groups(&messages), vec![vec![1, 2]]);
}

#[test]
fn a_reply_joins_its_parent_even_when_the_subject_was_rewritten() {
    // People rename threads mid-conversation. The reference link is the truth; the subject
    // is a hint used only where no link exists.
    let messages = vec![
        message(1, "<a@x>", "hello"),
        reply(2, "<b@x>", "completely different now", "<a@x>"),
    ];

    assert_eq!(groups(&messages), vec![vec![1, 2]]);
}

#[test]
fn a_deep_chain_stays_one_thread_however_it_is_ordered() {
    // Mail arrives out of order constantly — a backfill walks *backwards* through UIDs, so
    // the reply is threaded before the message it answers.
    let mut messages = vec![
        message(1, "<a@x>", "spec review"),
        reply(2, "<b@x>", "Re: spec review", "<a@x>"),
        Threadable {
            references: vec!["<a@x>".into(), "<b@x>".into()],
            ..reply(3, "<c@x>", "Re: spec review", "<b@x>")
        },
        Threadable {
            references: vec!["<a@x>".into(), "<b@x>".into(), "<c@x>".into()],
            ..reply(4, "<d@x>", "Re: spec review", "<c@x>")
        },
    ];

    assert_eq!(groups(&messages), vec![vec![1, 2, 3, 4]]);

    messages.reverse();
    assert_eq!(groups(&messages), vec![vec![1, 2, 3, 4]]);
}

#[test]
fn a_message_that_bridges_two_threads_merges_them() {
    // docs/03 §5: *re-thread incrementally when a message arrives that bridges two threads*.
    // Two roots exist independently; a later message references both, and they become one.
    let bridging = Threadable {
        references: vec!["<a@x>".into(), "<b@x>".into()],
        ..message(3, "<c@x>", "Re: merged")
    };

    let separate = vec![message(1, "<a@x>", "first"), message(2, "<b@x>", "second")];
    assert_eq!(groups(&separate), vec![vec![1], vec![2]]);

    let bridged = vec![separate[0].clone(), separate[1].clone(), bridging];
    assert_eq!(groups(&bridged), vec![vec![1, 2, 3]]);
}

#[test]
fn an_identical_subject_alone_never_merges_two_conversations() {
    // The single most damaging over-eager behaviour a mail client can have. Ten years of
    // "Re: lunch?" from different people must not become one conversation.
    let messages = vec![
        message(1, "<a@x>", "lunch?"),
        message(2, "<b@x>", "Re: lunch?"),
        message(3, "<c@x>", "Re: lunch?"),
    ];

    assert_eq!(groups(&messages), vec![vec![1], vec![2], vec![3]]);
}

#[test]
fn a_reference_to_a_message_we_do_not_have_still_groups_its_replies() {
    // Two replies to a message that was deleted, or never delivered here, belong together —
    // JWZ's empty-container case.
    let messages = vec![
        reply(1, "<a@x>", "Re: proposal", "<missing@x>"),
        reply(2, "<b@x>", "Re: proposal", "<missing@x>"),
    ];

    assert_eq!(groups(&messages), vec![vec![1, 2]]);
}

#[test]
fn a_reference_cycle_terminates() {
    // Real mail contains these: broken clients, mailing-list rewriting, forged headers. A
    // naive parent-walk hangs the sync thread forever, which looks like the app freezing.
    let a = Threadable {
        references: vec!["<b@x>".into()],
        in_reply_to: Some("<b@x>".into()),
        ..message(1, "<a@x>", "loop")
    };
    let b = Threadable {
        references: vec!["<a@x>".into()],
        in_reply_to: Some("<a@x>".into()),
        ..message(2, "<b@x>", "loop")
    };

    assert_eq!(groups(&[a, b]), vec![vec![1, 2]]);
}

#[test]
fn a_message_that_references_itself_terminates() {
    let looped = Threadable {
        references: vec!["<a@x>".into()],
        in_reply_to: Some("<a@x>".into()),
        ..message(1, "<a@x>", "self")
    };

    assert_eq!(groups(&[looped]), vec![vec![1]]);
}

#[test]
fn messages_with_no_message_id_do_not_all_collapse_together() {
    // A missing Message-ID is malformed but common. Treating absent ids as equal would put
    // every such message in one thread — standing rule 13, degrade visibly, not wrongly.
    let anonymous = |id: i64| Threadable {
        message_id: None,
        ..message(id, "", "notification")
    };

    assert_eq!(
        groups(&[anonymous(1), anonymous(2), anonymous(3)]),
        vec![vec![1], vec![2], vec![3]]
    );
}

#[test]
fn duplicate_message_ids_do_not_lose_a_message() {
    // Two messages claiming the same Message-ID happens with mailing lists and with Gmail's
    // "All Mail". Whatever they thread into, neither may vanish from the output.
    let messages = vec![
        message(1, "<same@x>", "duplicate"),
        message(2, "<same@x>", "duplicate"),
    ];

    let flattened: Vec<i64> = groups(&messages).into_iter().flatten().collect();
    assert_eq!(flattened.len(), 2, "no message may be dropped");
    assert!(flattened.contains(&1) && flattened.contains(&2));
}

/* ------------------------------------------------------------------------------- Gmail */

#[test]
fn gmail_thread_ids_win_over_the_algorithm() {
    // docs/03 §5: *Gmail: use X-GM-THRID for threading*. Google has already done this work
    // server-side and its answer is what the user sees in the Gmail web UI. Disagreeing with
    // it means the same conversation looks different in two places.
    let with_thrid = |id: i64, thrid: i64, message_id: &str| Threadable {
        gm_thrid: Some(thrid),
        thread_id: None,
        ..message(id, message_id, "unrelated subjects")
    };

    let messages = vec![
        with_thrid(1, 900, "<a@x>"),
        with_thrid(2, 900, "<b@x>"),
        with_thrid(3, 901, "<c@x>"),
    ];

    assert_eq!(groups(&messages), vec![vec![1, 2], vec![3]]);
}

#[test]
fn a_mailbox_where_only_some_messages_have_gmail_ids_threads_both_ways() {
    // Migrating an account, or an account where some messages predate the label, leaves a
    // mixture. Neither path may swallow the other.
    let messages = vec![
        Threadable {
            gm_thrid: Some(900),
            thread_id: None,
            ..message(1, "<a@x>", "gmail one")
        },
        Threadable {
            gm_thrid: Some(900),
            thread_id: None,
            ..message(2, "<b@x>", "gmail two")
        },
        message(3, "<c@x>", "plain"),
        reply(4, "<d@x>", "Re: plain", "<c@x>"),
    ];

    assert_eq!(groups(&messages), vec![vec![1, 2], vec![3, 4]]);
}

/* --------------------------------------------------------------------------- robustness */

#[test]
fn threading_is_stable_across_runs() {
    // The thread key is persisted. If it changed between syncs, every conversation would be
    // rebuilt and every `thread_id` in the database would churn.
    let messages = vec![
        message(1, "<a@x>", "stable"),
        reply(2, "<b@x>", "Re: stable", "<a@x>"),
        message(3, "<c@x>", "other"),
    ];

    assert_eq!(thread_messages(&messages), thread_messages(&messages));
}

#[test]
fn every_message_is_assigned_exactly_once() {
    // The property that matters most: threading may group things wrongly and be forgiven,
    // but a message that falls out of the assignment vanishes from the mailbox.
    let messages = vec![
        message(1, "<a@x>", "one"),
        reply(2, "<b@x>", "Re: one", "<a@x>"),
        message(3, "<c@x>", "two"),
        reply(4, "<d@x>", "Re: missing", "<gone@x>"),
        Threadable {
            message_id: None,
            ..message(5, "", "no id")
        },
        Threadable {
            gm_thrid: Some(7),
            thread_id: None,
            ..message(6, "<f@x>", "gmail")
        },
    ];

    let assignments = thread_messages(&messages);
    let mut ids: Vec<i64> = assignments.iter().map(|a| a.message_id).collect();
    ids.sort_unstable();

    assert_eq!(ids, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn an_empty_mailbox_produces_no_assignments() {
    assert!(thread_messages(&[]).is_empty());
}

#[test]
fn a_large_mailbox_threads_in_reasonable_time() {
    // JWZ is O(n) with hashing; a quadratic implementation passes every test above and then
    // takes minutes on a real mailbox. 20,000 messages in one chain is the pathological
    // shape — a long-running mailing-list thread.
    let mut messages = vec![message(1, "<m1@x>", "list thread")];

    for id in 2..=20_000i64 {
        messages.push(Threadable {
            references: vec![format!("<m{}@x>", id - 1)],
            ..reply(
                id,
                &format!("<m{id}@x>"),
                "Re: list thread",
                &format!("<m{}@x>", id - 1),
            )
        });
    }

    let started = std::time::Instant::now();
    let assignments = thread_messages(&messages);
    let elapsed = started.elapsed();

    assert_eq!(assignments.len(), 20_000);
    assert_eq!(groups(&messages).len(), 1, "one chain is one thread");
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "threading 20k messages took {elapsed:?} — this is the quadratic trap"
    );
}

/// A windowed pass must not re-key a conversation it can only half see.
///
/// `persist::rethread` normally runs over the newest 5,000 messages of an account, not the
/// whole of it. A conversation older than that window is only partly in it, and keying on the
/// smallest id *present* gave the visible half a new, higher key while the older half kept the
/// original — splitting a correct thread in two at the moment it received a reply.
///
/// Nothing repaired it afterwards. Every message still had a thread id, so `unthreaded_count`
/// stayed zero and the full pass that would have fixed it never ran again.
#[test]
fn a_window_does_not_re_key_a_conversation_it_cannot_fully_see() {
    // The whole conversation: an old root and two recent replies.
    let root = message(10, "<root@example.test>", "Budget");
    let first = reply(
        90_000,
        "<a@example.test>",
        "Re: Budget",
        "<root@example.test>",
    );
    let second = reply(
        90_001,
        "<b@example.test>",
        "Re: Budget",
        "<root@example.test>",
    );

    // A full pass sees all three and keys them on the oldest.
    let full = thread_messages(&[root, first.clone(), second.clone()]);
    let key = full[0].thread_key;
    assert_eq!(key, 10, "the full pass keys on the oldest message");
    assert!(full.iter().all(|entry| entry.thread_key == key));

    // Now a later windowed pass over only the recent half, each carrying the key it was given.
    // The old root is outside the window and is not passed in at all.
    let windowed = thread_messages(&[
        Threadable {
            thread_id: Some(key),
            ..first
        },
        Threadable {
            thread_id: Some(key),
            ..second
        },
    ]);

    for assignment in &windowed {
        assert_eq!(
            assignment.thread_key, key,
            "message {} was re-keyed to {}, splitting it from the older half of its own
             conversation, which still carries {key}",
            assignment.message_id, assignment.thread_key,
        );
    }
}

#[test]
fn a_new_conversation_in_a_window_is_still_keyed_on_its_oldest_message() {
    // The stored key only outranks the row id when there *is* one. Messages that have never
    // been threaded must key exactly as before, or the fix above would change every new
    // conversation as well as the ones it is meant to protect.
    let a = message(500, "<x@example.test>", "New");
    let b = reply(501, "<y@example.test>", "Re: New", "<x@example.test>");

    let assignments = thread_messages(&[a, b]);
    assert!(assignments.iter().all(|entry| entry.thread_key == 500));
}

#[test]
fn two_threads_bridged_in_a_window_merge_onto_the_older_key() {
    // A merge must still happen, and must settle on the lower of the two established keys —
    // not on the bridging message, which is newer than both.
    let older = Threadable {
        thread_id: Some(10),
        ..message(3_000, "<p@example.test>", "Plans")
    };
    let newer = Threadable {
        thread_id: Some(2_000),
        ..message(3_001, "<q@example.test>", "Plans")
    };
    let bridging = Threadable {
        references: vec!["<p@example.test>".into(), "<q@example.test>".into()],
        ..message(3_002, "<r@example.test>", "Re: Plans")
    };

    let assignments = thread_messages(&[older, newer, bridging]);
    assert!(
        assignments.iter().all(|entry| entry.thread_key == 10),
        "a merge should settle on the oldest established key: {assignments:?}"
    );
}
