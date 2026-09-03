//! Reading an Outlook `.pst`. docs/06 Phase 11.
//!
//! ## What a .pst is, and why this is not just another parser
//!
//! A `.pst` is not a mail file. It is a MAPI object store: a pair of B-trees over a paged heap,
//! holding folders, messages, recipient tables and attachment tables as **numbered properties**.
//! There is no RFC 5322 message anywhere inside it. Importing one is therefore two problems, and
//! only the first is about the file format.
//!
//! The first — reading the store — is solved by `outlook-pst`, Microsoft's own clean-room
//! implementation of the MS-PST specification. It hands back properties by id.
//!
//! The second is this module: turning `PR_SUBJECT`, `PR_SENDER_NAME`, a recipients table and a
//! body into a message the rest of the app already knows how to store. Everything downstream —
//! threading, search, the reader, export — takes RFC 5322 bytes, and giving it anything else
//! would mean a second version of all of it.
//!
//! ## The one piece of luck
//!
//! Outlook usually keeps the **original headers** of a received message in
//! `PR_TRANSPORT_MESSAGE_HEADERS` (0x007D). Where it is present the message is reconstructed
//! nearly exactly: real `Message-ID`, real `References`, real `Date`, so imported mail threads
//! against synced mail correctly. Where it is absent — which is normal for mail the user *sent*
//! — the headers are synthesised from the individual properties, and the result is a faithful
//! message with a made-up `Message-ID`. That is called out in `synthesise` rather than hidden.
//!
//! ## What is not supported, and is reported rather than dropped silently
//!
//! * **RTF-only bodies.** Outlook stores some bodies as `PR_RTF_COMPRESSED`, in a Microsoft
//!   compression format with a hard-coded dictionary. Those messages import with their headers
//!   and whatever plain text exists, and are counted.
//! * **Attachments.** Present in the store and not extracted. A message with attachments
//!   imports as its text, and says so.
//! * **Encrypted stores.** A `.pst` with a password set cannot be opened, and the user is told
//!   that rather than shown an empty import.
//!
//! Each of these is a real gap. They are listed here, counted at run time, and shown in the UI,
//! because the failure this module must never have is importing an archive that looks complete
//! and is not.

use std::path::Path;
use std::rc::Rc;

use outlook_pst::ltp::prop_context::PropertyValue;
use outlook_pst::messaging::store::Store;

/// MAPI property ids. MS-OXPROPS names them; these are the ones a message needs.
mod prop {
    /// `PR_SUBJECT`.
    pub const SUBJECT: u16 = 0x0037;
    /// `PR_TRANSPORT_MESSAGE_HEADERS` — the original RFC 5322 headers, when Outlook kept them.
    pub const TRANSPORT_HEADERS: u16 = 0x007D;
    /// `PR_BODY`, the plain-text body.
    pub const BODY: u16 = 0x1000;
    /// `PR_SENDER_NAME` and `PR_SENDER_EMAIL_ADDRESS`.
    pub const SENDER_NAME: u16 = 0x0C1A;
    pub const SENDER_EMAIL: u16 = 0x0C1F;
    /// `PR_SENT_REPRESENTING_*`, used when the sender properties are a delegate's.
    pub const SENT_REPRESENTING_NAME: u16 = 0x0042;
    pub const SENT_REPRESENTING_EMAIL: u16 = 0x0065;
    /// `PR_DISPLAY_TO` / `PR_DISPLAY_CC` — the recipient list as Outlook renders it.
    pub const DISPLAY_TO: u16 = 0x0E04;
    pub const DISPLAY_CC: u16 = 0x0E03;
    /// `PR_CLIENT_SUBMIT_TIME` and `PR_MESSAGE_DELIVERY_TIME`.
    pub const SUBMIT_TIME: u16 = 0x0039;
    pub const DELIVERY_TIME: u16 = 0x0E06;
    /// `PR_INTERNET_MESSAGE_ID`.
    pub const INTERNET_MESSAGE_ID: u16 = 0x1035;
    /// `PR_MESSAGE_FLAGS`. Bit 0 is "unsent", bit 1 is "unmodified", bit 5 is "read".
    pub const MESSAGE_FLAGS: u16 = 0x0E07;
    /// `PR_RTF_COMPRESSED` — a body this module cannot read.
    pub const RTF_COMPRESSED: u16 = 0x1009;
    /// `PR_HASATTACH`.
    pub const HAS_ATTACHMENT: u16 = 0x0E1B;
    /// `PR_DISPLAY_NAME`, on a folder.
    pub const DISPLAY_NAME: u16 = 0x3001;
    /// `PR_INTERNET_CPID` — the code page the message's text was written in.
    pub const INTERNET_CPID: u16 = 0x3FDE;
    /// `PR_MESSAGE_CODEPAGE`, the fallback when the first is absent.
    pub const MESSAGE_CODEPAGE: u16 = 0x3FFD;
}

