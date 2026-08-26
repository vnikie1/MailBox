//! Making a message body safe to display. docs/03-architecture.md §6.
//!
//! Message bodies are **hostile input**, and this is the file that decides whether that
//! matters. Everything here exists because the alternative has been exploited in real mail
//! clients: `<script>` that exfiltrates the mailbox, `onerror=` on a broken image, a
//! `<meta http-equiv="refresh">` that navigates the app somewhere else, a remote image whose
//! only purpose is to tell the sender you opened it and from which IP.
//!
//! The order matters and is not arbitrary:
//!
//! 1. **Sanitise first**, with `ammonia`, which parses with html5ever rather than matching
//!    patterns — so it cannot be defeated by the malformed markup that defeats every
//!    regex-based stripper. Nothing downstream may assume it is looking at valid HTML.
//! 2. **Then rewrite images**, because the rewriting works on attributes that only exist
//!    once the parse has settled what the attributes actually are.
//!
//! Doing it the other way round would let `<img src=x onerror=...>` past the rewriter and
//! rely on the sanitiser to catch the handler — which it would, but building a pipeline whose
//! safety depends on the *last* step rather than the first is how a later change breaks it.

use std::collections::{HashMap, HashSet};

use ammonia::Builder;

/// A `src` that has been withheld pending the user's consent. docs/03 §6.3.
///
/// Not a real scheme, and deliberately not fetchable: the CSP on the frame does not include
/// it, so even if something did try to load it the browser refuses.
const BLOCKED: &str = "blocked:remote";

/// The result of preparing one message for display.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Rendered {
    /// Sanitised HTML, safe to put in the sandboxed frame.
    pub html: String,
    /// How many remote images were withheld. Drives the "Load Remote Content" banner — a
    /// count, so the banner can say what it is offering rather than appearing unexplained.
    #[ts(type = "number")]
    pub blocked_remote: u32,
    /// How many `cid:` parts were inlined from the local cache.
    #[ts(type = "number")]
    pub inlined: u32,
    /// True when the message had no HTML part and this is its plain text, wrapped.
    pub from_plain_text: bool,
}

/// Tags that may never appear, whatever else is allowed. docs/03 §6.2.
fn forbidden_tags() -> HashSet<&'static str> {
    [
        "script", "iframe", "object", "embed", "form", "base", "link", "meta",
    ]
    .into_iter()
    .collect()
}

/// URL schemes a link or image may use.
///
/// `cid:` is here because the rewriter below turns those into `data:` *after* sanitisation —
/// stripping them first would lose the reference and the image with it.
///
/// `data:` is here **only** so that the attribute filter below gets a chance to see it.
/// Ammonia drops unknown schemes before any filter runs, so excluding it here would remove
/// legitimate inline images along with the dangerous ones. The filter is what enforces
/// docs/03 §6.2's actual rule: `data:` for images, nowhere else.
fn allowed_schemes() -> HashSet<&'static str> {
    ["http", "https", "mailto", "cid", "tel", "data"]
        .into_iter()
        .collect()
}

/// Enforces "`data:` URLs are for image sources and nothing else". docs/03 §6.2.
///
/// A `data:text/html` in an `href` is a whole document under the sender's control, one click
/// away. A `data:image/png` in an `<img src>` is a picture. The scheme allow-list cannot tell
/// them apart, so this does.
fn filter_data_urls(
    element: &str,
    attribute: &str,
    value: &str,
) -> Option<std::borrow::Cow<'static, str>> {
    if attribute == "style" {
        return Some(sanitise_style(value).into());
    }

    let lowered = value.trim_start().to_ascii_lowercase();

    if !lowered.starts_with("data:") {
        return Some(value.to_string().into());
    }

    if element == "img" && attribute == "src" && lowered.starts_with("data:image/") {
        return Some(value.to_string().into());
    }

    None
}

/// Strips the dangerous parts of an inline `style` attribute.
///
/// `ammonia` sanitises *markup*; it passes CSS through untouched, and CSS is not inert. Three
/// things matter here, and the third is the one that bites in practice:
///
/// * `expression()` — script execution in old IE, and the WebView is Chromium, but a
///   sanitiser that relies on the renderer's version is a sanitiser with an expiry date.
/// * `javascript:` inside `url()`.
/// * **`url(https://…)` as a tracking pixel.** `background-image` loads a remote resource
///   exactly like `<img>` does, and blocking `<img src>` while leaving CSS alone would mean
///   the "remote content blocked" banner was telling the user something untrue.
///
/// The frame's CSP would refuse these loads anyway. That is the point of defence in depth,
/// and this module's own doc comment says not to build a pipeline whose safety rests on the
/// last step — so it should not rest on the CSP either.
fn sanitise_style(style: &str) -> String {
    let lowered = style.to_ascii_lowercase();

    let dangerous = [
        "expression(",
        "javascript:",
        "@import",
        "behavior:",
        "-moz-binding",
    ];

    if dangerous.iter().any(|needle| lowered.contains(needle)) {
        return String::new();
    }

    // Any `url()` that is not an inline image. Declarations are dropped whole rather than
    // having the url edited out, because a half-removed declaration is unpredictable.
    if lowered.contains("url(") {
        return style
            .split(';')
            .filter(|declaration| {
                let lowered = declaration.to_ascii_lowercase();

                !lowered.contains("url(") || lowered.contains("url(data:image/")
            })
            .collect::<Vec<_>>()
            .join(";");
    }

    style.to_string()
}

