//! Parsing a message's MIME tree without letting its sender choose how deep we recurse.
//!
//! ## Why every parse goes through here
//!
//! `mailparse::parse_mail` recurses once per level of multipart nesting, and the nesting is
//! whatever the sender wrote. A ~300 KB message nested 5,000 deep overflows a 2 MiB stack — a
//! tokio worker's — and a stack overflow on Windows is not a panic that can be caught: it ends
//! the process. Because the crash happened before the body was stored, nothing recorded that the
//! message had been tried, so the app died again on every reopen, every prefetch of the rows
//! around it, every Rebuild, and when the message was opened as a `.eml`.
//!
//! The walkers that read the parsed tree were already capped at 32 levels. That cap protected
//! nothing, because by the time they ran `mailparse` had already done the deep recursion.
//!
//! ## How the depth is measured
//!
//! By walking the structure the same way `mailparse` will, iteratively, and stopping as soon as
//! it goes deeper than [`MAX_NESTING`]. That is an exact measurement rather than a guess: the
//! boundary search and the part splitting below mirror `parse_mail_recursive` in mailparse
//! 0.16, so a message is refused exactly when the real parse would have gone too deep. Counting
//! `boundary=` parameters would have been simpler and wrong — a digest of a hundred sibling
//! multiparts has a hundred boundaries and a depth of two.
//!
//! The cost is a second pass over the boundaries, which is linear per level, and levels are
//! bounded by the cap.

use mailparse::{MailHeader, MailHeaderMap, MailParseError, ParsedMail};

/// Deepest multipart nesting a message may have and still be parsed as MIME.
///
/// Real mail stays in single figures; forwarded-as-attachment chains reach perhaps a dozen.
/// Sixty-four levels of `mailparse` frames is a few tens of kilobytes even in a debug build.
pub const MAX_NESTING: usize = 64;

/// Why a message could not be parsed as a MIME tree.
#[derive(Debug)]
pub enum Unparsed<'a> {
    /// Nested deeper than [`MAX_NESTING`]. The top-level headers still parsed, and `body` is
    /// everything after them, for a caller that wants to show the message as plain text.
    TooDeep {
        headers: Vec<MailHeader<'a>>,
        body: &'a [u8],
    },
    /// Not a message `mailparse` can read at all.
    Malformed(MailParseError),
}

/// For logs. Deliberately not `Debug`'s output, which would print the whole body.
impl std::fmt::Display for Unparsed<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooDeep { .. } => {
                write!(f, "MIME nesting is deeper than {MAX_NESTING} levels")
            }
            Self::Malformed(error) => write!(f, "{error}"),
        }
    }
}

/// `mailparse::parse_mail`, refusing a tree deeper than [`MAX_NESTING`] before it recurses.
///
/// The only way this crate should parse a message body. A direct call to `parse_mail` on
/// anything a sender wrote is a crash someone can trigger by email.
pub fn parse(raw: &[u8]) -> Result<ParsedMail<'_>, Unparsed<'_>> {
    if nesting_exceeds(raw, MAX_NESTING) {
        let (headers, ix_body) = mailparse::parse_headers(raw).map_err(Unparsed::Malformed)?;

        tracing::warn!(
            bytes = raw.len(),
            limit = MAX_NESTING,
            "message nests deeper than the MIME limit; treating it as plain text"
        );

        return Err(Unparsed::TooDeep {
            headers,
            body: raw.get(ix_body..).unwrap_or_default(),
        });
    }

    mailparse::parse_mail(raw).map_err(Unparsed::Malformed)
}

/// Whether parsing `raw` would recurse more than `limit` levels.
///
/// An explicit stack of `(part, depth)` in place of the call stack. Parts whose headers do not
/// parse are not descended into, which is also what `mailparse` does — it gives up on the whole
/// message there — so the depth measured is never less than the depth the real parse can reach.
fn nesting_exceeds(raw: &[u8], limit: usize) -> bool {
    let mut pending: Vec<(&[u8], usize)> = vec![(raw, 0)];

    while let Some((part, depth)) = pending.pop() {
        let Ok((headers, ix_body)) = mailparse::parse_headers(part) else {
            continue;
        };

        // No Content-Type means text/plain, or message/rfc822 inside a multipart/digest.
        // Neither is multipart, and `mailparse` does not descend into an attached message.
        let Some(ctype) = headers
            .get_first_value("Content-Type")
            .map(|value| mailparse::parse_content_type(&value))
        else {
            continue;
        };

        let Some(boundary) = ctype.params.get("boundary") else {
            continue;
        };

        if !ctype.mimetype.starts_with("multipart/") || part.len() <= ix_body {
            continue;
        }

        let children = subparts(part, ix_body, boundary);
        if children.is_empty() {
            continue;
        }

        if depth + 1 > limit {
            return true;
        }

        pending.extend(children.into_iter().map(|child| (child, depth + 1)));
    }

    false
}

