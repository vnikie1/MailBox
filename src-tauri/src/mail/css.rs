//! Making a message's CSS safe to apply. docs/03 §6.
//!
//! `ammonia` sanitises *markup*. It passes CSS through untouched, and CSS is not inert: it can
//! make the renderer fetch things, and it can size a document against the frame it sits in.
//! This file is the filter for both places mail puts CSS — the `style` attribute and the
//! `<style>` element — so the two cannot drift apart.
//!
//! ## What it removes
//!
//! Whole declarations, never parts of them. A half-edited declaration is unpredictable, and a
//! dropped one is simply unstyled.
//!
//! * **Anything that loads.** `url()` pointing anywhere but an inline image, and every other way
//!   CSS names a resource: `image-set()` and `cross-fade()` take plain strings as URLs, which is
//!   how a filter that looks only for `url(` gets walked straight past; `@import` takes a string
//!   too. The frame's CSP refuses these loads regardless, and this module's parent says not to
//!   build a pipeline whose safety rests on the last step.
//! * **What once executed.** `expression()`, `javascript:`, `behavior`, `-moz-binding`. Dead in
//!   Chromium; a sanitiser that relies on the renderer's version is one with an expiry date.
//! * **Viewport-height units.** The frame is sized by measuring its content, so content sized
//!   in `vh` grows with the frame, and the frame grows with it — `height: 100vh` plus any margin
//!   is a document that gets taller every frame, for ever.
//! * **Colour-scheme queries.** Every `@media (prefers-color-scheme: …)` block. The frame paints a
//!   white card whatever the app's theme, because mail is written for white — and a frame reads
//!   `prefers-color-scheme` from the browser's preference, not from anything the app can set on
//!   the `<iframe>`. Measured, not assumed: `color-scheme: light` on the element changed nothing.
//!   Left in, a dark-mode block switches an email's text to white over the white card. Light
//!   blocks go too, so the same message looks the same in either theme: its own default design.
//! * **The app's own names.** Anything mentioning `halcyon`. The frame's guard rules live in a
//!   cascade layer called `halcyon-guard`, and layer names are global to the document: a
//!   message stylesheet that could close its own wrapper and write `@layer halcyon-guard {…}`
//!   would join the guard and outrank it. The same rule stops a message restyling the quote
//!   toggle and the data detectors the core adds.
//!
//! ## Why checks run on a normalised copy
//!
//! CSS escapes are decoded before a function name is matched, so `u\72 l(` *is* `url(`, and
//! `\40 import` is `@import`. Every check runs on the text with its escapes decoded and its case
//! folded, which is how the renderer reads it; comments are removed before that, as the
//! renderer removes them. Normalising can only ever make more things match, never fewer, so a
//! mistake here costs styling and not safety.
//!
//! ## The net under all of it
//!
//! Whatever survives is checked once more as a whole, and the lot is dropped if anything
//! dangerous remains. The splitting above is a heuristic tokeniser, not a CSS parser; the final
//! check is what makes its mistakes harmless.

/// Substrings that disqualify a declaration once its escapes are decoded and its case folded.
///
/// `image-set(` also catches `-webkit-image-set(`, and `cross-fade(` its prefixed form.
const FORBIDDEN: &[&str] = &[
    "expression(",
    "javascript:",
    "vbscript:",
    "@import",
    "behavior:",
    "-moz-binding",
    "image-set(",
    "cross-fade(",
    "image(",
    "src(",
    "halcyon",
];

/// Units measured against the frame's height, or that can be.
///
/// `vmin` and `vmax` because either can resolve to the height; the block and inline axis units
/// because a `writing-mode` turns either one vertical. `vw` is deliberately absent: the frame's
/// width is fixed by the reader pane and never follows the content, so it cannot feed back.
const VIEWPORT_HEIGHT_UNITS: &[&str] = &[
    "vh", "svh", "lvh", "dvh", "vb", "svb", "lvb", "dvb", "vi", "svi", "lvi", "dvi", "vmin",
    "svmin", "lvmin", "dvmin", "vmax", "svmax", "lvmax", "dvmax",
];