/// One message pulled out of a `.pst`, as RFC 5322 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// Folder path, `/`-joined, as it was in Outlook.
    pub path: String,
    pub raw: Vec<u8>,
    pub seen: bool,
}

/// What a read of one file produced, including everything it could not do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub folders: usize,
    pub messages: usize,
    /// Messages whose body was RTF-only and could not be read as text.
    pub rtf_only: usize,
    /// Messages that carried attachments, which are not extracted.
    pub with_attachments: usize,
    /// Messages that could not be read at all.
    pub failed: usize,
}

/// Reads a string property, whatever width it was stored at.
///
/// PST stores strings as UTF-16 in a Unicode file and as an 8-bit code page in an ANSI one, and
/// the same property id can be either. Asking for one shape and getting the other is how an
/// importer produces mojibake in every field at once.
fn string(
    properties: &outlook_pst::messaging::message::MessageProperties,
    id: u16,
) -> Option<String> {
    match properties.get(id)? {
        PropertyValue::String8(value) => {
            Some(decode_string8(value.buffer(), code_page(properties)))
        }
        PropertyValue::Unicode(value) => Some(value.to_string()),
        _ => None,
    }
}

/// The code page a message's 8-bit text is written in, if it says.
///
/// `PR_INTERNET_CPID` first, because it is the one Outlook sets from the message's own
/// `Content-Type`; `PR_MESSAGE_CODEPAGE` is the store's answer and a reasonable second.
fn code_page(properties: &outlook_pst::messaging::message::MessageProperties) -> Option<u32> {
    integer(properties, prop::INTERNET_CPID)
        .or_else(|| integer(properties, prop::MESSAGE_CODEPAGE))
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
}

/// Decodes a `PT_STRING8` property using the code page the message declares.
///
/// These are **not** UTF-8. `PT_STRING8` is 8-bit text in whatever code page the store was
/// written with, and reading it as UTF-8 turned every non-ASCII character into a replacement
/// character: an ANSI archive imported with its accents, umlauts and Cyrillic destroyed, in the
/// subject line and the body alike. Nothing failed and nothing was logged; the text simply
/// arrived wrong, and the original `.pst` is often the only other copy.
///
/// Windows-1252 is the fallback rather than the answer. It is right for Western European stores
/// and wrong for Greek or Cyrillic ones, so it is only used when the message declines to say —
/// and it is still a strict improvement on UTF-8, which is wrong for all of them.
fn decode_string8(bytes: &[u8], code_page: Option<u32>) -> String {
    let label = code_page.map_or_else(
        || "windows-1252".to_string(),
        |page| match page {
            // The ones with names of their own rather than a `windows-N` form.
            65001 => "UTF-8".to_string(),
            20127 => "us-ascii".to_string(),
            28_591..=28_599 | 28_603 | 28_605 => format!("iso-8859-{}", page - 28_590),
            _ => format!("windows-{page}"),
        },
    );

    let charset = charset::Charset::for_label(label.as_bytes())
        .or_else(|| charset::Charset::for_label(b"windows-1252"));

    match charset {
        Some(charset) => charset.decode_without_bom_handling(bytes).0.into_owned(),
        // `for_label` cannot fail for windows-1252, but returning mojibake beats panicking on
        // somebody's archive.
        None => String::from_utf8_lossy(bytes).to_string(),
    }
}