/// The byte ranges `mailparse` would parse as this multipart's children.
///
/// A transcription of the loop in mailparse 0.16's `parse_mail_recursive`, including its
/// quirks: a boundary only counts at the start of a line (or at the start of the search), the
/// line break before a boundary belongs to the boundary, and a missing close delimiter means the
/// last part runs to the end.
fn subparts<'a>(raw: &'a [u8], ix_body: usize, boundary: &str) -> Vec<&'a [u8]> {
    let delimiter = format!("--{boundary}");
    let delimiter = delimiter.as_bytes();
    let mut children = Vec::new();

    let Some(ix_boundary_start) = find_line_prefix(raw, ix_body, delimiter) else {
        return children;
    };

    let mut ix_boundary_end = ix_boundary_start + delimiter.len();

    while let Some(ix_part_start) = find_newline(raw, ix_boundary_end).map(|at| at + 1) {
        let ix_part_boundary_start = find_line_prefix(raw, ix_part_start, delimiter);
        let ix_part_end = ix_part_boundary_start
            .map(|at| strip_trailing_crlf(raw, ix_part_start, at))
            .unwrap_or(raw.len());

        children.push(&raw[ix_part_start..ix_part_end]);

        ix_boundary_end = ix_part_boundary_start
            .map(|at| at + delimiter.len())
            .unwrap_or(raw.len());

        if ix_boundary_end + 2 > raw.len()
            || (raw[ix_boundary_end] == b'-' && raw[ix_boundary_end + 1] == b'-')
        {
            break;
        }
    }

    children
}

/// First line feed at or after `from`.
fn find_newline(haystack: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .iter()
        .position(|&byte| byte == b'\n')
        .map(|at| from + at)
}

/// First occurrence of `key` at or after `from` that starts a line, or starts the search.
///
/// The same answer as mailparse's search for any occurrence followed by a check of the byte
/// before it, reached by visiting line starts instead of every byte.
fn find_line_prefix(haystack: &[u8], from: usize, key: &[u8]) -> Option<usize> {
    if haystack.get(from..)?.starts_with(key) {
        return Some(from);
    }

    let mut at = from;
    while let Some(newline) = find_newline(haystack, at) {
        let line = newline + 1;
        if haystack[line..].starts_with(key) {
            return Some(line);
        }
        at = line;
    }

    None
}