/// Sanitises message HTML.
///
/// `ammonia` is allow-list based: anything not named is removed. That is the right default
/// for hostile input — a deny-list is a list of the attacks someone has thought of.
fn sanitise(html: &str) -> String {
    let mut builder = Builder::default();

    builder
        .rm_tags(forbidden_tags())
        // Every `on*` handler. Ammonia's default allow-list has no event handlers in it, but
        // saying so explicitly means a future change to that default cannot quietly
        // reintroduce them.
        .rm_tag_attributes("*", ["onclick", "onerror", "onload", "onmouseover"])
        .url_schemes(allowed_schemes())
        // Links leave the app. docs/03 §6.6 — never in the WebView.
        .link_rel(Some("noopener noreferrer nofollow"))
        // Inline styles are how mail is laid out; without them every message is a wall of
        // unformatted text. The CSP on the frame is what makes them safe: no scripting, and
        // no loading of anything the rewriter has not already resolved.
        .add_generic_attributes(["style", "class", "align", "valign", "bgcolor"])
        // `id` on the two elements Outlook marks its quote boundary with, and nowhere else.
        // Without it `divRplyFwdMsg` is stripped before the quote folder ever sees it, and
        // every reply from Outlook shows its full history — which is most business mail.
        //
        // Safe because the frame is inert: no script to collide with, and the only thing an
        // `id` can do on its own is be a link target within the message. Kept to `div` and
        // `hr` rather than allowed generically so it stays a fix for a named problem.
        .add_tag_attributes("div", ["id"])
        .add_tag_attributes("hr", ["id"])
        .add_tag_attributes("img", ["src", "alt", "width", "height", "title"])
        .add_tag_attributes("table", ["cellpadding", "cellspacing", "border", "width"])
        .add_tag_attributes("td", ["colspan", "rowspan", "width", "height"])
        .add_tag_attributes("th", ["colspan", "rowspan", "width", "height"])
        .attribute_filter(|element, attribute, value| filter_data_urls(element, attribute, value));

    builder.clean(html).to_string()
}

/// Rewrites `<img src>` after sanitisation.
///
/// A hand-written pass over the sanitised markup rather than another parse. It is safe here
/// precisely *because* the input has already been through html5ever: the attribute syntax is
/// normalised, so a naive scan cannot be tricked by the kind of malformed markup that would
/// defeat it on raw input. On raw input this function would be a vulnerability.
fn rewrite_images(
    html: &str,
    inline: &HashMap<String, String>,
    load_remote: bool,
    remote: &HashMap<String, String>,
) -> (String, u32, u32) {
    let mut out = String::with_capacity(html.len());
    let mut blocked = 0u32;
    let mut inlined = 0u32;
    let mut rest = html;

    while let Some(position) = rest.find("src=\"") {
        let (before, after) = rest.split_at(position + 5);
        out.push_str(before);

        let Some(end) = after.find('"') else {
            // No closing quote: not something html5ever would emit, so stop rewriting and
            // pass the remainder through rather than guessing.
            out.push_str(after);
            return (out, blocked, inlined);
        };

        let url = &after[..end];

        if let Some(reference) = url.strip_prefix("cid:") {
            // docs/03 §6.4 — `cid:` resolves from the local cache **only**. A reference we do
            // not hold becomes a blocked placeholder rather than a request to anywhere.
            match inline.get(reference.trim()) {
                Some(data_uri) => {
                    out.push_str(data_uri);
                    inlined += 1;
                }
                None => {
                    out.push_str(BLOCKED);
                }
            }
        } else if url.starts_with("http://") || url.starts_with("https://") {
            match (load_remote, remote.get(url)) {
                // Loaded through the core, which is what keeps the sender from seeing the
                // user's IP. docs/03 §6.3.
                (true, Some(data_uri)) => out.push_str(data_uri),
                (true, None) => {
                    // Asked for but could not be fetched. Blocked rather than left as a live
                    // URL — otherwise "load remote content" would quietly become "let the
                    // frame make the request itself", which is the thing being prevented.
                    out.push_str(BLOCKED);
                    blocked += 1;
                }
                (false, _) => {
                    out.push_str(BLOCKED);
                    blocked += 1;
                }
            }
        } else if url.starts_with("data:image/") {
            // Already inline and already an image. docs/03 §6.2 permits exactly this.
            out.push_str(url);
        } else {
            out.push_str(BLOCKED);
        }

        rest = &after[end..];
    }

    out.push_str(rest);
    (out, blocked, inlined)
}

