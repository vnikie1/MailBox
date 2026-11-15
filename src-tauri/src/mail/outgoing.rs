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

/// A fresh `Message-ID` for this sender, for a draft that could not be built yet.
pub fn new_message_id(from: &Address) -> String {
    generate_message_id(from)
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
/// A message id in the form the headers require: `<id@host>`.
///
/// RFC 5322 §3.6.4 defines `msg-id` with the angle brackets as part of it, and `In-Reply-To` and
/// `References` are lists of `msg-id`. They are not decoration.
///
/// They go missing because the store keeps the bare value — `message_id` in the `message` table
/// is stripped on the way in, which is right for a database column and wrong for a header — and a
/// reply is assembled from stored values. `Message-ID` escaped the problem only because lettre
/// brackets that one itself.
///
/// A message went out with `In-Reply-To: AEBD9264-...@icloud.com` and no brackets. Lenient
/// clients thread it anyway, which is why this survived; a strict one starts a new conversation,
/// and docs/06 Phase 7 names Outlook as the strict one.
fn bracketed(id: &str) -> String {
    let trimmed = id.trim();

    if trimmed.starts_with('<') && trimmed.ends_with('>') {
        return trimmed.to_string();
    }

    format!(
        "<{}>",
        trimmed.trim_start_matches('<').trim_end_matches('>')
    )
}

/// Whether a class name is one of this app's own, and therefore meaningless in a mail message.
///
/// The CSS-module convention is `_name_hash_line`: `_paragraph_j5qtj_34`. The hash changes when
/// the stylesheet changes, so these names differ between builds and refer to a stylesheet the
/// recipient will never have.
fn is_internal_class(token: &str) -> bool {
    token.starts_with('_')
        && token.matches('_').count() >= 3
        && token
            .rsplit('_')
            .next()
            .is_some_and(|last| !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()))
}

/// Removes this app's own class names, keeping any the user's own content brought with it.
fn strip_internal_classes(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(at) = rest.find(" class=\"") {
        let (before, from_attr) = rest.split_at(at);
        let value_start = &from_attr[8..];

        let Some(end) = value_start.find('"') else {
            break; // an unterminated attribute; leave the remainder alone
        };

        let kept: Vec<&str> = value_start[..end]
            .split_whitespace()
            .filter(|token| !is_internal_class(token))
            .collect();

        out.push_str(before);
        if !kept.is_empty() {
            out.push_str(" class=\"");
            out.push_str(&kept.join(" "));
            out.push('"');
        }

        rest = &value_start[end + 1..];
    }

    out.push_str(rest);
    out
}

/// The editor's fragment, made into a whole HTML document.
///
/// ## Why this is not cosmetic
///
/// What the editor hands over is a run of `<p>` elements: no `<html>`, no `<head>`, no charset,
/// and every element tagged with a CSS-module class from this app's own stylesheet. Every other
/// mail client sends a complete document, and a bare fragment carrying machine-generated class
/// names is a shape that spam filters recognise, because it is the shape bulk mail arrives in.
///
/// A message sent this way was refused outright by iCloud —
/// `554 5.7.1 [CS01] Message rejected due to local policy` — while the same envelope reached
/// Outlook. That is not proof this was the cause, and it is not claimed as proof: Apple does not
/// say what its filter objected to. But the markup was wrong on its own terms before any of that,
/// leaking build-specific class names into other people's mailboxes, and a mail client that emits
/// a document fragment where a document belongs has a defect regardless of who accepts it.
///
/// A fragment that already carries `<html` is passed through, since it came from somewhere that
/// had already made a document of it.
fn document_html(fragment: &str) -> String {
    let cleaned = strip_internal_classes(fragment);

    if cleaned.to_ascii_lowercase().contains("<html") {
        return cleaned;
    }

    format!(
        "<!DOCTYPE html>
<html>
<head><meta charset=\"utf-8\"></head>
<body>{cleaned}</body>
</html>"
    )
}

/// The editor supplies the real thing — it knows what the user actually typed, which is always
/// a better plain-text version than anything recovered from markup afterwards.
pub fn text_from_html(html: &str) -> String {
    html_to_text(html, false)
}

/// The same, with every line inside a `<blockquote>` marked `> `, once per level.
///
/// For the text part of a reply. The editor's own text has the quoted message's words and not its
/// shape, so a reply's text part had the history run straight on from the answer with nothing to
/// say where one ended (L11). Not for incoming mail's derived text, which is a preview and a
/// search index, where the markers would only be noise.
pub fn quoted_text_from_html(html: &str) -> String {
    html_to_text(html, true)
}

