//! Reading and writing mbox. docs/06 Phase 11.
//!
//! ## Why this is hand-written
//!
//! mbox is a format with no length prefix and no escaping that is safe by construction: a
//! message ends where the *next* one begins, and the next one begins at a line starting
//! `From `. A line inside a message body that happens to start that way is therefore
//! indistinguishable from a separator unless the writer escaped it — and there are three
//! incompatible conventions for doing so, all called mbox.
//!
//! * **mboxo** escapes `From ` as `>From ` and does nothing else, which is lossy: a body line
//!   that genuinely read `>From ` comes back as `>From ` too, so the reader cannot tell.
//! * **mboxrd** escapes any run of `>` followed by `From ` by adding one more `>`. That is
//!   reversible, and it is what Thunderbird and most Unix tools write.
//! * **mboxcl/mboxcl2** add a `Content-Length` header and skip escaping. Both are rare, and a
//!   reader that trusts `Content-Length` on a file written by something else corrupts silently.
//!
//! This reads permissively — split on separators, strip one `>` from any `>+From ` line, which
//! is right for mboxrd and harmless for mboxo — and writes strict mboxrd. Round-tripping our
//! own export is therefore exact, and importing somebody else's is as close as the format
//! allows.
//!
//! ## Why it streams
//!
//! A Thunderbird `Inbox` of several gigabytes is ordinary. Reading one into a `Vec<u8>` to
//! split it would need the whole file resident, and on a 32-bit-ish memory budget it simply
//! fails. The reader holds one message at a time.

use std::io::{BufRead, Read, Write};

/// The largest single message this will assemble.
///
/// A guard against a malformed file rather than a policy about mail: without it, a file whose
/// separators are wrong — or which is not mbox at all — is read as one message the size of the
/// file, and the process dies holding all of it. Skipped messages are counted and reported.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// The largest single line this will read.
///
/// The message cap above is not enough on its own, and the gap is easy to miss: `read_until`
/// reads to the next newline, so a file with *no* newlines is one line, and the whole of it is
/// in memory before any cap on the assembled message gets a chance to look. Someone will point
/// this at a `.pst`. Bounding the line read means such a file arrives as chunks that start no
/// message and trip the message cap instead.
///
/// 8MB rather than something tighter because a legitimate line can be long — mailers exist
/// that write an entire HTML body unfolded — and a cap that splits a real line would corrupt
/// a message rather than reject a file.
const MAX_LINE_BYTES: u64 = 8 * 1024 * 1024;

/// True for a line that starts a new message.
///
/// Only the `From ` prefix is required. The full separator is `From <address> <date>`, but
/// mailers disagree about the address part — some write `From MAILER-DAEMON`, some `From -`,
/// some an address with no date — and rejecting a separator because its date was unparseable
/// merges two messages into one, which is worse than accepting a body line by mistake.
fn is_separator(line: &[u8]) -> bool {
    line.starts_with(b"From ")
}

/// Strips one level of mboxrd quoting from a body line.
///
/// `>From ` becomes `From `, `>>From ` becomes `>From `, and a line that is not quoted at all
/// is returned unchanged.
fn unescape(line: &[u8]) -> &[u8] {
    let mut at = 0;
    while at < line.len() && line[at] == b'>' {
        at += 1;
    }

    if at > 0 && line[at..].starts_with(b"From ") {
        &line[1..]
    } else {
        line
    }
}

/// Adds one level of mboxrd quoting to a body line that needs it.
fn needs_escape(line: &[u8]) -> bool {
    let mut at = 0;
    while at < line.len() && line[at] == b'>' {
        at += 1;
    }
    line[at..].starts_with(b"From ")
}

/// True for a line that could begin a header block.
///
/// RFC 5322 field names are printable ASCII other than colon, so this is a field name
/// followed by one. It exists because the separator alone is not enough to find message
/// boundaries in the files people actually have.
///
/// The format says a body line beginning `From ` must be escaped. In practice a great deal
/// of mbox in the world was written by something that did not: mail exported by a script, a
/// file concatenated by hand, an old tool that only ever implemented mboxo. In such a file an
/// ordinary sentence starting "From here on..." is read as a separator, the message ends
/// early, and its tail becomes a headerless message of its own. It is the commonest way an
/// importer corrupts a mailbox, and this project found it by writing a sample with exactly
/// that line in it.
///
/// Requiring a header on the next line rejects the sentence and keeps the real separator,
/// because a real one is always followed by the first header of the message it introduces.
/// The case it cannot save is a message with no headers at all, which is not a message.
fn starts_a_header(line: &[u8]) -> bool {
    let Some(colon) = line.iter().position(|byte| *byte == b':') else {
        return false;
    };

    if colon == 0 {
        return false;
    }

    line[..colon]
        .iter()
        .all(|byte| byte.is_ascii_graphic() && *byte != b':')
}
/// True for an empty line, with either line ending.
fn is_blank(line: &[u8]) -> bool {
    line.iter().all(|byte| *byte == b'\r' || *byte == b'\n')
}