/// Sanitised HTML with `src` attributes untouched.
///
/// Used only to enumerate which remote images a message wants, so that the fetch list is
/// built from markup the sanitiser has already approved. Building it from the raw HTML would
/// mean fetching URLs the sanitiser had decided to drop — the app going to a server on behalf
/// of markup it refused to render.
pub fn sanitise_for_enumeration(html: &str) -> String {
    sanitise(html)
}

/// Collects every remote image URL a sanitised body wants.
///
/// Used to fetch them through the core when the user asks for them, so that the frame never
/// makes a request of its own.
pub fn remote_urls(html: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut rest = html;

    while let Some(position) = rest.find("src=\"") {
        let after = &rest[position + 5..];

        let Some(end) = after.find('"') else { break };
        let url = &after[..end];

        if url.starts_with("http://") || url.starts_with("https://") {
            let url = url.to_string();
            if !urls.contains(&url) {
                urls.push(url);
            }
        }

        rest = &after[end..];
    }

    urls
}

/// HTML void elements — they have no closing tag, so they must not change nesting depth.
///
/// Getting this list wrong is not cosmetic here: the depth counter below decides where it is
/// safe to cut the document, and a `<br>` counted as an open tag would push everything after
/// it to a depth it never returns from, so no top-level boundary would ever be found again.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Whether an element at the top level begins the quoted reply.
///
/// Every mail client marks its quotes, and no two mark them the same way. These are the
/// markers the four clients most people use actually emit; anything unrecognised simply stays
/// visible, which is the right way to be wrong.
fn begins_quote(tag: &str, open_tag: &str) -> bool {
    let lowered = open_tag.to_ascii_lowercase();

    // Apple Mail, Thunderbird and most standards-following clients: a bare top-level
    // blockquote *is* the quote.
    if tag == "blockquote" {
        return true;
    }

    // Gmail wraps in `<div class="gmail_quote">`; Yahoo uses `yahoo_quoted`; Thunderbird
    // labels its attribution line `moz-cite-prefix`.
    if lowered.contains("gmail_quote")
        || lowered.contains("yahoo_quoted")
        || lowered.contains("moz-cite-prefix")
    {
        return true;
    }

    // Outlook marks the divider before a forwarded or replied-to message by id.
    lowered.contains("divrplyfwdmsg") || lowered.contains("appendonsend")
}

/// Splits sanitised HTML into the new message and the quoted reply below it.
///
/// Returns `None` when there is no quote, or when the quote is the whole message — collapsing
/// everything would leave the reader looking at an empty card and a disclosure triangle.
///
/// **The cut is only ever made at the top level.** The quoted part is wrapped in a `<details>`
/// afterwards, and cutting inside an open element would put the closing tags of that element
/// inside the wrapper and its opening tag outside — mangling the message rather than folding
/// it. Tracking depth is what makes the split safe, and it is safe to track depth by scanning
/// because this runs on *sanitised* markup: html5ever has already balanced the tags, so what
/// is scanned here is well-formed by construction. On raw input none of this would hold.
fn split_quoted(html: &str) -> Option<(String, String)> {
    let bytes = html.as_bytes();
    let mut depth = 0usize;
    let mut index = 0usize;
    let mut cut: Option<usize> = None;

    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }

        let Some(end) = html[index..].find('>').map(|offset| index + offset) else {
            break;
        };

        let open_tag = &html[index..=end];
        let inner = open_tag.trim_start_matches('<').trim_end_matches('>');
        let closing = inner.starts_with('/');
        let name: String = inner
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();

        if name.is_empty() {
            // A comment or a doctype. Neither nests, so neither moves the depth.
            index = end + 1;
            continue;
        }

        if closing {
            depth = depth.saturating_sub(1);
        } else if !VOID_ELEMENTS.contains(&name.as_str()) && !inner.ends_with('/') {
            // The check happens before the depth increases, so it only ever fires on an
            // element that starts at the top level.
            if depth == 0 && cut.is_none() && begins_quote(&name, open_tag) {
                cut = Some(index);
            }

            depth += 1;
        } else if depth == 0 && cut.is_none() && begins_quote(&name, open_tag) {
            cut = Some(index);
        }

        index = end + 1;
    }

    let cut = cut?;

    let visible = html[..cut].to_string();
    let quoted = html[cut..].to_string();

    // "Is there anything left?" measured on the text, not the markup: a reply whose visible
    // part is three empty divs and a signature separator has nothing above the quote, and
    // hiding the only content in the message is worse than showing the quote.
    if strip_tags(&visible).trim().is_empty() || strip_tags(&quoted).trim().is_empty() {
        return None;
    }

    Some((visible, quoted))
}

/// The text content of some markup, for "is there anything here" questions.
fn strip_tags(html: &str) -> String {
    let mut text = String::new();
    let mut inside = false;

    for character in html.chars() {
        match character {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => text.push(character),
            _ => {}
        }
    }

    text
}

