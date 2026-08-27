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
    /// Files to send with the message.
    pub attachments: Vec<Attachment>,
    /// This message's own `Message-ID`. Generated when absent.
    ///
    /// Ours rather than the library's, and that matters for one specific reason: it is how a
    /// send interrupted by a crash is resolved. On restart the outbox looks for this id in the
    /// account's Sent mailbox — present means it went, absent means it did not — which is the
    /// only way to avoid choosing between silently losing a message and silently sending it
    /// twice. See `sync::outbox`.
    pub message_id: Option<String>,
}

/// One file travelling with a message.
#[derive(Debug, Clone)]
pub struct Attachment {
    /// The name the recipient will see. Sanitised on the way out — see below.
    pub filename: String,
    pub mime: String,
    pub bytes: Vec<u8>,
    /// Set when this file is displayed *inside* the message rather than listed beside it.
    ///
    /// The value is the `Content-ID`, without angle brackets, and the HTML refers to it as
    /// `<img src="cid:THAT">`. An image dragged into the body is one of these; a file the user
    /// attached is not. The difference is not cosmetic — an inline image sent as an ordinary
    /// attachment shows up as a paperclip and a broken-image icon in the body.
    pub content_id: Option<String>,
}

/// What most providers will accept in one message.
///
/// 25MB is Gmail's limit and the de facto standard; a few allow more and none of the common
/// ones allow less. Enforced as a warning in the UI rather than a refusal here, because the
/// user may know their own server takes more — but a message that is silently too large comes
/// back as a bounce hours later, addressed to nobody the user recognises.
pub const ATTACHMENT_WARN_BYTES: u64 = 25 * 1024 * 1024;

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

    // With attachments the alternative becomes the first part of a `multipart/mixed`. That
    // nesting is the part clients disagree about least: a flat mixed part containing both a
    // text and an HTML body is read by some clients as two attachments and by others as a
    // message with the HTML shown as a file.
    // Inline images wrap the alternative in a `multipart/related` (RFC 2387). The nesting is
    // load-bearing and the order of the two wrappers is not interchangeable:
    //
    //     multipart/mixed          <- files listed beside the message
    //       multipart/related      <- the message and the images it refers to
    //         multipart/alternative
    //           text/plain
    //           text/html          <- <img src="cid:...">
    //         image/png            <- Content-ID: <...>
    //       application/pdf
    //
    // Put the related part *outside* the mixed one and the ordinary attachments become part of
    // the body's resource set, which Outlook renders as neither an attachment nor an image.
    let inline: Vec<&Attachment> = draft
        .attachments
        .iter()
        .filter(|attachment| attachment.content_id.is_some())
        .collect();

    let separate: Vec<&Attachment> = draft
        .attachments
        .iter()
        .filter(|attachment| attachment.content_id.is_none())
        .collect();

    let body = if inline.is_empty() {
        body
    } else {
        let mut related = MultiPart::related().multipart(body);

        for attachment in &inline {
            let content_type = ContentType::parse(&attachment.mime).unwrap_or_else(|_| {
                ContentType::parse("application/octet-stream").unwrap_or(ContentType::TEXT_PLAIN)
            });

            let cid = attachment.content_id.as_deref().unwrap_or_default();

            related = related.singlepart(
                SinglePart::builder()
                    .header(content_type)
                    // Angle brackets here and none in the `cid:` URL. RFC 2392 is explicit
                    // about the asymmetry, and clients that get it wrong show a broken image.
                    .header(header::ContentId::from(format!("<{cid}>")))
                    .header(header::ContentDisposition::inline())
                    .header(header::ContentTransferEncoding::Base64)
                    .body(attachment.bytes.clone()),
            );
        }

        related
    };

    let body = if separate.is_empty() {
        body
    } else {
        let mut mixed = MultiPart::mixed().multipart(body);

        for attachment in &separate {
            let content_type = ContentType::parse(&attachment.mime).unwrap_or_else(|_| {
                ContentType::parse("application/octet-stream").unwrap_or(ContentType::TEXT_PLAIN)
            });

            // The filename is sanitised even though it is going *out*. It came from the local
            // filesystem, but it lands in the recipient's download folder, and the traversal
            // and right-to-left tricks that matter on the way in matter identically on the way
            // out — this app must not be the thing that sends them.
            mixed = mixed.singlepart(
                lettre::message::Attachment::new(crate::platform::files::safe_file_name(
                    &attachment.filename,
                ))
                .body(attachment.bytes.clone(), content_type),
            );
        }

        mixed
    };

    let message = builder
        .multipart(body)
        .map_err(|error| BuildError::Assembly(error.to_string()))?;

    Ok(Built {
        bytes: message.formatted(),
        message_id,
    })
}