/// Moves `at` back over one LF or CRLF, without going before `floor`.
fn strip_trailing_crlf(raw: &[u8], floor: usize, mut at: usize) -> usize {
    if at > floor && raw[at - 1] == b'\n' {
        at -= 1;
        if at > floor && raw[at - 1] == b'\r' {
            at -= 1;
        }
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nested `levels` deep, with fixed-width boundaries so the size is predictable.
    fn nested(levels: usize) -> String {
        let mut raw = String::from("Subject: nest\r\n");
        for level in 0..levels {
            raw.push_str(&format!(
                "Content-Type: multipart/mixed; boundary=\"b{level:05}\"\r\n\r\n--b{level:05}\r\n"
            ));
        }
        raw.push_str("Content-Type: text/plain\r\n\r\ninnermost\r\n");
        raw
    }

    /// How deep `mailparse` actually built the tree.
    fn depth_of(part: &ParsedMail<'_>) -> usize {
        let mut depth = 0;
        let mut current = part;
        while let Some(first) = current.subparts.first() {
            depth += 1;
            current = first;
        }
        depth
    }

    /// The deepest branch of a parsed tree, not just the first.
    fn max_depth(part: &ParsedMail<'_>) -> usize {
        part.subparts
            .iter()
            .map(|child| 1 + max_depth(child))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn the_measurement_agrees_with_mailparse_on_awkward_shapes() {
        // Each of these is a quirk of how mailparse splits parts. Wherever the measurement and
        // the real parse disagree about depth, one of them is wrong, and only one direction of
        // wrong is safe — so they are required to agree exactly.
        let cases = [
            // A boundary mentioned mid-line in the preamble is not a delimiter.
            "Content-Type: multipart/mixed; boundary=\"x\"\r\n\r\npreamble --x not here\r\n\
             --x\r\nContent-Type: multipart/mixed; boundary=\"y\"\r\n\r\n--y\r\n\r\nleaf\r\n--y--\r\n--x--\r\n",
            // No close delimiter: the last part runs to the end.
            "Content-Type: multipart/mixed; boundary=\"x\"\r\n\r\n--x\r\n\
             Content-Type: multipart/alternative; boundary=\"y\"\r\n\r\n--y\r\n\r\nleaf",
            // One boundary a prefix of another.
            "Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\n\
             Content-Type: multipart/mixed; boundary=\"bb\"\r\n\r\n--bb\r\n\r\nleaf\r\n--bb--\r\n--b--\r\n",
            // Multipart with no boundary is a leaf.
            "Content-Type: multipart/mixed\r\n\r\n--x\r\n\r\nleaf\r\n",
            // The deep branch is the second child, not the first.
            "Content-Type: multipart/mixed; boundary=\"a\"\r\n\r\n--a\r\n\r\nshallow\r\n--a\r\n\
             Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\n\
             Content-Type: multipart/mixed; boundary=\"c\"\r\n\r\n--c\r\n\r\ndeep\r\n--c--\r\n--b--\r\n--a--\r\n",
            // A delimiter at the very end, with nothing after it.
            "Content-Type: multipart/mixed; boundary=\"x\"\r\n\r\n--x",
            // LF only.
            "Content-Type: multipart/mixed; boundary=x\n\n--x\nContent-Type: multipart/mixed; boundary=y\n\n--y\n\nleaf\n--y--\n--x--\n",
        ];

        for raw in cases {
            let real = max_depth(&mailparse::parse_mail(raw.as_bytes()).expect("parses"));

            for limit in 0..4 {
                assert_eq!(
                    nesting_exceeds(raw.as_bytes(), limit),
                    real > limit,
                    "limit {limit}, real depth {real}: {raw:?}"
                );
            }
        }
    }

    #[test]
    fn the_limit_is_exact_rather_than_approximate() {
        // At the limit it parses, one past it it does not. If the measurement and mailparse
        // disagreed about what a level is, one side of this would fail.
        let at_limit = nested(MAX_NESTING);
        let parsed = parse(at_limit.as_bytes()).expect("parsed at the limit");
        assert_eq!(depth_of(&parsed), MAX_NESTING);

        let over = nested(MAX_NESTING + 1);
        assert!(matches!(
            parse(over.as_bytes()),
            Err(Unparsed::TooDeep { .. })
        ));
    }

    #[test]
    fn many_siblings_are_not_mistaken_for_depth() {
        // A digest of a hundred forwarded messages, each with its own alternative part: a
        // hundred boundaries, depth two. A count of boundaries would refuse it.
        let mut raw = String::from("Content-Type: multipart/mixed; boundary=\"outer\"\r\n\r\n");
        for index in 0..100 {
            raw.push_str(&format!(
                "--outer\r\nContent-Type: multipart/alternative; boundary=\"inner{index}\"\r\n\r\n\
                 --inner{index}\r\nContent-Type: text/plain\r\n\r\npart {index}\r\n--inner{index}--\r\n"
            ));
        }
        raw.push_str("--outer--\r\n");

        let parsed = parse(raw.as_bytes()).expect("parsed");
        assert_eq!(parsed.subparts.len(), 100);
    }

    #[test]
    fn too_deep_still_hands_back_the_headers_and_the_body() {
        let raw = nested(MAX_NESTING + 10);

        let Err(Unparsed::TooDeep { headers, body }) = parse(raw.as_bytes()) else {
            panic!("expected TooDeep");
        };

        assert_eq!(headers.get_first_value("Subject").as_deref(), Some("nest"));
        assert!(String::from_utf8_lossy(body).contains("--b00000"));
    }

    #[test]
    fn a_boundary_reused_by_a_child_does_not_count_as_depth() {
        // A child with its parent's boundary sees none of it — the parent already split there —
        // so mailparse stops. The measurement has to stop in the same place.
        let raw = "Content-Type: multipart/mixed; boundary=\"same\"\r\n\r\n\
                   --same\r\nContent-Type: multipart/mixed; boundary=\"same\"\r\n\r\n\
                   --same\r\nContent-Type: text/plain\r\n\r\nx\r\n--same--\r\n";

        let parsed = parse(raw.as_bytes()).expect("parsed");
        assert!(depth_of(&parsed) <= 2);
    }

    #[test]
    fn crlf_and_lf_messages_measure_the_same() {
        let lf = nested(MAX_NESTING + 1).replace("\r\n", "\n");
        assert!(matches!(
            parse(lf.as_bytes()),
            Err(Unparsed::TooDeep { .. })
        ));

        let lf_ok = nested(MAX_NESTING).replace("\r\n", "\n");
        assert!(parse(lf_ok.as_bytes()).is_ok());
    }

    #[test]
    fn an_ordinary_message_parses_as_it_always_did() {
        let raw = b"Content-Type: multipart/alternative; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nplain\r\n\
--b\r\nContent-Type: text/html\r\n\r\n<p>html</p>\r\n--b--\r\n";

        let ours = parse(raw).expect("parsed");
        let theirs = mailparse::parse_mail(raw).expect("parsed");

        assert_eq!(ours.subparts.len(), theirs.subparts.len());
        assert_eq!(
            ours.subparts[1].get_body().ok(),
            theirs.subparts[1].get_body().ok()
        );
    }
}