/// What a read of one file produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub messages: usize,
    /// Messages dropped for exceeding [`MAX_MESSAGE_BYTES`].
    pub oversized: usize,
}

/// Splits an mbox stream, handing each message's raw bytes to `each`.
///
/// The callback may fail, and the failure stops the read — an import that cannot write to the
/// database should not spend twenty minutes parsing the rest of the file first.
pub fn read<R: BufRead>(
    source: R,
    each: impl FnMut(&[u8]) -> std::io::Result<()>,
) -> std::io::Result<Counts> {
    read_capped(source, MAX_MESSAGE_BYTES, each)
}

/// [`read`] with the message cap given explicitly, so a test can reach the oversize path
/// without building a 64MB string.
pub fn read_capped<R: BufRead>(
    mut source: R,
    cap: usize,
    mut each: impl FnMut(&[u8]) -> std::io::Result<()>,
) -> std::io::Result<Counts> {
    let mut counts = Counts::default();
    let mut current: Vec<u8> = Vec::new();
    let mut started = false;
    let mut oversize = false;

    // Read by line rather than by chunk, because the separator is defined in terms of lines
    // and a chunk boundary can fall inside one.
    let mut line: Vec<u8> = Vec::new();

    // Trailing newlines belong to the separator, not to the message: every message in an mbox
    // is followed by a blank line before the next `From `, and keeping it would append a
    // spurious empty line to every message on every round trip.
    let flush = |current: &mut Vec<u8>,
                 counts: &mut Counts,
                 oversize: &mut bool,
                 each: &mut dyn FnMut(&[u8]) -> std::io::Result<()>|
     -> std::io::Result<()> {
        if *oversize {
            counts.oversized += 1;
            current.clear();
            *oversize = false;
            return Ok(());
        }

        while current.last() == Some(&b'\n') || current.last() == Some(&b'\r') {
            current.pop();
        }

        if !current.is_empty() {
            counts.messages += 1;
            each(current)?;
        }

        current.clear();
        Ok(())
    };

    // One line of lookahead. A `From ` line is only a separator if a header follows it, and
    // that cannot be known until the next line has been read — see `is_separator`.
    let mut held: Option<Vec<u8>> = None;
    let mut previous_blank = true;
    let mut previous_blank_before_held = true;

    loop {
        line.clear();
        // Bounded per MAX_LINE_BYTES, taken from a *borrow* so the limit applies to this line
        // and the reader survives to read the next one.
        let read = (&mut source)
            .take(MAX_LINE_BYTES)
            .read_until(b'\n', &mut line)?;
        let at_end = read == 0;

        // Decide about the line held from the previous turn, now that its successor is known.
        if let Some(previous) = held.take() {
            let separator = is_separator(&previous)
                // At end of file `line` is empty, so this is false and a trailing `From ` line
                // is body. That is the right way round: a separator introduces a message, and
                // one with nothing after it introduces nothing. Written as `at_end || …` at
                // first, which silently ate the last line of any message whose final paragraph
                // began "From ".
                && starts_a_header(&line)
                // A separator is preceded by a blank line, or by nothing at all. Both
                // conditions are needed: the header test alone would take a quoted mail's
                // `From ` line mid-paragraph, and the blank-line test alone is what let a body
                // line ending a paragraph split a message in two.
                && previous_blank_before_held;

            if separator {
                if started {
                    flush(&mut current, &mut counts, &mut oversize, &mut each)?;
                }
                started = true;
                // The separator itself is not part of the message. Everything it carries — the
                // envelope sender and the delivery date — is guesswork reconstructed by
                // whichever tool wrote the file; the headers inside the message are the real
                // thing.
            } else if started {
                if current.len() + previous.len() > cap {
                    oversize = true;
                } else {
                    current.extend_from_slice(unescape(&previous));
                }
            }
            // Leading junk before the first separator falls through and is dropped. A file that
            // is not mbox at all lands here for its whole length and yields nothing, which is
            // the correct answer.
        }

        if at_end {
            break;
        }

        previous_blank_before_held = previous_blank;
        previous_blank = is_blank(&line);
        held = Some(std::mem::take(&mut line));
    }

    if started {
        flush(&mut current, &mut counts, &mut oversize, &mut each)?;
    }

    Ok(counts)
}

