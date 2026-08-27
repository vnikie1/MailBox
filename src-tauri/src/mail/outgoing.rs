//! Building a message to send. docs/06 Phase 7.
//!
//! Produces the `.eml` bytes that the outbox stores and SMTP transmits. Nothing here talks to
//! a network: a message is built, written to disk, and only then queued, so a crash between
//! the two loses a draft rather than half-sending something.
//!
//! ## Why every message is sent as both plain text and HTML
//!
//! `multipart/alternative` with the plain part **first**. Not for old clients — for the ones
//! that strip HTML by policy, for screen readers, for anyone reading on a watch, and because
//! a message with no text part reads as spam to more than one filter. The order is part of the
//! format: RFC 2046 §5.1.4 says the *last* part is the richest, and clients that show the first
//! one they understand rely on it.
//!
//! ## Where the strictness comes from
//!
//! docs/04's Phase 7 exit gate names Outlook Windows as the renderer to satisfy, and it is
//! unforgiving in specific ways: it wants a `Content-Type` on every part, it dislikes bare line
//! feeds, and it will not thread a reply whose `References` chain is malformed. `lettre` handles
//! the encoding; the parts this file owns are the headers and the shape.

use lettre::message::header::{self, ContentType};
use lettre::message::{Mailbox, MultiPart, SinglePart};
use lettre::Message;

use crate::sync::envelope::Address;

/// Everything needed to build one outgoing message.
#[derive(Debug, Clone, Default)]
pub struct Draft {
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    /// The body as the user wrote it, already sanitised HTML.
    pub html: String,
    /// The plain-text alternative. Derived from the HTML when the caller does not supply one.
    pub text: Option<String>,
    /// `Message-ID`s this reply continues, oldest first. See `reply::references`.
    pub references: Vec<String>,
    /// The immediate parent, for `In-Reply-To`.
    pub in_reply_to: Option<String>,
    /// This message's own `Message-ID`. Generated when absent.
    ///
    /// Ours rather than the library's, and that matters for one specific reason: it is how a
    /// send interrupted by a crash is resolved. On restart the outbox looks for this id in the
    /// account's Sent mailbox — present means it went, absent means it did not — which is the
    /// only way to avoid choosing between silently losing a message and silently sending it
    /// twice. See `sync::outbox`.
    pub message_id: Option<String>,
}

/// A built message and the identity it will be known by.
#[derive(Debug, Clone)]
pub struct Built {
    pub bytes: Vec<u8>,
    pub message_id: String,
}

/// Generates an RFC 5322 `Message-ID`.
///
/// The domain half comes from the sender's address rather than the machine's hostname. A
/// hostname would leak the name of the user's computer to every recipient, which standing rule
/// 16 rules out just as firmly as a tracking header would be.
fn generate_message_id(from: &Address) -> String {
    use base64::Engine;
    use rand::RngCore;

    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let unique = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);

    let domain = from
        .email
        .rsplit('@')
        .next()
        .filter(|domain| !domain.trim().is_empty())
        .unwrap_or("halcyon.invalid");

    format!("<{unique}@{domain}>")
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("{field} is not a usable address: {value}")]
    Address { field: &'static str, value: String },

    #[error("a message needs at least one recipient")]
    NoRecipients,

    #[error("the message could not be assembled: {0}")]
    Assembly(String),
}

/// Converts one of our addresses into lettre's.
///
/// The display name is passed through rather than escaped by hand: lettre applies RFC 2047
/// encoding where it is needed, which is the part that goes wrong when written by hand — a
/// name with an accent in it becomes mojibake in the recipient's client, and the sender never
/// finds out.
fn mailbox(field: &'static str, address: &Address) -> Result<Mailbox, BuildError> {
    let email = address.email.trim();

    let parsed = email.parse().map_err(|_| BuildError::Address {
        field,
        value: email.to_string(),
    })?;

    let name = address
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string);

    Ok(Mailbox::new(name, parsed))
}

/// Derives a plain-text alternative from HTML.
///
/// Deliberately crude, and only used when the caller has nothing better. It is not a renderer:
/// it unwraps the block structure so the text reads as paragraphs rather than one run-on line,
/// and decodes the handful of entities that appear in ordinary prose.
///
/// The editor supplies the real thing — it knows what the user actually typed, which is always
/// a better plain-text version than anything recovered from markup afterwards.
pub fn text_from_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut inside_tag = false;
    let mut tag = String::new();

    for character in html.chars() {
        match character {
            '<' => {
                inside_tag = true;
                tag.clear();
            }
            '>' => {
                inside_tag = false;
                let name: String = tag
                    .trim_start_matches('/')
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_ascii_lowercase();

                // Anything that ends a line in the rendered output ends one here.
                if matches!(
                    name.as_str(),
                    "p" | "div" | "br" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "blockquote"
                ) {
                    out.push('\n');
                }
            }
            _ if inside_tag => tag.push(character),
            _ => out.push(character),
        }
    }

    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        // Last, or it would corrupt the entities above.
        .replace("&amp;", "&");

    // Collapse the runs of blank lines the block-tag rule produces.
    let mut lines: Vec<&str> = Vec::new();
    let mut blank = false;
    for line in decoded.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            if blank {
                continue;
            }
            blank = true;
        } else {
            blank = false;
        }
        lines.push(trimmed);
    }

    lines.join("\r\n").trim().to_string()
}

