//! Serving a message body to the reader. docs/03 §4, docs/03 §6.
//!
//! The only route by which message HTML reaches the WebView, and it is sanitised on the way
//! through every time. There is deliberately no command that returns the raw stored HTML:
//! the sanitiser is not something a caller can forget to apply, because there is nothing
//! else to call.

use std::collections::HashMap;
use std::time::Duration;

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

/// Ceiling on the **total** a message's remote images may add, as they will appear in the
/// document.
///
/// The per-image and per-count caps do not bound the thing that matters. Sixty images of eight
/// mebibytes each is 480 MiB, and every byte of it is base64'd at 4/3 and then crosses the IPC
/// boundary into a string the WebView parses. That is not hypothetical: the largest render this
/// install has logged is **12.98 MB from a 76 KB stored message**, all of the difference being
/// inlined images, and nothing in the code would have stopped it at ten times that.
///
/// Measured against the same log, the p90 rendered body is 3.16 MB, so eight mebibytes leaves
/// the overwhelming majority of real mail untouched and clips only the tail that was never
/// going to be read as a document anyway.
///
/// Counted on the encoded string rather than the fetched bytes, because that is what is
/// actually held: the data URI is what goes in the map, in the HTML, over IPC and into the DOM.
const MAX_REMOTE_TOTAL_BYTES: usize = 8 * 1024 * 1024;

const REMOTE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a message's remote images may hold the message back, all together.
///
/// The body is handed to the reader only when its images are in, and each request had ten
/// seconds of its own: sixty images six at a time was a hundred seconds of a blank reader for a
/// sender with a slow server (L35, 2026-11-14 audit). What has not arrived by now is left out,
/// and the reader says so as it does for any image that did not arrive.
const REMOTE_DEADLINE: Duration = Duration::from_secs(12);

/// How long to wait for a connection, apart from the response.
const REMOTE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How many remote images to fetch at once.
///
/// They were fetched **one after another**, and that is what "the content loading is very very
/// slow" turned out to be: a newsletter with thirty-two images made thirty-two round trips in
/// series before a single word appeared, because the body is not handed over until they are all
/// in. On the message this was measured against that came to 1.9 seconds of waiting.
///
/// Six, which is what a browser allows per host and comfortably below what a CDN would treat as
/// a flood. The images in one message usually share a host, so that is the number that applies.
const REMOTE_CONCURRENCY: usize = 6;

/// How long a fetched image stays in the on-disk cache.
///
/// Re-opening a message re-fetched every image every time, so the second read of a newsletter
/// cost exactly as much as the first — and told the sender it had been opened again. Caching is
/// faster *and* quieter: the tracking pixel fires once rather than once per read.
const REMOTE_CACHE_DAYS: u64 = 30;

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

    let Ok(parsed) = crate::mail::mime::parse(&raw) else {
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

    // Through the one builder that checks the type. `mime` is the sender's Content-Type, and
    // `image/png" style="…` passes a `starts_with("image/")` test.
    if let Some(data_uri) = render::image_data_uri(&mime, &bytes) {
        out.insert(reference, data_uri);
    }
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
/// The HTTP client, built once.
///
/// Rebuilding it per message threw away the connection pool and the DNS cache, so every image
/// paid for a fresh TCP and TLS handshake even when thirty of them shared one host.
///
/// No cookie store, and a generic agent. A client that carried cookies would let one sender's
/// pixel identify the reader to the next.
///
/// And only the public internet. A message's `<img src>` could name `http://192.168.1.1/…` or
/// `http://localhost:8080/…`, and this would fetch it from inside the user's network — a request
/// to their router or to a service on their own machine, made by a stranger's mail (L35,
/// 2026-11-14 audit). Addresses are checked in the URL, after every redirect, and in what a name
/// resolves to, which is what stops a hostname that points at the LAN.
fn remote_client() -> Option<&'static reqwest::Client> {
    static CLIENT: std::sync::OnceLock<Option<reqwest::Client>> = std::sync::OnceLock::new();

    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(REMOTE_TIMEOUT)
                .connect_timeout(REMOTE_CONNECT_TIMEOUT)
                .user_agent("Mozilla/5.0")
                .dns_resolver(std::sync::Arc::new(PublicOnly))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= 3 {
                        attempt.error("too many redirects")
                    } else if url_allowed(attempt.url()) {
                        attempt.follow()
                    } else {
                        attempt.stop()
                    }
                }))
                .build()
                .ok()
        })
        .as_ref()
}

