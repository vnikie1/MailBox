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
//! ## Why the images are real now
//!
//! Every test in this file used to pass `HashMap::new()` as the remote map — including the one
//! named `loading_remote_images_is_no_slower`, whose own comment admitted "nothing here fetches".
//! So the file asserted a threefold growth ceiling while excluding the only thing that actually
//! makes a message grow: a remote image is replaced by a base64 data URI four thirds its size,
//! and the budget could not see a single one of them.
//!
//! It was not a theoretical gap. The largest render this install has logged is **12.98 MB out of
//! a 76 KB stored message** — a 170-fold expansion, entirely from inlined images, under a test
//! that claimed to cap growth at three. The tests below now substitute images the way the app
//! does and bound the result against what was actually put in.
//!
//! ## The fixture
//!
//! A real marketing newsletter out of a real mailbox: 53KB, 709 tags, 148 inline `style`
//! attributes, 32 links, and eight remote images. Exactly the shape that is slow, and exactly
//! the shape most mail is.

use std::collections::HashMap;
use std::time::Instant;

use halcyon_lib::mail::render::{remote_urls, render, sanitise_for_enumeration};

const NEWSLETTER: &str = include_str!("fixtures/slow-newsletter.html");

/// What one message may cost to render.
///
/// docs/06 Phase 3 budgets 100ms for opening a message, and that budget has to cover reading the
/// body off disk and handing it to the WebView as well. 150ms leaves the rendering itself an
/// order of magnitude more than it needs while still failing long before anybody would call it
/// "very very slow".
const BUDGET_MS: u128 = 150;

/// How much larger than its input the output may be, before any image is substituted.
///
/// Some growth is expected and correct: the sanitiser rewrites attributes, links get `rel` and
/// `target`, and the quote fold adds a wrapper. A tenfold expansion is not that — it is the
/// symptom that sent somebody looking, because every one of those bytes crosses the IPC boundary
/// and is then parsed by the WebView.
const MAX_GROWTH: usize = 3;

/// A stand-in image, at the mean size of this install's own remote cache (126 KB encoded).
///
/// The bytes are uniform because nothing here decodes them — what is being measured is how many
/// of them the renderer moves, and how many times.
fn stand_in_image() -> String {
    format!("data:image/png;base64,{}", "A".repeat(126 * 1024))
}

/// The remote map the app would build for this message.
///
/// Enumerated exactly as `ipc::body` enumerates it — from the *sanitised* markup, not the raw
/// HTML — so the keys here are the keys `render` will look up. Building the map any other way
/// would produce a test that silently substitutes nothing, which is the failure this file is
/// recovering from.
fn remote_images() -> HashMap<String, String> {
    remote_urls(&sanitise_for_enumeration(NEWSLETTER))
        .into_iter()
        .map(|url| (url, stand_in_image()))
        .collect()
}

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

/// The test that used to be vacuous, and the one that matters.
#[test]
fn inlining_images_costs_the_images_and_nothing_more() {
    let inline = HashMap::new();
    let remote = remote_images();

    // Guards the test against itself. With an empty map every assertion below passes for the
    // wrong reason, which is exactly how this file spent its life claiming to bound growth.
    assert!(
        !remote.is_empty(),
        "the fixture is supposed to reference remote images; if it stopped, this test stopped \
         testing anything"
    );

    let substituted: usize = remote.values().map(String::len).sum();
    let rendered = render(Some(NEWSLETTER), None, &inline, true, &remote);

    assert_eq!(
        rendered.loaded_remote as usize,
        remote.len(),
        "every image in the map should have been substituted — if this is short, the keys do not \
         match what `render` looks up and the rest of this test is measuring nothing"
    );
    assert_eq!(rendered.failed_remote, 0);
    assert_eq!(rendered.blocked_remote, 0);

    // The document may grow by the images it was given, plus whatever the sanitiser costs. What
    // it may not do is carry any image more than once: the first version of this expansion put
    // a copy in the map, a copy in the rewritten HTML and a copy in the response, and the only
    // way to see that is to count the bytes against what went in.
    let ceiling = NEWSLETTER.len() * MAX_GROWTH + substituted;

    assert!(
        rendered.html.len() <= ceiling,
        "{}KB of message and {}KB of images became {}KB — more than the sum of its parts, so \
         something is being copied twice",
        NEWSLETTER.len() / 1024,
        substituted / 1024,
        rendered.html.len() / 1024,
    );
}

#[test]
fn loading_remote_images_is_no_slower() {
    // The path most readers are on, since images load by default — and with the images actually
    // present, which is the difference between measuring this path and merely taking its branch.
    let inline = HashMap::new();
    let remote = remote_images();

    let _ = render(Some(NEWSLETTER), None, &inline, true, &remote);

    let started = Instant::now();
    let rendered = render(Some(NEWSLETTER), None, &inline, true, &remote);
    let elapsed = started.elapsed().as_millis();

    println!(
        "render with images: {}KB in, {}KB out, {elapsed}ms",
        NEWSLETTER.len() / 1024,
        rendered.html.len() / 1024,
    );

    assert!(
        elapsed <= BUDGET_MS,
        "rendering with {} images inlined took {elapsed}ms, over the {BUDGET_MS}ms budget",
        remote.len()
    );
}
