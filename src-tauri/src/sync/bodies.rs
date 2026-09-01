//! Fetching, caching and parsing message bodies. docs/06 Phase 5 §3.
//!
//! *Lazy body fetch on selection + prefetch of the next 3 rows. Cache `.eml` on disk.*
//!
//! Bodies are fetched on demand rather than during sync for a simple reason of scale: the
//! envelopes for a 50,000-message mailbox are a few tens of megabytes, and the bodies are
//! several gigabytes. Almost none of them will ever be read.
//!
//! **Standing rule 11 governs this file: a message body is hostile input.** Everything here
//! is written to survive a body that is malformed on purpose — truncated MIME, a declared
//! character set that is a lie, a part that claims a size it does not have, nesting designed
//! to blow a stack. Nothing panics, nothing recurses without a bound, and nothing is
//! unwrapped.
//!
//! What this module does **not** do is render anything. The extracted HTML is stored and
//! deliberately not exposed over IPC — sanitising it and putting it in a sandboxed frame is
//! docs/03 §6, which is Phase 6's work. Until then the reader shows the plain-text part, so
//! no untrusted markup can reach the WebView at all.

use std::path::{Path, PathBuf};

use mailparse::{MailHeaderMap, ParsedMail};
use rusqlite::{params, Transaction};

use crate::db::DbError;

use super::session::{ImapSession, SyncError};

/// Largest body we will hold in memory and store.
///
/// Gmail's own `APPENDLIMIT` is 35MB, and a mail server will happily hand over a message
/// with a 200MB attachment. Reading that into a `Vec<u8>` on a background task is how an
/// app gets killed by the OS rather than reporting a problem.
const MAX_BODY_BYTES: usize = 40 * 1024 * 1024;

/// How deep a MIME tree may nest before we stop descending.
///
/// A hand-crafted message can nest `multipart/mixed` thousands of levels deep. `mailparse`
/// has already parsed the structure by the time we walk it, so this bounds *our* recursion,
/// which is the part that would otherwise overflow the stack on a body someone chose.
const MAX_MIME_DEPTH: usize = 32;

/// How much text goes in the list's preview column.
const PREVIEW_CHARS: usize = 300;

/// What was extracted from one message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Body {
    pub text: Option<String>,
    /// Stored, not served. Phase 6 sanitises it; until then nothing sends it to the UI.
    pub html: Option<String>,
    pub preview: String,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub part_id: String,
    pub filename: Option<String>,
    pub mime: String,
    pub size: i64,
    pub content_id: Option<String>,
    /// An image referenced by `cid:` from the HTML part, rather than a file to download.
    pub is_inline: bool,
}

/// Where a message's raw source is cached.
///
/// One file per message under the account, named by message id. Keeping the `.eml` means a
/// reply can quote the original exactly, and Phase 6 can re-render without another round
/// trip to the server.
pub fn cache_path(root: &Path, account_id: i64, message_id: i64) -> PathBuf {
    root.join("bodies")
        .join(account_id.to_string())
        .join(format!("{message_id}.eml"))
}

/// Fetches one message's full source from the server.
pub async fn fetch(session: &mut ImapSession, uid: u32) -> Result<Vec<u8>, SyncError> {
    let raw = super::fetch::body(session, uid).await?;

    if raw.len() > MAX_BODY_BYTES {
        tracing::warn!(
            uid,
            bytes = raw.len(),
            "body exceeds the size cap; not stored"
        );

        return Err(SyncError::Imap(async_imap::error::Error::Bad(format!(
            "message {uid} is larger than the {MAX_BODY_BYTES}-byte cap"
        ))));
    }

    Ok(raw)
}

/// Writes the raw source to the on-disk cache.
pub fn write_cache(
    root: &Path,
    account_id: i64,
    message_id: i64,
    raw: &[u8],
) -> std::io::Result<PathBuf> {
    let path = cache_path(root, account_id, message_id);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&path, raw)?;
    Ok(path)
}

