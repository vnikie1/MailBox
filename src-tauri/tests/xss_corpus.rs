//! Every payload in the corpus, through the function the reader actually calls. docs/06 Phase 6.
//!
//! ## Why `render` and not `sanitise`
//!
//! `sanitise` is the allowlist, and testing it alone would be testing ammonia's configuration.
//! What protects somebody reading their mail is the whole pipeline: sanitise, then rewrite
//! images, then detect links, then fold the quote. Each of those rebuilds markup, and a
//! transformation applied *after* the allowlist is exactly where a hole would appear -- the
//! sanitiser would be innocent and the message would still execute.
//!
//! ## What "blocked" means here
//!
//! Not "the payload is absent". A sanitiser is allowed to keep the visible text of a hostile
//! message, and stripping the word `alert` out of somebody's mail would be its own bug. What is
//! asserted is that the output cannot execute or phone home: no script element, no event handler
//! attribute, no executable URL scheme, no framing or embedding element, no stylesheet, and
//! nothing that navigates the reader somewhere else.
//!
//! Run it and read it:
//!
//!     cargo test --test xss_corpus -- --nocapture

use std::collections::HashMap;

use halcyon_lib::mail::render::render;

const CORPUS: &str = include_str!("fixtures/xss-corpus.txt");

/// The payload lines, without comments and blanks.
fn payloads() -> Vec<&'static str> {
    CORPUS
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

/// Substrings that must never survive into rendered output, and why each one matters.
const FORBIDDEN: &[(&str, &str)] = &[
    ("<script", "a script element would execute"),
    ("<iframe", "a frame loads another document, and its scripts"),
    ("<object", "an object element embeds a plugin document"),
    ("<embed", "an embed element does the same"),
    ("<applet", "an applet element does the same"),
    ("<frame", "a frame or frameset loads another document"),
    ("<portal", "a portal embeds another document"),
    (
        "<style",
        "a stylesheet can fetch a remote resource and hide content",
    ),
    ("<link", "a link element pulls a remote stylesheet"),
    (
        "<meta",
        "a meta refresh navigates; a charset changes how the parser reads",
    ),
    (
        "<base",
        "a base element rewrites every relative URL in the message",
    ),
    (
        "<form",
        "a form posts the reader's input to a server the sender chose",
    ),
    ("<input", "a form control without a form is still a control"),
    (
        "<button",
        "formaction on a button posts without a form element",
    ),
    ("<isindex", "an isindex submits to an arbitrary action"),
    ("javascript:", "an executable URL scheme"),
    ("vbscript:", "an executable URL scheme"),
    (
        "data:text/html",
        "a data URL that is a document executes as one",
    ),
    ("srcdoc", "srcdoc is an inline document"),
    ("expression(", "a CSS expression executes in old renderers"),
    ("behavior:", "a CSS behaviour binds script"),
    ("@import", "an import pulls a remote stylesheet"),
    ("formaction", "posts without a form element"),
    ("http-equiv", "a meta refresh navigates"),
];

/// The interiors of real tags: everything between an unescaped `<` and the next `>`.
///
/// The distinction is the whole point. `<xmp>` renders its contents as literal text, and the
/// sanitiser handles it by escaping them — so the output of
/// `<xmp><img src=x onerror=alert(1)></xmp>` is `&lt;img src=x onerror=alert(1)&gt;`, which a
/// browser draws as characters on the page and never parses as markup.
///
/// The first version of this scanned the raw string and reported that payload as a survivor. It
/// was a false alarm, and a convincing one: the words `onerror=` really are in the output. A
/// corpus test that cries wolf is worse than none, because the next real finding gets waved
/// through as "probably the xmp thing again".
fn tag_interiors(html: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = html;

    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            break;
        };
        found.push(&after[..end]);
        rest = &after[end + 1..];
    }

    found
}

