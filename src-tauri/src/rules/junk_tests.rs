//! Tests for the junk classifier.
//!
//! The accuracy question is settled by `src/bin/junkgate.rs` against a real labelled corpus,
//! because accuracy is not something a hand-written test can measure — a corpus I write is a
//! measure of my imagination, not the filter. What is left here is everything that can be
//! stated as a rule: the arithmetic that must not blow up, and the behaviours that must hold
//! regardless of how good the classification is.

use rusqlite::Connection;

use super::junk::{self, Classifier, Verdict};

fn store() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open");
    crate::db::migrate::run(&mut conn).expect("migrate");
    conn
}

/// Enough obviously-separable mail to get the classifier past its minimum.
fn train_both(conn: &mut Connection, count: i64) {
    let tx = conn.transaction().expect("tx");

    for index in 0..count {
        junk::train(
            &tx,
            "colleague@work.test",
            &format!("Project update {index}"),
            "Here are the notes from the meeting about the quarterly planning review.",
            false,
        )
        .expect("train ham");

        junk::train(
            &tx,
            "winner@lottery.test",
            &format!("CONGRATULATIONS claim your prize {index}"),
            "You have won! Click here to claim your cash prize now. Limited offer!",
            true,
        )
        .expect("train junk");
    }

    tx.commit().expect("commit");
}

#[test]
fn an_untrained_filter_declines_to_answer() {
    // A fresh install has no training data. Answering confidently from nothing marks real mail
    // as junk on day one, which is how people turn a junk filter off and never turn it back on.
    let conn = store();
    let classifier = Classifier::load(&conn).expect("load");

    assert!(!classifier.ready());
    assert_eq!(
        classifier.score("anyone@example.test", "Anything", "Any body at all"),
        Verdict::Undecided
    );
}

#[test]
fn undecided_is_not_junk() {
    // The distinction the `Verdict` type exists to preserve. If "no idea" collapsed into a
    // score, it would read as either 0.0 or 0.5 and one of those crosses a threshold.
    assert!(!Verdict::Undecided.is_junk());
    assert_eq!(Verdict::Undecided.probability(), None);
}

#[test]
fn a_barely_trained_filter_still_declines() {
    let mut conn = store();
    train_both(&mut conn, junk::MIN_CORPUS - 1);

    let classifier = Classifier::load(&conn).expect("load");
    assert!(!classifier.ready(), "answered below the minimum corpus");
}

#[test]
fn it_separates_what_it_was_trained_on() {
    // The weakest possible claim about accuracy, and the only one that belongs in a unit test:
    // a classifier that cannot separate two corpora with no vocabulary in common is broken in
    // a way no amount of tuning explains.
    let mut conn = store();
    train_both(&mut conn, junk::MIN_CORPUS * 2);

    let classifier = Classifier::load(&conn).expect("load");
    assert!(classifier.ready());

    let junk_verdict = classifier.score(
        "winner@lottery.test",
        "CONGRATULATIONS claim your prize now",
        "You have won! Click here to claim your cash prize now. Limited offer!",
    );
    let ham_verdict = classifier.score(
        "colleague@work.test",
        "Project update",
        "Here are the notes from the meeting about the quarterly planning review.",
    );

    let junk_score = junk_verdict.probability().expect("scored");
    let ham_score = ham_verdict.probability().expect("scored");

    assert!(junk_score > ham_score, "{junk_score} !> {ham_score}");
    assert!(junk_verdict.is_junk(), "junk scored {junk_score}");
    assert!(!ham_verdict.is_junk(), "ham scored {ham_score}");
}

#[test]
fn a_score_is_always_a_real_number_between_zero_and_one() {
    // Fisher's method sums logs and exponentiates a chi-square tail. Both have ways of
    // producing NaN or infinity, and a NaN compares false against every threshold — so a broken
    // score would present as a filter that quietly stops working rather than as a crash.
    let mut conn = store();
    train_both(&mut conn, junk::MIN_CORPUS * 2);

    let classifier = Classifier::load(&conn).expect("load");

    let long_body = "prize offer winner claim cash free ".repeat(2_000);
    let many_emoji = "🙂".repeat(500);
    let awkward = [
        ("", "", ""),
        ("winner@lottery.test", "", ""),
        ("", "prize", ""),
        ("winner@lottery.test", "prize offer", long_body.as_str()),
        ("\u{0}\u{1}", "\u{feff}", "\u{202e}\u{202d}"),
        ("δοκιμή@παράδειγμα.test", "θέμα", "σώμα μηνύματος"),
        ("a@b.test", "🎉🎉🎉", many_emoji.as_str()),
    ];

    for (from, subject, body) in awkward {
        if let Some(score) = classifier.score(from, subject, body).probability() {
            assert!(score.is_finite(), "{score} for {subject:?}");
            assert!((0.0..=1.0).contains(&score), "{score} for {subject:?}");
        }
    }
}