/// Parses a raw message into the parts the app needs.
///
/// Never fails: a body that cannot be parsed at all still yields an empty `Body` rather than
/// an error, because a message the user can see and not read is better than a message that
/// vanished. Standing rule 13.
pub fn parse(raw: &[u8]) -> Body {
    let Ok(parsed) = mailparse::parse_mail(raw) else {
        tracing::debug!("message could not be parsed as MIME; storing nothing");
        return Body::default();
    };

    let mut body = Body::default();
    walk(&parsed, 0, &mut String::new(), &mut body);

    // Prefer the plain-text part. Where a sender supplied only HTML — which marketing mail
    // usually does — a rough text rendering is derived so the list still has a preview and
    // the reader has something to show before Phase 6.
    if body.text.is_none() {
        if let Some(html) = &body.html {
            let derived = text_from_html(html);
            if !derived.trim().is_empty() {
                body.text = Some(derived);
            }
        }
    }

    body.preview = build_preview(body.text.as_deref().unwrap_or_default());
    body
}

/// Walks the MIME tree, depth-bounded.
fn walk(part: &ParsedMail<'_>, depth: usize, path: &mut String, body: &mut Body) {
    if depth > MAX_MIME_DEPTH {
        tracing::debug!("MIME nesting exceeded the depth cap; not descending further");
        return;
    }

    let mime = part.ctype.mimetype.to_ascii_lowercase();

    if part.subparts.is_empty() {
        collect_leaf(part, &mime, path, body);
        return;
    }

    for (index, child) in part.subparts.iter().enumerate() {
        let mut child_path = if path.is_empty() {
            format!("{}", index + 1)
        } else {
            format!("{path}.{}", index + 1)
        };

        walk(child, depth + 1, &mut child_path, body);
    }
}

/// Records one leaf part: body text, HTML, or an attachment.
fn collect_leaf(part: &ParsedMail<'_>, mime: &str, path: &str, body: &mut Body) {
    let disposition = part.get_content_disposition();
    let filename = disposition.params.get("filename").cloned();

    let is_attachment = matches!(
        disposition.disposition,
        mailparse::DispositionType::Attachment
    ) || filename.is_some();

    let content_id = part.headers.get_first_value("Content-ID").map(|id| {
        id.trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .to_string()
    });

    let is_inline = matches!(disposition.disposition, mailparse::DispositionType::Inline)
        || content_id.is_some();

    // An inline image referenced by cid: is an attachment as far as storage is concerned,
    // but must not put a paperclip on the row — every HTML newsletter has a tracking pixel,
    // and a mailbox where every message claims an attachment tells the user nothing.
    if is_attachment || (is_inline && !mime.starts_with("text/")) {
        let size = part
            .get_body_raw()
            .map(|bytes| bytes.len() as i64)
            .unwrap_or(0);

        body.attachments.push(Attachment {
            part_id: path.to_string(),
            filename,
            mime: mime.to_string(),
            size,
            content_id,
            is_inline: is_inline && !is_attachment,
        });

        return;
    }

    // `get_body` applies the declared transfer encoding and character set. A lie in either
    // gives replacement characters rather than an error, which is the right trade: garbled
    // text is readable-ish, a failed parse is a blank message.
    let Ok(text) = part.get_body() else {
        return;
    };

    // Mail is CRLF on the wire; stored text is LF. The reader splits paragraphs on a blank
    // line, and a surviving carriage return renders as a stray character rather than a break.
    let text = text.replace("\r\n", "\n").replace('\r', "\n");

    // The *first* part of each kind wins. A `multipart/alternative` lists parts worst-first
    // by RFC 2046, but nested alternatives and forwarded mail make "last wins" pick text out
    // of a quoted original rather than the message itself.
    match mime {
        "text/plain" if body.text.is_none() => body.text = Some(text),
        "text/html" if body.html.is_none() => body.html = Some(text),
        _ => {}
    }
}