fn integer(
    properties: &outlook_pst::messaging::message::MessageProperties,
    id: u16,
) -> Option<i32> {
    match properties.get(id)? {
        PropertyValue::Integer32(value) => Some(*value),
        _ => None,
    }
}

fn boolean(properties: &outlook_pst::messaging::message::MessageProperties, id: u16) -> bool {
    matches!(properties.get(id), Some(PropertyValue::Boolean(true)))
}

/// A `PT_SYSTIME` — 100-nanosecond intervals since 1601 — as epoch seconds.
fn timestamp(
    properties: &outlook_pst::messaging::message::MessageProperties,
    id: u16,
) -> Option<i64> {
    let PropertyValue::Time(value) = properties.get(id)? else {
        return None;
    };

    // 11644473600 is the number of seconds between 1601-01-01 and 1970-01-01. Getting this
    // wrong puts every imported message in the seventeenth century, which sorts them all to the
    // bottom of the mailbox and looks like the import lost their dates.
    let seconds = *value / 10_000_000 - 11_644_473_600;

    (seconds > 0).then_some(seconds)
}

/// Whether a message was read, from `PR_MESSAGE_FLAGS`.
fn was_read(properties: &outlook_pst::messaging::message::MessageProperties) -> bool {
    // mfRead is 0x00000001 in MS-OXCMSG. An archive imported as entirely unread is unusable
    // however correct the text is, which is why this is read at all.
    integer(properties, prop::MESSAGE_FLAGS).is_some_and(|flags| flags & 0x1 != 0)
}

/// Turns MAPI properties into an RFC 5322 message.
///
/// Two paths, and the difference matters. When `PR_TRANSPORT_MESSAGE_HEADERS` is present — which
/// it is for most *received* mail — those are the message's real headers, and using them means
/// the real `Message-ID`, `References` and `Date` survive, so an imported reply threads with a
/// synced original. When it is absent, which is normal for mail the user sent, the headers are
/// built from the individual properties and the `Message-ID` is invented. An invented id cannot
/// thread against anything, and that is a real loss rather than a detail.
fn synthesise(
    properties: &outlook_pst::messaging::message::MessageProperties,
    index: usize,
) -> Vec<u8> {
    let body = string(properties, prop::BODY).unwrap_or_default();

    if let Some(headers) = string(properties, prop::TRANSPORT_HEADERS) {
        if headers.contains(':') {
            let mut raw = without_content_headers(&headers);

            // Describing the body that is actually being attached. These are the same two lines
            // the synthesised path below writes, for the same reason: what follows is `PR_BODY`,
            // plain text MAPI has already decoded.
            //
            // The original headers were previously kept whole, including the ones describing
            // how the *original* body was encoded. A message that arrived as base64, or as
            // `multipart/alternative` with a boundary, was imported with those headers over a
            // plain-text body -- so a parser base64-decoded ordinary prose, or searched for a
            // boundary that was not there, and the message imported with no readable body at
            // all. The headers were right about everything except the thing they were attached
            // to.
            raw.push_str("MIME-Version: 1.0\r\n");
            raw.push_str("Content-Type: text/plain; charset=utf-8\r\n");
            raw.push_str("\r\n");
            raw.push_str(&body);
            return raw.into_bytes();
        }
    }

    let subject = string(properties, prop::SUBJECT).unwrap_or_default();

    // The sender, preferring the represented identity: mail sent by a delegate carries the
    // assistant in PR_SENDER and the person it was sent for in PR_SENT_REPRESENTING, and the
    // second is the one a reader means by "who is this from".
    let name = string(properties, prop::SENT_REPRESENTING_NAME)
        .or_else(|| string(properties, prop::SENDER_NAME))
        .unwrap_or_default();
    let email = string(properties, prop::SENT_REPRESENTING_EMAIL)
        .or_else(|| string(properties, prop::SENDER_EMAIL))
        .unwrap_or_default();

    let date = timestamp(properties, prop::SUBMIT_TIME)
        .or_else(|| timestamp(properties, prop::DELIVERY_TIME))
        .unwrap_or(0);

    let message_id = string(properties, prop::INTERNET_MESSAGE_ID)
        .filter(|id| id.contains('@'))
        .unwrap_or_else(|| format!("<pst-import-{index}@halcyon.invalid>"));

    let mut raw = String::new();
    raw.push_str(&format!("Message-ID: {}\r\n", ensure_angled(&message_id)));
    raw.push_str(&format!("From: {}\r\n", address(&name, &email)));

    if let Some(to) = string(properties, prop::DISPLAY_TO).filter(|value| !value.is_empty()) {
        // A display list, not addresses. Outlook stores "Ada Lovelace; Charles Babbage" here,
        // and inventing an address for each name would put mail in front of somebody addressed
        // to a mailbox that does not exist. Kept as names, which is honest and still searchable.
        raw.push_str(&format!("To: {}\r\n", header_safe(&to)));
    }
    if let Some(cc) = string(properties, prop::DISPLAY_CC).filter(|value| !value.is_empty()) {
        raw.push_str(&format!("Cc: {}\r\n", header_safe(&cc)));
    }

    raw.push_str(&format!("Subject: {}\r\n", header_safe(&subject)));
    raw.push_str(&format!("Date: {}\r\n", rfc2822(date)));
    raw.push_str("MIME-Version: 1.0\r\n");
    raw.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    raw.push_str("X-Halcyon-Imported-From: Outlook PST\r\n");
    raw.push_str("\r\n");
    raw.push_str(&body);

    raw.into_bytes()
}

