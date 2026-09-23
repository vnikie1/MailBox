//! Mail whose look lives in a stylesheet, through the function the reader calls.
//!
//! Reported from using the app: a Pi-hole daily report showed its numbers but not its bar chart,
//! and none of its layout. The report's chart is a column of empty `<span class='bar'>` elements
//! with a width each — everything that makes a bar visible is a `.bar` rule in a `<style>` block
//! in the head, and every `<style>` block was being thrown away.
//!
//! The fixture is the report's shape, not the report: its stylesheet is reproduced rule for rule,
//! and its devices are invented, because the real one names every machine on somebody's network.

use std::collections::HashMap;

use halcyon_lib::mail::render::render;

const REPORT: &str = include_str!("fixtures/stylesheet-report.html");

fn rendered() -> halcyon_lib::mail::render::Rendered {
    render(Some(REPORT), None, &HashMap::new(), false, &HashMap::new())
}

#[test]
fn the_chart_keeps_what_draws_it() {
    let rendered = rendered();

    // The rule that gives an empty span a size and a colour.
    assert!(
        rendered
            .css
            .contains(".bar{display:inline-block;height:8px;background:#0071e3;border-radius:4px"),
        "{}",
        rendered.css
    );

    // And what the rule hangs on in the body: the class, and each bar's own width.
    for width in ["width:120px", "width:80px", "width:40px"] {
        assert!(rendered.html.contains(width), "{width}: {}", rendered.html);
    }
    assert!(rendered.html.contains("class=\"bar\""), "{}", rendered.html);
}

#[test]
fn the_rest_of_the_layout_comes_with_it() {
    let css = rendered().css;

    for rule in ["body{", ".card{", "th{", "td.n{", "h2{", ".sub{"] {
        assert!(css.contains(rule), "{rule} is missing from: {css}");
    }
}

#[test]
fn the_body_still_carries_no_stylesheet() {
    // The stylesheet travels beside the HTML, never in it. The XSS corpus forbids `<style` in
    // the body outright, and this is the document shape most likely to put one back.
    let rendered = rendered();

    assert!(!rendered.html.contains("<style"), "{}", rendered.html);
    assert!(!rendered.html.contains("inline-block"), "{}", rendered.html);
    assert!(
        rendered.html.contains("Per device"),
        "the message itself is missing"
    );
}