/// A rough plain-text rendering of an HTML part.
///
/// Deliberately crude — it exists so an HTML-only message has *something* in the preview and
/// in the reader before Phase 6 builds the real renderer. It strips tags rather than
/// interpreting them, and it drops `<script>` and `<style>` contents entirely so their source
/// never reaches the list.
fn text_from_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut skip_until: Option<&str> = None;
    let lowered = html.to_ascii_lowercase();
    let bytes = lowered.as_bytes();
    let mut index = 0usize;

    while index < html.len() {
        if let Some(end_tag) = skip_until {
            if lowered[index..].starts_with(end_tag) {
                skip_until = None;
                index += end_tag.len();
                continue;
            }
            index += 1;
            continue;
        }

        let byte = bytes.get(index).copied().unwrap_or(b' ');

        if byte == b'<' {
            // Comments, including Outlook's conditional blocks. Real mail is full of
            // `<!--[if mso]> ... <![endif]-->`, and without this the preview column of a
            // marketing message opens with `<!--[if !mso]><!--> <!--[if` instead of a
            // sentence. Seen in a real Gmail inbox, in three messages out of the first six.
            if lowered[index..].starts_with("<!--") {
                skip_until = Some("-->");
                index += 4;
                continue;
            }

            if lowered[index..].starts_with("<script") {
                skip_until = Some("</script>");
                index += 7;
                continue;
            }
            if lowered[index..].starts_with("<style") {
                skip_until = Some("</style>");
                index += 6;
                continue;
            }

            // Block-level tags become paragraph breaks so the text is not one wall.
            if lowered[index..].starts_with("</p")
                || lowered[index..].starts_with("<br")
                || lowered[index..].starts_with("</div")
                || lowered[index..].starts_with("</tr")
            {
                out.push('\n');
            }

            in_tag = true;
            index += 1;
            continue;
        }

        if byte == b'>' {
            in_tag = false;
            index += 1;
            continue;
        }

        if !in_tag {
            // Index by char boundary, not by byte, or a multi-byte character is split.
            if let Some(character) = html[index..].chars().next() {
                out.push(character);
                index += character.len_utf8();
                continue;
            }
        }

        index += 1;
    }

    decode_entities(&collapse_whitespace(&out))
}

fn collapse_whitespace(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut blank_run = 0usize;

    for line in value.lines() {
        let trimmed = line.split_whitespace().collect::<Vec<_>>().join(" ");

        if trimmed.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }

        out.push_str(&trimmed);
        out.push('\n');
    }

    out.trim().to_string()
}

/// Decodes character entities, named and numeric.
///
/// The numeric forms are not an edge case: newsletters are full of `&#8202;` hair spaces and
/// `&#8217;` typographic apostrophes, and leaving them raw puts `&#8202;` in the middle of a
/// preview line. Seen in a real inbox.
fn decode_entities(value: &str) -> String {
    let named = value
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&hellip;", "…")
        // Invisible padding. Bulk senders put hundreds of these in a row to push their own
        // text past whatever a client shows as a preview, and undecoded they are five visible
        // characters where the sender meant none: a real inbox showed a row reading
        // "Sign up to rider Insurance at Rs 3/trip. &shy; &shy; &shy; &shy;".
        .replace("&shy;", "\u{00ad}")
        .replace("&zwnj;", "\u{200c}")
        .replace("&zwj;", "\u{200d}")
        // Last, or `&amp;lt;` decodes to `<` rather than to the literal `&lt;` the sender
        // wrote — the classic double-decode.
        .replace("&amp;", "&");

    decode_numeric_entities(&named)
}