/// The largest stylesheet a message may bring, across all of its `<style>` elements.
///
/// Real newsletters carry 20–60 KB and the heaviest seen run past 150 KB of repeated media
/// queries. Beyond this the message is shown unstyled rather than filtered, because a
/// multi-megabyte stylesheet is either an attack on the filter or not mail.
pub const STYLESHEET_BUDGET: usize = 512 * 1024;

/// Filters the contents of a `style` attribute.
pub fn filter_declarations(style: &str) -> String {
    filter(style, false)
}

/// Filters the contents of a message's `<style>` elements.
///
/// The result never contains a `<`, so it cannot close the `<style>` element it is put in —
/// `</style` is the only way out of one, and a `<` is replaced by the escape `\3c ` that reads
/// as the same character inside a CSS string.
pub fn filter_stylesheet(css: &str) -> String {
    if css.len() > STYLESHEET_BUDGET {
        return String::new();
    }

    filter(css, true)
}

fn filter(css: &str, sheet: bool) -> String {
    let chars: Vec<char> = css.chars().collect();
    let mut out = String::with_capacity(css.len());
    let mut segment = String::new();
    let mut depth = 0usize;
    let mut at = 0;

    while at < chars.len() {
        let here = chars[at];

        // Comments go entirely, as the renderer drops them. Kept, they are how `u/**/rl(`
        // would reach a check that never sees the letters `url` side by side.
        if here == '/' && chars.get(at + 1) == Some(&'*') {
            at += 2;
            while at < chars.len() && !(chars[at] == '*' && chars.get(at + 1) == Some(&'/')) {
                at += 1;
            }
            at += 2;
            continue;
        }

        // `<!--` and `-->` around the whole sheet, which a generation of email templates wrote
        // to hide CSS from clients that did not understand it. The renderer ignores both. Left
        // in, the `<` would be escaped below into an identifier that silently swallows the
        // first rule of every one of those messages.
        if starts_with(&chars, at, "<!--") {
            at += 4;
            continue;
        }
        if starts_with(&chars, at, "-->") {
            at += 3;
            continue;
        }

        match here {
            '"' | '\'' => at = copy_string(&chars, at, &mut segment),
            // An escape is one unit: `\;` is a literal semicolon, not a separator.
            '\\' => {
                segment.push(here);
                if let Some(&next) = chars.get(at + 1) {
                    segment.push(next);
                    at += 2;
                } else {
                    at += 1;
                }
            }
            ';' | '{' | '}' => {
                let keep = !is_refused(&normalise(&segment));
                if keep {
                    out.push_str(&segment);
                }
                segment.clear();

                match here {
                    ';' if keep => out.push(';'),
                    '{' if sheet => {
                        out.push('{');
                        depth += 1;
                    }
                    // A `}` with nothing open is dropped rather than emitted: in the frame the
                    // sheet sits inside a wrapper, and an extra closer would end the wrapper.
                    '}' if sheet && depth > 0 => {
                        out.push('}');
                        depth -= 1;
                    }
                    _ => {}
                }

                at += 1;
            }
            _ if begins_url(&chars, at) => at = copy_url(&chars, at, &mut segment),
            _ => {
                segment.push(here);
                at += 1;
            }
        }
    }

    if !is_refused(&normalise(&segment)) {
        out.push_str(&segment);
    }

    // Closed rather than left open, so the wrapper the frame puts round it still ends where it
    // was meant to.
    if sheet {
        for _ in 0..depth {
            out.push('}');
        }
    }

    if is_refused(&normalise(&out)) {
        return String::new();
    }

    let out = out.replace('<', "\\3c ");

    // Inside an attribute a raw `>` is legal, but the core's own passes over the sanitised
    // markup find tags by their `<` and `>`, and one inside a style value ends the tag early for
    // them. In a declaration a `>` can only be inside a string, where the escape is identical.
    if sheet {
        out
    } else {
        out.replace('>', "\\3e ")
    }
}

