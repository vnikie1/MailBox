//! Selecting a mailbox and fetching envelopes. docs/03 §5, docs/06 Phase 5 §2.
//!
//! The fetch is issued as a raw command rather than through `Session::uid_fetch`, and the
//! responses are read straight off the wire. That is not a preference — `async_imap::Fetch`
//! exposes `uid`, `size`, `modseq`, `flags()` and `envelope()`, but has no accessor for
//! Gmail's `X-GM-THRID`, `X-GM-MSGID` or `X-GM-LABELS`, and docs/03 §5 requires all three.
//! Reading `ResponseData::parsed()` gives every attribute the server sent, so one round trip
//! covers both the standard and the Gmail path instead of two.
//!
//! No `unwrap()` in this module, per docs/06 Phase 5.

use async_imap::imap_proto::{AttributeValue, Response, Status};
use futures::StreamExt;

use super::envelope::{self, Envelope};
use super::session::{Caps, ImapSession, SyncError};

/// docs/03 §5: envelopes for the newest 500 UIDs of the Inbox, rendered immediately.
pub const FIRST_PAGE: u32 = 500;

/// Backfill batch size. docs/03 §5.
pub const BACKFILL_BATCH: u32 = 500;

/// The state of a mailbox after `SELECT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selected {
    pub uid_validity: u32,
    pub uid_next: u32,
    pub exists: u32,
    pub highest_modseq: Option<u64>,
}

/// One message as the server described it.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub uid: u32,
    pub envelope: Envelope,
    pub flags: Flags,
    pub size: u32,
    /// Epoch seconds of the server's INTERNALDATE — when the server received it, which is
    /// what the list sorts by. The `Date:` header is what the *sender's* clock said, and
    /// sorting by that puts a message with a wrong clock at the top of the mailbox forever.
    pub internal_date: i64,
    pub modseq: Option<u64>,
    pub gm_thrid: Option<i64>,
    pub gm_msgid: Option<String>,
    /// Parsed from the fetched `References` header. IMAP's ENVELOPE carries `In-Reply-To`
    /// but not `References`, and JWZ needs both.
    pub references: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub draft: bool,
    pub deleted: bool,
}

impl Flags {
    fn read(names: &[String]) -> Self {
        let has = |needle: &str| {
            names
                .iter()
                .any(|name| name.trim_start_matches('\\').eq_ignore_ascii_case(needle))
        };

        Self {
            seen: has("Seen"),
            answered: has("Answered"),
            flagged: has("Flagged"),
            draft: has("Draft"),
            deleted: has("Deleted"),
        }
    }
}

/// Selects a mailbox and checks `UIDVALIDITY` against what we stored.
///
/// docs/03 §5: *On UIDVALIDITY change: drop and re-sync that mailbox. Do not try to be
/// clever.* Returning the error rather than handling it here keeps the decision with the
/// caller, which is the only place that can drop rows inside a transaction.
pub async fn select(
    session: &mut ImapSession,
    path: &str,
    stored_uid_validity: Option<u32>,
    caps: Caps,
) -> Result<Selected, SyncError> {
    let mailbox = if caps.has_modseq() {
        session.select_condstore(path).await?
    } else {
        session.select(path).await?
    };

    let uid_validity = mailbox.uid_validity.unwrap_or(0);

    if let Some(stored) = stored_uid_validity {
        if stored != 0 && uid_validity != 0 && stored != uid_validity {
            return Err(SyncError::UidValidityChanged {
                mailbox: path.to_string(),
                stored,
                found: uid_validity,
            });
        }
    }

    Ok(Selected {
        uid_validity,
        uid_next: mailbox.uid_next.unwrap_or(0),
        exists: mailbox.exists,
        highest_modseq: mailbox.highest_modseq,
    })
}

/// The FETCH item list.
///
/// `BODY.PEEK[...]` rather than `BODY[...]`: the non-peek form sets `\Seen` on every message
/// it touches, so a first sync would mark an entire mailbox read. That is unrecoverable —
/// the flags go to the server — and it is the single most destructive mistake available in
/// this file.
fn fetch_items(caps: Caps) -> String {
    let mut items = String::from(
        "UID FLAGS INTERNALDATE RFC822.SIZE ENVELOPE \
         BODY.PEEK[HEADER.FIELDS (REFERENCES)]",
    );

    if caps.has_modseq() {
        items.push_str(" MODSEQ");
    }

    if caps.gmail {
        items.push_str(" X-GM-THRID X-GM-MSGID");
    }

    format!("({items})")
}