/// `&#8202;` and `&#x200a;`.
fn decode_numeric_entities(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(start) = rest.find("&#") {
        out.push_str(&rest[..start]);

        let after = &rest[start + 2..];
        let Some(end) = after.find(';') else {
            // No terminator: not an entity, just an ampersand followed by a hash.
            out.push_str(&rest[start..]);
            return out;
        };

        // A `;` a long way off is punctuation, not the end of an entity.
        if end > 8 {
            out.push_str(&rest[start..start + 2]);
            rest = after;
            continue;
        }

        let digits = &after[..end];

        let code = if let Some(hex) = digits.strip_prefix(['x', 'X']) {
            u32::from_str_radix(hex, 16).ok()
        } else {
            digits.parse::<u32>().ok()
        };

        match code.and_then(char::from_u32) {
            Some(character) => out.push(character),
            // Unparseable or not a character: keep the source text rather than dropping it.
            None => out.push_str(&rest[start..start + 2 + end + 1]),
        }

        rest = &after[end + 1..];
    }

    out.push_str(rest);
    out
}

/// Whether a whole line is HTML markup rather than prose.
///
/// Deliberately narrow: it matches a line that *is* a comment or a bare tag, not a line that
/// merely contains an angle bracket. Prose like "true if x < y" and a quoted address like
/// "<ada@example.test>" must survive — over-stripping a preview loses the message.
fn is_markup_residue(line: &str) -> bool {
    if line.starts_with("<!--") || line.starts_with("<![") || line == "-->" {
        return true;
    }

    // A line that is exactly one tag, e.g. "<br>" or "</div>".
    line.starts_with('<') && line.ends_with('>') && !line[1..].contains('<') && line.contains('/')
}

/// Characters that occupy no visible width.
///
/// Hair spaces, zero-width joiners and the like are used as spacers in HTML mail and survive
/// into a text part as a line that looks blank and is not.
fn is_invisible(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            // Soft hyphen. Marketing mail pads with these by the hundred to push the preview
            // text a client shows past whatever the sender wants hidden, and it is invisible
            // in every renderer, so a preview made of them is a blank row.
            '\u{00ad}'
                | '\u{200a}'
                | '\u{200b}'
                | '\u{200c}'
                | '\u{200d}'
                | '\u{2060}'
                | '\u{feff}'
        )
}

/// The list's snippet.
///
/// Quoted lines and signature blocks are skipped: a reply whose preview reads
/// "> On Tuesday, someone wrote:" tells the reader nothing about *this* message.
fn build_preview(text: &str) -> String {
    let mut collected = String::new();

    // Decoded first, so the invisible-character filter below sees a soft hyphen rather than the
    // five characters `&shy;` and drops the line as the padding it is.
    let text = decode_entities(text);

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('>') || trimmed == "--" {
            continue;
        }

        // Markup residue in a *plain-text* part. Not hypothetical and not our bug: Google's
        // own `text/plain` alternative contains `<!--[if !mso]><!-->`, so the preview for a
        // Google security email opened with that instead of a sentence.
        //
        // `body_text` keeps it — that is faithfully what the sender sent, and the reader
        // shows the message as written. The preview is a summary for scanning a list, and a
        // summary made of conditional comments summarises nothing.
        if is_markup_residue(trimmed) {
            continue;
        }

        // A line of nothing but invisible characters. Hair spaces and zero-width joiners are
        // used as spacers in HTML mail and survive into the text part as a line that looks
        // blank and is not.
        if trimmed.chars().all(is_invisible) {
            continue;
        }

        if !collected.is_empty() {
            collected.push(' ');
        }
        collected.push_str(trimmed);

        if collected.chars().count() >= PREVIEW_CHARS {
            break;
        }
    }

    collected.chars().take(PREVIEW_CHARS).collect()
}