fn html_to_text(html: &str, mark_quotes: bool) -> String {
    // Lines, each with how deep in quotes it began.
    let mut lines: Vec<(usize, String)> = vec![(0, String::new())];
    let mut depth = 0usize;
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
                let closing = tag.starts_with('/');
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
                    if name == "blockquote" {
                        depth = if closing {
                            depth.saturating_sub(1)
                        } else {
                            depth + 1
                        };
                    }
                    lines.push((depth, String::new()));
                }
            }
            _ if inside_tag => tag.push(character),
            _ => {
                if let Some((line_depth, line)) = lines.last_mut() {
                    // A line belongs to the quote it starts in.
                    if line.is_empty() {
                        *line_depth = depth;
                    }
                    line.push(character);
                }
            }
        }
    }

    let decode = |text: &str| {
        text.replace("&nbsp;", " ")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            // Last, or it would corrupt the entities above.
            .replace("&amp;", "&")
    };

    // Collapse the runs of blank lines the block-tag rule produces: one blank line at most
    // between two lines of text, and in a quote, a blank line of the quote's own.
    let mut out: Vec<(bool, String)> = Vec::new();
    let mut blank = false;
    for (line_depth, raw) in &lines {
        let decoded = decode(raw);
        let text = decoded.trim_end();
        let line_depth = if mark_quotes { *line_depth } else { 0 };

        if text.trim().is_empty() {
            if blank {
                continue;
            }
            blank = true;
            out.push((true, ">".repeat(line_depth)));
        } else {
            blank = false;
            if line_depth == 0 {
                out.push((false, text.to_string()));
            } else {
                out.push((
                    false,
                    format!("{} {}", ">".repeat(line_depth), text.trim_start()),
                ));
            }
        }
    }

    // No blank lines at either end, marked or not.
    let first = out
        .iter()
        .position(|(blank, _)| !blank)
        .unwrap_or(out.len());
    let last = out
        .iter()
        .rposition(|(blank, _)| !blank)
        .map_or(first, |at| at + 1);

    out[first..last.max(first)]
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<Vec<_>>()
        .join("\r\n")
        .trim()
        .to_string()
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

/// Builds the RFC 5322 bytes for a message to send.
pub fn build(draft: &Draft) -> Result<Built, BuildError> {
    if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
        return Err(BuildError::NoRecipients);
    }

    build_as(draft, Purpose::Send)
}

/// Builds the bytes of a draft to file in the Drafts mailbox.
///
/// Two differences from [`build`], both because a draft is filed rather than sent:
///
/// * **No recipient is needed.** People write the body first. A draft that could not be built
///   until someone was in To was kept only as a local row nothing read, so Save as Draft on a
///   message with no recipient yet simply lost it (C3, 2026-11-14 audit).
/// * **`Bcc` is kept.** A draft is the writer's own copy. A sent message must never carry its
///   blind copies; a draft that dropped them lost them when it was reopened.
pub fn build_draft(draft: &Draft) -> Result<Built, BuildError> {
    build_as(draft, Purpose::Draft)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Send,
    Draft,
}

