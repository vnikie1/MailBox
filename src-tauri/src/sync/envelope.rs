//! Turning an IMAP `ENVELOPE` into a row we can store and show.
//!
//! Every human-readable field in a mail header arrives RFC 2047 encoded — `=?UTF-8?B?...?=`
//! — whenever it contains anything outside ASCII, which for most people's mail is most of the
//! time. A client that stores those raw shows `=?UTF-8?Q?Bj=C3=B6rn?=` in the sender column,
//! and that is the difference between a mail client and a demo.
//!
//! Standing rule 13 governs the whole file: **parse leniently, degrade visibly.** A header
//! that cannot be decoded keeps its raw text rather than being dropped, because a message
//! with a mangled subject is still a message the user needs to see.

use crate::sync::threading::subject_base;

/// One address, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

impl Address {
    /// What the list shows: a display name where there is one, otherwise the address.
    pub fn display(&self) -> &str {
        match &self.name {
            Some(name) if !name.trim().is_empty() => name,
            _ => &self.email,
        }
    }
}

/// An envelope, decoded into the shape the `message` table wants.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Envelope {
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub subject: String,
    pub subject_base: String,
    pub from: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub reply_to: Vec<Address>,
    /// Epoch seconds from the `Date` header, or 0 when it is missing or unparseable.
    pub date_sent: i64,
}

impl Envelope {
    /// The denormalised search column. docs/03 §3 — `from_all` feeds FTS5.
    pub fn from_all(&self) -> String {
        join_for_search(&self.from)
    }

    pub fn to_all(&self) -> String {
        let mut all = self.to.clone();
        all.extend(self.cc.iter().cloned());
        join_for_search(&all)
    }

    pub fn from_name(&self) -> Option<String> {
        self.from.first().and_then(|a| a.name.clone())
    }

    pub fn from_addr(&self) -> Option<String> {
        self.from.first().map(|a| a.email.clone())
    }
}

fn join_for_search(addresses: &[Address]) -> String {
    let mut parts = Vec::with_capacity(addresses.len() * 2);

    for address in addresses {
        if let Some(name) = &address.name {
            if !name.trim().is_empty() {
                parts.push(name.clone());
            }
        }
        if !address.email.is_empty() {
            parts.push(address.email.clone());
        }
    }

    parts.join(" ")
}

/// Decodes RFC 2047 encoded words, leaving anything undecodable as it was.
///
/// `mailparse` does the work, including the character sets — ISO-8859-1 and the Windows
/// code pages are still very much in circulation, and a UTF-8-only decoder turns a German
/// sender's name into replacement characters.
pub fn decode_words(raw: &str) -> String {
    let decoded = mailparse::parse_header(format!("X: {raw}").as_bytes())
        .map(|(header, _)| header.get_value())
        .unwrap_or_else(|_| raw.to_string());

    // Header folding puts CRLF + whitespace inside long subjects. They are continuation
    // markers, not content, and a subject rendered with a newline in it breaks the row
    // height the list is built around.
    unfold(&decoded)
}

fn unfold(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_space = false;

    for character in value.chars() {
        let is_space =
            character == '\r' || character == '\n' || character == '\t' || character == ' ';

        if is_space {
            if !last_was_space && !out.is_empty() {
                out.push(' ');
            }
            last_was_space = true;
        } else {
            out.push(character);
            last_was_space = false;
        }
    }

    out.trim_end().to_string()
}

/// Bytes from an IMAP envelope field, decoded.
///
/// IMAP delivers these as raw octets with no character set declared beyond whatever RFC 2047
/// says inline, so this is lossy-UTF-8 first and encoded-word decoding second.
fn decode_bytes(raw: Option<&[u8]>) -> Option<String> {
    let bytes = raw?;
    if bytes.is_empty() {
        return None;
    }

    let text = String::from_utf8_lossy(bytes);
    Some(decode_words(&text))
}

/// Builds an `Address` from an IMAP address structure's four parts.
fn address_from_parts(
    name: Option<&[u8]>,
    mailbox: Option<&[u8]>,
    host: Option<&[u8]>,
) -> Option<Address> {
    let mailbox = mailbox.map(|b| String::from_utf8_lossy(b).to_string());
    let host = host.map(|b| String::from_utf8_lossy(b).to_string());

    let email = match (mailbox, host) {
        (Some(mailbox), Some(host)) if !mailbox.is_empty() && !host.is_empty() => {
            format!("{mailbox}@{host}")
        }
        // A group syntax marker ("undisclosed-recipients:;") has a mailbox and no host. It is
        // not an address, and inventing one would put a fake recipient in the reader.
        (Some(mailbox), None) if !mailbox.is_empty() => return None,
        _ => return None,
    };

    Some(Address {
        name: decode_bytes(name),
        email,
    })
}