/// Writes a parsed body into the store.
pub fn persist(
    tx: &Transaction<'_>,
    message_id: i64,
    body: &Body,
    raw_path: Option<&Path>,
) -> Result<(), DbError> {
    // Only non-inline attachments earn a paperclip. Every HTML newsletter carries a tracking
    // pixel as an inline part, and a mailbox where every row claims an attachment is a
    // mailbox where the paperclip means nothing.
    let has_attachment = body.attachments.iter().any(|a| !a.is_inline);

    tx.execute(
        "UPDATE message
            SET body_text = ?2,
                body_html = ?3,
                preview = ?4,
                body_state = 'full',
                raw_path = ?5,
                has_attachment = ?6
          WHERE id = ?1",
        params![
            message_id,
            body.text,
            body.html,
            body.preview,
            raw_path.map(|path| path.to_string_lossy().to_string()),
            i64::from(has_attachment),
        ],
    )?;

    // Replaced rather than appended: re-fetching a body must not double its attachment list.
    tx.execute(
        "DELETE FROM attachment WHERE message_id = ?1",
        params![message_id],
    )?;

    for attachment in &body.attachments {
        tx.execute(
            "INSERT INTO attachment (message_id, part_id, filename, mime, size, content_id, is_inline)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                message_id,
                attachment.part_id,
                attachment.filename,
                attachment.mime,
                attachment.size,
                attachment.content_id,
                i64::from(attachment.is_inline),
            ],
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod preview_entity_tests {
    use super::*;

    #[test]
    fn invisible_padding_does_not_reach_the_list() {
        // Straight out of a real inbox: Uber's plain-text part, which pads with soft hyphens so
        // that a client showing raw text displays the sender's choice of preview rather than the
        // next sentence. Undecoded, the row read "... &shy; &shy; &shy; &shy;".
        let preview = build_preview(
            "Sign up to rider Insurance at Rs 3/trip.
&shy; &shy; &shy; &shy;
Real second line.",
        );

        assert!(
            !preview.contains("&shy;"),
            "raw entities reached the preview: {preview}"
        );
        assert!(preview.contains("Sign up to rider Insurance"));
        assert!(
            preview.contains("Real second line"),
            "the padding line should be skipped, not the one after it: {preview}"
        );
    }

    #[test]
    fn a_line_of_nothing_but_padding_is_skipped_entirely() {
        let preview = build_preview(
            "&shy;&shy;&shy;
The actual sentence.",
        );
        assert!(preview.starts_with("The actual sentence"), "got: {preview}");
    }

    #[test]
    fn an_ampersand_in_prose_survives() {
        // The failure mode of an over-eager decoder: "Marks & Spencer" is not an entity, and
        // neither is "R&D". Dropping the ampersand would corrupt somebody's mail.
        let preview = build_preview("Marks & Spencer and R&D spending");
        assert!(preview.contains("Marks & Spencer"), "got: {preview}");
        assert!(preview.contains("R&D"), "got: {preview}");
    }

    #[test]
    fn the_soft_hyphen_itself_counts_as_invisible() {
        assert!(
            is_invisible('\u{00ad}'),
            "a decoded soft hyphen must count as invisible"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: &[u8] = b"From: ada@example.test\r\n\
Subject: plain\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hello there.\r\n\
\r\n\
Second paragraph.\r\n";

    const ALTERNATIVE: &[u8] = b"From: ada@example.test\r\n\
Subject: both\r\n\
Content-Type: multipart/alternative; boundary=\"b\"\r\n\
\r\n\
--b\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
The plain version.\r\n\
--b\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>The HTML version.</p></body></html>\r\n\
--b--\r\n";

    #[test]
    fn a_plain_message_yields_its_text() {
        let body = parse(PLAIN);

        assert_eq!(
            body.text.as_deref().map(str::trim),
            Some("Hello there.\n\nSecond paragraph.")
        );
        assert!(body.html.is_none());
        assert!(body.attachments.is_empty());
    }

    #[test]
    fn the_plain_part_is_preferred_over_the_html_one() {
        // Both are the same message; the plain part is what the sender wrote for readers who
        // cannot render HTML, and it is what the reader shows until Phase 6 can sanitise the
        // other one safely.
        let body = parse(ALTERNATIVE);

        assert_eq!(
            body.text.as_deref().map(str::trim),
            Some("The plain version.")
        );
        assert!(body
            .html
            .as_deref()
            .unwrap_or_default()
            .contains("The HTML version."));
    }

    #[test]
    fn an_html_only_message_still_produces_readable_text() {
        // Most marketing mail is HTML only. Without a derived text part the list preview
        // would be empty and the reader blank for a large fraction of a real mailbox.
        let raw = b"Content-Type: text/html; charset=utf-8\r\n\r\n\
<html><body><p>Your order shipped.</p><p>Track it here.</p></body></html>";

        let body = parse(raw);
        let text = body.text.unwrap_or_default();

        assert!(text.contains("Your order shipped."), "{text}");
        assert!(text.contains("Track it here."), "{text}");
        assert!(!text.contains('<'), "tags must not survive: {text}");
    }

    #[test]
    fn script_and_style_contents_never_reach_the_text() {
        // Standing rule 11. This text goes into the list preview and the FTS index; script
        // source in either is both useless and a signal that markup is leaking through.
        let raw = b"Content-Type: text/html; charset=utf-8\r\n\r\n\
<html><head><style>.a{color:red}</style></head>\
<body><script>alert('x')</script><p>Real content.</p></body></html>";

        let body = parse(raw);
        let text = body.text.unwrap_or_default();

        assert!(text.contains("Real content."), "{text}");
        assert!(!text.contains("alert"), "{text}");
        assert!(!text.contains("color:red"), "{text}");
    }

    #[test]
    fn html_entities_are_decoded_in_the_derived_text() {
        let raw = b"Content-Type: text/html; charset=utf-8\r\n\r\n\
<p>Ben &amp; Jerry&#39;s &lt;tasty&gt;&nbsp;treats</p>";

        let text = parse(raw).text.unwrap_or_default();

        assert!(text.contains("Ben & Jerry's"), "{text}");
        assert!(text.contains("<tasty>"), "{text}");
    }

    #[test]
    fn outlook_conditional_comments_do_not_reach_the_preview() {
        // Found in a real inbox: three of the first six messages opened with
        // "<!--[if !mso]><!--> <!--[if" instead of a sentence. Marketing mail is full of
        // these and a preview column that shows them is unreadable.
        let raw = br#"Content-Type: text/html; charset=utf-8

<html><head>
<!--[if mso]><style>.x{color:red}</style><![endif]-->
</head><body>
<!--[if !mso]><!--><div>Real content here.</div><!--<![endif]-->
</body></html>"#;

        let text = parse(raw).text.unwrap_or_default();

        assert!(text.contains("Real content here."), "{text}");
        assert!(!text.contains("mso"), "{text}");
        assert!(!text.contains("endif"), "{text}");
        assert!(!text.contains("[if"), "{text}");
    }

    #[test]
    fn numeric_entities_are_decoded() {
        // Newsletters are full of these. A preview reading "Exclusively! &#8202; Dear
        // Reader" is what leaving them raw looks like — also seen in a real inbox.
        let raw = br#"Content-Type: text/html; charset=utf-8

<p>Exclusively!&#8202;Dear&#8217;s&#x2014;end</p>"#;

        let text = parse(raw).text.unwrap_or_default();

        assert!(!text.contains("&#"), "no raw entity may survive: {text}");
        assert!(text.contains("Dear"), "{text}");
        assert!(text.contains('—'), "hex entities decode too: {text}");
        assert!(
            text.contains('\u{2019}'),
            "decimal entities decode too: {text}"
        );
    }

    #[test]
    fn a_stray_ampersand_is_left_alone() {
        // "Fish &#38; chips" is an entity; "AT&T & more" is not, and neither is a URL with
        // a query string. Mangling those is worse than leaving an entity undecoded.
        assert_eq!(decode_numeric_entities("plain & text"), "plain & text");
        assert_eq!(decode_numeric_entities("a&#nonsense; b"), "a&#nonsense; b");
        assert_eq!(decode_numeric_entities("url?a=1&#38;b=2"), "url?a=1&b=2");
    }

    #[test]
    fn entities_are_not_double_decoded() {
        // A sender who wrote a literal "&lt;" escapes it as "&amp;lt;". Decoding &amp; first
        // would turn it into "<" and lose what they actually typed.
        assert_eq!(decode_entities("&amp;lt;tag&amp;gt;"), "&lt;tag&gt;");
    }

    #[test]
    fn multibyte_characters_survive_the_html_stripper() {
        // The stripper walks bytes to find tags and must step by character to copy them.
        // Getting this wrong splits a UTF-8 sequence and produces replacement characters.
        let raw = "Content-Type: text/html; charset=utf-8\r\n\r\n<p>Björn — naïve café 日本語</p>"
            .as_bytes();

        let text = parse(raw).text.unwrap_or_default();

        assert!(text.contains("Björn"), "{text}");
        assert!(text.contains("café"), "{text}");
        assert!(text.contains("日本語"), "{text}");
        assert!(
            !text.contains('\u{FFFD}'),
            "no replacement characters: {text}"
        );
    }

    #[test]
    fn an_attachment_is_recorded_and_kept_out_of_the_body() {
        let raw = b"Content-Type: multipart/mixed; boundary=\"m\"\r\n\r\n\
--m\r\n\
Content-Type: text/plain\r\n\r\n\
See attached.\r\n\
--m\r\n\
Content-Type: application/pdf\r\n\
Content-Disposition: attachment; filename=\"invoice.pdf\"\r\n\r\n\
%PDF-1.4 fake\r\n\
--m--\r\n";

        let body = parse(raw);

        assert_eq!(body.text.as_deref().map(str::trim), Some("See attached."));
        assert_eq!(body.attachments.len(), 1);

        let attachment = &body.attachments[0];
        assert_eq!(attachment.filename.as_deref(), Some("invoice.pdf"));
        assert_eq!(attachment.mime, "application/pdf");
        assert!(!attachment.is_inline);
    }

    #[test]
    fn an_inline_image_does_not_count_as_an_attachment() {
        // Every HTML newsletter carries a tracking pixel as an inline part. A mailbox where
        // every row shows a paperclip is a mailbox where the paperclip means nothing.
        let raw = b"Content-Type: multipart/related; boundary=\"r\"\r\n\r\n\
--r\r\n\
Content-Type: text/html\r\n\r\n\
<p>Hello <img src=\"cid:pixel\"></p>\r\n\
--r\r\n\
Content-Type: image/gif\r\n\
Content-ID: <pixel>\r\n\
Content-Disposition: inline\r\n\r\n\
GIF89a\r\n\
--r--\r\n";

        let body = parse(raw);

        assert_eq!(body.attachments.len(), 1, "it is still stored");
        assert!(body.attachments[0].is_inline, "but marked inline");
        assert_eq!(body.attachments[0].content_id.as_deref(), Some("pixel"));
    }

    #[test]
    fn a_truncated_message_does_not_panic() {
        // Standing rule 11: bodies are hostile input, and a body cut off mid-boundary is the
        // gentlest example. Every one of these must return rather than unwind.
        for raw in [
            &b""[..],
            &b"Content-Type: multipart/mixed; boundary=\"m\"\r\n\r\n--m\r\n"[..],
            &b"Content-Type: text/plain\r\n"[..],
            &b"\r\n\r\n\r\n"[..],
            &b"\xff\xfe\x00\x00 not text at all"[..],
        ] {
            let _ = parse(raw);
        }
    }

    #[test]
    fn deeply_nested_mime_terminates() {
        // A message can nest multipart parts thousands deep. Unbounded recursion here
        // overflows the stack, which aborts the process — a crash triggered by opening a
        // message someone sent you.
        let mut raw = String::from("Content-Type: multipart/mixed; boundary=\"b0\"\r\n\r\n");

        for depth in 0..200 {
            raw.push_str(&format!(
                "--b{depth}\r\nContent-Type: multipart/mixed; boundary=\"b{}\"\r\n\r\n",
                depth + 1
            ));
        }
        raw.push_str("--b200\r\nContent-Type: text/plain\r\n\r\ndeep\r\n--b200--\r\n");

        let body = parse(raw.as_bytes());

        // The assertion is that it returned at all.
        assert!(body.preview.len() <= PREVIEW_CHARS);
    }

    #[test]
    fn the_preview_skips_quoted_lines_and_signatures() {
        // A reply whose preview reads "> On Tuesday someone wrote:" says nothing about the
        // message the user is looking at.
        let raw = b"Content-Type: text/plain\r\n\r\n\
> On Tuesday, Ada wrote:\r\n\
> the original message\r\n\
\r\n\
Yes, that works for me.\r\n\
\r\n\
--\r\n\
Sent from my phone\r\n";

        let body = parse(raw);

        assert!(
            body.preview.starts_with("Yes, that works for me."),
            "{}",
            body.preview
        );
        assert!(!body.preview.contains("On Tuesday"), "{}", body.preview);
    }

    #[test]
    fn markup_residue_in_a_plain_text_part_stays_out_of_the_preview() {
        // Real mail, not a hypothetical: Google's own text/plain alternative contains
        // Outlook conditional comments, so the preview for a Google security email opened
        // with "<!--[if !mso]><!-->" instead of a sentence.
        let raw = "Content-Type: text/plain; charset=utf-8

\n Keep track of your Google Account data
\n 
\n <!--[if !mso]><!-->
\n <!--[if false]><!-->
\n {200a}
\n You are receiving this because you signed in.
";

        let body = parse(raw.as_bytes());

        assert!(!body.preview.contains("mso"), "{}", body.preview);
        assert!(!body.preview.contains("[if"), "{}", body.preview);
        assert!(body.preview.contains("Keep track"), "{}", body.preview);
        assert!(
            body.preview.contains("You are receiving"),
            "{}",
            body.preview
        );

        // The stored body stays faithful — the reader shows what the sender actually sent.
        assert!(
            body.text
                .as_deref()
                .unwrap_or_default()
                .contains("[if !mso]"),
            "body_text must not be edited"
        );
    }

    #[test]
    fn prose_containing_an_angle_bracket_is_not_mistaken_for_markup() {
        // Over-stripping a preview loses the message, which is worse than leaving residue in.
        assert!(!is_markup_residue("true if x < y and y > z"));
        assert!(!is_markup_residue("reply to <ada@example.test> please"));
        assert!(!is_markup_residue("<ada@example.test>"));
        assert!(!is_markup_residue("a normal sentence."));

        assert!(is_markup_residue("<!--[if !mso]><!-->"));
        assert!(is_markup_residue("<![endif]-->"));
        assert!(is_markup_residue("-->"));
        assert!(is_markup_residue("</div>"));
    }

    #[test]
    fn the_preview_is_bounded() {
        let long = "word ".repeat(5_000);
        let raw = format!("Content-Type: text/plain\r\n\r\n{long}");

        let body = parse(raw.as_bytes());

        assert!(body.preview.chars().count() <= PREVIEW_CHARS);
    }

    #[test]
    fn a_message_with_no_readable_part_still_parses() {
        // Degrade visibly: an empty body is a message the user can see and not read, which
        // is strictly better than one that failed to store.
        let raw = b"Content-Type: application/octet-stream\r\n\r\n\x00\x01\x02";

        let body = parse(raw);

        assert!(body.text.is_none() || body.text.as_deref() == Some(""));
        assert_eq!(body.preview, "");
    }

    #[test]
    fn cache_paths_are_per_account_and_per_message() {
        let root = Path::new("C:/store");

        let a = cache_path(root, 1, 42);
        let b = cache_path(root, 2, 42);

        assert_ne!(a, b, "the same message id in two accounts must not collide");
        assert!(a.ends_with("42.eml"));
        assert!(a.to_string_lossy().contains("bodies"));
    }
}