/// Copies a quoted string, and returns where it ended.
///
/// Ends where the renderer ends it: at the matching quote, or at a raw newline, which makes it a
/// bad string that the renderer abandons. Agreeing on where strings end is what keeps the
/// separators outside them lined up with the renderer's.
fn copy_string(chars: &[char], start: usize, into: &mut String) -> usize {
    let quote = chars[start];
    into.push(quote);

    let mut at = start + 1;
    while at < chars.len() {
        let here = chars[at];
        into.push(here);
        at += 1;

        match here {
            '\\' => {
                if let Some(&next) = chars.get(at) {
                    into.push(next);
                    at += 1;
                }
            }
            '\n' => return at,
            _ if here == quote => return at,
            _ => {}
        }
    }

    at
}

/// Whether an unquoted `url(` begins here, as a token of its own.
fn begins_url(chars: &[char], at: usize) -> bool {
    let named = chars.len() >= at + 4
        && chars[at].eq_ignore_ascii_case(&'u')
        && chars[at + 1].eq_ignore_ascii_case(&'r')
        && chars[at + 2].eq_ignore_ascii_case(&'l')
        && chars[at + 3] == '(';

    let standalone = at == 0 || !is_ident_char(chars[at - 1]);

    named && standalone
}

/// Copies a `url(…)`, and returns where it ended.
///
/// Its own case because an unquoted URL may contain `;` — every inline image does, in
/// `data:image/png;base64,` — and splitting there would cut a picture in half.
fn copy_url(chars: &[char], start: usize, into: &mut String) -> usize {
    let mut at = start;
    for _ in 0..4 {
        into.push(chars[at]);
        at += 1;
    }

    // A quoted argument is an ordinary string, and the main loop already copies those.
    let mut probe = at;
    while probe < chars.len() && chars[probe].is_whitespace() {
        probe += 1;
    }
    if matches!(chars.get(probe), Some('"' | '\'')) {
        return at;
    }

    while at < chars.len() {
        let here = chars[at];
        into.push(here);
        at += 1;

        match here {
            '\\' => {
                if let Some(&next) = chars.get(at) {
                    into.push(next);
                    at += 1;
                }
            }
            ')' => return at,
            _ => {}
        }
    }

    at
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '\\' || !c.is_ascii()
}

fn starts_with(chars: &[char], at: usize, needle: &str) -> bool {
    needle
        .chars()
        .enumerate()
        .all(|(offset, want)| chars.get(at + offset) == Some(&want))
}

/// The text as the renderer reads it: escapes decoded, case folded.
fn normalise(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(here) = chars.next() {
        if here != '\\' {
            out.push(here);
            continue;
        }

        let mut hex = String::new();
        while hex.len() < 6 {
            match chars.peek() {
                Some(&digit) if digit.is_ascii_hexdigit() => {
                    hex.push(digit);
                    chars.next();
                }
                _ => break,
            }
        }

        if hex.is_empty() {
            // `\` and any other character is that character. A backslash before a newline is a
            // line continuation, which contributes nothing.
            if let Some(next) = chars.next() {
                if next != '\n' {
                    out.push(next);
                }
            }
            continue;
        }

        // One whitespace character after a hex escape belongs to the escape.
        if chars.peek().is_some_and(|next| next.is_whitespace()) {
            chars.next();
        }

        let decoded = u32::from_str_radix(&hex, 16)
            .ok()
            .and_then(char::from_u32)
            .filter(|decoded| *decoded != '\0')
            .unwrap_or('\u{FFFD}');
        out.push(decoded);
    }

    out.to_lowercase()
}

/// Whether a piece of CSS is kept out of the frame.
///
/// Three kinds of reason, and only the first is about safety. The other two are about the
/// frame: what it is sized by, and what colour it always is.
fn is_refused(normalised: &str) -> bool {
    FORBIDDEN.iter().any(|needle| normalised.contains(needle))
        || loads_a_url(normalised)
        || sized_against_the_viewport(normalised)
        || normalised.contains("prefers-color-scheme")
}