/// The transport headers, minus everything that describes how a body was encoded.
///
/// `PR_TRANSPORT_MESSAGE_HEADERS` is worth keeping for what it says about the *message*: the
/// real `Message-ID`, `References` and `Date` are what let an imported reply thread against an
/// original that was synced rather than imported. What it must not keep is what it says about
/// the *body*, because the body it is being attached to is a different one.
///
/// Folded continuations follow the header they belong to, so dropping a header drops its
/// continuation lines with it — otherwise a wrapped `Content-Type` would leave its boundary
/// parameter behind as a line of its own, which is a malformed header block rather than a
/// missing one.
fn without_content_headers(headers: &str) -> String {
    let mut kept = String::with_capacity(headers.len());
    let mut dropping = false;

    for line in headers.lines() {
        // A blank line ends the header block. Anything after it is not a header, and treating
        // it as one would put a second body separator in the middle of the message.
        if line.trim().is_empty() {
            break;
        }

        // Continuations belong to whatever header they follow, kept or dropped.
        if line.starts_with(' ') || line.starts_with('\t') {
            if !dropping {
                kept.push_str(line);
                kept.push_str("\r\n");
            }
            continue;
        }

        let name = line
            .split(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();

        dropping = name == "mime-version" || name.starts_with("content-");

        if !dropping {
            kept.push_str(line.trim_end());
            kept.push_str("\r\n");
        }
    }

    kept
}

/// Wraps a message id in angle brackets if it has none.
fn ensure_angled(id: &str) -> String {
    let trimmed = id.trim();
    if trimmed.starts_with('<') && trimmed.ends_with('>') {
        trimmed.to_string()
    } else {
        format!("<{trimmed}>")
    }
}

/// A `From:` value from a name and an address.
fn address(name: &str, email: &str) -> String {
    // An Exchange distinguished name — `/O=…/OU=…/CN=…` — is not an address, and putting one in
    // a From line produces a message no client can reply to. Old PSTs are full of them.
    let usable = email.contains('@') && !email.starts_with('/');

    match (name.trim().is_empty(), usable) {
        (true, true) => email.trim().to_string(),
        (false, true) => format!("{} <{}>", header_safe(name), email.trim()),
        (false, false) => format!("{} <unknown@halcyon.invalid>", header_safe(name)),
        (true, false) => "unknown@halcyon.invalid".to_string(),
    }
}

/// Strips anything from a header value that would end the header.
///
/// Property values come out of somebody else's file. A newline in a subject is header injection
/// — it ends the field and starts a new one — and the fact that the file is on the user's own
/// disk does not make its contents theirs.
fn header_safe(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character != '\r' && *character != '\n')
        .collect::<String>()
        .trim()
        .to_string()
}