/// Writes one message into an mbox stream, escaping it as mboxrd.
///
/// The separator line needs an address and a date. `from` is written as given; the date is
/// the asctime form the format has always used, which nothing parses and everything expects.
pub fn write_message<W: Write>(
    sink: &mut W,
    from: &str,
    date: &str,
    raw: &[u8],
) -> std::io::Result<()> {
    // An empty or multi-line sender would forge a separator inside the file. `-` is the
    // conventional stand-in for an unknown envelope sender and cannot contain a newline.
    let sender = if from.is_empty() || from.contains(['\r', '\n', ' ']) {
        "-"
    } else {
        from
    };

    writeln!(sink, "From {sender} {date}")?;

    for line in raw.split_inclusive(|byte| *byte == b'\n') {
        if needs_escape(line) {
            sink.write_all(b">")?;
        }
        sink.write_all(line)?;
    }

    // Exactly one blank line between messages, and a newline first if the message did not end
    // with one. Without the first, the next separator is appended to the last body line and
    // the file has one fewer message than it should.
    if !raw.ends_with(b"\n") {
        sink.write_all(b"\n")?;
    }
    sink.write_all(b"\n")?;

    Ok(())
}

/// The date format an mbox separator line carries: `Thu Jan  1 00:00:00 1970`.
pub fn separator_date(epoch_seconds: i64) -> String {
    use chrono::{Local, TimeZone};

    match Local.timestamp_opt(epoch_seconds, 0).single() {
        Some(when) => when.format("%a %b %e %H:%M:%S %Y").to_string(),
        None => "Thu Jan  1 00:00:00 1970".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(input: &str) -> (Vec<String>, Counts) {
        let mut found = Vec::new();
        let counts = read(input.as_bytes(), |raw| {
            found.push(String::from_utf8_lossy(raw).to_string());
            Ok(())
        })
        .expect("read");
        (found, counts)
    }

    #[test]
    fn two_messages_are_two_messages() {
        let (found, counts) = collect(
            "From a@x Thu Jan  1 00:00:00 1970\nSubject: one\n\nbody one\n\n\
             From b@x Thu Jan  1 00:00:00 1970\nSubject: two\n\nbody two\n",
        );

        assert_eq!(counts.messages, 2);
        assert!(found[0].starts_with("Subject: one"));
        assert!(found[1].starts_with("Subject: two"));
    }

    #[test]
    fn a_quoted_from_line_is_body_and_not_a_separator() {
        // The whole reason the format needs escaping. Without unquoting, this message ends
        // early and half of it becomes a second message with no headers.
        let (found, counts) = collect(
            "From a@x Thu Jan  1 00:00:00 1970\nSubject: one\n\n\
             >From the desk of somebody\nstill the same message\n",
        );

        assert_eq!(counts.messages, 1);
        assert!(found[0].contains("From the desk of somebody"));
        assert!(!found[0].contains(">From the desk"));
    }

    #[test]
    fn deeper_quoting_loses_exactly_one_level() {
        let (found, _) =
            collect("From a@x Thu Jan  1 00:00:00 1970\nSubject: one\n\n>>From a quote\n");

        assert!(found[0].contains("\n>From a quote"));
    }

    #[test]
    fn a_line_starting_with_gt_that_is_not_a_from_line_is_untouched() {
        let (found, _) =
            collect("From a@x Thu Jan  1 00:00:00 1970\nSubject: one\n\n> ordinary quoted text\n");

        assert!(found[0].contains("> ordinary quoted text"));
    }

    #[test]
    fn an_unescaped_from_line_in_a_body_does_not_split_the_message() {
        // The commonest way an importer corrupts a mailbox, and the reason the reader looks
        // ahead. A conforming writer escapes this line; a great deal of real mbox was not
        // written by a conforming writer. Without the lookahead the message ends here and
        // "From here on..." becomes a second, headerless message.
        let (found, counts) = collect(
            "From a@x d\nSubject: one\n\nIt weaves patterns.\n\n\
             From here on it is my own work.\n",
        );

        assert_eq!(counts.messages, 1, "the message was split: {found:?}");
        assert!(found[0].contains("From here on it is my own work."));
    }

    #[test]
    fn a_real_separator_after_a_blank_line_still_separates() {
        // The other half of the same decision. Rejecting too much merges two messages into
        // one, which loses a message as surely as splitting invents one.
        let (found, counts) = collect(
            "From a@x d\nSubject: one\n\nbody one\n\n\
             From b@x d\nSubject: two\n\nbody two\n",
        );

        assert_eq!(counts.messages, 2, "{found:?}");
    }

    #[test]
    fn a_from_line_that_is_not_preceded_by_a_blank_line_is_body() {
        // A quoted mail pasted mid-paragraph. The header test alone would accept this, because
        // a quoted message carries real headers after its From line.
        let (found, counts) = collect(
            "From a@x d\nSubject: one\n\nAs you wrote:\n\
             From b@x d\nSubject: the quoted one\n",
        );

        assert_eq!(counts.messages, 1, "{found:?}");
        assert!(found[0].contains("the quoted one"));
    }

    #[test]
    fn a_file_that_is_not_mbox_yields_nothing() {
        // Rather than one enormous message. Somebody will point this at a .pst or a .zip.
        let (found, counts) = collect("this is not an mbox file\nnot at all\n");

        assert_eq!(counts.messages, 0);
        assert!(found.is_empty());
    }

    #[test]
    fn a_separator_with_no_date_still_separates() {
        // Mailers disagree about what follows `From `. Refusing one because its date would not
        // parse merges two messages into one, which is the worse failure.
        let (_, counts) =
            collect("From MAILER-DAEMON\nSubject: one\n\nbody\n\nFrom -\nSubject: two\n\nbody\n");

        assert_eq!(counts.messages, 2);
    }

    #[test]
    fn writing_then_reading_gives_back_what_went_in() {
        // The round trip that matters: export to mbox, import it again, get the same bytes.
        let original = "Subject: round trip\n\nFrom here to there\n>From there to here\nend\n";

        let mut file: Vec<u8> = Vec::new();
        write_message(
            &mut file,
            "a@x",
            "Thu Jan  1 00:00:00 1970",
            original.as_bytes(),
        )
        .expect("write");

        let mut found = Vec::new();
        read(file.as_slice(), |raw| {
            found.push(String::from_utf8_lossy(raw).to_string());
            Ok(())
        })
        .expect("read");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0], original.trim_end());
    }

    #[test]
    fn a_body_line_starting_with_from_is_escaped_on_the_way_out() {
        let mut file: Vec<u8> = Vec::new();
        write_message(&mut file, "a@x", "d", b"Subject: x\n\nFrom the top\n").expect("write");

        let text = String::from_utf8(file).expect("utf8");
        assert!(text.contains("\n>From the top"));
    }

    #[test]
    fn a_sender_with_a_newline_cannot_forge_a_separator() {
        // The separator is a line. A sender carrying one would end the message early and inject
        // whatever followed as a new one — the mbox equivalent of header injection.
        let mut file: Vec<u8> = Vec::new();
        write_message(&mut file, "a@x\nFrom evil@x Thu", "d", b"Subject: x\n").expect("write");

        let text = String::from_utf8(file).expect("utf8");
        assert_eq!(text.lines().filter(|l| l.starts_with("From ")).count(), 1);
        assert!(text.starts_with("From - d"));
    }

    #[test]
    fn an_oversized_message_is_skipped_and_counted_rather_than_read() {
        // A file whose separators are wrong reads as one message the size of the file. Without
        // the cap the process dies holding all of it, and the user is told nothing.
        //
        // Run against a small cap rather than the real 64MB one: the behaviour under test is
        // the branch, and a test that allocates 64MB to reach it is a slow test that will one
        // day be deleted for being slow.
        let input = "From a@x d\nSubject: huge\n\naaaa\nbbbb\ncccc\ndddd\n";

        let mut found = Vec::new();
        let counts = read_capped(input.as_bytes(), 8, |raw| {
            found.push(raw.to_vec());
            Ok(())
        })
        .expect("read");

        assert!(found.is_empty());
        assert_eq!(counts.oversized, 1);
        assert_eq!(counts.messages, 0);
    }

    #[test]
    fn an_oversized_message_does_not_swallow_the_one_after_it() {
        // The skip has to reset. The first version cleared the buffer but left the flag set,
        // so one bad message silently discarded the rest of the file.
        let input = "From a@x d\nSubject: huge\n\naaaaaaaaaaaaaaaaaaaa\n\
                     \nFrom b@x d\nSubject: small\n";

        let mut found = Vec::new();
        let counts = read_capped(input.as_bytes(), 24, |raw| {
            found.push(String::from_utf8_lossy(raw).to_string());
            Ok(())
        })
        .expect("read");

        assert_eq!(counts.oversized, 1);
        assert_eq!(counts.messages, 1);
        assert!(found[0].contains("small"));
    }
}
