//! Data detectors. docs/04 Phase 6.
//!
//! Mail's quiet trick: a date in a message becomes an event you can add, a tracking number
//! becomes a parcel you can follow, a phone number becomes a call. The message is not edited —
//! the detections are wrapped in anchors the reader intercepts, so what the sender wrote is
//! still exactly what is shown.
//!
//! Two constraints shape everything here, and both are about not breaking the message.
//!
//! **Only text nodes are ever touched.** Running a pattern over the whole document would
//! rewrite the inside of attributes and turn `<a href="tel:...">` into nested anchors or
//! worse. The scanner below walks the markup and only considers the runs between `>` and `<`,
//! and skips anything already inside a link, a style block or a fold's summary.
//!
//! **Detection is deliberately conservative.** A false positive is worse than a miss: it puts
//! a link on ordinary prose, and a link in a message is something the user is entitled to
//! believe the sender put there. Every pattern here is narrow, and the tests are mostly about
//! what must *not* match.

/// What a detection turns into when the reader acts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A parcel tracking number. UPS, FedEx, USPS, Royal Mail, DHL.
    Tracking,
    /// A telephone number in international form.
    Phone,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Tracking => "tracking",
            Kind::Phone => "phone",
        }
    }
}

/// Whether a run of characters is a plausible tracking number.
///
/// Narrow on purpose. These are the carrier formats that are unambiguous enough not to fire on
/// an order reference or an invoice number, which are the same shape and mean nothing to a
/// carrier's website.
fn tracking_kind(candidate: &str) -> bool {
    let upper = candidate.to_ascii_uppercase();
    let digits = upper.chars().filter(char::is_ascii_digit).count();

    // UPS: 1Z followed by 16 alphanumerics.
    if upper.starts_with("1Z")
        && upper.len() == 18
        && upper[2..].chars().all(|c| c.is_alphanumeric())
    {
        return true;
    }

    // Royal Mail and other S10: two letters, nine digits, two letters ending in a country code.
    if upper.len() == 13
        && upper[..2].chars().all(|c| c.is_ascii_alphabetic())
        && upper[2..11].chars().all(|c| c.is_ascii_digit())
        && upper[11..].chars().all(|c| c.is_ascii_alphabetic())
    {
        return true;
    }

    // FedEx (12 or 15) and USPS (20 or 22): all digits, and only at these exact lengths.
    // Anything else of similar shape is far more likely to be an order number.
    matches!(upper.len(), 12 | 15 | 20 | 22) && digits == upper.len()
}

/// Whether a run is a phone number worth offering to dial.
///
/// International form only — a leading `+` and 8 to 15 digits, which is E.164. Detecting bare
/// local numbers means detecting every four-digit year and every invoice reference, and the
/// resulting `tel:` link would be wrong as often as right.
fn phone_kind(candidate: &str) -> bool {
    let Some(rest) = candidate.strip_prefix('+') else {
        return false;
    };

    let digits = rest.chars().filter(char::is_ascii_digit).count();

    (8..=15).contains(&digits)
        && rest
            .chars()
            .all(|c| c.is_ascii_digit() || c == ' ' || c == '-' || c == '(' || c == ')')
}

/// Classifies one candidate run.
fn classify(candidate: &str) -> Option<Kind> {
    if phone_kind(candidate) {
        return Some(Kind::Phone);
    }
    if tracking_kind(candidate) {
        return Some(Kind::Tracking);
    }

    None
}

/// Splits a text run into candidate tokens, keeping their positions.
///
/// A phone number contains spaces and brackets, so tokens are not simply whitespace-separated:
/// a run is extended across separators while it still looks like it could be part of one.
fn tokens(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;

    for (index, byte) in bytes.iter().enumerate() {
        let part_of_token = byte.is_ascii_alphanumeric()
            || matches!(byte, b'+' | b'-' | b'(' | b')')
            // A space continues a token only when a `+` opened it *and* a digit follows, so
            // "+44 20 7946 0958 to confirm" ends at the number rather than swallowing the
            // rest of the sentence — which is what made it stop looking like a phone number.
            || (*byte == b' '
                && start.is_some_and(|from| bytes.get(from) == Some(&b'+'))
                && bytes.get(index + 1).is_some_and(u8::is_ascii_digit));

        match (part_of_token, start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                spans.push((from, index));
                start = None;
            }
            _ => {}
        }
    }

    if let Some(from) = start {
        spans.push((from, bytes.len()));
    }

    spans
}

/// Wraps detections in one text run.
fn mark_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;

    for (from, to) in tokens(text) {
        if from < cursor {
            continue;
        }

        // Trailing punctuation belongs to the sentence, not to the number.
        let raw = &text[from..to];
        let trimmed = raw.trim_end_matches([')', '-', '.', ',']);
        if trimmed.is_empty() {
            continue;
        }

        let Some(kind) = classify(trimmed) else {
            continue;
        };

        out.push_str(&text[cursor..from]);
        out.push_str(&format!(
            "<a class=\"halcyon-detected\" data-detected=\"{}\" data-value=\"{}\" href=\"#\">{}</a>",
            kind.as_str(),
            escape_attribute(trimmed),
            escape_text(trimmed)
        ));

        cursor = from + trimmed.len();
    }

    out.push_str(&text[cursor..]);
    out
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Elements whose text must be left completely alone.
///
/// `a` because a link inside a link is invalid and would break the reader's own click
/// handling; `style` because its content is CSS, not prose, and wrapping part of it in an
/// anchor would produce markup inside a stylesheet.
const OPAQUE: &[&str] = &["a", "style", "script", "summary", "textarea", "title"];