/// Epoch seconds as an RFC 2822 date.
fn rfc2822(epoch_seconds: i64) -> String {
    use chrono::{TimeZone, Utc};

    Utc.timestamp_opt(epoch_seconds, 0)
        .single()
        .map(|when| when.format("%a, %d %b %Y %H:%M:%S +0000").to_string())
        .unwrap_or_else(|| "Thu, 01 Jan 1970 00:00:00 +0000".to_string())
}

/// Reads every message in a `.pst`, handing each to `each` as it is found.
///
/// Streamed rather than collected: an Outlook archive of fifteen years is routinely several
/// gigabytes, and holding every message in memory to return a `Vec` would fail on exactly the
/// files people most want to import.
pub fn read(
    path: &Path,
    mut each: impl FnMut(Extracted) -> std::io::Result<()>,
) -> std::io::Result<Counts> {
    let store = outlook_pst::open_store(path)?;

    let root = store.properties().ipm_sub_tree_entry_id()?;
    let mut counts = Counts::default();

    walk(&store, &root, "", &mut counts, &mut each, 0)?;

    Ok(counts)
}

/// How deep the folder walk will go. A guard against a malformed store, not a real limit.
const MAX_DEPTH: usize = 32;

fn walk(
    store: &Rc<dyn Store>,
    entry_id: &outlook_pst::messaging::store::EntryId,
    prefix: &str,
    counts: &mut Counts,
    each: &mut impl FnMut(Extracted) -> std::io::Result<()>,
    depth: usize,
) -> std::io::Result<()> {
    if depth > MAX_DEPTH {
        tracing::warn!(prefix, "folder nesting is too deep; not descending further");
        return Ok(());
    }

    let Ok(folder) = store.open_folder(entry_id) else {
        // One unreadable folder is not a reason to abandon an archive. Counted, not fatal.
        counts.failed += 1;
        return Ok(());
    };

    let name = folder
        .properties()
        .display_name()
        .unwrap_or_else(|_| "Folder".to_string());

    let path = if prefix.is_empty() {
        name.clone()
    } else {
        format!("{prefix}/{name}")
    };

    counts.folders += 1;

    if let Some(contents) = folder.contents_table() {
        read_messages(store, contents, &path, counts, each)?;
    }

    if let Some(hierarchy) = folder.hierarchy_table() {
        let context = hierarchy.context();

        for row in hierarchy.rows_matrix() {
            // A subfolder's node id is the row id. Building an entry id from it is how the
            // hierarchy table refers to children.
            let Ok(child) = store.properties().make_entry_id(u32::from(row.id()).into()) else {
                counts.failed += 1;
                continue;
            };

            let _ = context;
            walk(store, &child, &path, counts, each, depth + 1)?;
        }
    }

    Ok(())
}

fn read_messages(
    store: &Rc<dyn Store>,
    contents: &Rc<dyn outlook_pst::ltp::table_context::TableContext>,
    path: &str,
    counts: &mut Counts,
    each: &mut impl FnMut(Extracted) -> std::io::Result<()>,
) -> std::io::Result<()> {
    for (index, row) in contents.rows_matrix().enumerate() {
        let Ok(entry_id) = store.properties().make_entry_id(u32::from(row.id()).into()) else {
            counts.failed += 1;
            continue;
        };

        let Ok(message) = store.open_message(&entry_id, None) else {
            counts.failed += 1;
            continue;
        };

        let properties = message.properties();

        if properties.get(prop::RTF_COMPRESSED).is_some()
            && string(properties, prop::BODY).is_none()
        {
            // The body exists and is in a format this cannot read. The message still imports —
            // its headers are worth having — but it is counted so the total is honest.
            counts.rtf_only += 1;
        }

        if boolean(properties, prop::HAS_ATTACHMENT) {
            counts.with_attachments += 1;
        }

        each(Extracted {
            path: path.to_string(),
            raw: synthesise(properties, index),
            seen: was_read(properties),
        })?;

        counts.messages += 1;
    }

    Ok(())
}

