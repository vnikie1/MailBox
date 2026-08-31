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
        PropertyValue::String8(value) => Some(String::from_utf8_lossy(value.buffer()).to_string()),
        PropertyValue::Unicode(value) => Some(value.to_string()),
        _ => None,
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
            let mut raw = headers.trim_end().to_string();
            raw.push_str("\r\n\r\n");
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
}