/// Wraps detected data in sanitised message HTML.
///
/// Runs last, on markup html5ever has already balanced. Text between tags is the only thing it
/// looks at, so no attribute and no tag name can be rewritten by a pattern that happens to
/// match inside one.
pub fn mark(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut index = 0usize;
    let mut opaque_depth = 0usize;

    while index < html.len() {
        let Some(open) = html[index..].find('<').map(|offset| index + offset) else {
            break;
        };

        let text = &html[index..open];
        if opaque_depth == 0 {
            out.push_str(&mark_text(text));
        } else {
            out.push_str(text);
        }

        let Some(close) = html[open..].find('>').map(|offset| open + offset) else {
            out.push_str(&html[open..]);
            return out;
        };

        let tag = &html[open..=close];
        out.push_str(tag);

        let inner = tag.trim_start_matches('<').trim_end_matches('>');
        let closing = inner.starts_with('/');
        let name: String = inner
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();

        if OPAQUE.contains(&name.as_str()) {
            if closing {
                opaque_depth = opaque_depth.saturating_sub(1);
            } else if !inner.ends_with('/') {
                opaque_depth += 1;
            }
        }

        index = close + 1;
    }

    if index < html.len() {
        let tail = &html[index..];
        if opaque_depth == 0 {
            out.push_str(&mark_text(tail));
        } else {
            out.push_str(tail);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ups_tracking_number_is_detected() {
        let marked = mark("<p>Your parcel 1Z999AA10123456784 is on its way.</p>");

        assert!(marked.contains("data-detected=\"tracking\""), "{marked}");
        assert!(marked.contains("1Z999AA10123456784"));
    }

    #[test]
    fn an_international_phone_number_is_detected() {
        let marked = mark("<p>Call +44 20 7946 0958 to confirm.</p>");

        assert!(marked.contains("data-detected=\"phone\""), "{marked}");
    }

    #[test]
    fn ordinary_prose_and_numbers_are_left_alone() {
        // The important half. A false positive puts a link on text the sender did not link,
        // and a link in a message is something the user is entitled to trust.
        let samples = [
            "<p>The meeting is in room 204 on the 3rd floor.</p>",
            "<p>Invoice 4491 for 2026 totals 1200.</p>",
            "<p>Reference ABC123 applies.</p>",
            "<p>Call 555 1234 for details.</p>",
            "<p>Our VAT number is GB123456789.</p>",
        ];

        for sample in samples {
            let marked = mark(sample);
            assert!(
                !marked.contains("halcyon-detected"),
                "false positive in {sample}: {marked}"
            );
        }
    }

    #[test]
    fn nothing_inside_an_existing_link_is_touched() {
        // A link inside a link is invalid, and it would break the reader's own click handling
        // — which resolves the nearest anchor and would find the wrong one.
        let marked = mark(r#"<a href="https://x.test">1Z999AA10123456784</a>"#);

        assert!(!marked.contains("halcyon-detected"), "{marked}");
    }

    #[test]
    fn attributes_and_tag_names_are_never_rewritten() {
        // The reason this runs on text runs rather than over the whole document. A pattern
        // matching inside an attribute would corrupt the markup rather than annotate it.
        let html = r#"<img src="https://x.test/1Z999AA10123456784.png" alt="+44 20 7946 0958">"#;
        let marked = mark(html);

        assert_eq!(marked, html, "markup must be untouched");
    }

    #[test]
    fn text_inside_a_style_block_is_not_marked() {
        let marked = mark("<style>.x { content: '1Z999AA10123456784'; }</style>");

        assert!(!marked.contains("halcyon-detected"), "{marked}");
    }

    #[test]
    fn markup_in_the_detected_text_is_escaped() {
        // The value goes into an attribute and the label into text. Neither may carry markup
        // out of the detector — this is still a message body.
        let marked = mark("<p>+44 20 7946 0958</p>");

        assert!(!marked.contains("data-value=\"\""), "{marked}");
        assert!(marked.contains("href=\"#\""), "{marked}");
    }

    #[test]
    fn a_number_at_the_end_of_a_sentence_keeps_its_full_stop_outside_the_link() {
        // Otherwise the tracking number handed to the carrier has a full stop on the end and
        // returns "not found", which reads as the app being broken.
        let marked = mark("<p>Tracking 1Z999AA10123456784.</p>");

        assert!(marked.contains("1Z999AA10123456784</a>."), "{marked}");
    }

    #[test]
    fn an_empty_document_and_unclosed_markup_do_not_panic() {
        // Standing rule 13. This runs on every message, and a message is hostile input.
        assert_eq!(mark(""), "");
        assert_eq!(mark("<p>unclosed"), "<p>unclosed");
        assert_eq!(mark("<<>>"), "<<>>");
        mark("<a href=\"x\"");
    }
}