/// Runs a raw `UID FETCH` and collects every message in the response.
///
/// Reads until the tagged completion line for *this* command. A server may interleave
/// unsolicited untagged responses — `EXISTS`, `EXPUNGE`, another session's flag change — and
/// stopping at the first thing that is not a FETCH would desynchronise the connection for
/// every command after it.
async fn uid_fetch(
    session: &mut ImapSession,
    sequence: &str,
    caps: Caps,
) -> Result<Vec<Fetched>, SyncError> {
    let command = format!("UID FETCH {sequence} {}", fetch_items(caps));
    let request = session.run_command(&command).await?;

    let mut fetched = Vec::new();

    while let Some(response) = session.read_response().await {
        let response = response?;

        match response.parsed() {
            Response::Fetch(_, attributes) => {
                if let Some(message) = from_attributes(attributes) {
                    fetched.push(message);
                }
            }

            Response::Done { tag, status, .. } if *tag == request => {
                return match status {
                    Status::Ok => Ok(fetched),
                    _ => Err(SyncError::Imap(async_imap::error::Error::Bad(format!(
                        "UID FETCH failed for {sequence}"
                    )))),
                };
            }

            // Anything else is an unsolicited update. Ignored here and picked up by the
            // incremental pass; consuming it is what keeps the stream in step.
            _ => {}
        }
    }

    // The connection closed before the command completed.
    Err(SyncError::Imap(async_imap::error::Error::ConnectionLost))
}

