//! Nothing from a message's `<head>` may reach the reader as text.
//!
//! Reported from using the app: every HTML message showed its subject line as a stray paragraph
//! above the message, flush left and outside the layout, looking like a rendering fault.
//!
//! It was the `<title>`. `rm_tags` takes a tag out of ammonia's allowlist, and the default for a
//! disallowed tag is to *unwrap* it — the element goes and its text stays. Every marketing
//! message carries a `<title>`, and its text is the subject.
//!
//! Tested against the real newsletter it was found on rather than a hand-written snippet,
//! because the thing that made this invisible for four phases is that it only shows up in mail
//! shaped like real mail: a full document with a head, not the body fragments the unit tests
//! use.

use std::collections::HashMap;

use halcyon_lib::mail::render::render;

const NEWSLETTER: &str = include_str!("fixtures/newsletter-with-title.html");

/// The `<title>` of the message the fixture came from, in full.
///
/// The full string on purpose. The newsletter also carries its headline as an `<h1>` in the body
/// — "What to know about Trump's efforts to block voting by mail" — and that belongs there. Only
/// the `<title>` has the "On Politics: " prefix, so this distinguishes the leak from the article.
const TITLE: &str = "On Politics: What to know about";

#[test]
fn the_title_does_not_become_the_first_line_of_the_message() {
    assert!(
        NEWSLETTER.contains("<title>"),
        "the fixture no longer has a title, so this proves nothing"
    );

    let rendered = render(
        Some(NEWSLETTER),
        None,
        &HashMap::new(),
        false,
        &HashMap::new(),
    );

    assert!(
        !rendered.html.contains(TITLE),
        "the subject leaked out of <title> and into the message body"
    );
}

#[test]
fn the_message_itself_still_arrives() {
    // The other half: removing head content must not take the message with it. A fix that
    // emptied the body would pass the assertion above and be far worse.
    let rendered = render(
        Some(NEWSLETTER),
        None,
        &HashMap::new(),
        false,
        &HashMap::new(),
    );

    assert!(
        rendered.html.len() > 1_000,
        "the body came back nearly empty: {} bytes",
        rendered.html.len()
    );
    assert!(
        rendered.html.contains("Good evening"),
        "the message text is missing"
    );
}

#[test]
fn head_metadata_leaves_no_text_behind() {
    // The general rule, on a small document where every part is visible at once.
    let source = r#"<html>
        <head>
          <title>SUBJECT LINE</title>
          <style>.hidden { display: none }</style>
          <script>var tracking = 1;</script>
        </head>
        <body><p>The actual message.</p><noscript>Turn on scripts.</noscript></body>
      </html>"#;

    let rendered = render(Some(source), None, &HashMap::new(), false, &HashMap::new());

    assert!(rendered.html.contains("The actual message."));
    assert!(!rendered.html.contains("SUBJECT LINE"), "{}", rendered.html);
    assert!(
        !rendered.html.contains("display: none"),
        "{}",
        rendered.html
    );
    assert!(!rendered.html.contains("var tracking"), "{}", rendered.html);
    assert!(
        !rendered.html.contains("Turn on scripts."),
        "{}",
        rendered.html
    );
}