fn build_as(draft: &Draft, purpose: Purpose) -> Result<Built, BuildError> {
    let message_id = match draft.message_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => generate_message_id(&draft.from),
    };

    let from = mailbox("From", &draft.from)?;

    let mut builder = Message::builder()
        .from(from.clone())
        .message_id(Some(message_id.clone()))
        .subject(draft.subject.trim());

    if purpose == Purpose::Draft {
        // The envelope is lettre's delivery list, and it refuses an empty one. A draft is never
        // delivered, so it is addressed to its own writer and the envelope goes no further.
        let own = lettre::address::Envelope::new(Some(from.email.clone()), vec![from.email])
            .map_err(|error| BuildError::Assembly(error.to_string()))?;
        builder = builder.envelope(own).keep_bcc();
    }

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
            builder = builder.in_reply_to(bracketed(parent));
        }
    }
    if !draft.references.is_empty() {
        let chain: Vec<String> = draft.references.iter().map(|id| bracketed(id)).collect();

        builder = builder.references(chain.join(" "));
    }

    // Identifies the client in a way a postmaster can act on, and nothing more. Standing rule
    // 16 rules out anything that identifies the *user* — no build id, no machine name, no
    // install id, which would turn a courtesy header into a tracking one.
    builder = builder.user_agent(format!("Halcyon/{}", env!("CARGO_PKG_VERSION")));

    // The editor's text, unless the message quotes another: that text has the quote's words but
    // not its markers, so the history ran straight on from the answer (L11).
    let quotes = draft.html.to_ascii_lowercase().contains("<blockquote");
    let text = match draft.text.as_deref() {
        Some(text) if !text.trim().is_empty() && !quotes => text.to_string(),
        _ => quoted_text_from_html(&draft.html),
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
                .body(document_html(&draft.html)),
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
            let file = lettre::message::Attachment::new(crate::platform::files::safe_file_name(
                &attachment.filename,
            ));

            mixed = mixed.singlepart(if attachment.mime.eq_ignore_ascii_case("message/rfc822") {
                file.body(attached_message(&attachment.bytes), content_type)
            } else {
                file.body(attachment.bytes.clone(), content_type)
            });
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

/// A message attached whole, the way RFC 2046 §5.2.1 allows: 7bit or 8bit, with CRLF line ends.
///
/// Forward as Attachment went out as quoted-printable or base64, whichever lettre judged best
/// for the bytes. Neither is permitted for `message/rfc822`, and some clients show such a part
/// as a file of encoded text rather than as a message. And the cached source was attached with
/// the line ends it was stored with, bare LF included, which SMTP does not carry (L8).
///
/// A source neither form can carry, raw 8-bit that is not UTF-8 or a line past 998 characters,
/// keeps lettre's choice, its line ends made CRLF first. Better a part some clients show oddly
/// than a message that fails to send.
fn attached_message(bytes: &[u8]) -> lettre::message::Body {
    use lettre::message::{header::ContentTransferEncoding, Body};

    let mut crlf = Vec::with_capacity(bytes.len() + bytes.len() / 40);
    for (at, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' && (at == 0 || bytes[at - 1] != b'\r') {
            crlf.push(b'\r');
        }
        crlf.push(*byte);
    }

    let as_body = |encoding| match String::from_utf8(crlf.clone()) {
        Ok(text) => Body::new_with_encoding(text, encoding),
        Err(_) => Body::new_with_encoding(crlf.clone(), encoding),
    };

    as_body(ContentTransferEncoding::SevenBit)
        .or_else(|_| as_body(ContentTransferEncoding::EightBit))
        .unwrap_or_else(|_| Body::new(crlf))
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

/// A display name as a header value: quoted when it is ASCII, RFC 2047 encoded when it is not.
///
/// `mailbox()` hands names to lettre, which does this, and its comment says exactly why it
/// matters: written by hand is "where names turn into mojibake, and the sender never finds
/// out". The redirect block below *is* written by hand, and did not do it — so a redirect from
/// or to anybody with an accent in their name put raw 8-bit bytes into a header RFC 5322
/// requires to be ASCII. Strict servers reject that outright and lenient ones garble it.
fn header_name(name: &str) -> String {
    if name.is_ascii() {
        // Quoted, and any quote or backslash inside escaped. A display name is arbitrary text
        // from an arbitrary sender, and an unescaped one can close the quoting early and inject
        // a second address into the header — which for a redirect would mean silently sending
        // someone's mail somewhere they never named.
        let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
        return format!("\"{escaped}\"");
    }

    // RFC 2047 §2 caps an encoded-word at 75 characters including its delimiters. `=?UTF-8?B?`
    // and `?=` cost 12, and base64 turns three bytes into four characters, so 45 bytes of input
    // per word stays inside the limit. Split on character boundaries, never mid-character, or
    // the decoder is handed half a codepoint.
    //
    // Adjacent encoded-words separated by whitespace are rejoined by the decoder, so a long
    // name arrives whole.
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;

    let mut words: Vec<String> = Vec::new();
    let mut chunk = String::new();

    for character in name.chars() {
        if chunk.len() + character.len_utf8() > 45 {
            words.push(format!("=?UTF-8?B?{}?=", engine.encode(&chunk)));
            chunk.clear();
        }
        chunk.push(character);
    }

    if !chunk.is_empty() {
        words.push(format!("=?UTF-8?B?{}?=", engine.encode(&chunk)));
    }

    words.join(" ")
}

/// Formats an address list for a header.
fn header_list(addresses: &[Address]) -> String {
    addresses
        .iter()
        .map(|address| match address.name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => {
                format!("{} <{}>", header_name(name), address.email.trim())
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

    // CRLF, not LF. Every one of these five lines used to end with a bare newline, which is
    // what a literal line break inside a Rust string literal produces -- easy to write and
    // invisible on screen, because the source looks exactly like the intended output.
    //
    // RFC 5322 §2.1 makes CRLF the line separator, and this block is *prepended* to the
    // original message, whose own headers are already CRLF. The result was a header section
    // that changed line ending partway down: strict servers reject it outright, and a parser
    // that resynchronises on CRLF can run the Resent- block together with the first original
    // header. For a redirect that is the worst possible place for it, because the headers that
    // got mangled are the ones naming who the message is now going to.
    let mut block = String::new();
    block.push_str(&format!(
        "Resent-From: {}\r\n",
        header_list(std::slice::from_ref(&request.from))
    ));

    if !request.to.is_empty() {
        block.push_str(&format!("Resent-To: {}\r\n", header_list(&request.to)));
    }
    if !request.cc.is_empty() {
        block.push_str(&format!("Resent-Cc: {}\r\n", header_list(&request.cc)));
    }

    block.push_str(&format!("Resent-Date: {}\r\n", request.date.trim()));
    block.push_str(&format!("Resent-Message-ID: {resent_id}\r\n"));

    // The original, less any blind copies it records. A Sent copy written by Gmail's web client
    // or most other clients keeps its `Bcc:` — it is the sender's own record — and passing it on
    // untouched told the new recipient everyone the user had blind-copied (M15, 2026-11-14
    // audit). The one change a redirect makes to the original, and the reason it is allowed.
    let original = without_headers(request.original, &["bcc", "resent-bcc"]);

    let mut bytes = Vec::with_capacity(block.len() + original.len());
    bytes.extend_from_slice(block.as_bytes());
    bytes.extend_from_slice(&original);

    // The message keeps its original `Message-ID`, which is what makes it the same message
    // rather than a copy. But the outbox knows this hop by its own `Resent-Message-ID`. Keyed on
    // the original's, a redirect interrupted by a crash was taken for sent if the original was
    // in Sent already, as it is for a redirect of the user's own message or one passed on once
    // before (L9). The sender looks for either header.

    Ok(Built {
        bytes,
        message_id: resent_id,
    })
}

/// The message with its date moved to `date`: `Resent-Date` if it has one, `Date` otherwise.
///
/// Called by the sender as a message leaves, so the date is when it went rather than when it was
/// built. Send Later sent the date it was written, hours or days earlier, and recipients file by
/// it (M12, 2026-11-14 audit). A redirect's `Date` belongs to the original and stays; the hop's
/// own time is `Resent-Date`, and only the topmost one — the hop being sent — is moved.
pub fn restamp_date(raw: &[u8], date: &str) -> Vec<u8> {
    let field = if header_present(raw, "resent-date") {
        "Resent-Date"
    } else {
        "Date"
    };

    let replacement = format!("{field}: {date}\r\n");
    let wanted = field.to_ascii_lowercase();

    let mut out = Vec::with_capacity(raw.len() + replacement.len());
    let mut at = 0usize;
    let mut state = Restamp::Looking;

    while at < raw.len() {
        let end = raw[at..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(raw.len(), |offset| at + offset + 1);
        let line = &raw[at..end];

        if line == b"\r\n" || line == b"\n" {
            out.extend_from_slice(&raw[at..]);
            return out;
        }

        let continuation = matches!(line.first(), Some(b' ' | b'\t'));

        if !continuation {
            if state == Restamp::Replacing {
                state = Restamp::Done;
            }
            if state == Restamp::Looking && field_name(line).as_deref() == Some(wanted.as_str()) {
                out.extend_from_slice(replacement.as_bytes());
                state = Restamp::Replacing;
            }
        }

        if state != Restamp::Replacing {
            out.extend_from_slice(line);
        }

        at = end;
    }

    out
}

#[derive(PartialEq, Eq)]
enum Restamp {
    Looking,
    Replacing,
    Done,
}

/// A header line's field name, lower-cased.
fn field_name(line: &[u8]) -> Option<String> {
    line.iter().position(|&byte| byte == b':').map(|colon| {
        String::from_utf8_lossy(&line[..colon])
            .trim()
            .to_ascii_lowercase()
    })
}

/// Whether the header section has a field of this (lower-case) name.
fn header_present(raw: &[u8], name: &str) -> bool {
    raw.split(|&byte| byte == b'\n')
        .take_while(|line| *line != b"\r" && !line.is_empty())
        .filter(|line| !matches!(line.first(), Some(b' ' | b'\t')))
        .any(|line| field_name(line).as_deref() == Some(name))
}

/// The raw message without the named header fields, continuation lines and all.
///
/// Only the header section is looked at; the body is copied as it is, byte for byte. Names are
/// compared without regard to case, as RFC 5322 has them. A field that is not removed keeps its
/// exact bytes, line endings included.
pub(crate) fn without_headers(raw: &[u8], names: &[&str]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0usize;
    let mut skipping = false;

    while at < raw.len() {
        let end = raw[at..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(raw.len(), |offset| at + offset + 1);
        let line = &raw[at..end];

        // The blank line ends the headers; everything from it on is copied whole.
        if line == b"\r\n" || line == b"\n" {
            out.extend_from_slice(&raw[at..]);
            return out;
        }

        let continuation = matches!(line.first(), Some(b' ' | b'\t'));
        if !continuation {
            let name = line.iter().position(|&byte| byte == b':').map(|colon| {
                String::from_utf8_lossy(&line[..colon])
                    .trim()
                    .to_ascii_lowercase()
            });
            skipping = name.is_some_and(|name| names.iter().any(|wanted| *wanted == name));
        }

        if !skipping {
            out.extend_from_slice(line);
        }

        at = end;
    }

    out
}

#[cfg(test)]
mod threading_header_tests {
    use super::*;

    #[test]
    fn a_bare_stored_id_gets_the_brackets_the_header_requires() {
        // The store keeps message ids bare, which is right for a column and wrong for a header.
        assert_eq!(bracketed("abc@example.test"), "<abc@example.test>");
    }

    #[test]
    fn an_id_that_already_has_them_is_left_alone() {
        assert_eq!(bracketed("<abc@example.test>"), "<abc@example.test>");
    }

    #[test]
    fn whitespace_and_a_half_bracketed_id_are_both_repaired() {
        assert_eq!(bracketed("  abc@example.test "), "<abc@example.test>");
        assert_eq!(bracketed("<abc@example.test"), "<abc@example.test>");
    }

    #[test]
    fn a_reply_carries_a_bracketed_parent_and_chain() {
        // The end-to-end shape. A real reply went out with neither header bracketed, and lenient
        // clients threaded it anyway -- which is exactly why nobody noticed.
        let draft = Draft {
            from: Address {
                name: None,
                email: "me@example.test".into(),
            },
            to: vec![Address {
                name: None,
                email: "you@example.test".into(),
            }],
            subject: "Re: hello".into(),
            html: "<p>hi</p>".into(),
            in_reply_to: Some("parent@example.test".into()),
            references: vec!["root@example.test".into(), "parent@example.test".into()],
            ..Draft::default()
        };

        let built = build(&draft).expect("build");
        let raw = String::from_utf8_lossy(&built.bytes).to_string();

        // Headers fold across lines; unfold before matching, or a long References looks
        // truncated. CRLF, because that is what the builder emits -- unfolding on a bare LF
        // leaves the carriage return sitting inside the header value, so a folded header still
        // fails to match and the test reports the wrong thing about why.
        let unfolded = raw.replace("\r\n ", " ").replace("\r\n\t", " ");

        assert!(
            unfolded.contains("In-Reply-To: <parent@example.test>"),
            "In-Reply-To is not bracketed: {unfolded}"
        );
        assert!(
            unfolded.contains("References: <root@example.test> <parent@example.test>"),
            "the References chain is not bracketed, or not in order: {unfolded}"
        );
    }
}

#[cfg(test)]
mod html_document_tests {
    use super::*;

    #[test]
    fn the_apps_own_class_names_do_not_leave_the_machine() {
        // These are CSS-module names from this app's stylesheet. The hash changes between builds,
        // and the stylesheet is not something the recipient has, so they are noise that also
        // happens to advertise how the message was made.
        let out = document_html("<p class=\"_paragraph_j5qtj_34\">Hello</p>");

        assert!(
            !out.contains("_paragraph_j5qtj_34"),
            "an internal class survived: {out}"
        );
        assert!(out.contains("Hello"));
        assert!(
            !out.contains("class="),
            "the attribute should go when nothing is left in it"
        );
    }

    #[test]
    fn a_class_the_user_brought_with_them_is_kept() {
        // Pasted content carries its own markup, and throwing it away would be a different kind
        // of wrong: the sender chose it, and it means something wherever it came from.
        let out = document_html("<p class=\"signature _bold_ab12_7\">Bye</p>");

        assert!(
            out.contains("class=\"signature\""),
            "the user's class was dropped: {out}"
        );
        assert!(!out.contains("_bold_ab12_7"));
    }

    #[test]
    fn what_goes_on_the_wire_is_a_document_rather_than_a_fragment() {
        let out = document_html("<p>Body</p>");

        for required in [
            "<!DOCTYPE html>",
            "<html>",
            "<head>",
            "charset=\"utf-8\"",
            "<body>",
            "</html>",
        ] {
            assert!(
                out.contains(required),
                "the document is missing {required}: {out}"
            );
        }
    }

    #[test]
    fn something_that_is_already_a_document_is_left_alone() {
        // A forwarded message arrives as a whole document. Wrapping it again would nest one
        // inside another, which renderers handle differently and none handle well.
        let original = "<html><body><p>Forwarded</p></body></html>";
        assert_eq!(document_html(original), original);
    }

    #[test]
    fn an_unterminated_class_attribute_does_not_lose_the_message() {
        // Malformed input should cost the attribute, never the body. The editor should not
        // produce this, but "should not" is not a guarantee about what a paste can contain.
        let out = document_html("<p class=\"broken>Still here</p>");
        assert!(out.contains("Still here"), "the body was lost: {out}");
    }
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

    /// C3 in the 2026-11-14 audit. A draft with nobody in To yet — the usual way a message
    /// starts — could not be built, so it was kept only as a local row nothing ever read, and
    /// Save as Draft lost it. A draft is filed, not sent, and needs no recipient.
    #[test]
    fn a_draft_with_no_recipients_yet_is_built_all_the_same() {
        let mut draft = draft();
        draft.to.clear();
        draft.cc.clear();
        draft.bcc.clear();

        let built = build_draft(&draft).expect("a draft needs no recipient");
        let text = String::from_utf8_lossy(&built.bytes);

        assert!(text.contains("Subject:"), "{text}");
        assert!(!text.contains("\r\nTo:"), "{text}");
        assert!(!built.message_id.is_empty());
    }

    /// A draft is the writer's own copy, so it keeps the blind copies they typed — a sent
    /// message must never carry them, and a saved draft that dropped them lost them on reopen.
    #[test]
    fn a_draft_keeps_its_bcc_and_a_sent_message_does_not() {
        let mut draft = draft();
        draft.bcc = vec![address(None, "secret@example.test")];

        let saved = String::from_utf8_lossy(&build_draft(&draft).expect("draft").bytes).to_string();
        assert!(saved.contains("Bcc: secret@example.test"), "{saved}");

        let sent = String::from_utf8_lossy(&build(&draft).expect("message").bytes).to_string();
        assert!(!sent.contains("secret@example.test"), "{sent}");
    }

    #[test]
    fn two_drafts_started_in_the_same_second_are_two_drafts() {
        // L7: the identity of a draft with no recipients was `draft-<seconds>`, so two windows
        // saving in the same second wrote one draft and each overwrote the other.
        let mut draft = draft();
        draft.to.clear();

        let first = build_draft(&draft).expect("first").message_id;
        let second = build_draft(&draft).expect("second").message_id;
        assert_ne!(first, second);
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
    fn a_replys_text_part_marks_what_it_quotes() {
        // The editor's text has the quoted message's words and not its shape (L11).
        let mut draft = draft();
        draft.html = "<p>Yes, Thursday works.</p><p><br></p>\
                      <p>On 27 Aug 2026, at 09:34, Ada Lovelace &lt;ada@example.test&gt; wrote:</p>\
                      <blockquote type=\"cite\"><p>Can we meet on Thursday?</p><p><br></p>\
                      <p>Or Friday &amp; Monday.</p>\
                      <blockquote><p>Earlier: the figures.</p></blockquote></blockquote>"
            .into();
        draft.text = Some(
            "Yes, Thursday works.\n\nOn 27 Aug 2026, at 09:34, Ada Lovelace <ada@example.test> \
             wrote:\nCan we meet on Thursday?\n\nOr Friday & Monday.\nEarlier: the figures."
                .into(),
        );

        let bytes = build(&draft).expect("build").bytes;
        let parsed = mailparse::parse_mail(&bytes).expect("parse");
        let plain = parsed
            .subparts
            .iter()
            .find(|part| part.ctype.mimetype == "text/plain")
            .expect("a text part")
            .get_body()
            .expect("body");
        let lines: Vec<&str> = plain.lines().map(str::trim_end).collect();

        assert_eq!(
            lines,
            [
                "Yes, Thursday works.",
                "",
                "On 27 Aug 2026, at 09:34, Ada Lovelace <ada@example.test> wrote:",
                "",
                "> Can we meet on Thursday?",
                ">",
                "> Or Friday & Monday.",
                ">",
                ">> Earlier: the figures.",
            ],
            "{plain}"
        );
    }

    #[test]
    fn a_message_that_quotes_nothing_keeps_the_editors_text() {
        let mut draft = draft();
        draft.text = Some("Exactly as typed.".into());

        let bytes = build(&draft).expect("build").bytes;
        let parsed = mailparse::parse_mail(&bytes).expect("parse");
        let plain = parsed.subparts[0].get_body().expect("body");

        assert_eq!(plain.trim(), "Exactly as typed.");
        assert!(!text_from_html("<blockquote>x</blockquote>").contains('>'));
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

    #[test]
    fn a_message_forwarded_as_an_attachment_goes_as_a_message() {
        // It went as quoted-printable or base64, neither allowed for message/rfc822, with the
        // cached copy's bare LF line ends (L8).
        let original = "From: ada@example.test\nSubject: Café receipt\n\nTotal: 12 €\nThanks.\n";
        let draft = draft_with(vec![Attachment {
            filename: "Café receipt.eml".into(),
            mime: "message/rfc822".into(),
            bytes: original.as_bytes().to_vec(),
            content_id: None,
        }]);

        let bytes = build(&draft).expect("build").bytes;
        let parsed = mailparse::parse_mail(&bytes).expect("parse");
        let part = parsed
            .subparts
            .iter()
            .find(|part| part.ctype.mimetype == "message/rfc822")
            .expect("the attached message");

        let encoding = part
            .headers
            .iter()
            .find(|header| {
                header
                    .get_key_ref()
                    .eq_ignore_ascii_case("Content-Transfer-Encoding")
            })
            .map(|header| header.get_value().to_ascii_lowercase());
        assert!(
            matches!(encoding.as_deref(), Some("7bit" | "8bit")),
            "{encoding:?}"
        );

        let inner = String::from_utf8(part.get_body_raw().expect("body")).expect("utf-8");
        assert!(inner.contains("Subject: Café receipt\r\n"), "{inner:?}");
        assert!(
            !inner.replace("\r\n", "").contains('\n'),
            "a bare LF survived"
        );
    }

    #[test]
    fn a_message_no_mail_line_can_carry_is_still_sent() {
        let long = format!("Subject: x\r\n\r\n{}\r\n", "a".repeat(2_000));
        assert!(build(&draft_with(vec![Attachment {
            filename: "long.eml".into(),
            mime: "message/rfc822".into(),
            bytes: long.into_bytes(),
            content_id: None,
        }]))
        .is_ok());
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

    /// M12 in the 2026-11-14 audit: Send Later sent the message with the `Date` it was built
    /// with — when it was written, hours or days before it went. Recipients sort by that date, so
    /// a message scheduled for Monday morning arrived filed under Friday night.
    #[test]
    fn a_message_is_dated_when_it_is_sent() {
        let raw = b"From: me@halcyon.test\r\n\
Date: Fri, 13 Nov 2026 23:10:00 +0000\r\n\
Subject: Monday\r\n\
\r\n\
Date: in the body is not a header.\r\n";

        let out = restamp_date(raw, "Mon, 16 Nov 2026 09:00:00 +0000");
        let text = String::from_utf8_lossy(&out);

        assert!(
            text.contains("Date: Mon, 16 Nov 2026 09:00:00 +0000\r\n"),
            "{text}"
        );
        assert!(!text.contains("Fri, 13 Nov"), "{text}");
        assert!(
            text.ends_with("\r\n\r\nDate: in the body is not a header.\r\n"),
            "{text}"
        );

        // A folded Date is replaced whole.
        let folded = b"Date: Fri, 13 Nov 2026\r\n 23:10:00 +0000\r\nSubject: x\r\n\r\nbody";
        let text = String::from_utf8_lossy(&restamp_date(folded, "NOW")).to_string();
        assert_eq!(text, "Date: NOW\r\nSubject: x\r\n\r\nbody");
    }

    #[test]
    fn a_redirect_keeps_the_original_date_and_restamps_its_own() {
        // The original's `Date` is part of the original, which a redirect passes on unaltered.
        // When *this* hop happened is `Resent-Date`, and that is what a later send moves.
        let raw = b"Resent-From: me@halcyon.test\r\n\
Resent-Date: Fri, 13 Nov 2026 23:10:00 +0000\r\n\
From: ada@example.test\r\n\
Date: Tue, 1 Sep 2026 08:00:00 +0000\r\n\
\r\n\
body";

        let text = String::from_utf8_lossy(&restamp_date(raw, "NOW")).to_string();
        assert!(text.contains("Resent-Date: NOW\r\n"), "{text}");
        assert!(
            text.contains("Date: Tue, 1 Sep 2026 08:00:00 +0000"),
            "{text}"
        );
    }

    /// M15 in the 2026-11-14 audit. A Sent copy written by Gmail's web client, or by most other
    /// clients, keeps its `Bcc:` header — it is the sender's own record — and Redirect passed the
    /// original through untouched, so redirecting one of your own sent messages told the new
    /// recipient everyone you had blind-copied.
    #[test]
    fn a_redirect_never_passes_on_the_originals_blind_copies() {
        let raw = b"From: Me <me@halcyon.test>\r\n\
To: Ada <ada@example.test>\r\n\
Bcc: Secret One <secret1@example.test>,\r\n\
\tsecret2@example.test\r\n\
Resent-Bcc: old-hop@example.test\r\n\
Subject: Plans\r\n\
Message-ID: <plans@halcyon.test>\r\n\
\r\n\
The body may mention Bcc: in passing and keeps it.\r\n"
            .to_vec();

        let built = redirect(&redirect_request(&raw)).expect("redirect");
        let output = String::from_utf8_lossy(&built.bytes);

        assert!(!output.contains("secret1"), "{output}");
        assert!(
            !output.contains("secret2"),
            "the folded line survived: {output}"
        );
        assert!(!output.contains("old-hop"), "{output}");
        assert!(
            output.contains("To: Ada <ada@example.test>\r\nSubject: Plans"),
            "{output}"
        );
        assert!(
            output.ends_with("\r\n\r\nThe body may mention Bcc: in passing and keeps it.\r\n"),
            "{output}"
        );
    }

    #[test]
    fn a_redirect_encodes_a_non_ascii_display_name() {
        // The build path hands names to lettre, which encodes them, and
        // `a_display_name_with_an_accent_survives` proves it. The redirect block is written by
        // hand and was not: an accented name went into a `Resent-` header as raw 8-bit bytes,
        // which RFC 5322 does not allow there. `mailbox()`'s own comment names the outcome —
        // mojibake in the recipient's client, and the sender never finds out.
        let raw = original();
        let mut request = redirect_request(&raw);
        request.from = address(Some("Zoë Naïve"), "zoe@halcyon.test");
        request.to = vec![address(Some("José Álvarez"), "jose@example.test")];

        let built = redirect(&request).expect("redirect");
        let output = String::from_utf8_lossy(&built.bytes).to_string();

        // Only the `Resent-` block is ours; the original below it is kept byte for byte and may
        // legitimately contain anything.
        let block = &output[..output.find("Resent-Message-ID:").expect("id")];

        assert!(
            block.is_ascii(),
            "the resent block carries raw 8-bit bytes:\n{block}"
        );
        assert!(
            block.contains("=?UTF-8?B?"),
            "nothing was encoded:\n{block}"
        );
        assert!(!block.contains("Zoë"), "a raw name survived:\n{block}");
        assert!(!block.contains("Álvarez"), "a raw name survived:\n{block}");

        // The addresses themselves are untouched — only the names are encoded.
        assert!(block.contains("<zoe@halcyon.test>"));
        assert!(block.contains("<jose@example.test>"));
    }

    #[test]
    fn an_ascii_display_name_is_quoted_rather_than_encoded() {
        // Encoding everything would be correct and unreadable. An ASCII name has no reason to
        // arrive as base64 in somebody's mail client.
        assert_eq!(header_name("Grace Hopper"), "\"Grace Hopper\"");
    }

    #[test]
    fn a_display_name_cannot_close_its_own_quoting() {
        // The injection guard, kept from the version this replaced: an unescaped quote would
        // end the quoted string early and let the rest of the name be read as a second
        // address — for a redirect, mail sent somewhere the user never named.
        let encoded = header_name("Grace \" <evil@example.test>, x");

        assert!(encoded.starts_with('"') && encoded.ends_with('"'));
        assert!(
            encoded.contains("\\\""),
            "the quote was not escaped: {encoded}"
        );
    }

    #[test]
    fn a_long_non_ascii_name_is_split_into_encoded_words_within_the_limit() {
        // RFC 2047 caps an encoded-word at 75 characters including its delimiters.
        let long = "Ünnervärderad ".repeat(8);
        let encoded = header_name(&long);

        for word in encoded.split(' ') {
            assert!(
                word.len() <= 75,
                "an encoded word exceeds the RFC 2047 limit at {} chars: {word}",
                word.len()
            );
            assert!(word.starts_with("=?UTF-8?B?") && word.ends_with("?="));
        }

        assert!(encoded.is_ascii());
    }

    #[test]
    fn every_resent_header_ends_with_crlf() {
        // All five of these once ended with a bare LF, because a literal line break inside a
        // Rust string literal produces one and the source looks exactly like the wanted output.
        // RFC 5322 §2.1 requires CRLF, and this block is prepended to an original whose own
        // headers already use it -- so the header section changed line ending partway down.
        let raw = original();
        let mut request = redirect_request(&raw);
        request.cc = vec![address(None, "cc@example.test")];

        let built = redirect(&request).expect("redirect");
        let output = String::from_utf8_lossy(&built.bytes).to_string();

        for header in [
            "Resent-From:",
            "Resent-To:",
            "Resent-Cc:",
            "Resent-Date:",
            "Resent-Message-ID:",
        ] {
            let start = output.find(header).unwrap_or_else(|| panic!("no {header}"));
            let line_end = output[start..]
                .find('\n')
                .unwrap_or_else(|| panic!("{header} has no line ending"));

            assert!(
                output[start..start + line_end].ends_with('\r'),
                "{header} ends with a bare LF:\n{output}"
            );
        }

        // And nowhere in the block a lone LF that is not part of a CRLF.
        let block = &output[..output.find("Resent-Message-ID:").expect("id") + 40];
        assert!(
            !block.replace("\r\n", "").contains('\n'),
            "the resent block contains a bare newline:\n{block}"
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
        assert!(
            output.contains("Message-ID: <original-1@example.test>"),
            "{output}"
        );
        // Known to the outbox by this hop's own id: the original may be in Sent already (L9).
        assert_eq!(built.message_id, "<resent-1@halcyon.test>");
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