/// Folds the quoted reply into a `<details>` the reader can open.
///
/// `<details>` rather than a class the app toggles from outside, because the frame runs no
/// script and never will (docs/03 §6.1, standing rule 11). It is the one interactive control
/// in HTML that needs none, which makes it exactly the right tool here: the message stays
/// completely inert and the quote still folds.
fn fold_quote(html: &str) -> String {
    match split_quoted(html) {
        Some((visible, quoted)) => format!(
            "{visible}<details class=\"halcyon-quote\">\
             <summary class=\"halcyon-quote-toggle\">Show quoted text</summary>\
             <div class=\"halcyon-quote-body\">{quoted}</div></details>"
        ),
        None => html.to_string(),
    }
}

/// Wraps plain text as HTML, for messages with no HTML part.
///
/// Escaped, not sanitised: the input is text, and treating it as markup would turn a message
/// that happens to contain `<b>` into bold rather than showing what the sender typed.
fn from_plain(text: &str) -> String {
    let escape = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };

    match split_plain_quote(text) {
        Some((visible, quoted)) => format!(
            "<pre class=\"halcyon-plain\">{}</pre>\
             <details class=\"halcyon-quote\">\
             <summary class=\"halcyon-quote-toggle\">Show quoted text</summary>\
             <pre class=\"halcyon-plain halcyon-quote-body\">{}</pre></details>",
            escape(visible.trim_end()),
            escape(&quoted)
        ),
        None => format!("<pre class=\"halcyon-plain\">{}</pre>", escape(text)),
    }
}

/// Finds where a plain-text reply stops being new and starts being quoted.
///
/// Two markers, in the order they appear. The attribution line ("On Tuesday, X wrote:") is
/// preferred when both are present, because it belongs to the quote rather than to the reply —
/// splitting below it would leave a dangling half-sentence at the bottom of the visible part.
fn split_plain_quote(text: &str) -> Option<(&str, String)> {
    let lines: Vec<&str> = text.lines().collect();

    let attribution = lines.iter().position(|line| {
        let trimmed = line.trim();
        // Deliberately narrow. "On" and "wrote:" on one line is the near-universal form; a
        // looser rule would fold ordinary sentences that happen to contain the word "wrote".
        (trimmed.starts_with("On ") && trimmed.ends_with("wrote:"))
            || trimmed.starts_with("-----Original Message-----")
            || trimmed.starts_with("________________________________")
    });

    // A run of quoted lines that continues to the end of the message. Anything shorter is
    // someone quoting a line mid-reply to answer it, which must stay where it is.
    let quoted_run = lines
        .iter()
        .position(|line| line.trim_start().starts_with('>'));
    let quoted_run = quoted_run.filter(|start| {
        lines[*start..]
            .iter()
            .all(|line| line.trim().is_empty() || line.trim_start().starts_with('>'))
    });

    let cut = match (attribution, quoted_run) {
        (Some(a), Some(q)) => a.min(q),
        (Some(a), None) => a,
        (None, Some(q)) => q,
        (None, None) => return None,
    };

    if cut == 0 {
        return None;
    }

    let visible_end: usize = lines[..cut].iter().map(|line| line.len() + 1).sum();
    let visible = text.get(..visible_end.min(text.len()))?;

    if visible.trim().is_empty() {
        return None;
    }

    Some((visible, lines[cut..].join("\n")))
}