#[test]
fn untraining_reverses_training() {
    // Marking a message junk and then immediately not-junk must leave no trace of the first
    // judgement, or the user's correction is only half applied.
    let mut conn = store();

    let before = {
        let tx = conn.transaction().expect("tx");
        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM junk_token", [], |row| row.get(0))
            .expect("count");
        tx.commit().expect("commit");
        count
    };

    let tx = conn.transaction().expect("tx");
    junk::train(&tx, "a@b.test", "Subject here", "Body text here", true).expect("train");
    junk::untrain(&tx, "a@b.test", "Subject here", "Body text here", true).expect("untrain");
    tx.commit().expect("commit");

    let (ham, junk_count) = junk::corpus_size(&conn).expect("size");
    assert_eq!(
        (ham, junk_count),
        (0, 0),
        "the corpus count survived untraining"
    );

    let remaining: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(count), 0) FROM junk_token",
            [],
            |row| row.get(0),
        )
        .expect("sum");

    assert_eq!(remaining, 0, "token counts survived untraining");
    assert!(
        conn.query_row("SELECT COUNT(*) FROM junk_token", [], |row| row
            .get::<_, i64>(0))
            .expect("count")
            >= before
    );
}

#[test]
fn tokens_are_bounded_in_length() {
    // Long tokens are base64 chunks, hashes and message ids: unique per message, so each one is
    // a perfect predictor of exactly one message and pure overfitting.
    let long = "a".repeat(200);
    let text = format!("hello {long} x yes");
    let tokens = junk::tokenise(&text);

    assert!(tokens.contains(&"hello".to_string()));
    assert!(tokens.contains(&"yes".to_string()));
    assert!(!tokens.iter().any(|t| t.len() > 24), "{tokens:?}");
    assert!(!tokens.iter().any(|t| t.len() < 3), "{tokens:?}");
}

#[test]
fn tokenising_ignores_case() {
    assert_eq!(junk::tokenise("FREE Free free"), vec!["free".to_string()]);
}

#[test]
fn bare_numbers_are_not_tokens() {
    // Dates, prices, quantities and ids. They vary per message and generalise to nothing.
    let tokens = junk::tokenise("invoice 12345 for 2003 dollars");
    assert!(!tokens.contains(&"12345".to_string()), "{tokens:?}");
    assert!(!tokens.contains(&"2003".to_string()), "{tokens:?}");
    assert!(tokens.contains(&"invoice".to_string()));
}

#[test]
fn where_a_word_appears_is_itself_a_signal() {
    // "offer" in a subject line means something different from "offer" three paragraphs into a
    // message from a colleague, so the two must not share a token.
    let features = junk::features("sales@vendor.test", "offer", "offer");

    assert!(features.contains(&"subj:offer".to_string()), "{features:?}");
    assert!(features.contains(&"offer".to_string()), "{features:?}");
    assert!(
        features.iter().any(|f| f.starts_with("from:")),
        "{features:?}"
    );
}

#[test]
fn training_only_ever_reads_labels_a_human_applied() {
    // The failure mode that turns a working filter into a confidently wrong one: feeding the
    // classifier's own guesses back in as training data. It converges on its own mistakes.
    let mut conn = store();

    conn.execute(
        "INSERT INTO account (id, display_name, email, provider, auth_kind, cred_ref)
         VALUES (1, 'T', 'me@t.test', 'other', 'password', 'halcyon:me')",
        [],
    )
    .expect("account");
    conn.execute(
        "INSERT INTO mailbox (id, account_id, remote_path, display_name, role)
         VALUES (1, 1, 'INBOX', 'Inbox', 'inbox')",
        [],
    )
    .expect("mailbox");

    // Marked junk by the filter, not by the user, and never opened.
    conn.execute(
        "INSERT INTO message (
             id, account_id, mailbox_id, uid, subject, date_sent, date_received, size,
             from_all, to_all, body_text, has_attachment, flag_seen, flag_flagged,
             is_junk, junk_by_user, junk_score
         ) VALUES (1, 1, 1, 1, 'guessed', 0, 0, 10, 'x@y.test', '', 'body', 0, 0, 0, 1, 0, 0.97)",
        [],
    )
    .expect("message");

    let tx = conn.transaction().expect("tx");
    let (ham, junk_count) = junk::train_from_user_labels(&tx).expect("train");
    tx.commit().expect("commit");

    assert_eq!(
        (ham, junk_count),
        (0, 0),
        "the filter trained on its own guess"
    );
}