/// Whether an address is on the public internet: not this machine, not the local network, and
/// not one of the ranges set aside for something else.
fn is_public(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;

    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || a == 0
                // Carrier-grade NAT, 100.64.0.0/10: the provider's network, not the internet.
                || (a == 100 && (64..128).contains(&b))
                // Benchmarking, 198.18.0.0/15, and everything from 240.0.0.0 up.
                || (a == 198 && (b == 18 || b == 19))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // Unique local, fc00::/7, and link-local, fe80::/10. Written out because the
                // standard library's names for them are newer than this crate's minimum Rust.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // Documentation, 2001:db8::/32.
                || (v6.segments()[0] == 0x2001 && v6.segments()[1] == 0x0db8))
        }
    }
}

/// Whether a remote image's URL may be fetched at all: web addresses only, and none naming this
/// machine or the local network outright. A name is checked again when it is resolved.
fn url_allowed(url: &url::Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }

    match url.host() {
        Some(url::Host::Ipv4(ip)) => is_public(ip.into()),
        Some(url::Host::Ipv6(ip)) => is_public(ip.into()),
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            !(name == "localhost" || name.ends_with(".localhost"))
        }
        None => false,
    }
}

/// A resolver that answers only with public addresses. See `remote_client`.
///
/// A name that resolves only to the local network — `router.local`, or a public name an
/// attacker has pointed at `127.0.0.1` — fails to resolve, as far as the request knows.
struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();

        Box::pin(async move {
            let public: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|address| is_public(address.ip()))
                .collect();

            if public.is_empty() {
                return Err(format!("{host} has no public address").into());
            }

            let addresses: reqwest::dns::Addrs = Box::new(public.into_iter());
            Ok(addresses)
        })
    }
}

/// Where fetched remote images are kept between reads.
fn remote_cache_dir() -> Option<std::path::PathBuf> {
    let root = crate::db::default_path().parent()?.join("remote");
    std::fs::create_dir_all(&root).ok()?;
    Some(root)
}

/// The cache file for one URL.
///
/// SHA-256 of the URL, so the name is fixed-length, contains no path separators, and cannot be
/// steered by a sender into writing outside the directory.
fn cache_file(dir: &std::path::Path, url: &str) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(url.as_bytes());
    dir.join(format!("{digest:x}"))
}

/// Drops cached images nothing has read in a while. Runs once per process.
///
/// A cache with no eviction is a disk leak with a long fuse: every image in every newsletter,
/// kept for ever. Modified time rather than an index, because the file *is* the record.
fn sweep_remote_cache(dir: &std::path::Path) {
    static SWEPT: std::sync::Once = std::sync::Once::new();

    SWEPT.call_once(|| {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        let cutoff =
            std::time::SystemTime::now() - Duration::from_secs(REMOTE_CACHE_DAYS * 24 * 60 * 60);

        let mut removed = 0usize;

        for entry in entries.filter_map(Result::ok) {
            let stale = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .is_ok_and(|modified| modified < cutoff);

            if stale && std::fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }

        if removed > 0 {
            tracing::debug!(removed, "remote image cache swept");
        }
    });
}