/// A `url(` whose argument is anything but an inline image.
fn loads_a_url(normalised: &str) -> bool {
    normalised.match_indices("url(").any(|(at, needle)| {
        let argument = normalised[at + needle.len()..]
            .trim_start()
            .trim_start_matches(['"', '\'']);

        !argument.starts_with("data:image/")
    })
}

/// A number carrying one of the viewport-height units.
fn sized_against_the_viewport(normalised: &str) -> bool {
    let bytes = normalised.as_bytes();
    let mut at = 0;

    while at < bytes.len() {
        if !bytes[at].is_ascii_digit() {
            at += 1;
            continue;
        }

        // The whole number, including a fraction and an exponent, so `1e2vh` is not read as the
        // number 1 in the unit `e2vh`.
        while at < bytes.len() && (bytes[at].is_ascii_digit() || bytes[at] == b'.') {
            at += 1;
        }
        if at < bytes.len() && bytes[at] == b'e' {
            let mut exponent = at + 1;
            if exponent < bytes.len() && matches!(bytes[exponent], b'+' | b'-') {
                exponent += 1;
            }
            if exponent < bytes.len() && bytes[exponent].is_ascii_digit() {
                at = exponent;
                while at < bytes.len() && bytes[at].is_ascii_digit() {
                    at += 1;
                }
            }
        }

        let unit_start = at;
        while at < bytes.len() && bytes[at].is_ascii_alphabetic() {
            at += 1;
        }

        if VIEWPORT_HEIGHT_UNITS.contains(&&normalised[unit_start..at]) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /* -------------------------------------------------------------- what must survive */

    #[test]
    fn a_stylesheet_that_loads_nothing_comes_through_whole() {
        // The Pi-hole report's own rules, which is the mail this was written for: its bar chart
        // is an empty span whose every visible property is here.
        let css = ".bar{display:inline-block;height:8px;background:#0071e3;border-radius:4px}\
                   td.n{text-align:right}th{background:#f5f5f7}";

        assert_eq!(filter_stylesheet(css), css);
    }

    #[test]
    fn an_inline_image_survives_its_own_semicolon() {
        // `data:image/png;base64,` has a `;` in it, and splitting there would keep half a
        // declaration and lose the picture.
        let css = ".logo{background:url(data:image/png;base64,iVBORw0KGgo=) no-repeat}";

        assert_eq!(filter_stylesheet(css), css);
    }

    #[test]
    fn media_queries_and_nesting_keep_their_shape() {
        let css = "@media (max-width:600px){.col{display:block;width:100%}}p{margin:0}";

        assert_eq!(filter_stylesheet(css), css);
    }

    #[test]
    fn the_comment_wrapper_old_templates_use_is_removed_and_the_first_rule_kept() {
        let filtered = filter_stylesheet("<!--\n.first{color:red}\n.second{color:blue}\n-->");

        assert!(filtered.contains(".first{color:red}"), "{filtered}");
        assert!(filtered.contains(".second{color:blue}"), "{filtered}");
        assert!(!filtered.contains('<'), "{filtered}");
    }

    #[test]
    fn the_width_of_the_viewport_is_allowed() {
        // The frame's width is set by the pane and never follows the content.
        let css = ".wide{width:100vw}";

        assert_eq!(filter_stylesheet(css), css);
    }

    /* -------------------------------------------------------------- what must not load */

    #[test]
    fn no_url_but_an_inline_image_survives() {
        for attack in [
            "body{background:url(https://evil.test/p.gif)}",
            "body{background:url('https://evil.test/p.gif')}",
            "body{background:url( \"https://evil.test/p.gif\" )}",
            "body{background:URL(https://evil.test/p.gif)}",
            "li{list-style:url(//evil.test/p.gif)}",
            "@font-face{font-family:x;src:url(https://evil.test/x.woff2)}",
            "a{cursor:url(https://evil.test/c.cur),auto}",
        ] {
            let filtered = filter_stylesheet(attack);
            assert!(!filtered.contains("evil.test"), "{attack} -> {filtered}");
        }
    }

    #[test]
    fn a_url_spelled_with_escapes_is_still_a_url() {
        // `u\72 l(` is `url(` once the renderer has decoded it. A filter that matched the raw
        // text would let this straight through.
        for attack in [
            r"body{background:u\72 l(https://evil.test/p.gif)}",
            r"body{background:\75 rl(https://evil.test/p.gif)}",
            r"body{background:u\rl(https://evil.test/p.gif)}",
        ] {
            let filtered = filter_stylesheet(attack);
            assert!(!filtered.contains("evil.test"), "{attack} -> {filtered}");
        }
    }

    #[test]
    fn the_functions_that_take_a_url_as_a_plain_string_are_removed() {
        // None of these contain the letters `url(`, which is the whole point of them here.
        for attack in [
            r#"body{background-image:image-set("https://evil.test/p.gif" 1x)}"#,
            r#"body{background-image:-webkit-image-set("https://evil.test/p.gif" 1x)}"#,
            r#"body{background-image:cross-fade("https://evil.test/a.png", red 50%)}"#,
            r#"body{background-image:image("https://evil.test/p.gif")}"#,
            r#"body{background-image:src("https://evil.test/p.gif")}"#,
        ] {
            let filtered = filter_stylesheet(attack);
            assert!(!filtered.contains("evil.test"), "{attack} -> {filtered}");
        }
    }

    #[test]
    fn imports_are_removed_however_they_are_written() {
        for attack in [
            "@import url(https://evil.test/a.css);p{color:red}",
            r#"@import "https://evil.test/a.css";p{color:red}"#,
            r"@\69mport 'https://evil.test/a.css';p{color:red}",
            r"@IMPORT 'https://evil.test/a.css';p{color:red}",
        ] {
            let filtered = filter_stylesheet(attack);
            assert!(!filtered.contains("evil.test"), "{attack} -> {filtered}");
            assert!(
                filtered.contains("p{color:red}"),
                "the rule after an import is its own rule and stays: {attack} -> {filtered}"
            );
        }
    }

    #[test]
    fn a_comment_cannot_hide_a_function_name() {
        let filtered = filter_stylesheet("body{background:u/**/rl(https://evil.test/p.gif)}");

        // The renderer reads this as two tokens, so it is not a url as written. Removing the
        // comment joins them into one that is — and the check runs on what is emitted, which is
        // the joined form, so it is caught either way.
        assert!(!filtered.contains("/*"), "{filtered}");
        assert!(!filtered.contains("evil.test"), "{filtered}");
    }

    #[test]
    fn what_once_executed_is_removed() {
        for attack in [
            "div{width:expression(alert(1))}",
            "div{background:url(javascript:alert(1))}",
            "div{behavior:url(#default#time2)}",
            "div{-moz-binding:url(https://evil.test/x.xml#x)}",
        ] {
            let filtered = filter_stylesheet(attack).to_lowercase();
            for needle in ["expression(", "javascript:", "behavior:", "-moz-binding"] {
                assert!(!filtered.contains(needle), "{attack} -> {filtered}");
            }
        }
    }

    /* ------------------------------------------------------------- what must not escape */

    #[test]
    fn a_stylesheet_can_never_close_its_style_element() {
        for attack in [
            r#"a{content:"</style><img src=x onerror=alert(1)>"}"#,
            "a{color:red}</style><script>alert(1)</script>",
            "a{font-family:'<'}",
        ] {
            let filtered = filter_stylesheet(attack);
            assert!(!filtered.contains('<'), "{attack} -> {filtered}");
        }
    }

    #[test]
    fn the_braces_always_balance() {
        for attack in [
            "a{color:red}}}}",
            "a{color:red",
            "}}} a{color:red}",
            "@media screen{a{color:red}",
        ] {
            let filtered = filter_stylesheet(attack);
            let opens = filtered.matches('{').count();
            let closes = filtered.matches('}').count();
            assert_eq!(opens, closes, "{attack} -> {filtered}");

            let mut depth = 0i32;
            for c in filtered.chars() {
                depth += match c {
                    '{' => 1,
                    '}' => -1,
                    _ => 0,
                };
                assert!(
                    depth >= 0,
                    "closed more than it opened: {attack} -> {filtered}"
                );
            }
        }
    }

    #[test]
    fn a_message_cannot_write_itself_into_the_frames_guard() {
        // Layer names are global to the document. If a message stylesheet could open
        // `@layer halcyon-guard`, its rules would sit in the same layer as the frame's own and
        // come after them — and win.
        for attack in [
            "} @layer halcyon-guard{html{overflow:auto!important}}",
            r"} @layer h\61lcyon-guard{html{overflow:auto!important}}",
            ".halcyon-quote-toggle{display:none}",
        ] {
            let filtered = filter_stylesheet(attack).to_lowercase();
            assert!(!filtered.contains("halcyon"), "{attack} -> {filtered}");
            assert!(!filtered.contains("h\\61"), "{attack} -> {filtered}");
        }
    }

    /* ------------------------------------------------------- what must not run away */

    #[test]
    fn nothing_is_sized_against_the_height_of_the_frame() {
        // The frame is measured to fit its content. Content in viewport-height units fits the
        // frame instead, and with any margin at all, the two chase each other upward for ever.
        for attack in [
            ".hero{min-height:100vh;padding:20px}",
            ".hero{height:calc(100vh - 10px)}",
            ".hero{height:50dvh}",
            ".hero{height:1e2vh}",
            ".hero{height:.5vmax}",
            "--tall:100vh",
        ] {
            let filtered = filter_stylesheet(attack);
            assert!(!filtered.contains("vh"), "{attack} -> {filtered}");
            assert!(!filtered.contains("vmax"), "{attack} -> {filtered}");
        }

        // The rest of the rule stays.
        assert!(filter_stylesheet(".hero{min-height:100vh;padding:20px}").contains("padding:20px"));
    }

    #[test]
    fn a_colour_scheme_query_is_dropped_and_the_default_design_kept() {
        // Chromium answers `prefers-color-scheme` inside the frame from the browser's own
        // preference, so in a dark app an email's dark block would switch on over the white card.
        let filtered = filter_stylesheet(
            "p{color:#1c1c1e}@media (prefers-color-scheme: dark){p{color:#fff}}\
             @media screen and (PREFERS-COLOR-SCHEME:light){p{color:#000}}",
        );

        assert!(filtered.contains("p{color:#1c1c1e}"), "{filtered}");
        assert!(
            !filtered.to_lowercase().contains("prefers-color-scheme"),
            "{filtered}"
        );
    }

    /* ---------------------------------------------------------------- style attributes */

    #[test]
    fn a_style_attribute_keeps_its_formatting_and_loses_its_loads() {
        let filtered =
            filter_declarations("color:#333;background:url(https://evil.test/p.gif);padding:8px");

        assert!(filtered.contains("color:#333"), "{filtered}");
        assert!(filtered.contains("padding:8px"), "{filtered}");
        assert!(!filtered.contains("evil.test"), "{filtered}");
    }

    #[test]
    fn a_style_attribute_cannot_break_a_tag_scan() {
        // The core's later passes find tags by `<` and `>`. A raw one in a style value would end
        // the tag early for them and hide the `src` that follows.
        let filtered = filter_declarations(r#"font-family:"a>b<c""#);

        assert!(!filtered.contains('>'), "{filtered}");
        assert!(!filtered.contains('<'), "{filtered}");
    }

    #[test]
    fn a_style_attribute_has_no_braces_to_balance() {
        let filtered = filter_declarations("color:red}a{color:blue");

        assert!(!filtered.contains('{'), "{filtered}");
        assert!(!filtered.contains('}'), "{filtered}");
    }

    #[test]
    fn an_oversized_stylesheet_is_dropped_rather_than_filtered() {
        let huge = "p{color:red}".repeat(STYLESHEET_BUDGET / 8);

        assert_eq!(filter_stylesheet(&huge), "");
    }
}