/// Parses a `Date:` header into epoch seconds.
///
/// Returns 0 rather than failing: a message with an unparseable date is still a message, and
/// docs/03 §13 says degrade visibly. The list sorts it to the bottom, which is visible.
pub fn parse_date(raw: &str) -> i64 {
    mailparse::dateparse(raw).unwrap_or(0)
}

/// Strips the angle brackets from a `Message-ID`, and rejects obvious rubbish.
///
/// Stored without brackets so that `References` matching is a plain string comparison. The
/// length cap is not arbitrary: a malformed header can carry kilobytes, and it would go
/// straight into an index.
fn normalise_message_id(raw: Option<String>) -> Option<String> {
    let value = raw?;
    let trimmed = value
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();

    if trimmed.is_empty() || trimmed.len() > 998 {
        return None;
    }

    Some(trimmed.to_string())
}

/// Splits a `References` header into individual ids.
pub fn parse_references(raw: &str) -> Vec<String> {
    raw.split_whitespace()
        .filter_map(|token| normalise_message_id(Some(token.to_string())))
        .collect()
}

/// Builds an `Envelope` from `imap_proto`'s parsed structure.
pub fn from_imap(envelope: &async_imap::imap_proto::Envelope<'_>) -> Envelope {
    let addresses = |list: &Option<Vec<async_imap::imap_proto::Address<'_>>>| -> Vec<Address> {
        list.as_ref()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|address| {
                        address_from_parts(
                            address.name.as_deref(),
                            address.mailbox.as_deref(),
                            address.host.as_deref(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let subject = decode_bytes(envelope.subject.as_deref()).unwrap_or_default();

    Envelope {
        message_id: normalise_message_id(decode_bytes(envelope.message_id.as_deref())),
        in_reply_to: normalise_message_id(decode_bytes(envelope.in_reply_to.as_deref())),
        subject_base: subject_base(&subject),
        subject,
        from: addresses(&envelope.from),
        to: addresses(&envelope.to),
        cc: addresses(&envelope.cc),
        bcc: addresses(&envelope.bcc),
        reply_to: addresses(&envelope.reply_to),
        date_sent: decode_bytes(envelope.date.as_deref())
            .map(|date| parse_date(&date))
            .unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ascii_passes_through_untouched() {
        assert_eq!(decode_words("Quarterly report"), "Quarterly report");
    }

    #[test]
    fn base64_encoded_words_are_decoded() {
        // The single most common thing in a real mailbox that a naive client gets wrong.
        assert_eq!(decode_words("=?UTF-8?B?QmrDtnJu?="), "Björn");
        assert_eq!(decode_words("=?UTF-8?B?TcO2Z2xpY2hrZWl0?="), "Möglichkeit");
    }

    #[test]
    fn quoted_printable_encoded_words_are_decoded() {
        assert_eq!(decode_words("=?UTF-8?Q?Bj=C3=B6rn?="), "Björn");
        // Underscore means space inside a Q-encoded word — not an underscore.
        assert_eq!(decode_words("=?UTF-8?Q?Hello_World?="), "Hello World");
    }

    #[test]
    fn legacy_character_sets_are_decoded_rather_than_mangled() {
        // ISO-8859-1 and the Windows code pages are still in wide circulation. A UTF-8-only
        // decoder turns these into replacement characters, which is worse than not decoding.
        assert_eq!(decode_words("=?ISO-8859-1?Q?Bj=F6rn?="), "Björn");
        assert_eq!(decode_words("=?windows-1252?Q?caf=E9?="), "café");
    }

    #[test]
    fn a_subject_mixing_encoded_and_plain_text_decodes_both_halves() {
        let decoded = decode_words("Re: =?UTF-8?B?bcO2dGU=?= tomorrow");

        assert!(decoded.contains("möte"), "{decoded}");
        assert!(decoded.starts_with("Re:"), "{decoded}");
        assert!(decoded.ends_with("tomorrow"), "{decoded}");
    }

    #[test]
    fn an_undecodable_header_keeps_its_raw_text_rather_than_vanishing() {
        // Standing rule 13. A message with a mangled subject is still a message the user
        // needs to see; a message that was dropped because its header was malformed is a
        // message that has disappeared from their mailbox.
        let broken = "=?UTF-8?B?!!!not-base64!!!?=";
        let decoded = decode_words(broken);

        assert!(!decoded.is_empty(), "must not decode to nothing");
    }

    #[test]
    fn folded_headers_are_flattened_to_one_line() {
        // Long subjects arrive split across lines with leading whitespace. A newline in a
        // subject breaks the fixed row height the message list is built on.
        let folded = "A very long subject line that the\r\n sending server folded in two";
        let decoded = decode_words(folded);

        assert!(!decoded.contains('\n'));
        assert!(!decoded.contains('\r'));
        assert_eq!(
            decoded,
            "A very long subject line that the sending server folded in two"
        );
    }

    #[test]
    fn message_ids_are_stored_without_their_brackets() {
        // Stored bare so that matching a References entry against a Message-ID is a string
        // comparison rather than a parse on every candidate.
        assert_eq!(
            normalise_message_id(Some("<abc@example.test>".into())),
            Some("abc@example.test".into())
        );
        assert_eq!(
            normalise_message_id(Some("  <abc@example.test>  ".into())),
            Some("abc@example.test".into())
        );
        assert_eq!(
            normalise_message_id(Some("abc@example.test".into())),
            Some("abc@example.test".into())
        );
    }

    #[test]
    fn an_absurd_message_id_is_rejected_rather_than_indexed() {
        // A malformed header can carry kilobytes, and this value goes into ix_msg_msgid.
        assert_eq!(normalise_message_id(Some("<>".into())), None);
        assert_eq!(normalise_message_id(Some("   ".into())), None);
        assert_eq!(normalise_message_id(Some("x".repeat(2000))), None);
        assert_eq!(normalise_message_id(None), None);
    }

    #[test]
    fn references_split_on_whitespace_and_keep_their_order() {
        let parsed = parse_references("<a@x> <b@x>\r\n <c@x>");

        assert_eq!(parsed, vec!["a@x", "b@x", "c@x"]);
    }

    #[test]
    fn an_empty_references_header_yields_nothing() {
        assert!(parse_references("").is_empty());
        assert!(parse_references("   \r\n ").is_empty());
    }

    #[test]
    fn an_address_needs_both_halves_to_be_an_address() {
        // "undisclosed-recipients:;" is group syntax, not a mailbox. Inventing an address
        // for it would put a recipient in the reader that nobody ever wrote to.
        assert_eq!(
            address_from_parts(None, Some(b"ada"), Some(b"example.test")),
            Some(Address {
                name: None,
                email: "ada@example.test".into()
            })
        );

        assert_eq!(
            address_from_parts(None, Some(b"undisclosed-recipients"), None),
            None
        );
        assert_eq!(address_from_parts(Some(b"Nobody"), None, None), None);
        assert_eq!(address_from_parts(None, None, None), None);
    }

    #[test]
    fn an_address_display_name_is_decoded() {
        let address = address_from_parts(
            Some("=?UTF-8?B?QmrDtnJu?=".as_bytes()),
            Some(b"bjorn"),
            Some(b"example.test"),
        )
        .expect("address");

        assert_eq!(address.name.as_deref(), Some("Björn"));
        assert_eq!(address.display(), "Björn");
    }

    #[test]
    fn an_address_with_no_name_displays_its_email() {
        let address = Address {
            name: None,
            email: "ada@example.test".into(),
        };
        assert_eq!(address.display(), "ada@example.test");

        // An empty name is the same as no name — servers send both.
        let blank = Address {
            name: Some("   ".into()),
            email: "ada@example.test".into(),
        };
        assert_eq!(blank.display(), "ada@example.test");
    }

    #[test]
    fn the_search_columns_carry_names_and_addresses_both() {
        // FTS5 searches these. Someone looking for "Björn" and someone looking for
        // "bjorn@example.test" must both find the message.
        let envelope = Envelope {
            from: vec![Address {
                name: Some("Björn Nilsson".into()),
                email: "bjorn@example.test".into(),
            }],
            to: vec![Address {
                name: None,
                email: "ada@example.test".into(),
            }],
            cc: vec![Address {
                name: Some("Grace".into()),
                email: "grace@example.test".into(),
            }],
            ..Envelope::default()
        };

        assert!(envelope.from_all().contains("Björn Nilsson"));
        assert!(envelope.from_all().contains("bjorn@example.test"));

        // cc is searchable alongside to — a message someone was copied on is one they expect
        // to find by their own name.
        assert!(envelope.to_all().contains("ada@example.test"));
        assert!(envelope.to_all().contains("Grace"));
    }

    #[test]
    fn dates_parse_from_the_shapes_servers_actually_send() {
        assert!(parse_date("Tue, 25 Aug 2026 19:30:00 +0000") > 0);
        assert!(parse_date("25 Aug 2026 19:30:00 -0700") > 0);
        assert!(parse_date("Tue, 25 Aug 2026 19:30:00 GMT") > 0);
    }

    #[test]
    fn an_unparseable_date_is_zero_rather_than_an_error() {
        // Degrade visibly: the message sorts to the bottom of the list, where its wrong date
        // is obvious, instead of failing the whole sync batch it arrived in.
        assert_eq!(parse_date(""), 0);
        assert_eq!(parse_date("not a date at all"), 0);
    }

    #[test]
    fn the_subject_base_is_computed_alongside_the_subject() {
        // Threading and the conversation title both read it, so it is derived once here
        // rather than recomputed at every call site.
        let subject = "Re: Re: quarterly report";
        let envelope = Envelope {
            subject_base: subject_base(subject),
            subject: subject.to_string(),
            ..Envelope::default()
        };

        assert_eq!(envelope.subject, "Re: Re: quarterly report");
        assert_eq!(envelope.subject_base, "quarterly report");
    }
}