/// A message being passed on unaltered. docs/01 §6, RFC 5322 §3.6.6.
///
/// Not a forward. A forward is a **new** message from the user that quotes the original; a
/// redirect is the original itself, sent on to somebody else, so a reply to it goes back to
/// whoever wrote it rather than to the person who passed it on.
///
/// That distinction only survives if the original bytes survive. Rebuilding the message from
/// the stored HTML would produce something that claims to be from the original sender while
/// being, byte for byte, our reconstruction of it — different encoding, different boundaries,
/// signatures broken, and no way for the recipient to tell. So a redirect works on the cached
/// raw source or it does not happen at all.
pub struct Redirect<'a> {
    /// The original message, exactly as it arrived.
    pub original: &'a [u8],
    /// Who is passing it on.
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    /// Present for the delivery envelope; deliberately never written as a header.
    pub bcc: Vec<Address>,
    /// RFC 5322 date for the `Resent-Date` header.
    pub date: String,
    /// A fresh id for this act of resending. The message keeps its original `Message-ID`.
    pub resent_message_id: Option<String>,
}

/// Formats an address list for a header.
fn header_list(addresses: &[Address]) -> String {
    addresses
        .iter()
        .map(|address| match address.name.as_deref().map(str::trim) {
            // Quoted, and any quote or backslash inside escaped. A display name is arbitrary
            // text from an arbitrary sender, and an unescaped one can close the quoting early
            // and inject a second address into the header — which for a redirect would mean
            // silently sending someone's mail somewhere they never named.
            Some(name) if !name.is_empty() => {
                let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{escaped}\" <{}>", address.email.trim())
            }
            _ => address.email.trim().to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Builds the redirected message: the original, with a `Resent-` block on the front.
///
/// **Prepended, not appended, and that is the specification rather than a shortcut.** RFC 5322
/// §3.6.6 says each new resent block goes at the top, so the most recent hop reads first and
/// the trail of who passed it on stays in order.
///
/// `Resent-Bcc` is never written, for the same reason `Bcc` never is: a blind recipient named
/// in a header is not blind. The blind addresses reach the transport through the delivery
/// envelope instead.
pub fn redirect(request: &Redirect<'_>) -> Result<Built, BuildError> {
    if request.to.is_empty() && request.cc.is_empty() && request.bcc.is_empty() {
        return Err(BuildError::NoRecipients);
    }

    if request.original.is_empty() {
        return Err(BuildError::Assembly(
            "the original message is not in the local cache, so it cannot be redirected              unaltered"
                .into(),
        ));
    }

    // Validated by round-tripping through the same parser the builder uses, so a redirect
    // cannot put an address into a header that an ordinary send would have rejected.
    mailbox("Resent-From", &request.from)?;
    for address in &request.to {
        mailbox("Resent-To", address)?;
    }
    for address in &request.cc {
        mailbox("Resent-Cc", address)?;
    }
    for address in &request.bcc {
        mailbox("Resent-Bcc", address)?;
    }

    let resent_id = match request.resent_message_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => generate_message_id(&request.from),
    };

    let mut block = String::new();
    block.push_str(&format!(
        "Resent-From: {}
",
        header_list(std::slice::from_ref(&request.from))
    ));

    if !request.to.is_empty() {
        block.push_str(&format!(
            "Resent-To: {}
",
            header_list(&request.to)
        ));
    }
    if !request.cc.is_empty() {
        block.push_str(&format!(
            "Resent-Cc: {}
",
            header_list(&request.cc)
        ));
    }

    block.push_str(&format!(
        "Resent-Date: {}
",
        request.date.trim()
    ));
    block.push_str(&format!(
        "Resent-Message-ID: {resent_id}
"
    ));

    let mut bytes = Vec::with_capacity(block.len() + request.original.len());
    bytes.extend_from_slice(block.as_bytes());
    bytes.extend_from_slice(request.original);

    // The message keeps its **original** `Message-ID`, which is what makes it the same message
    // rather than a copy. That is also the id the outbox searches the Sent mailbox for when
    // resolving an interrupted send, and it still works: the search is scoped to Sent, and the
    // original arrived in the Inbox.
    let message_id = original_message_id(request.original).unwrap_or(resent_id);

    Ok(Built { bytes, message_id })
}

/// Reads the `Message-ID` out of a raw message's headers.
///
/// Stops at the blank line. A `Message-ID:` occurring in the body — quoted in a reply, say —
/// is not this message's identity, and treating it as such would have the outbox looking for
/// the wrong message when deciding whether a send completed.
fn original_message_id(raw: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(raw);

    for line in text.lines() {
        if line.trim().is_empty() {
            break;
        }

        if let Some(value) = line
            .strip_prefix("Message-ID:")
            .or_else(|| line.strip_prefix("Message-Id:"))
            .or_else(|| line.strip_prefix("message-id:"))
        {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }

    None
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

#[cfg(test)]
mod attachment_tests {
    use super::*;

    fn draft_with(attachments: Vec<Attachment>) -> Draft {
        Draft {
            from: Address {
                name: Some("Me".into()),
                email: "me@halcyon.test".into(),
            },
            to: vec![Address {
                name: None,
                email: "ada@example.test".into(),
            }],
            subject: "Here it is".into(),
            html: "<p>Attached.</p>".into(),
            attachments,
            ..Draft::default()
        }
    }

    fn rendered(draft: &Draft) -> String {
        String::from_utf8_lossy(&build(draft).expect("build").bytes).to_string()
    }

    fn address(name: Option<&str>, email: &str) -> Address {
        Address {
            name: name.map(str::to_string),
            email: email.to_string(),
        }
    }

    fn draft() -> Draft {
        draft_with(Vec::new())
    }

    #[test]
    fn an_attachment_nests_the_alternative_inside_a_mixed_part() {
        // The nesting clients disagree about least. A flat mixed part holding both bodies is
        // read by some as two attachments and by others as a message whose HTML is a file.
        let output = rendered(&draft_with(vec![Attachment {
            content_id: None,
            filename: "notes.txt".into(),
            mime: "text/plain".into(),
            bytes: b"hello".to_vec(),
        }]));

        let mixed = output.find("multipart/mixed").expect("mixed part");
        let alternative = output
            .find("multipart/alternative")
            .expect("alternative part");

        assert!(
            mixed < alternative,
            "the alternative must be inside the mixed part"
        );
        assert!(output.contains("notes.txt"), "{output}");
    }

    #[test]
    fn a_message_without_attachments_is_not_wrapped_in_a_mixed_part() {
        // An empty mixed wrapper is legal and pointless, and some clients draw a paperclip for
        // it — a message that claims an attachment and has none.
        let output = rendered(&draft_with(Vec::new()));

        assert!(!output.contains("multipart/mixed"), "{output}");
        assert!(output.contains("multipart/alternative"));
    }

    #[test]
    fn an_outgoing_filename_is_sanitised_too() {
        // It came from this machine's filesystem, but it lands in the recipient's download
        // folder. Traversal and right-to-left overrides matter identically on the way out, and
        // this app must not be the thing that sends them.
        let output = rendered(&draft_with(vec![Attachment {
            content_id: None,
            filename: "../../evil\u{202E}fdp.exe".into(),
            mime: "application/octet-stream".into(),
            bytes: b"x".to_vec(),
        }]));

        let disposition = output
            .lines()
            .find(|line| line.starts_with("Content-Disposition:"))
            .expect("a disposition header");

        // What actually matters is that there are no path *separators* left. A literal ".."
        // with nothing to separate is an ordinary, harmless filename component — checking for
        // the substring alone would also match the base64 body and the MIME boundary, which is
        // what the first version of this assertion did.
        assert!(!disposition.contains('/'), "{disposition}");
        assert!(!disposition.contains('\\'), "{disposition}");
        assert!(!disposition.contains('\u{202E}'), "{disposition}");

        // And the real extension is still the last thing in the name, which is the whole point
        // of stripping the override.
        assert!(disposition.contains(".exe\""), "{disposition}");
    }

    #[test]
    fn an_unusable_mime_type_falls_back_rather_than_failing_the_send() {
        // Standing rule 13. A file whose type could not be guessed is still a file the user
        // asked to send, and octet-stream is what every client falls back to anyway.
        let output = rendered(&draft_with(vec![Attachment {
            content_id: None,
            filename: "mystery.bin".into(),
            mime: "not/a/valid/type".into(),
            bytes: b"x".to_vec(),
        }]));

        assert!(output.contains("mystery.bin"), "{output}");
    }

    #[test]
    fn several_attachments_all_travel() {
        let output = rendered(&draft_with(vec![
            Attachment {
                content_id: None,
                filename: "one.txt".into(),
                mime: "text/plain".into(),
                bytes: b"1".to_vec(),
            },
            Attachment {
                content_id: None,
                filename: "two.txt".into(),
                mime: "text/plain".into(),
                bytes: b"2".to_vec(),
            },
        ]));

        assert!(output.contains("one.txt"), "{output}");
        assert!(output.contains("two.txt"), "{output}");
    }

    /* ------------------------------------------------------- inline images and redirect */

    fn image(cid: Option<&str>) -> Attachment {
        Attachment {
            filename: "chart.png".into(),
            mime: "image/png".into(),
            bytes: vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
            content_id: cid.map(str::to_string),
        }
    }

    #[test]
    fn an_inline_image_travels_in_a_related_part_wrapping_the_body() {
        // The nesting is the whole feature. An image sent as an ordinary attachment shows the
        // recipient a paperclip and a broken-image icon where the picture should be.
        let mut draft = draft();
        draft.html = r#"<p>See <img src="cid:chart-1"></p>"#.into();
        draft.attachments = vec![image(Some("chart-1"))];

        let output = rendered(&draft);

        assert!(output.contains("multipart/related"), "{output}");
        assert!(output.contains("Content-ID: <chart-1>"), "{output}");
        assert!(output.contains("multipart/alternative"), "{output}");

        // Angle brackets on the header, none in the URL. RFC 2392 is explicit about the
        // asymmetry and clients that get it wrong show nothing at all.
        assert!(output.contains("cid:chart-1"), "{output}");
        assert!(!output.contains("cid:<chart-1>"), "{output}");
    }

    #[test]
    fn an_inline_image_is_marked_inline_rather_than_as_an_attachment() {
        let mut draft = draft();
        draft.attachments = vec![image(Some("chart-1"))];

        let output = rendered(&draft);
        assert!(output.contains("Content-Disposition: inline"), "{output}");
    }

    #[test]
    fn an_ordinary_attachment_still_travels_in_a_mixed_part() {
        // The regression this guards: adding the related wrapper must not swallow files the
        // user attached, which would leave them displayed nowhere and downloadable nowhere.
        let mut draft = draft();
        draft.attachments = vec![image(None)];

        let output = rendered(&draft);

        assert!(output.contains("multipart/mixed"), "{output}");
        assert!(!output.contains("multipart/related"), "{output}");
        assert!(
            output.contains("Content-Disposition: attachment"),
            "{output}"
        );
    }

    #[test]
    fn a_message_with_both_nests_related_inside_mixed() {
        // Order matters and is not interchangeable. Related outside mixed makes the ordinary
        // attachments part of the body's resource set, which Outlook renders as neither an
        // attachment nor an image.
        let mut draft = draft();
        draft.attachments = vec![image(Some("chart-1")), image(None)];

        let output = rendered(&draft);

        let mixed = output.find("multipart/mixed").expect("mixed part");
        let related = output.find("multipart/related").expect("related part");

        assert!(
            mixed < related,
            "related was not nested inside mixed:\n{output}"
        );
    }

    fn original() -> Vec<u8> {
        concat!(
            "From: Ada Lovelace <ada@example.test>\r\n",
            "To: Vishal Singh <me@halcyon.test>\r\n",
            "Subject: The quarterly figures\r\n",
            "Date: Mon, 25 Aug 2026 09:00:00 +0000\r\n",
            "Message-ID: <original-1@example.test>\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Here they are.\r\n",
        )
        .as_bytes()
        .to_vec()
    }

    fn redirect_request(raw: &[u8]) -> Redirect<'_> {
        Redirect {
            original: raw,
            from: address(Some("Vishal Singh"), "me@halcyon.test"),
            to: vec![address(Some("Grace Hopper"), "grace@example.test")],
            cc: Vec::new(),
            bcc: Vec::new(),
            date: "Thu, 27 Aug 2026 12:00:00 +0000".into(),
            resent_message_id: Some("<resent-1@halcyon.test>".into()),
        }
    }

    #[test]
    fn a_redirect_keeps_the_original_message_byte_for_byte() {
        // This is what separates a redirect from a forward. Rebuilding the message would give
        // the recipient something that claims to be from the original sender while being our
        // reconstruction of it — different encoding, broken signatures, and no way to tell.
        let raw = original();
        let built = redirect(&redirect_request(&raw)).expect("redirect");
        let output = String::from_utf8_lossy(&built.bytes);

        assert!(
            output.ends_with(&String::from_utf8_lossy(&raw).to_string()),
            "the original was altered:\n{output}"
        );
    }

    #[test]
    fn a_redirect_prepends_its_resent_block() {
        // RFC 5322 §3.6.6: each new resent block goes at the *top*, so the most recent hop
        // reads first and the trail stays in order.
        let raw = original();
        let built = redirect(&redirect_request(&raw)).expect("redirect");
        let output = String::from_utf8_lossy(&built.bytes);

        assert!(output.starts_with("Resent-From:"), "{output}");

        let resent = output.find("Resent-From:").expect("resent block");
        let from = output.find("From: Ada").expect("original From");
        assert!(resent < from, "the resent block is not first:\n{output}");

        assert!(
            output.contains("Resent-To: \"Grace Hopper\" <grace@example.test>"),
            "{output}"
        );
        assert!(
            output.contains("Resent-Date: Thu, 27 Aug 2026 12:00:00 +0000"),
            "{output}"
        );
        assert!(
            output.contains("Resent-Message-ID: <resent-1@halcyon.test>"),
            "{output}"
        );
    }

    #[test]
    fn a_redirect_keeps_the_original_authorship() {
        // A reply to a redirected message must go back to whoever wrote it, not to the person
        // who passed it on. That only holds if the original `From` survives untouched.
        let raw = original();
        let built = redirect(&redirect_request(&raw)).expect("redirect");
        let output = String::from_utf8_lossy(&built.bytes);

        assert!(
            output.contains("From: Ada Lovelace <ada@example.test>"),
            "{output}"
        );
        assert_eq!(built.message_id, "<original-1@example.test>");
    }

    #[test]
    fn a_redirect_never_writes_a_resent_bcc_header() {
        // Same reason `Bcc` is never written: a blind recipient named in a header is not blind.
        let raw = original();
        let mut request = redirect_request(&raw);
        request.bcc = vec![address(None, "secret@example.test")];

        let built = redirect(&request).expect("redirect");
        let output = String::from_utf8_lossy(&built.bytes);

        assert!(!output.contains("Resent-Bcc"), "{output}");
        assert!(!output.contains("secret@example.test"), "{output}");
    }

    #[test]
    fn a_redirect_with_no_cached_original_is_refused() {
        // Rather than silently sending a rebuilt approximation that claims to be the original.
        let mut request = redirect_request(&[]);
        request.original = &[];

        assert!(matches!(redirect(&request), Err(BuildError::Assembly(_))));
    }

    #[test]
    fn a_display_name_cannot_inject_a_second_recipient() {
        // A display name is arbitrary text from an arbitrary sender. Unescaped, a quote in it
        // closes the quoting early and everything after is read as another address — which for
        // a redirect means silently sending someone's mail to an address they never named.
        let raw = original();
        let mut request = redirect_request(&raw);
        request.to = vec![address(
            Some("Grace\" <attacker@evil.test>, \"x"),
            "grace@example.test",
        )];

        let built = redirect(&request).expect("redirect");
        let output = String::from_utf8_lossy(&built.bytes);

        let header_line = output
            .lines()
            .find(|line| line.starts_with("Resent-To:"))
            .expect("resent-to");

        assert!(
            !header_line.contains("<attacker@evil.test>")
                || header_line.contains("\\\" <attacker@evil.test>"),
            "an unescaped quote injected an address: {header_line}"
        );
    }

    #[test]
    fn a_redirect_needs_somebody_to_send_to() {
        let raw = original();
        let mut request = redirect_request(&raw);
        request.to = Vec::new();

        assert!(matches!(redirect(&request), Err(BuildError::NoRecipients)));
    }
}
