//! Serving a message body to the reader. docs/03 §4, docs/03 §6.
//!
//! The only route by which message HTML reaches the WebView, and it is sanitised on the way
//! through every time. There is deliberately no command that returns the raw stored HTML:
//! the sanitiser is not something a caller can forget to apply, because there is nothing
//! else to call.

use std::collections::HashMap;
use std::time::Duration;

use base64::Engine;
use mailparse::MailHeaderMap;
use tauri::State;

use crate::db::Db;
use crate::mail::render::{self, Rendered};

use super::mail::AppError;

/// Ceiling on one remote image.
///
/// A sender controls the size of what they link to. Without a cap, "load remote content"
/// invites them to hand back a gigabyte.
const MAX_REMOTE_BYTES: u64 = 8 * 1024 * 1024;

/// Ceiling on how many remote images one message may pull.
///
/// A newsletter with four hundred images is four hundred requests; the point of proxying is
/// that the core decides how much of that actually happens.
const MAX_REMOTE_IMAGES: usize = 60;

const REMOTE_TIMEOUT: Duration = Duration::from_secs(10);

/// Builds `cid:` → data URI from the cached `.eml`.
///
/// Read from the cache, never from the network — docs/03 §6.4. The `.eml` is already on disk
/// because the body fetch put it there, so an embedded signature image or an inline
/// screenshot costs nothing and reveals nothing.
fn inline_images(raw_path: Option<&str>) -> HashMap<String, String> {
    let mut images = HashMap::new();

    let Some(path) = raw_path else {
        return images;
    };

    let Ok(raw) = std::fs::read(path) else {
        return images;
    };

    let Ok(parsed) = mailparse::parse_mail(&raw) else {
        return images;
    };

    collect_parts(&parsed, 0, &mut images);
    images
}

fn collect_parts(
    part: &mailparse::ParsedMail<'_>,
    depth: usize,
    out: &mut HashMap<String, String>,
) {
    // The same depth cap as the body parser, for the same reason: a message can nest as
    // deeply as its sender likes.
    if depth > 32 {
        return;
    }

    for child in &part.subparts {
        collect_parts(child, depth + 1, out);
    }

    let mime = part.ctype.mimetype.to_ascii_lowercase();
    if !mime.starts_with("image/") {
        return;
    }

    let Some(content_id) = part.headers.get_first_value("Content-ID") else {
        return;
    };

    let reference = content_id
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_string();

    if reference.is_empty() {
        return;
    }

    let Ok(bytes) = part.get_body_raw() else {
        return;
    };

    // An inline image larger than this is not a signature logo; it is an attachment that
    // happens to be marked inline, and inlining it into the HTML would put megabytes through
    // the IPC boundary as base64.
    if bytes.len() > 2 * 1024 * 1024 {
        return;
    }

    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    out.insert(reference, format!("data:{mime};base64,{encoded}"));
}

/// Fetches the remote images a message wants, through the core.
///
/// docs/03 §6.3 asks to *proxy through the Rust core so the sender never sees the user's IP,
/// and strip the `Referer`.* **The second half of that is achieved and the first half is not,
/// and the spec is wrong about it.** The Rust core runs on the user's own machine, so a request
/// it makes leaves from the user's own address. Nothing here is a proxy in the sense that word
/// implies; making one would mean routing a user's mail through a server this project does not
/// have and standing rule 16 would not allow.
///
/// What fetching here *does* buy, which is real and worth having:
///
/// * **No cookies.** A store would let one sender's pixel identify the reader to the next.
/// * **No `Referer`**, so the request does not name the message that triggered it.
/// * **A generic `User-Agent`**, naming no client, no version and no machine.
/// * **The frame never makes the request itself.** It gets `data:` URIs, so there is no
///   network access from the document at all — which is what keeps a malicious message from
///   correlating anything or reaching a host the sanitiser refused.
///
/// So the sender learns that the message was opened, roughly when, and the IP it was opened
/// from. They do not learn which client, which message, or anything that persists between
/// senders. The user-facing copy in Settings says exactly this, because a promise of anonymity
/// that is not kept is worse than no promise.
async fn fetch_remote(urls: &[String]) -> HashMap<String, String> {
    let mut fetched = HashMap::new();

    let Ok(client) = reqwest::Client::builder()
        .timeout(REMOTE_TIMEOUT)
        // No cookie store, and a generic agent. A client that carried cookies would let one
        // sender's pixel identify the reader to the next.
        .user_agent("Mozilla/5.0")
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
    else {
        return fetched;
    };

    for url in urls.iter().take(MAX_REMOTE_IMAGES) {
        let Ok(response) = client.get(url).send().await else {
            continue;
        };

        if !response.status().is_success() {
            continue;
        }

        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .split(';')
            .next()
            .unwrap_or("application/octet-stream")
            .trim()
            .to_ascii_lowercase();

        // Only images. A server that answers an `<img src>` with HTML is not serving an
        // image, and turning that into a data: URI would put the sender's markup back into
        // the document the sanitiser just cleaned.
        if !mime.starts_with("image/") {
            continue;
        }

        if response.content_length().unwrap_or(0) > MAX_REMOTE_BYTES {
            continue;
        }

        let Ok(bytes) = response.bytes().await else {
            continue;
        };

        if bytes.len() as u64 > MAX_REMOTE_BYTES {
            continue;
        }

        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        fetched.insert(url.clone(), format!("data:{mime};base64,{encoded}"));
    }

    fetched
}