/// Reads one FETCH response's attributes into a `Fetched`.
///
/// Returns `None` only when there is no UID, which makes the row unstorable — everything
/// else degrades to a default rather than dropping the message. Standing rule 13.
fn from_attributes(attributes: &[AttributeValue<'_>]) -> Option<Fetched> {
    let mut uid = None;
    let mut envelope = Envelope::default();
    let mut flag_names: Vec<String> = Vec::new();
    let mut size = 0u32;
    let mut internal_date = 0i64;
    let mut modseq = None;
    let mut gm_thrid = None;
    let mut gm_msgid = None;
    let mut references = Vec::new();

    for attribute in attributes {
        match attribute {
            AttributeValue::Uid(value) => uid = Some(*value),
            AttributeValue::Rfc822Size(value) => size = *value,
            AttributeValue::ModSeq(value) => modseq = Some(*value),
            AttributeValue::GmailThrId(value) => gm_thrid = Some(*value as i64),
            AttributeValue::GmailMsgId(value) => gm_msgid = Some(value.to_string()),

            AttributeValue::Flags(names) => {
                flag_names = names.iter().map(|name| name.to_string()).collect();
            }

            AttributeValue::Envelope(parsed) => {
                envelope = envelope::from_imap(parsed);
            }

            AttributeValue::InternalDate(raw) => {
                // INTERNALDATE is "dd-Mon-yyyy HH:MM:SS +ZZZZ", which is not RFC 2822 —
                // the dashes defeat a plain date parser, so it is normalised first.
                internal_date = parse_internal_date(raw);
            }

            AttributeValue::BodySection {
                data: Some(bytes), ..
            } => {
                let text = String::from_utf8_lossy(bytes);
                references = extract_references(&text);
            }

            _ => {}
        }
    }

    Some(Fetched {
        uid: uid?,
        envelope,
        flags: Flags::read(&flag_names),
        size,
        internal_date,
        modseq,
        gm_thrid,
        gm_msgid,
        references,
    })
}

/// IMAP's INTERNALDATE format into epoch seconds.
///
/// `17-Jul-1996 02:44:25 -0700`. Replacing the dashes in the date turns it into something a
/// standard RFC 2822 parser accepts, which avoids hand-rolling month names.
pub fn parse_internal_date(raw: &str) -> i64 {
    let trimmed = raw.trim().trim_matches('"');

    // Only the date portion uses dashes; the zone offset does too, so replace at most twice.
    let normalised = trimmed.replacen('-', " ", 2);

    envelope::parse_date(&normalised)
}

/// Pulls the `References` values out of a fetched header block.
fn extract_references(header_block: &str) -> Vec<String> {
    let mut collected = String::new();
    let mut in_references = false;

    for line in header_block.lines() {
        let is_continuation = line.starts_with(' ') || line.starts_with('\t');

        if in_references && is_continuation {
            collected.push(' ');
            collected.push_str(line.trim());
            continue;
        }

        in_references = false;

        if let Some(rest) = line
            .strip_prefix("References:")
            .or_else(|| line.strip_prefix("references:"))
            .or_else(|| line.strip_prefix("REFERENCES:"))
        {
            in_references = true;
            collected.push_str(rest.trim());
        }
    }

    envelope::parse_references(&collected)
}

/// The newest `count` UIDs below `uid_next`, as an IMAP sequence set.
///
/// A range rather than a `UID SEARCH` first: the range costs no round trip, and a mailbox
/// with gaps simply returns fewer messages than asked for, which is correct. Searching would
/// be exact and would double the latency of the very first thing the user waits for.
pub fn newest_range(uid_next: u32, count: u32) -> String {
    let highest = uid_next.saturating_sub(1);

    if highest == 0 {
        return "1:*".to_string();
    }

    let lowest = highest.saturating_sub(count.saturating_sub(1)).max(1);

    format!("{lowest}:{highest}")
}

/// Every UID in the selected mailbox, newest first.
///
/// One round trip, and it is what makes backfill proportional to the number of *messages*
/// rather than to the size of the UID space. Those are wildly different numbers on a mailbox
/// that has been archived from for years: the real Gmail Inbox this was found on holds 214
/// messages with `uid_next` at 106,287, so walking the numeric range in windows of 500 meant
/// 213 round trips — about seventy minutes — to fetch 214 messages. Scaled to the 50k-message
/// mailbox in docs/04's Phase 5 exit gate, the range walk simply does not finish.
///
/// The newest page deliberately still uses `newest_range` above: it costs no round trip and
/// it is the thing the user is actually waiting for. Being approximate is fine there, because
/// this search is what guarantees nothing is ultimately missed.
pub async fn all_uids(session: &mut ImapSession) -> Result<Vec<u32>, SyncError> {
    let mut uids: Vec<u32> = session.uid_search("ALL").await?.into_iter().collect();

    // Newest first, so backfill continues in the direction the user reads.
    uids.sort_unstable_by(|a, b| b.cmp(a));
    Ok(uids)
}

/// One backfill window taken from a known UID list, ending just below `before_uid`.
///
/// Returns the sequence set and the lowest UID it covers, or `None` when nothing older is
/// left — which is how the backfill loop knows it has finished rather than fetching `1:1`
/// forever.
///
/// `uids` must be sorted newest first, as `all_uids` returns it.
pub fn backfill_window(uids: &[u32], before_uid: u32, count: u32) -> Option<(String, u32)> {
    let window: Vec<u32> = uids
        .iter()
        .copied()
        .filter(|uid| *uid < before_uid)
        .take(count as usize)
        .collect();

    let lowest = *window.last()?;

    // Listed explicitly rather than as a range: the UIDs are genuinely sparse, and a range
    // spanning the gaps is exactly the cost this function exists to avoid.
    let set = window
        .iter()
        .map(|uid| uid.to_string())
        .collect::<Vec<_>>()
        .join(",");

    Some((set, lowest))
}

/// Fetches envelopes for a sequence set.
pub async fn envelopes(
    session: &mut ImapSession,
    sequence: &str,
    caps: Caps,
) -> Result<Vec<Fetched>, SyncError> {
    uid_fetch(session, sequence, caps).await
}

/// Fetches one message's full source, for the body cache. docs/06 Phase 5 §3.
pub async fn body(session: &mut ImapSession, uid: u32) -> Result<Vec<u8>, SyncError> {
    // PEEK again: opening a message is what marks it read, and that is a decision the UI
    // makes explicitly. Fetching a body must never do it as a side effect.
    let mut stream = session.uid_fetch(uid.to_string(), "BODY.PEEK[]").await?;

    let mut raw = Vec::new();

    while let Some(item) = stream.next().await {
        let fetch = item?;

        if let Some(bytes) = fetch.body() {
            raw = bytes.to_vec();
        }
    }

    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(gmail: bool, modseq: bool) -> Caps {
        Caps {
            gmail,
            condstore: modseq,
            ..Caps::default()
        }
    }

    #[test]
    fn the_fetch_always_peeks() {
        // The most destructive mistake available in this file. BODY[...] without PEEK sets
        // \Seen on every message it touches, the flags go to the server, and an entire
        // mailbox is marked read with no way back.
        let items = fetch_items(caps(false, false));

        assert!(items.contains("BODY.PEEK["), "{items}");
        assert!(!items.contains("BODY[",), "{items}");
    }

    #[test]
    fn gmail_attributes_are_requested_only_from_gmail() {
        // X-GM-THRID against a server that does not advertise X-GM-EXT-1 is a BAD response,
        // and a BAD response mid-sync aborts the batch.
        assert!(fetch_items(caps(true, false)).contains("X-GM-THRID"));
        assert!(!fetch_items(caps(false, false)).contains("X-GM-THRID"));
    }

    #[test]
    fn modseq_is_requested_only_where_it_exists() {
        assert!(fetch_items(caps(false, true)).contains("MODSEQ"));
        assert!(!fetch_items(caps(false, false)).contains("MODSEQ"));
    }

    #[test]
    fn the_fetch_asks_for_references_because_the_envelope_does_not_carry_them() {
        // IMAP's ENVELOPE has In-Reply-To but not References. JWZ needs both, and without
        // this header fetch every deep thread would break into pairs.
        assert!(fetch_items(caps(false, false)).contains("HEADER.FIELDS (REFERENCES)"));
    }

    #[test]
    fn the_first_page_is_the_newest_five_hundred() {
        // docs/03 §5. uid_next is one past the highest, so the newest UID is uid_next - 1.
        assert_eq!(newest_range(1001, 500), "501:1000");
        assert_eq!(newest_range(501, 500), "1:500");
    }

    #[test]
    fn a_mailbox_with_fewer_messages_than_a_page_asks_for_what_is_there() {
        // Clamped at 1 — "0:99" is not a valid sequence set and some servers reject it.
        assert_eq!(newest_range(100, 500), "1:99");
        assert_eq!(newest_range(2, 500), "1:1");
    }

    #[test]
    fn an_empty_mailbox_produces_a_harmless_range() {
        // uid_next of 0 or 1 means nothing has ever been delivered. "1:*" returns nothing
        // rather than erroring, which is what an empty mailbox should do.
        assert_eq!(newest_range(0, 500), "1:*");
        assert_eq!(newest_range(1, 500), "1:*");
    }

    /// UIDs as `all_uids` returns them: descending, and only the ones that exist.
    fn descending(uids: &[u32]) -> Vec<u32> {
        let mut sorted = uids.to_vec();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        sorted
    }

    #[test]
    fn backfill_walks_backwards_in_batches() {
        let uids = descending(&(1..=1000).collect::<Vec<_>>());

        let (set, lowest) = backfill_window(&uids, 1001, 500).expect("first window");
        assert_eq!(lowest, 501);
        assert!(set.starts_with("1000,999,"), "{set}");
        assert!(set.ends_with(",501"), "{set}");

        let (_, lowest) = backfill_window(&uids, lowest, 500).expect("second window");
        assert_eq!(lowest, 1);
    }

    #[test]
    fn backfill_stops_rather_than_looping_at_the_bottom() {
        // The termination condition. Without it the engine refetches the last window forever,
        // which is a connection storm aimed at one mailbox — what the soak test looks for.
        let uids = descending(&[1, 2, 3]);

        assert_eq!(backfill_window(&uids, 1, 500), None);
        assert_eq!(backfill_window(&uids, 0, 500), None);
        assert_eq!(backfill_window(&[], 5000, 500), None);

        // The last partial batch is still returned.
        let (set, lowest) = backfill_window(&uids, 2, 500).expect("partial");
        assert_eq!(set, "1");
        assert_eq!(lowest, 1);
    }

    #[test]
    fn backfill_asks_only_for_uids_that_exist() {
        // The point of the whole change. A mailbox archived from for years has a handful of
        // messages scattered across an enormous UID space; asking for the range between them
        // is what made a 214-message Inbox take 213 round trips.
        let uids = descending(&[7, 4001, 98_000, 106_286]);

        let (set, lowest) = backfill_window(&uids, 200_000, 500).expect("window");

        assert_eq!(set, "106286,98000,4001,7");
        assert_eq!(lowest, 7);
        assert!(!set.contains(':'), "must not span the gaps: {set}");
    }

    #[test]
    fn backfill_covers_every_uid_with_no_gap_and_no_overlap() {
        // A gap loses mail; an overlap refetches it. Walk a sparse mailbox and check both.
        let present: Vec<u32> = (1..=2000).filter(|uid| uid % 7 == 0).collect();
        let uids = descending(&present);

        let mut cursor = u32::MAX;
        let mut covered: Vec<u32> = Vec::new();

        while let Some((set, lowest)) = backfill_window(&uids, cursor, 50) {
            covered.extend(set.split(',').map(|uid| uid.parse::<u32>().expect("uid")));
            cursor = lowest;
        }

        covered.sort_unstable();

        assert_eq!(
            covered, present,
            "backfill must cover every existing UID exactly once"
        );
    }

    #[test]
    fn flags_are_read_with_and_without_their_backslash() {
        let flags = Flags::read(&[
            "\\Seen".to_string(),
            "answered".to_string(),
            "\\Flagged".to_string(),
        ]);

        assert!(flags.seen);
        assert!(flags.answered);
        assert!(flags.flagged);
        assert!(!flags.draft);
        assert!(!flags.deleted);
    }

    #[test]
    fn an_unknown_keyword_flag_is_ignored_rather_than_breaking_the_row() {
        // Servers and users invent keywords freely — "$Forwarded", "NonJunk", "\\Recent".
        let flags = Flags::read(&[
            "$Forwarded".to_string(),
            "NonJunk".to_string(),
            "\\Recent".to_string(),
            "\\Seen".to_string(),
        ]);

        assert!(flags.seen);
        assert_eq!(
            flags,
            Flags {
                seen: true,
                ..Flags::default()
            }
        );
    }

    #[test]
    fn internal_dates_parse_from_imaps_own_format() {
        // "17-Jul-1996 02:44:25 -0700" is not RFC 2822 and a plain parser returns 0 for it,
        // which would put every message at the epoch and destroy the list's ordering.
        let parsed = parse_internal_date("17-Jul-1996 02:44:25 -0700");

        assert!(parsed > 0, "INTERNALDATE must parse");
        assert_eq!(parsed, 837596665);
    }

    #[test]
    fn a_quoted_internal_date_parses_too() {
        assert_eq!(
            parse_internal_date("\"17-Jul-1996 02:44:25 -0700\""),
            837596665
        );
    }

    #[test]
    fn references_are_extracted_from_a_fetched_header_block() {
        let block = "References: <a@x> <b@x>\r\n";

        assert_eq!(extract_references(block), vec!["a@x", "b@x"]);
    }

    #[test]
    fn a_folded_references_header_is_joined_before_splitting() {
        // Long threads fold References across many lines. Reading only the first line
        // truncates the ancestry and splits the conversation.
        let block = "References: <a@x>\r\n <b@x>\r\n\t<c@x>\r\n";

        assert_eq!(extract_references(block), vec!["a@x", "b@x", "c@x"]);
    }

    #[test]
    fn a_header_block_with_no_references_yields_nothing() {
        assert!(extract_references("Subject: hello\r\n").is_empty());
        assert!(extract_references("").is_empty());
    }

    #[test]
    fn a_continuation_line_after_a_different_header_is_not_read_as_references() {
        // Folding belongs to whichever header opened it. Attributing a Subject continuation
        // to References would inject rubbish into the threading id table.
        let block = "References: <a@x>\r\nSubject: a long one\r\n folded here\r\n";

        assert_eq!(extract_references(block), vec!["a@x"]);
    }
}