/// Quotes a plain-text body the way a reply should, prefixing every line with `> `.
///
/// Every line, including the blank ones. A quote whose empty lines are unprefixed splits into
/// separate blocks in most clients, so a two-paragraph quote arrives looking like two quotes
/// with the reply's own text apparently between them.
pub fn quote_text(body: &str) -> String {
    body.lines()
        .map(|line| {
            if line.is_empty() {
                ">".to_string()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// Builds the RFC 5322 bytes for a draft.
pub fn build(draft: &Draft) -> Result<Built, BuildError> {
    if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
        return Err(BuildError::NoRecipients);
    }

    let message_id = match draft.message_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => generate_message_id(&draft.from),
    };

    let mut builder = Message::builder()
        .from(mailbox("From", &draft.from)?)
        .message_id(Some(message_id.clone()))
        .subject(draft.subject.trim());

    for address in &draft.to {
        builder = builder.to(mailbox("To", address)?);
    }
    for address in &draft.cc {
        builder = builder.cc(mailbox("Cc", address)?);
    }
    // Present on the builder so the transport knows where to deliver, and stripped from the
    // headers by lettre before transmission — which is the whole contract of Bcc.
    for address in &draft.bcc {
        builder = builder.bcc(mailbox("Bcc", address)?);
    }

    // Threading. `In-Reply-To` is the immediate parent and `References` is the whole chain;
    // Outlook in particular refuses to thread when the two disagree.
    if let Some(parent) = draft.in_reply_to.as_deref().map(str::trim) {
        if !parent.is_empty() {
            builder = builder.in_reply_to(parent.to_string());
        }
    }
    if !draft.references.is_empty() {
        builder = builder.references(draft.references.join(" "));
    }

    // Identifies the client in a way a postmaster can act on, and nothing more. Standing rule
    // 16 rules out anything that identifies the *user* — no build id, no machine name, no
    // install id, which would turn a courtesy header into a tracking one.
    builder = builder.user_agent(format!("Halcyon/{}", env!("CARGO_PKG_VERSION")));

    let text = match draft.text.as_deref() {
        Some(text) if !text.trim().is_empty() => text.to_string(),
        _ => text_from_html(&draft.html),
    };

    // Plain first, HTML second. RFC 2046 §5.1.4: the last part is the richest, and clients
    // that display the first part they understand depend on that order.
    let body = MultiPart::alternative()
        .singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .header(header::ContentTransferEncoding::QuotedPrintable)
                .body(text),
        )
        .singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_HTML)
                .header(header::ContentTransferEncoding::QuotedPrintable)
                .body(draft.html.clone()),
        );

    let message = builder
        .multipart(body)
        .map_err(|error| BuildError::Assembly(error.to_string()))?;

    Ok(Built {
        bytes: message.formatted(),
        message_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(name: Option<&str>, email: &str) -> Address {
        Address {
            name: name.map(str::to_string),
            email: email.to_string(),
        }
    }

    fn draft() -> Draft {
        Draft {
            from: address(Some("Vishal Singh"), "me@halcyon.test"),
            to: vec![address(Some("Ada Lovelace"), "ada@example.test")],
            subject: "Re: The quarterly figures".into(),
            html: "<p>Yes, that works.</p>".into(),
            ..Draft::default()
        }
    }

    fn rendered(draft: &Draft) -> String {
        String::from_utf8_lossy(&build(draft).expect("build").bytes).to_string()
    }

    #[test]
    fn a_message_carries_both_a_text_and_an_html_part() {
        // Not for old clients: for the ones that strip HTML by policy, for screen readers, and
        // because a message with no text part scores as spam with more than one filter.
        let output = rendered(&draft());

        assert!(output.contains("multipart/alternative"), "{output}");
        assert!(output.contains("text/plain"), "{output}");
        assert!(output.contains("text/html"), "{output}");
    }

    #[test]
    fn the_plain_part_comes_before_the_html_one() {
        // RFC 2046 §5.1.4 — the last part is the richest. Reversed, a client that shows the
        // first part it understands would show HTML source to someone who asked for text.
        let output = rendered(&draft());

        let plain = output.find("text/plain").expect("plain part");
        let html = output.find("text/html").expect("html part");

        assert!(plain < html, "the HTML part came first");
    }

    #[test]
    fn a_reply_carries_both_threading_headers() {
        // Outlook refuses to thread when In-Reply-To and References disagree, and a reply that
        // starts a new conversation on the recipient's screen is the most visible way a mail
        // client looks amateur.
        let mut draft = draft();
        draft.in_reply_to = Some("<parent@example.test>".into());
        draft.references = vec!["<root@example.test>".into(), "<parent@example.test>".into()];

        let output = rendered(&draft);

        assert!(
            output.contains("In-Reply-To: <parent@example.test>"),
            "{output}"
        );
        assert!(
            output.contains("References: <root@example.test> <parent@example.test>"),
            "{output}"
        );
    }

    #[test]
    fn bcc_never_appears_in_the_headers() {
        // The entire contract of the field. A Bcc that ships in the headers tells every
        // recipient who was secretly copied.
        let mut draft = draft();
        draft.bcc = vec![address(None, "secret@example.test")];

        let output = rendered(&draft);

        assert!(
            !output.to_ascii_lowercase().contains("bcc:"),
            "a Bcc header was transmitted: {output}"
        );
        assert!(
            !output.contains("secret@example.test"),
            "a blind recipient's address was transmitted: {output}"
        );
    }

    #[test]
    fn a_message_with_no_recipients_is_refused() {
        let mut draft = draft();
        draft.to.clear();

        assert!(matches!(build(&draft), Err(BuildError::NoRecipients)));
    }

    #[test]
    fn an_unusable_address_is_reported_with_the_field_it_came_from() {
        // "Invalid address" alone means opening three token fields to find which.
        let mut draft = draft();
        draft.cc = vec![address(None, "not an address")];

        match build(&draft) {
            Err(BuildError::Address { field, value }) => {
                assert_eq!(field, "Cc");
                assert_eq!(value, "not an address");
            }
            other => panic!("expected an address error, got {other:?}"),
        }
    }

    #[test]
    fn a_display_name_with_an_accent_survives() {
        // Encoded per RFC 2047 rather than sent as raw bytes. Written by hand this is where
        // names turn into mojibake, and the sender never finds out.
        let mut draft = draft();
        draft.from = address(Some("Zoë Naïve"), "zoe@halcyon.test");

        let output = rendered(&draft);

        assert!(
            output.contains("=?utf-8?") || output.contains("=?UTF-8?"),
            "the name was not encoded: {output}"
        );
        assert!(
            !output.contains("Zoë Naïve"),
            "a non-ASCII name went out raw: {output}"
        );
    }

    #[test]
    fn the_user_agent_identifies_the_client_and_not_the_user() {
        // Standing rule 16. A build id or a machine name here turns a courtesy header into a
        // tracking one that follows the user to every recipient.
        let output = rendered(&draft());

        assert!(output.contains("Halcyon/"), "{output}");
        assert!(!output.to_ascii_lowercase().contains("windows"));
        assert!(!output.contains(env!("CARGO_PKG_NAME").to_uppercase().as_str()));
    }

    /* ------------------------------------------------------------- the text alternative */

    #[test]
    fn html_becomes_readable_plain_text() {
        let text = text_from_html("<p>First para.</p><p>Second para.</p>");

        assert!(text.contains("First para."));
        assert!(text.contains("Second para."));
        assert!(
            !text.contains('<'),
            "markup leaked into the text part: {text:?}"
        );
    }

    #[test]
    fn block_elements_become_line_breaks_rather_than_one_run_on_line() {
        let text = text_from_html("<div>One</div><div>Two</div><br>Three");

        assert!(text.contains("One"), "{text:?}");
        assert!(text.contains("Two"), "{text:?}");
        assert!(
            text.lines().count() >= 3,
            "everything ran together: {text:?}"
        );
    }

    #[test]
    fn entities_are_decoded_in_the_right_order() {
        // `&amp;lt;` must become `&lt;`, not `<`. Decoding `&amp;` first corrupts every other
        // entity in the message.
        assert_eq!(
            text_from_html("<p>&amp;lt; &amp; &lt;b&gt;</p>"),
            "&lt; & <b>"
        );
    }

    #[test]
    fn the_supplied_text_is_preferred_over_anything_derived() {
        // The editor knows what the user typed, which always beats what can be recovered from
        // markup afterwards.
        let mut draft = draft();
        draft.text = Some("The exact words typed.".into());

        assert!(rendered(&draft).contains("The exact words typed"));
    }

    /* -------------------------------------------------------------------- quoting */

    #[test]
    fn quoting_prefixes_every_line_including_the_blank_ones() {
        // A quote whose blank lines are unprefixed splits into separate blocks in most
        // clients, so a two-paragraph quote arrives as two quotes with the reply apparently
        // in between.
        let quoted = quote_text("First.\n\nSecond.");

        assert_eq!(quoted, "> First.\r\n>\r\n> Second.");
    }
}