/// An `on...=` handler inside a real tag, found without pulling in a regex crate.
///
/// Looks for `on` preceded by whitespace and followed by letters and then `=`, which is what an
/// event handler attribute looks like and what the word "onward" in body text does not.
fn has_event_handler(html: &str) -> Option<String> {
    tag_interiors(html)
        .iter()
        .find_map(|tag| handler_in_tag(tag))
}

/// The handler-shaped attribute in one tag's interior, if there is one.
fn handler_in_tag(tag: &str) -> Option<String> {
    // A leading space so an attribute at the very start of the interior still has the
    // whitespace boundary the scan below looks for: `img onerror=...` after the element name.
    let padded = format!(" {tag}");
    let lower = padded.to_ascii_lowercase();
    let bytes = lower.as_bytes();

    for (at, _) in lower.match_indices("on") {
        if at == 0 || !bytes[at - 1].is_ascii_whitespace() {
            continue;
        }

        let mut end = at + 2;
        while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
            end += 1;
        }

        if end == at + 2 {
            continue; // "on" by itself, not an attribute name
        }

        // Whitespace before the `=` is legal, and is how this gets past a naive check.
        let mut probe = end;
        while probe < bytes.len() && bytes[probe].is_ascii_whitespace() {
            probe += 1;
        }

        if probe < bytes.len() && bytes[probe] == b'=' {
            return Some(lower[at..probe + 1].to_string());
        }
    }

    None
}

/// Renders every payload and returns a description of each one that kept something dangerous.
fn survivors(load_remote: bool) -> (usize, Vec<String>) {
    let payloads = payloads();
    let inline = HashMap::new();
    let remote = HashMap::new();
    let mut failures = Vec::new();

    for payload in &payloads {
        let rendered = render(Some(payload), None, &inline, load_remote, &remote);
        let lower = rendered.html.to_ascii_lowercase();

        for (needle, why) in FORBIDDEN {
            if lower.contains(needle) {
                failures.push(format!(
                    "  {payload}\n    -> kept {needle:?} ({why})\n    -> rendered: {}",
                    rendered.html
                ));
            }
        }

        if let Some(handler) = has_event_handler(&rendered.html) {
            failures.push(format!(
                "  {payload}\n    -> kept the event handler {handler:?}\n    -> rendered: {}",
                rendered.html
            ));
        }
    }

    (payloads.len(), failures)
}

#[test]
fn the_corpus_is_fully_blocked_with_images_off() {
    let (total, failures) = survivors(false);

    assert!(
        total >= 60,
        "the corpus has shrunk to {total} payloads; it is meant to grow, not shrink"
    );

    println!(
        "XSS corpus: {total} payloads, images blocked, {} survived",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "{} of {total} payloads survived rendering:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn the_corpus_is_fully_blocked_with_images_on() {
    // The path most readers are actually on, since images load by default. Allowing images may
    // widen what can be *fetched*; it must never widen what can be executed.
    let (total, failures) = survivors(true);

    println!(
        "XSS corpus: {total} payloads, images allowed, {} survived",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "{} of {total} payloads survived rendering:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn the_check_can_actually_fail() {
    // A corpus test that cannot fail proves nothing, and this one is a list of substrings --
    // the easiest kind to get quietly wrong. Feed the detectors what a broken sanitiser would
    // emit and confirm they object.
    assert!(
        has_event_handler("<img src=x onerror=alert(1)>").is_some(),
        "an unquoted event handler was not detected"
    );
    assert!(
        has_event_handler("<img src=x ONERROR = \"alert(1)\">").is_some(),
        "a spaced, upper-case event handler was not detected"
    );
    assert!(
        has_event_handler("<p>the onward journey</p>").is_none(),
        "ordinary prose was mistaken for an event handler"
    );
    assert!(
        has_event_handler("&lt;img src=x onerror=alert(1)&gt;").is_none(),
        "escaped text was mistaken for a live event handler, which is what <xmp> produces"
    );
    assert!(
        FORBIDDEN.iter().any(|(needle, _)| *needle == "<script"),
        "the forbidden list no longer covers script elements"
    );
}