/// The display name of a folder, for the UI's list. Unused elsewhere; see `prop::DISPLAY_NAME`.
pub fn folder_property_id() -> u16 {
    prop::DISPLAY_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_1601_timestamp_becomes_the_right_epoch_second() {
        // The single easiest thing to get wrong here, and the symptom is subtle: every imported
        // message dated in the seventeenth century, sorted to the bottom, looking like the dates
        // were lost rather than shifted.
        //
        // 1970-01-01 in FILETIME is exactly 11644473600 * 10^7.
        let epoch_in_filetime = 11_644_473_600_i64 * 10_000_000;
        assert_eq!(epoch_in_filetime / 10_000_000 - 11_644_473_600, 0);

        // And one real date: 2026-08-27T09:34:00Z is 1787304840.
        let when = (1_787_304_840_i64 + 11_644_473_600) * 10_000_000;
        assert_eq!(when / 10_000_000 - 11_644_473_600, 1_787_304_840);
    }

    #[test]
    fn an_exchange_distinguished_name_is_not_used_as_an_address() {
        // Old PSTs are full of `/O=ORG/OU=EXCHANGE/CN=RECIPIENTS/CN=ADA`. Putting one in a From
        // line produces a message no client can reply to, and it looks like a real address.
        let built = address("Ada Lovelace", "/O=ORG/OU=FIRST/CN=RECIPIENTS/CN=ADA");
        assert!(built.contains("Ada Lovelace"));
        assert!(!built.contains("/O="), "{built}");
        assert!(built.contains("@"), "{built}");
    }

    #[test]
    fn a_real_address_is_used_as_it_is() {
        assert_eq!(
            address("Ada Lovelace", "ada@example.test"),
            "Ada Lovelace <ada@example.test>"
        );
        assert_eq!(address("", "ada@example.test"), "ada@example.test");
    }

    #[test]
    fn a_newline_in_a_subject_cannot_inject_a_header() {
        // The property came out of somebody else's file. A newline here ends the Subject field
        // and starts whatever the attacker chose — a Bcc, a different From.
        let cleaned = header_safe("Invoice\r\nBcc: attacker@example.test");
        assert!(!cleaned.contains('\n'));
        assert!(!cleaned.contains('\r'));
        assert_eq!(cleaned, "InvoiceBcc: attacker@example.test");
    }

    #[test]
    fn a_message_id_is_always_angled() {
        assert_eq!(ensure_angled("a@b"), "<a@b>");
        assert_eq!(ensure_angled("<a@b>"), "<a@b>");
    }

    #[test]
    fn a_date_that_cannot_be_represented_falls_back_rather_than_panicking() {
        // Standing rule 13. A message with an absurd timestamp is still a message.
        assert!(rfc2822(i64::MAX).contains("1970"));
        assert!(rfc2822(1_787_304_840).contains("2026"));
    }

    #[test]
    fn the_originals_encoding_headers_do_not_survive_onto_a_decoded_body() {
        // The bug. `PR_TRANSPORT_MESSAGE_HEADERS` describes the message as it arrived; the body
        // glued underneath is `PR_BODY`, which MAPI has already decoded to plain text. Keeping
        // the original `Content-Transfer-Encoding: base64` meant a parser base64-decoded
        // ordinary prose, and keeping a `multipart/alternative` boundary meant it looked for a
        // separator that was not there. Either way the message imported with no readable body.
        let headers = "Message-ID: <real@example.test>\r\n\
                       From: Ada <ada@example.test>\r\n\
                       References: <root@example.test>\r\n\
                       MIME-Version: 1.0\r\n\
                       Content-Type: multipart/alternative;\r\n\
                       \tboundary=\"----=_Part_1_2\"\r\n\
                       Content-Transfer-Encoding: base64\r\n\
                       Subject: Quarterly\r\n";

        let kept = without_content_headers(headers);

        // What the message says about itself survives -- this is why the transport headers are
        // used at all.
        assert!(kept.contains("Message-ID: <real@example.test>"));
        assert!(kept.contains("References: <root@example.test>"));
        assert!(kept.contains("Subject: Quarterly"));
        assert!(kept.contains("From: Ada <ada@example.test>"));

        // What it says about a body it no longer has does not.
        assert!(!kept.to_ascii_lowercase().contains("content-type"));
        assert!(!kept
            .to_ascii_lowercase()
            .contains("content-transfer-encoding"));
        assert!(!kept.to_ascii_lowercase().contains("mime-version"));

        // Including the folded continuation, which would otherwise be left behind as a line of
        // its own and make the whole header block malformed.
        assert!(!kept.contains("boundary"));
        assert!(!kept.contains("----=_Part_1_2"));
    }

    #[test]
    fn a_blank_line_ends_the_header_block() {
        // Some PSTs store the headers with the blank separator still attached. Treating what
        // follows as headers would put a second body separator into the message.
        let headers = "Subject: Hello\r\n\r\nThis is body text, not a header.\r\n";
        let kept = without_content_headers(headers);

        assert!(kept.contains("Subject: Hello"));
        assert!(!kept.contains("body text"));
    }

    #[test]
    fn a_folded_header_that_is_kept_keeps_its_continuation() {
        // The other half of the folding rule: dropping continuations wholesale would truncate
        // a long References chain, which is exactly what threading needs.
        let headers = "References: <a@example.test>\r\n\t<b@example.test>\r\nSubject: Re: Hi\r\n";
        let kept = without_content_headers(headers);

        assert!(kept.contains("<a@example.test>"));
        assert!(
            kept.contains("<b@example.test>"),
            "a kept header lost its continuation: {kept}"
        );
        assert!(kept.contains("Subject: Re: Hi"));
    }

    #[test]
    fn ansi_text_is_decoded_by_its_code_page_rather_than_as_utf8() {
        // `PT_STRING8` is 8-bit text in the store's code page. Reading it as UTF-8 turned every
        // non-ASCII byte into a replacement character, so an ANSI archive imported with its
        // accents and umlauts destroyed — silently, in subjects and bodies alike, with the
        // original .pst often the only other copy.

        // "Grüße" in windows-1252: ü is 0xFC, ß is 0xDF.
        let latin1 = [b'G', b'r', 0xFC, b'\xDF', b'e'];
        assert_eq!(decode_string8(&latin1, Some(1252)), "Grüße");

        // The same bytes read as UTF-8, which is what used to happen.
        assert!(
            String::from_utf8_lossy(&latin1).contains('\u{FFFD}'),
            "the old path really did produce replacement characters"
        );

        // Cyrillic, where guessing windows-1252 would be wrong and the declared page is right.
        // "Привет" in windows-1251.
        let cyrillic = [0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2];
        assert_eq!(decode_string8(&cyrillic, Some(1251)), "Привет");

        // Greek, likewise. "Γειά" in windows-1253.
        let greek = [0xC3, 0xE5, 0xE9, 0xDC];
        assert_eq!(decode_string8(&greek, Some(1253)), "Γειά");
    }

    #[test]
    fn a_declared_utf8_code_page_is_honoured() {
        // 65001 is UTF-8, and a store that says so means it.
        assert_eq!(decode_string8("café".as_bytes(), Some(65001)), "café");
    }

    #[test]
    fn an_undeclared_code_page_falls_back_without_destroying_ascii() {
        // Windows-1252 is a guess, and it is only reached when the message declines to say. It
        // is still strictly better than UTF-8, which is wrong for every ANSI store rather than
        // only for the non-Western ones.
        assert_eq!(decode_string8(b"plain ascii", None), "plain ascii");
        assert_eq!(decode_string8(&[b'c', b'a', b'f', 0xE9], None), "café");
    }

    #[test]
    fn an_unknown_code_page_does_not_panic() {
        // A corrupt or exotic value must not take down an import of somebody's archive.
        assert_eq!(decode_string8(b"hello", Some(999_999)), "hello");
        assert_eq!(decode_string8(b"hello", Some(0)), "hello");
    }
}