/// Prepares a message for the reader.
pub fn render(
    html: Option<&str>,
    plain: Option<&str>,
    inline: &HashMap<String, String>,
    load_remote: bool,
    remote: &HashMap<String, String>,
) -> Rendered {
    let Some(html) = html.filter(|value| !value.trim().is_empty()) else {
        // Nothing stored at all. Distinct from "a message whose text happens to be empty":
        // bodies are fetched lazily *after* selection (docs/03 §5), so this is the ordinary
        // state of every message for the second or two before its body lands.
        //
        // It must return an empty string rather than an empty `<pre>` wrapper. The reader
        // decides between "here is the message" and "this one is still downloading" by
        // asking whether there is any HTML, and a 33-byte wrapper answers yes — which put a
        // blank white card on screen for the whole of the download instead of a line of
        // text saying what was happening.
        let Some(plain) = plain.filter(|value| !value.trim().is_empty()) else {
            return Rendered::default();
        };

        return Rendered {
            html: from_plain(plain),
            from_plain_text: true,
            ..Rendered::default()
        };
    };

    let clean = sanitise(html);
    let (rewritten, blocked_remote, inlined) = rewrite_images(&clean, inline, load_remote, remote);

    // Folded *after* the images are rewritten, so the counts in the banner describe the whole
    // message rather than only the part that is showing. A quote full of blocked images is
    // still a message with blocked images.
    let folded = fold_quote(&rewritten);

    Rendered {
        html: folded,
        blocked_remote,
        inlined,
        from_plain_text: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_html(html: &str) -> Rendered {
        render(Some(html), None, &HashMap::new(), false, &HashMap::new())
    }

    /* ------------------------------------------------------------------ the hard rules */

    #[test]
    fn script_tags_do_not_survive() {
        // The whole reason this file exists. A script in a message body runs with whatever
        // the frame can reach, and the frame is inside a mail client.
        let rendered = render_html("<p>hi</p><script>alert('x')</script>");

        assert!(!rendered.html.contains("<script"));
        assert!(!rendered.html.contains("alert"));
        assert!(rendered.html.contains("hi"));
    }

    #[test]
    fn event_handlers_do_not_survive() {
        // `onerror` on an image that cannot load is the classic delivery vehicle, because it
        // fires without the user doing anything at all.
        let rendered = render_html(
            r#"<img src="x" onerror="alert(1)"><p onclick="alert(2)">text</p><body onload="x()">"#,
        );

        assert!(!rendered.html.contains("onerror"), "{}", rendered.html);
        assert!(!rendered.html.contains("onclick"), "{}", rendered.html);
        assert!(!rendered.html.contains("onload"), "{}", rendered.html);
        assert!(!rendered.html.contains("alert"), "{}", rendered.html);
    }

    #[test]
    fn frames_objects_forms_and_meta_refresh_do_not_survive() {
        // docs/03 §6.2 names each of these. A form posts the user's mailbox somewhere; a
        // meta refresh navigates the app.
        let rendered = render_html(
            r#"<iframe src="https://x.test"></iframe>
               <object data="x"></object>
               <embed src="x">
               <form action="https://x.test"><input name="a"></form>
               <meta http-equiv="refresh" content="0;url=https://x.test">
               <p>kept</p>"#,
        );

        for forbidden in [
            "<iframe",
            "<object",
            "<embed",
            "<form",
            "<meta",
            "http-equiv",
        ] {
            assert!(
                !rendered.html.contains(forbidden),
                "{forbidden}: {}",
                rendered.html
            );
        }
        assert!(rendered.html.contains("kept"));
    }

    #[test]
    fn javascript_urls_do_not_survive() {
        let rendered = render_html(r#"<a href="javascript:alert(1)">click</a>"#);

        assert!(!rendered.html.contains("javascript:"), "{}", rendered.html);
        // The text stays — removing the link must not remove the sentence around it.
        assert!(rendered.html.contains("click"));
    }

    #[test]
    fn a_non_image_data_url_does_not_survive() {
        // docs/03 §6.2 permits `data:` for images only. `data:text/html` is a whole document
        // under the sender's control.
        let rendered = render_html(
            r#"<img src="data:text/html;base64,PHNjcmlwdD4="><a href="data:text/html,x">l</a>"#,
        );

        assert!(
            !rendered.html.contains("data:text/html"),
            "{}",
            rendered.html
        );
    }

    #[test]
    fn a_data_image_url_is_allowed_through() {
        let source = "data:image/png;base64,iVBORw0KGgo=";
        let rendered = render_html(&format!(r#"<img src="{source}">"#));

        assert!(rendered.html.contains(source), "{}", rendered.html);
        assert_eq!(rendered.blocked_remote, 0);
    }

    /* ---------------------------------------------------------------- remote content */

    #[test]
    fn remote_images_are_blocked_by_default_and_counted() {
        // docs/03 §6.3. A remote image is a read receipt the sender did not ask permission
        // for, and it carries the user's IP address with it.
        let rendered = render_html(
            r#"<img src="https://tracker.test/pixel.gif"><img src="http://other.test/a.png">"#,
        );

        assert!(!rendered.html.contains("tracker.test"), "{}", rendered.html);
        assert!(!rendered.html.contains("other.test"), "{}", rendered.html);
        assert_eq!(rendered.blocked_remote, 2, "the banner needs the count");
    }

    #[test]
    fn remote_images_load_from_the_core_when_asked_for() {
        // Fetched by Rust and handed over as data, so the frame never makes the request and
        // the sender never sees the user's address.
        let mut remote = HashMap::new();
        remote.insert(
            "https://cdn.test/logo.png".to_string(),
            "data:image/png;base64,AAAA".to_string(),
        );

        let rendered = render(
            Some(r#"<img src="https://cdn.test/logo.png">"#),
            None,
            &HashMap::new(),
            true,
            &remote,
        );

        assert!(rendered.html.contains("data:image/png;base64,AAAA"));
        assert!(
            !rendered.html.contains("cdn.test"),
            "the URL must not remain"
        );
        assert_eq!(rendered.blocked_remote, 0);
    }

    #[test]
    fn a_remote_image_that_could_not_be_fetched_stays_blocked() {
        // "Load remote content" must not degrade into "let the frame fetch it itself", which
        // is precisely the thing being prevented.
        let rendered = render(
            Some(r#"<img src="https://cdn.test/gone.png">"#),
            None,
            &HashMap::new(),
            true,
            &HashMap::new(),
        );

        assert!(rendered.html.contains(BLOCKED));
        assert!(!rendered.html.contains("cdn.test"));
        assert_eq!(rendered.blocked_remote, 1);
    }

    #[test]
    fn the_remote_urls_of_a_body_can_be_listed_for_fetching() {
        let urls = remote_urls(
            r#"<img src="https://a.test/1.png"><img src="https://b.test/2.png"><img src="cid:x">"#,
        );

        assert_eq!(urls, vec!["https://a.test/1.png", "https://b.test/2.png"]);
    }

    #[test]
    fn a_repeated_remote_url_is_listed_once() {
        // A newsletter uses the same spacer gif forty times. Fetching it forty times is
        // forty requests announcing the same thing.
        let urls =
            remote_urls(r#"<img src="https://a.test/x.gif"><img src="https://a.test/x.gif">"#);

        assert_eq!(urls.len(), 1);
    }

    /* -------------------------------------------------------------------- cid: images */

    #[test]
    fn inline_images_resolve_from_the_local_cache() {
        // docs/03 §6.4 — from the cache **only**. This is what makes an embedded signature
        // image or an inline screenshot show up without a single network request.
        let mut inline = HashMap::new();
        inline.insert(
            "logo@example".to_string(),
            "data:image/png;base64,BBBB".to_string(),
        );

        let rendered = render(
            Some(r#"<img src="cid:logo@example">"#),
            None,
            &inline,
            false,
            &HashMap::new(),
        );

        assert!(rendered.html.contains("data:image/png;base64,BBBB"));
        assert_eq!(rendered.inlined, 1);
        assert_eq!(rendered.blocked_remote, 0, "an inline image is not remote");
    }

    #[test]
    fn a_cid_reference_we_do_not_hold_becomes_a_placeholder_not_a_request() {
        let rendered = render_html(r#"<img src="cid:missing@example">"#);

        assert!(rendered.html.contains(BLOCKED));
        assert!(!rendered.html.contains("cid:missing"));
        assert_eq!(rendered.inlined, 0);
    }

    /* ------------------------------------------------------------ formatting survives */

    #[test]
    fn the_formatting_that_makes_mail_readable_survives() {
        // A sanitiser that strips styling turns every message into a wall of text, which is
        // its own kind of broken. The CSP is what makes keeping this safe.
        let rendered = render_html(
            r##"<table width="600"><tr><td style="padding:8px" bgcolor="#eee">
               <p style="font-size:16px"><b>Bold</b> and <i>italic</i></p>
               <a href="https://example.test/x">a link</a></td></tr></table>"##,
        );

        assert!(rendered.html.contains("<table"), "{}", rendered.html);
        assert!(rendered.html.contains("style="), "{}", rendered.html);
        assert!(rendered.html.contains("<b>"), "{}", rendered.html);
        assert!(
            rendered.html.contains("https://example.test/x"),
            "{}",
            rendered.html
        );
    }

    #[test]
    fn css_cannot_smuggle_script_or_a_tracking_pixel() {
        // Found by this file's own hostile-markup test. `ammonia` sanitises markup and passes
        // CSS through untouched — so `background:url(https://tracker)` would have been a
        // tracking pixel that the "remote content blocked" banner said nothing about, which
        // makes the banner a lie rather than merely incomplete.
        let tracking =
            render_html(r#"<div style="background:url(https://tracker.test/p.gif)">seen</div>"#);
        assert!(!tracking.html.contains("tracker.test"), "{}", tracking.html);
        assert!(
            tracking.html.contains("seen"),
            "the content stays: {}",
            tracking.html
        );

        for attack in [
            r#"<div style="background:url(javascript:alert(1))">x</div>"#,
            r#"<div style="width:expression(alert(1))">x</div>"#,
            r#"<div style="@import 'https://evil.test/a.css'">x</div>"#,
            r#"<div style="behavior:url(#default#time2)">x</div>"#,
        ] {
            let rendered = render_html(attack).html.to_lowercase();

            assert!(!rendered.contains("javascript:"), "{attack}: {rendered}");
            assert!(!rendered.contains("expression("), "{attack}: {rendered}");
            assert!(!rendered.contains("@import"), "{attack}: {rendered}");
            assert!(!rendered.contains("behavior:"), "{attack}: {rendered}");
        }
    }

    #[test]
    fn ordinary_styling_is_not_thrown_away_by_the_css_filter() {
        // Over-stripping turns every message into unformatted text, which is its own
        // failure. Only declarations that load something or execute something go.
        let rendered =
            render_html(r#"<div style="color:#333;font-size:14px;padding:8px">styled</div>"#);

        assert!(rendered.html.contains("color"), "{}", rendered.html);
        assert!(rendered.html.contains("font-size"), "{}", rendered.html);

        // An inline image in CSS is fine — it loads nothing.
        let inline_bg =
            render_html(r#"<div style="background:url(data:image/png;base64,AAAA)">x</div>"#);
        assert!(
            inline_bg.html.contains("data:image/png"),
            "{}",
            inline_bg.html
        );
    }

    #[test]
    fn links_carry_noopener_and_noreferrer() {
        // They open in the default browser, and the sender learns nothing about where the
        // click came from.
        let rendered = render_html(r#"<a href="https://example.test">x</a>"#);

        assert!(rendered.html.contains("noopener"), "{}", rendered.html);
        assert!(rendered.html.contains("noreferrer"), "{}", rendered.html);
    }

    /* ------------------------------------------------------------------- plain text */

    #[test]
    fn a_message_with_no_html_is_wrapped_rather_than_left_blank() {
        let rendered = render(
            None,
            Some("Just some text.\n\nSecond para."),
            &HashMap::new(),
            false,
            &HashMap::new(),
        );

        assert!(rendered.from_plain_text);
        assert!(rendered.html.contains("Just some text."));
        assert!(rendered.html.contains("halcyon-plain"));
    }

    #[test]
    fn plain_text_is_escaped_not_interpreted() {
        // A message that happens to contain "<b>" is showing you those characters, not
        // asking for bold — and "<script>" in a text part must not become one.
        let rendered = render(
            None,
            Some("<script>alert(1)</script> and <b>not bold</b>"),
            &HashMap::new(),
            false,
            &HashMap::new(),
        );

        assert!(!rendered.html.contains("<script"), "{}", rendered.html);
        assert!(!rendered.html.contains("<b>"), "{}", rendered.html);
        assert!(
            rendered.html.contains("&lt;script&gt;"),
            "{}",
            rendered.html
        );
    }

    #[test]
    fn an_empty_html_part_falls_back_to_the_text_part() {
        // Senders do produce an empty HTML alternative. Rendering nothing would show a blank
        // message beside a subject line, which reads as the app failing.
        let rendered = render(
            Some("   "),
            Some("the real content"),
            &HashMap::new(),
            false,
            &HashMap::new(),
        );

        assert!(rendered.from_plain_text);
        assert!(rendered.html.contains("the real content"));
    }

    /* ---------------------------------------------------------------- hostile input */

    #[test]
    fn malformed_markup_does_not_defeat_the_sanitiser() {
        // The reason docs/03 §6.2 names a parser rather than "strip these strings": every
        // one of these gets past a regex, and none gets past html5ever.
        let attacks = [
            r#"<scr<script>ipt>alert(1)</script>"#,
            "<img src=\"x\" onerror
=\"alert(1)\">",
            r#"<IMG SRC=JaVaScRiPt:alert(1)>"#,
            r#"<a href="  javascript:alert(1)">x</a>"#,
            r#"<svg/onload=alert(1)>"#,
            r#"<div style="background:url(javascript:alert(1))">x</div>"#,
            r#"<!--<script>alert(1)</script>-->"#,
            r#"<p>unclosed"#,
        ];

        for attack in attacks {
            let rendered = render_html(attack).html.to_lowercase();

            // The test is that nothing *executable* survives, not that the letters "alert"
            // are absent. `<scr<script>ipt>alert(1)` correctly comes out as the escaped text
            // "ipt&gt;alert(1)" — visible, inert, and exactly what a sanitiser should do with
            // markup it has taken apart. Asserting on the string would have made this test
            // fail on the sanitiser working.
            assert!(
                !rendered.contains("<script"),
                "script tag survived {attack}: {rendered}"
            );
            assert!(
                !rendered.contains("javascript:"),
                "js url survived {attack}: {rendered}"
            );
            assert!(
                !rendered.contains("onerror"),
                "handler survived {attack}: {rendered}"
            );
            assert!(
                !rendered.contains("onload"),
                "handler survived {attack}: {rendered}"
            );
        }
    }

    /* ------------------------------------------------------------------ quoted replies */

    #[test]
    fn a_quoted_reply_is_folded_behind_a_disclosure() {
        let rendered =
            render_html("<p>My answer.</p><blockquote><p>Their question.</p></blockquote>");

        assert!(rendered.html.contains("halcyon-quote"), "{}", rendered.html);
        assert!(rendered.html.contains("<details"), "{}", rendered.html);

        // Folded, not discarded — the quote is still there for anyone who opens it.
        assert!(rendered.html.contains("Their question."));

        // And the reply itself is above the fold, where it belongs.
        let fold = rendered.html.find("<details").expect("fold");
        let answer = rendered.html.find("My answer.").expect("answer");
        assert!(answer < fold, "the reply must be above the fold");
    }

    #[test]
    fn the_common_clients_quote_markers_are_all_recognised() {
        // Every client marks its quotes and no two do it alike. A marker we miss is a wall of
        // quoted text in the reader; there is no harm in a marker we do not need.
        let markers = [
            r#"<div class="gmail_quote"><p>old</p></div>"#,
            r#"<div class="yahoo_quoted"><p>old</p></div>"#,
            r#"<div class="moz-cite-prefix">On x wrote:</div><blockquote>old</blockquote>"#,
            r#"<div id="divRplyFwdMsg"><p>old</p></div>"#,
            "<blockquote><p>old</p></blockquote>",
        ];

        for markup in markers {
            let rendered = render_html(&format!("<p>new</p>{markup}"));
            assert!(
                rendered.html.contains("halcyon-quote"),
                "not folded: {markup} -> {}",
                rendered.html
            );
        }
    }

    #[test]
    fn a_message_that_is_only_a_quote_is_left_alone() {
        // Folding everything leaves the reader looking at an empty card and a triangle, which
        // reads as the message having failed to load.
        let rendered = render_html("<blockquote><p>Only the quote.</p></blockquote>");

        assert!(!rendered.html.contains("<details"), "{}", rendered.html);
        assert!(rendered.html.contains("Only the quote."));
    }

    #[test]
    fn a_quote_nested_inside_the_reply_does_not_split_the_markup() {
        // The important one. Cutting inside an open element would put that element's closing
        // tag inside the <details> and its opening tag outside, mangling the message. The
        // split is only ever made at the top level, so a blockquote inside a div is not a
        // split point.
        let rendered =
            render_html("<div><p>Answer</p><blockquote><p>Question</p></blockquote></div>");

        assert!(
            !rendered.html.contains("<details"),
            "must not cut inside an open element: {}",
            rendered.html
        );

        // Nothing was lost by declining to fold.
        assert!(rendered.html.contains("Answer"));
        assert!(rendered.html.contains("Question"));
    }

    #[test]
    fn a_void_element_does_not_confuse_the_depth_counter() {
        // `<br>` and `<img>` have no closing tag. Counting them as open would push everything
        // after them to a depth that never returns to zero, so no quote would ever be folded
        // again — a silent failure that only shows up on real mail.
        let rendered = render_html(
            "<p>Answer</p><br><hr><img src=\"data:image/gif;base64,R0lGOD\">\
             <blockquote><p>Question</p></blockquote>",
        );

        assert!(rendered.html.contains("<details"), "{}", rendered.html);
    }

    #[test]
    fn plain_text_quotes_fold_on_the_attribution_line() {
        let rendered = render(
            None,
            Some("Yes, that works.\n\nOn Tuesday, Ada wrote:\n> Does this work?\n"),
            &HashMap::new(),
            false,
            &HashMap::new(),
        );

        assert!(rendered.from_plain_text);
        assert!(rendered.html.contains("halcyon-quote"), "{}", rendered.html);
        assert!(rendered.html.contains("Yes, that works."));

        // The attribution belongs to the quote, not the reply: splitting below it would leave
        // "On Tuesday, Ada wrote:" dangling under the visible text with nothing after it.
        let fold = rendered.html.find("<details").expect("fold");
        let attribution = rendered.html.find("On Tuesday").expect("attribution");
        assert!(attribution > fold, "{}", rendered.html);
    }

    #[test]
    fn a_quoted_line_answered_inline_is_not_folded() {
        // Someone quoting one line to reply underneath it. The quote is not a trailing block,
        // and folding from the first ">" would hide the reply that follows it.
        let rendered = render(
            None,
            Some("> Does this work?\nYes.\n> And this?\nAlso yes.\n"),
            &HashMap::new(),
            false,
            &HashMap::new(),
        );

        assert!(!rendered.html.contains("<details"), "{}", rendered.html);
        assert!(rendered.html.contains("Also yes."));
    }

    #[test]
    fn plain_text_in_a_fold_is_still_escaped() {
        // The fold moves text into a second <pre>; the escaping must move with it. A quote
        // containing markup is extremely ordinary — it is usually the previous HTML mail.
        let rendered = render(
            None,
            Some("Reply.\n\nOn Tuesday, Ada wrote:\n> <script>alert(1)</script>\n"),
            &HashMap::new(),
            false,
            &HashMap::new(),
        );

        assert!(!rendered.html.contains("<script"), "{}", rendered.html);
        assert!(
            rendered.html.contains("&lt;script&gt;"),
            "{}",
            rendered.html
        );
    }

    #[test]
    fn a_body_that_has_not_been_downloaded_yet_renders_to_nothing_at_all() {
        // Not a cosmetic point. Bodies arrive lazily after selection, so between the click
        // and the download every message passes through this state. The reader tells "still
        // downloading" apart from "here it is" by asking whether there is any HTML, so
        // anything non-empty here — even an empty `<pre>` wrapper — puts a blank white card
        // on screen instead of a line of text saying what is happening.
        for (html, plain) in [(None, None), (Some(""), None), (Some("   "), Some("\n \t"))] {
            let rendered = render(html, plain, &HashMap::new(), false, &HashMap::new());

            assert!(
                rendered.html.is_empty(),
                "rendered {:?} bytes for html={html:?} plain={plain:?}",
                rendered.html.len()
            );
        }
    }

    #[test]
    fn a_very_large_body_is_handled() {
        // Newsletters really are megabytes of nested tables.
        let huge = format!("<div>{}</div>", "<p>text</p>".repeat(50_000));

        let rendered = render_html(&huge);
        assert!(rendered.html.len() > 1000);
    }
}