/// Fetches one remote image and returns it as a `data:` URI.
async fn fetch_one(client: &reqwest::Client, url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !url_allowed(&parsed) {
        tracing::debug!("remote image: not a public web address; not fetched");
        return None;
    }

    let response = client.get(parsed).send().await.ok()?;

    if !response.status().is_success() {
        return None;
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

    // Only images. A server that answers an `<img src>` with HTML is not serving an image, and
    // turning that into a data: URI would put the sender's markup back into the document the
    // sanitiser just cleaned. Checked in full, not by prefix: the header is the remote server's
    // to write, and `image/x"><link …` starts with `image/`.
    if !render::is_image_mime(&mime) {
        return None;
    }

    let bytes = read_capped(response, MAX_REMOTE_BYTES).await?;

    render::image_data_uri(&mime, &bytes)
}

/// A response's body, or `None` once it passes `cap` bytes.
///
/// Read as it arrives and abandoned at the cap. `bytes()` read the whole body first and checked
/// its size after, and a server that sends no `Content-Length` can send without end: everything
/// it sent in the ten seconds allowed was held in memory before being refused (L35).
async fn read_capped(mut response: reqwest::Response, cap: u64) -> Option<Vec<u8>> {
    if response.content_length().unwrap_or(0) > cap {
        return None;
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if (body.len() + chunk.len()) as u64 > cap {
            return None;
        }
        body.extend_from_slice(&chunk);
    }

    Some(body)
}

/// Books one image against the budget, or refuses it.
///
/// Separated from the fetch loop because the loop cannot run without a network and this is the
/// part with arithmetic worth checking. Saturating, so a hostile `Content-Length` cannot wrap
/// the total back to something small and let everything through.
fn book(spent: &mut usize, len: usize) -> bool {
    let next = spent.saturating_add(len);
    if next > MAX_REMOTE_TOTAL_BYTES {
        return false;
    }

    *spent = next;
    true
}

async fn fetch_remote(urls: &[String]) -> HashMap<String, String> {
    use futures::StreamExt;

    let Some(client) = remote_client() else {
        return HashMap::new();
    };

    let cache = remote_cache_dir();
    if let Some(dir) = cache.as_deref() {
        sweep_remote_cache(dir);
    }

    let wanted: Vec<String> = urls.iter().take(MAX_REMOTE_IMAGES).cloned().collect();

    // Concurrently, and bounded. In series this was one round trip per image before the reader
    // could show anything at all.
    let mut stream = futures::stream::iter(wanted.into_iter().map(|url| {
        let cache = cache.clone();

        async move {
            // A hit costs no request, which is the point twice over: it is faster, and it does
            // not tell the sender the message has been opened again.
            if let Some(dir) = cache.as_deref() {
                if let Ok(hit) = std::fs::read_to_string(cache_file(dir, &url)) {
                    if !hit.is_empty() {
                        return Some((url, hit));
                    }
                }
            }

            let data = fetch_one(client, &url).await?;

            if let Some(dir) = cache.as_deref() {
                // Best effort. A cache that cannot be written is slow, not broken.
                let _ = std::fs::write(cache_file(dir, &url), &data);
            }

            Some((url, data))
        }
    }))
    // Ordered, where this used to be `buffer_unordered`. The concurrency is identical — the same
    // six requests are in flight — but results arrive in the order the message asks for them,
    // which matters now that the budget below can stop part way: what a reader sees first is
    // what gets the budget, rather than whichever request happened to finish first.
    .buffered(REMOTE_CONCURRENCY);

    let mut images = HashMap::new();
    let mut spent = 0_usize;
    let deadline = tokio::time::Instant::now() + REMOTE_DEADLINE;

    loop {
        // Out of time: what has not arrived is left out, as the budget below leaves it out.
        let Ok(next) = tokio::time::timeout_at(deadline, stream.next()).await else {
            tracing::debug!(
                fetched = images.len(),
                "remote images: out of time, leaving the rest"
            );
            break;
        };
        let Some(entry) = next else {
            break;
        };
        let Some((url, data)) = entry else {
            continue;
        };

        if !book(&mut spent, data.len()) {
            // Stop rather than skip. The remaining futures are dropped un-polled, so the
            // requests are never made — which is the quiet half of the point: an image that
            // cannot be shown should not still be telling the sender the message was opened.
            //
            // Nothing else needs to know. `render` counts any URL missing from this map as a
            // remote image that was allowed and did not arrive, which is exactly what happened,
            // and the reader already has a banner that says so.
            tracing::debug!(
                fetched = images.len(),
                spent,
                "remote images: budget spent, leaving the rest"
            );
            break;
        }

        images.insert(url, data);
    }

    images
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

    // Only when the message actually references one. `inline_images` reads the whole `.eml`
    // off disk and walks its MIME tree to depth 32, and it ran on every render whether or not
    // there was a single `cid:` in the markup — which the log says is the overwhelming
    // majority, `inlined=0` even on the largest message this install has rendered.
    //
    // Scanned over the bytes rather than lowercasing the HTML first: a scheme is
    // case-insensitive, and allocating a second copy of a multi-megabyte string to find out
    // whether it contains four characters would cost more than the parse being avoided.
    let wants_inline = html.as_deref().is_some_and(|markup| {
        markup
            .as_bytes()
            .windows(4)
            .any(|w| w.eq_ignore_ascii_case(b"cid:"))
    });

    let inline = if wants_inline {
        inline_images(raw_path.as_deref())
    } else {
        HashMap::new()
    };

    // Sanitise once to discover what the message wants, fetch that, then render with what
    // came back. The first pass is what guarantees the URL list contains only things that
    // survived sanitisation — collecting them from the raw HTML would mean fetching URLs the
    // sanitiser had already decided to drop.
    let remote = if load_remote {
        // Enumerated from the *sanitised* markup, not the raw stored HTML: the fetch list
        // must contain only URLs the sanitiser approved, or the app would make a request on
        // behalf of markup it had already refused to render.
        let wanted = render::remote_image_urls(html.as_deref().unwrap_or_default());

        fetch_remote(&wanted).await
    } else {
        HashMap::new()
    };

    let rendered = render::render(
        html.as_deref(),
        text.as_deref(),
        inline,
        load_remote,
        remote,
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
///
/// Returns the link as it is handed to the shell, or `None` when it may not be. The scheme is
/// compared without regard to case, as RFC 3986 says it is, and the link is read without the
/// spaces around it: `HTTPS://` and a link with a leading space were both refused in silence
/// (L33, 2026-11-14 audit). What goes to the shell is the parsed URL, not the sender's text.
fn permitted(target: &str) -> Option<String> {
    let parsed = url::Url::parse(target.trim()).ok()?;

    match parsed.scheme() {
        "http" | "https" | "mailto" => Some(parsed.to_string()),
        "tel" => {
            let number = parsed.path();
            (!number.is_empty() && number.chars().all(|c| c.is_ascii_digit() || c == '+'))
                .then(|| format!("tel:{number}"))
        }
        _ => None,
    }
}

#[tauri::command]
pub async fn open_external(url: String, visible_text: String) -> Result<LinkOutcome, AppError> {
    let Some(target) = permitted(&url) else {
        return Ok(LinkOutcome {
            opened: false,
            mismatch: None,
        });
    };

    if let Some(claimed) = mismatched_host(&target, &visible_text) {
        return Ok(LinkOutcome {
            opened: false,
            mismatch: Some(claimed),
        });
    }

    open_in_browser(&target)?;

    Ok(LinkOutcome {
        opened: true,
        mismatch: None,
    })
}

/// Opens a link the user has confirmed despite the host mismatch.
#[tauri::command]
pub async fn open_external_confirmed(url: String) -> Result<(), AppError> {
    match permitted(&url) {
        Some(target) if target.starts_with("http://") || target.starts_with("https://") => {
            open_in_browser(&target)
        }
        _ => Ok(()),
    }
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

/// Who a host belongs to: the part of it that was registered, by the Public Suffix List.
///
/// `links.example.com` and `example.com` have one owner, and a link from one shown as the
/// other is how most companies track their own mail, not a trick. `bank.co.uk` and
/// `evil.co.uk` have two, which a rule like "the last two labels" would miss. An address is
/// compared whole: the list knows nothing of numbers, and would call `10.0.1.15` and
/// `192.168.1.15` the same.
fn owner(host: &str) -> &str {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<std::net::IpAddr>().is_ok() {
        return host;
    }

    psl::domain_str(host).unwrap_or(host)
}

/// Whether the link text claims one destination and the href goes to another.
fn mismatched_host(url: &str, visible_text: &str) -> Option<HostMismatch> {
    let shown = host_of(visible_text)?;
    let actual = host_of(url)?;

    if owner(&shown) == owner(&actual) {
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

    Ok(remote_images_from(stored.as_deref()))
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

/// How `remote_images_enabled` reads a stored value.
///
/// Split out of the command so it can be tested without a `Db` and a Tauri `State`. The command
/// is then only the plumbing that fetches the row.
fn remote_images_from(stored: Option<&str>) -> bool {
    // Absent means on. A fresh install gets the default without a row being written first, and a
    // database from before this setting existed behaves the same as a new one — which is the
    // case that would otherwise silently keep the old blocked-by-default behaviour for every
    // existing user.
    stored.is_none_or(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The assertion really is on constants, which is the point: what is being pinned is a
    // *relationship between them* that no amount of runtime testing would catch, and that a
    // later edit to any one of the three could silently break.
    #[allow(clippy::assertions_on_constants)]
    #[test]
    fn the_total_budget_is_the_one_that_binds() {
        // The per-image and per-count caps look like they bound a message and do not: sixty
        // images of eight mebibytes is 480 MiB of fetched bytes, 640 MiB once base64'd, all of
        // it crossing IPC into one string. The total has to be smaller than that product or it
        // is not a limit at all.
        assert!(
            MAX_REMOTE_TOTAL_BYTES < MAX_REMOTE_IMAGES * MAX_REMOTE_BYTES as usize,
            "a total budget no smaller than the product of the other two caps binds nothing"
        );

        // And large enough not to clip ordinary mail. Measured against this install's log, the
        // p90 rendered body is 3.16 MB; a budget under that would be cutting images out of
        // messages nobody would call excessive.
        assert!(
            MAX_REMOTE_TOTAL_BYTES >= 4 * 1024 * 1024,
            "a budget this small would clip newsletters that are merely ordinary"
        );
    }

    #[test]
    fn the_budget_fills_and_then_refuses() {
        let mut spent = 0;

        assert!(book(&mut spent, MAX_REMOTE_TOTAL_BYTES / 2));
        assert_eq!(spent, MAX_REMOTE_TOTAL_BYTES / 2);

        assert!(book(&mut spent, MAX_REMOTE_TOTAL_BYTES / 2));
        assert_eq!(spent, MAX_REMOTE_TOTAL_BYTES);

        // Full. And a refusal must not spend anything, or the running total would drift away
        // from what has actually been kept.
        assert!(!book(&mut spent, 1));
        assert_eq!(spent, MAX_REMOTE_TOTAL_BYTES);
    }

    #[test]
    fn one_enormous_image_cannot_wrap_the_total() {
        // The size comes from a response the sender controls. Plain addition would wrap and
        // report a tiny total, which would let every image through — the opposite of the cap.
        let mut spent = MAX_REMOTE_TOTAL_BYTES - 1;

        assert!(!book(&mut spent, usize::MAX));
        assert_eq!(
            spent,
            MAX_REMOTE_TOTAL_BYTES - 1,
            "a refusal spends nothing"
        );
    }

    #[test]
    fn an_inline_image_with_a_hostile_type_is_not_inlined() {
        // H12. The type is the sender's Content-Type and was pasted into the data URI, and the
        // data URI into the markup after sanitising, so a quote in it ended the attribute.
        let raw = "Content-Type: multipart/related; boundary=r\r\n\r\n\
--r\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:x\"><img src=\"cid:y\">\r\n\
--r\r\nContent-Type: image/png\" style=\"background:url(https://evil.test/p)\r\nContent-ID: <x>\r\n\r\nPNG\r\n\
--r\r\nContent-Type: IMAGE/PNG\r\nContent-ID: <y>\r\n\r\nPNG\r\n--r--\r\n";

        let parsed = mailparse::parse_mail(raw.as_bytes()).expect("parse");
        let mut images = HashMap::new();
        collect_parts(&parsed, 0, &mut images);

        assert!(!images.contains_key("x"), "{images:?}");
        assert!(
            images
                .get("y")
                .is_some_and(|uri| uri.starts_with("data:image/png;base64,")),
            "an ordinary image, whatever the case of its type, still inlines: {images:?}"
        );
    }

    #[test]
    fn only_a_well_formed_image_type_makes_a_data_uri() {
        assert!(render::image_data_uri("image/png", b"x").is_some());
        assert!(render::image_data_uri("image/svg+xml", b"x").is_some());
        assert!(render::image_data_uri("image/vnd.microsoft.icon", b"x").is_some());

        for hostile in [
            "image/",
            "image/png\"",
            "image/png\" onerror=\"x",
            "image/x><link rel=preconnect href=https://evil.test>",
            "image/png;charset=x",
            "text/html",
        ] {
            assert!(
                render::image_data_uri(hostile, b"x").is_none(),
                "{hostile:?}"
            );
            assert!(!render::is_image_mime(hostile), "{hostile:?}");
        }
    }

    #[test]
    fn remote_images_default_to_on() {
        // The setting the owner asked for, and the one default in this app chosen against the
        // security advice. Asserted so it cannot drift back by accident.
        assert!(
            remote_images_from(None),
            "a fresh install should load images"
        );
        assert!(
            remote_images_from(Some("1")),
            "an upgraded install with no row should load images"
        );
    }

    #[test]
    fn remote_images_can_be_turned_off() {
        assert!(!remote_images_from(Some("0")));
        assert!(!remote_images_from(Some("false")));
    }

    #[test]
    fn a_stored_value_is_read_case_insensitively() {
        // Written as "1" by this app, but a value edited by hand should still be understood.
        assert!(remote_images_from(Some("TRUE")));
        assert!(remote_images_from(Some("true")));
    }

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
            assert!(permitted(hostile).is_none(), "{hostile}");
        }
    }

    #[test]
    fn only_the_detectors_reduced_tel_form_is_permitted() {
        // The detector hands over `+` and digits. Anything else in a `tel:` did not come from
        // it, and `tel:` is only on the list because of it.
        assert!(permitted("tel:+442079460958").is_some());
        assert!(permitted("tel:02079460958").is_some());

        assert!(permitted("tel:").is_none());
        assert!(permitted("tel:+44 20 7946 0958").is_none());
        assert!(permitted("tel:*99#").is_none());
        assert!(permitted("tel:x@evil.test").is_none());
    }

    #[test]
    fn the_ordinary_schemes_still_open() {
        assert!(permitted("https://example.test").is_some());
        assert!(permitted("http://example.test").is_some());
        assert!(permitted("mailto:ada@example.test").is_some());
    }

    #[test]
    fn a_scheme_in_capitals_or_a_link_with_spaces_around_it_still_opens() {
        // L33: both were refused in silence. Schemes are case-insensitive (RFC 3986 §3.1).
        assert_eq!(
            permitted("HTTPS://Example.test/Offer").as_deref(),
            Some("https://example.test/Offer"),
            "the scheme and host are lowered; the path is the sender's"
        );
        assert_eq!(
            permitted("  https://example.test/x \n").as_deref(),
            Some("https://example.test/x")
        );
        assert_eq!(
            permitted("MAILTO:ada@example.test").as_deref(),
            Some("mailto:ada@example.test")
        );
        // And capitals are no way round the list.
        assert!(permitted("MS-MSDT:/id PCWDiagnostic").is_none());
        assert!(permitted("JavaScript:alert(1)").is_none());
    }

    #[test]
    fn a_remote_image_is_never_fetched_from_this_machine_or_its_network() {
        // L35. A message's <img src> pointed at the user's router, or a service on their own
        // machine, and the core fetched it from inside their network.
        for refused in [
            "http://localhost:8080/admin",
            "http://LOCALHOST./x",
            "http://printer.localhost/",
            "http://127.0.0.1/",
            "http://192.168.1.1/reboot",
            "http://10.0.0.8/",
            "http://172.16.4.2/",
            "http://169.254.169.254/latest/meta-data",
            "http://100.64.1.1/",
            "http://0.0.0.0/",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://[fe80::1]/",
            "http://[::ffff:192.168.1.1]/",
            "ftp://example.com/x.png",
            "file:///C:/Windows/win.ini",
        ] {
            let url = url::Url::parse(refused).expect(refused);
            assert!(!url_allowed(&url), "{refused}");
        }

        for allowed in [
            "https://cdn.example.com/logo.png",
            "http://93.184.216.34/pixel.gif",
            "https://[2606:2800:220:1:248:1893:25c8:1946]/x.png",
        ] {
            let url = url::Url::parse(allowed).expect(allowed);
            assert!(url_allowed(&url), "{allowed}");
        }
    }

    #[tokio::test]
    async fn a_name_that_resolves_only_to_this_machine_does_not_resolve() {
        use reqwest::dns::Resolve;
        use std::str::FromStr;

        // What stops a public-looking name pointed at 127.0.0.1. `localhost` stands in for it:
        // it resolves, and only to loopback.
        let name = reqwest::dns::Name::from_str("localhost").expect("name");
        assert!(PublicOnly.resolve(name).await.is_err());
    }

    #[tokio::test]
    async fn a_body_with_no_length_is_abandoned_at_the_cap() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // L35. A server that sends no Content-Length and never stops. `bytes()` held all of it
        // before checking the size.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await;
            let _ = socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nTransfer-Encoding: chunked\r\n\r\n",
                )
                .await;
            let chunk = format!("{:x}\r\n{}\r\n", 4096, "x".repeat(4096));
            loop {
                if socket.write_all(chunk.as_bytes()).await.is_err() {
                    break;
                }
            }
        });

        // A plain client: the app's own would refuse 127.0.0.1, which is the test above.
        let response = reqwest::Client::new()
            .get(format!("http://{address}/endless.png"))
            .send()
            .await
            .expect("response");

        let read = tokio::time::timeout(Duration::from_secs(5), read_capped(response, 64 * 1024))
            .await
            .expect("abandoned promptly, not read to the end");
        assert!(read.is_none());
    }

    #[test]
    fn hosts_are_compared_by_who_registered_them() {
        // H15. A company's own tracking subdomain is the company.
        assert!(mismatched_host("https://links.example.com/c/1", "example.com").is_none());
        assert!(mismatched_host("https://example.com/", "shop.example.com").is_none());
        assert!(mismatched_host("https://mail.example.co.uk/", "www.example.co.uk").is_none());

        // Two registrations under one public suffix are two owners.
        let claimed = mismatched_host("https://evil.co.uk/login", "bank.co.uk").expect("a prompt");
        assert_eq!(claimed.shown, "bank.co.uk");
        assert_eq!(claimed.actual, "evil.co.uk");
        assert!(mismatched_host("https://login.example.com.evil.test/", "example.com").is_some());

        // An address is compared whole.
        assert!(mismatched_host("http://10.0.1.15/", "192.168.1.15").is_some());
        assert!(mismatched_host("http://192.168.1.15/x", "192.168.1.15").is_none());
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