/// Returns a message's body, ready for the frame.
///
/// `load_remote` says whether remote images may be fetched for this message. It comes from the
/// `reader.loadRemoteImages` setting, which the user owns, and can be overridden per message
/// from the banner in either direction.
///
/// **This is the one thing in the app whose default was chosen against the security advice.**
/// A remote image is the standard read receipt nobody consented to: a unique URL per recipient
/// tells the sender the message was opened, when, how often, and roughly from where. Standing
/// rule 11 had it blocked by default for that reason. The owner of this app asked for it on,
/// which is their call to make about their own mail, and the setting is theirs to change back.
/// Everything else rule 11 requires is unchanged — the sandboxed frame, no scripting, and the
/// Rust-side sanitiser all still apply.
#[tauri::command]
pub async fn message_body(
    db: State<'_, Db>,
    message_id: i64,
    load_remote: bool,
) -> Result<Rendered, AppError> {
    let stored: Option<(Option<String>, Option<String>, Option<String>)> = db
        .read(move |conn| {
            let row = conn
                .query_row(
                    "SELECT body_html, body_text, raw_path FROM message WHERE id = ?1",
                    rusqlite::params![message_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .ok();

            Ok(row)
        })
        .await?;

    let Some((html, text, raw_path)) = stored else {
        tracing::warn!(message_id, "message_body: no row for this id");
        return Ok(Rendered::default());
    };

    tracing::info!(
        message_id,
        html_len = html.as_deref().map(str::len).unwrap_or(0),
        text_len = text.as_deref().map(str::len).unwrap_or(0),
        "message_body: loaded"
    );

    let inline = inline_images(raw_path.as_deref());

    // Sanitise once to discover what the message wants, fetch that, then render with what
    // came back. The first pass is what guarantees the URL list contains only things that
    // survived sanitisation — collecting them from the raw HTML would mean fetching URLs the
    // sanitiser had already decided to drop.
    let remote = if load_remote {
        // Enumerated from the *sanitised* markup, not the raw stored HTML: the fetch list
        // must contain only URLs the sanitiser approved, or the app would make a request on
        // behalf of markup it had already refused to render.
        let wanted = render::remote_urls(&render::sanitise_for_enumeration(
            html.as_deref().unwrap_or_default(),
        ));

        fetch_remote(&wanted).await
    } else {
        HashMap::new()
    };

    let rendered = render::render(
        html.as_deref(),
        text.as_deref(),
        &inline,
        load_remote,
        &remote,
    );

    tracing::info!(
        message_id,
        out_len = rendered.html.len(),
        blocked = rendered.blocked_remote,
        inlined = rendered.inlined,
        plain = rendered.from_plain_text,
        "message_body: rendered"
    );

    Ok(rendered)
}

/// Opens a link from a message in the default browser. docs/03 §6.6.
///
/// Never in the WebView: a link opened there has no address bar, so the user cannot see where
/// they have been taken — which is the entire mechanism of a phishing page.
///
/// `visible_text` is what the message *displayed* as the link. When that looks like a URL and
/// its host differs from where the link actually goes, the caller is told to confirm first.
/// That is the oldest trick in mail and the cheapest to catch.
/// Whether a URL from a message may be handed to the shell at all.
///
/// A short allow-list. A message must not be able to launch an arbitrary scheme handler —
/// `ms-msdt:`, `search-ms:` and friends have all been used to run code from a link, and the
/// shell will happily resolve any scheme something on the machine has registered.
///
/// `tel:` is permitted **only in the reduced form the data detectors produce**: `+` and
/// digits, nothing else. The reader never passes a sender's `tel:` through — it builds one
/// from digits it recognised — so a `tel:` carrying anything else did not come from the
/// detector and has no business being opened.
fn scheme_permitted(target: &str) -> bool {
    if target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
    {
        return true;
    }

    match target.strip_prefix("tel:") {
        Some(number) => {
            !number.is_empty() && number.chars().all(|c| c.is_ascii_digit() || c == '+')
        }
        None => false,
    }
}

#[tauri::command]
pub async fn open_external(url: String, visible_text: String) -> Result<LinkOutcome, AppError> {
    let target = url.trim();

    if !scheme_permitted(target) {
        return Ok(LinkOutcome {
            opened: false,
            mismatch: None,
        });
    }

    if let Some(claimed) = mismatched_host(target, &visible_text) {
        return Ok(LinkOutcome {
            opened: false,
            mismatch: Some(claimed),
        });
    }

    open_in_browser(target)?;

    Ok(LinkOutcome {
        opened: true,
        mismatch: None,
    })
}

/// Opens a link the user has confirmed despite the host mismatch.
#[tauri::command]
pub async fn open_external_confirmed(url: String) -> Result<(), AppError> {
    let target = url.trim();

    if !(target.starts_with("http://") || target.starts_with("https://")) {
        return Ok(());
    }

    open_in_browser(target)
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct LinkOutcome {
    pub opened: bool,
    /// Set when the visible text claimed a different host from the real destination. Carries
    /// both so the confirmation can show them side by side.
    pub mismatch: Option<HostMismatch>,
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct HostMismatch {
    pub shown: String,
    pub actual: String,
    pub url: String,
}

/// The host a string appears to name, if it looks like a URL at all.
fn host_of(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches(['.', ',', ')', '>']);

    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else if trimmed.starts_with("www.") || (trimmed.contains('.') && !trimmed.contains(' ')) {
        format!("https://{trimmed}")
    } else {
        return None;
    };

    let parsed = url::Url::parse(&candidate).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();

    // A bare "example" is not a host, and neither is a sentence with a full stop in it.
    if !host.contains('.') {
        return None;
    }

    Some(host.trim_start_matches("www.").to_string())
}

/// Whether the link text claims one destination and the href goes to another.
fn mismatched_host(url: &str, visible_text: &str) -> Option<HostMismatch> {
    let shown = host_of(visible_text)?;
    let actual = host_of(url)?;

    if shown == actual {
        return None;
    }

    Some(HostMismatch {
        shown,
        actual,
        url: url.to_string(),
    })
}

/// `ShellExecuteW` with no verb honours the user's default browser.
fn open_in_browser(url: &str) -> Result<(), AppError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide: Vec<u16> = std::ffi::OsStr::new(url)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR::null(),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };

    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(AppError {
            code: "browser".into(),
            message: "Halcyon could not open your browser.".into(),
        })
    }
}

/// Whether remote images load without being asked for. docs/01 §5.
///
/// Defaults to **on**, at the owner's request. See `message_body` for what that costs and what
/// it does not change.
#[tauri::command]
pub async fn remote_images_enabled(db: State<'_, Db>) -> Result<bool, AppError> {
    let stored: Option<String> = db
        .read(|conn| {
            Ok(conn
                .query_row(
                    "SELECT value FROM setting WHERE key = 'reader.loadRemoteImages'",
                    [],
                    |row| row.get(0),
                )
                .ok())
        })
        .await?;

    // Absent means on. A fresh install gets the default without needing a row written first,
    // and a database from before this setting existed behaves the same as a new one.
    Ok(stored.is_none_or(|value| value == "1" || value.eq_ignore_ascii_case("true")))
}

#[tauri::command]
pub async fn set_remote_images_enabled(db: State<'_, Db>, enabled: bool) -> Result<(), AppError> {
    db.write(move |tx| {
        tx.execute(
            "INSERT INTO setting (key, value) VALUES ('reader.loadRemoteImages', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![if enabled { "1" } else { "0" }],
        )?;
        Ok(())
    })
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_whose_text_matches_its_destination_opens_without_a_prompt() {
        assert!(mismatched_host("https://example.test/path", "https://example.test").is_none());
        assert!(mismatched_host("https://example.test/path", "example.test").is_none());
        assert!(mismatched_host("https://www.example.test/x", "example.test").is_none());
    }

    #[test]
    fn a_message_cannot_launch_an_arbitrary_scheme_handler() {
        // Each of these resolves to something on a Windows machine, and several have been
        // used to run code from a link. The allow-list is the only thing standing between a
        // message body and the shell.
        for hostile in [
            "ms-msdt:/id PCWDiagnostic",
            "search-ms:query=x",
            "file:///C:/Windows/System32/cmd.exe",
            "javascript:alert(1)",
            "vbscript:msgbox",
            "data:text/html,<script>alert(1)</script>",
            "ms-officecmd:{}",
            "\\\\evil.test\\share",
        ] {
            assert!(!scheme_permitted(hostile), "{hostile}");
        }
    }

    #[test]
    fn only_the_detectors_reduced_tel_form_is_permitted() {
        // The detector hands over `+` and digits. Anything else in a `tel:` did not come from
        // it, and `tel:` is only on the list because of it.
        assert!(scheme_permitted("tel:+442079460958"));
        assert!(scheme_permitted("tel:02079460958"));

        assert!(!scheme_permitted("tel:"));
        assert!(!scheme_permitted("tel:+44 20 7946 0958"));
        assert!(!scheme_permitted("tel:*99#"));
        assert!(!scheme_permitted("tel:x@evil.test"));
    }

    #[test]
    fn the_ordinary_schemes_still_open() {
        assert!(scheme_permitted("https://example.test"));
        assert!(scheme_permitted("http://example.test"));
        assert!(scheme_permitted("mailto:ada@example.test"));
    }

    #[test]
    fn link_text_that_names_another_host_is_caught() {
        // The oldest trick in mail: the text says one bank, the href goes somewhere else.
        let caught =
            mismatched_host("https://evil.test/login", "https://bank.test").expect("caught");

        assert_eq!(caught.shown, "bank.test");
        assert_eq!(caught.actual, "evil.test");
    }

    #[test]
    fn ordinary_link_text_is_not_treated_as_a_claim() {
        // Most links read "Click here" or "Unsubscribe". Prompting on those would train the
        // user to dismiss the warning, which is worse than not warning at all.
        for text in [
            "Click here",
            "Unsubscribe",
            "View in browser",
            "Read more",
            "",
        ] {
            assert!(
                mismatched_host("https://example.test/x", text).is_none(),
                "prompted on {text:?}"
            );
        }
    }

    #[test]
    fn a_subdomain_of_the_shown_host_still_prompts() {
        // "example.test" shown, "example.test.evil.test" actual is the classic near-miss.
        let caught = mismatched_host("https://example.test.evil.test/x", "example.test");

        assert!(caught.is_some());
    }

    #[test]
    fn hosts_are_compared_case_insensitively_and_without_www() {
        assert!(mismatched_host("https://EXAMPLE.test/x", "www.example.test").is_none());
    }

    #[tokio::test]
    async fn a_scheme_that_is_not_web_or_mail_is_refused() {
        // `ms-msdt:` and `search-ms:` have both been used to run code from a link. A message
        // must not be able to launch an arbitrary handler on the user's machine.
        for url in [
            "ms-msdt:/id PCWDiagnostic",
            "file:///C:/Windows/System32/cmd.exe",
            "javascript:alert(1)",
            "search-ms:query=x",
        ] {
            let outcome = open_external(url.to_string(), String::new())
                .await
                .expect("no error");

            assert!(!outcome.opened, "{url} must not open");
            assert!(outcome.mismatch.is_none());
        }
    }
}
