//! How long it takes to prepare one real message for the reader, and how big the result is.
//!
//! ## Why this exists
//!
//! Reported from using the app: "the content loading is very very slow". The log said what the
//! complaint could not — a 53KB newsletter took **1.9 seconds** between the body being read from
//! the database and the rendered HTML coming back, and the 53KB became 524KB on the way.
//!
//! Neither number is visible from inside the app. `render` is pure and fast to call, so the
//! honest place to hold the line is here, against the actual message it was found on.
//!
//! ## The fixture
//!
//! A real marketing newsletter out of a real mailbox: 53KB, 709 tags, 148 inline `style`
//! attributes, 32 links. Exactly the shape that is slow, and exactly the shape most mail is.

use std::collections::HashMap;
use std::time::Instant;

use halcyon_lib::mail::render::render;

const NEWSLETTER: &str = include_str!("fixtures/slow-newsletter.html");

/// What one message may cost to render.
///
/// docs/06 Phase 3 budgets 100ms for opening a message, and that budget has to cover reading the
/// body off disk and handing it to the WebView as well. 150ms leaves the rendering itself an
/// order of magnitude more than it needs while still failing long before anybody would call it
/// "very very slow".
const BUDGET_MS: u128 = 150;

/// How much larger than its input the output may be.
///
/// Some growth is expected and correct: the sanitiser rewrites attributes, links get `rel` and
/// `target`, and the quote fold adds a wrapper. A tenfold expansion is not that — it is the
/// symptom that sent somebody looking, because every one of those bytes crosses the IPC boundary
/// and is then parsed by the WebView.
const MAX_GROWTH: usize = 3;

#[test]
fn a_real_newsletter_renders_inside_the_budget() {
    let inline = HashMap::new();
    let remote = HashMap::new();

    // Once to warm anything lazily initialised, then measured.
    let _ = render(Some(NEWSLETTER), None, &inline, false, &remote);

    let started = Instant::now();
    let rendered = render(Some(NEWSLETTER), None, &inline, false, &remote);
    let elapsed = started.elapsed().as_millis();

    println!(
        "render: {}KB in, {}KB out, {elapsed}ms",
        NEWSLETTER.len() / 1024,
        rendered.html.len() / 1024,
    );

    assert!(
        elapsed <= BUDGET_MS,
        "rendering one message took {elapsed}ms, over the {BUDGET_MS}ms budget"
    );
}

#[test]
fn rendering_does_not_multiply_the_message() {
    let inline = HashMap::new();
    let remote = HashMap::new();

    let rendered = render(Some(NEWSLETTER), None, &inline, false, &remote);
    let growth = rendered.html.len() / NEWSLETTER.len().max(1);

    assert!(
        growth < MAX_GROWTH,
        "{}KB became {}KB — {growth}x — and every byte of that crosses the IPC boundary",
        NEWSLETTER.len() / 1024,
        rendered.html.len() / 1024,
    );
}

#[test]
fn loading_remote_images_is_no_slower() {
    // The path most readers are on, since images load by default. Nothing here fetches — the
    // map is empty — so this measures the same work with the other branch taken.
    let inline = HashMap::new();
    let remote = HashMap::new();

    let started = Instant::now();
    let _ = render(Some(NEWSLETTER), None, &inline, true, &remote);
    let elapsed = started.elapsed().as_millis();

    assert!(
        elapsed <= BUDGET_MS,
        "rendering with images allowed took {elapsed}ms, over the {BUDGET_MS}ms budget"
    );
}
