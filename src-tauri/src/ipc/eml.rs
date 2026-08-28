//! Opening a `.eml` file in a read-only window. docs/06 Phase 10.
//!
//! ## Why it is read-only, and why that is not a limitation
//!
//! A `.eml` on disk is not in any mailbox. It has no account, no UID and no server, so nothing
//! that changes a message — flag, archive, mark read, move — has anywhere to write. Presenting
//! those controls and having them do nothing would be worse than not showing them; presenting
//! them and quietly inventing a mailbox to put the file in would be worse still.
//!
//! Reply and Forward *are* possible, because they only need the headers, but they are not here.
//! They would need an account chosen for the From line, and the file itself cannot say which —
//! it may be addressed to an identity this app has never seen. That belongs in the compose
//! window's own account picker, and wiring it through a viewer that has no account is a way of
//! making that choice invisible.
//!
//! ## Why it goes through the same sanitiser
//!
//! A `.eml` is more hostile than a synced message, not less: it arrived as a file, from a
//! download or an attachment, and nothing upstream has looked at it. It takes exactly the path
//! a mailbox message takes — `mail::render::render`, remote images withheld — and there is no
//! branch here that could skip it.

use std::collections::HashMap;

use mailparse::MailHeaderMap;
use serde::Serialize;
use ts_rs::TS;

use crate::mail::render::{self, Rendered};

use super::mail::AppError;

/// Everything the viewer window shows.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct EmlMessage {
    pub subject: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub date: String,
    pub body: Rendered,
    /// Names only. A `.eml` viewer does not save attachments — see the module note.
    pub attachments: Vec<String>,
}

/// Pulls the best HTML and plain parts out of a parsed message.
fn bodies(part: &mailparse::ParsedMail<'_>, html: &mut Option<String>, plain: &mut Option<String>) {
    let mime = part.ctype.mimetype.to_ascii_lowercase();

    if part.subparts.is_empty() {
        // An attachment is not the body, however text-shaped it is. A forwarded `.txt` would
        // otherwise become the message when the real body is HTML.
        let disposition = part.get_content_disposition();
        if disposition.disposition == mailparse::DispositionType::Attachment {
            return;
        }

        if mime == "text/html" && html.is_none() {
            *html = part.get_body().ok();
        } else if mime == "text/plain" && plain.is_none() {
            *plain = part.get_body().ok();
        }

        return;
    }

    for sub in &part.subparts {
        bodies(sub, html, plain);
    }
}

/// Collects `cid:` parts, so an embedded image shows without touching the network.
fn inline_images(part: &mailparse::ParsedMail<'_>, into: &mut HashMap<String, String>) {
    use base64::Engine;

    if part.subparts.is_empty() {
        let id = part
            .headers
            .get_first_value("Content-ID")
            .map(|value| value.trim_matches(['<', '>']).to_string());

        if let (Some(id), Ok(bytes)) = (id, part.get_body_raw()) {
            let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
            into.insert(id, format!("data:{};base64,{encoded}", part.ctype.mimetype));
        }

        return;
    }

    for sub in &part.subparts {
        inline_images(sub, into);
    }
}

fn attachment_names(part: &mailparse::ParsedMail<'_>, into: &mut Vec<String>) {
    if part.subparts.is_empty() {
        let disposition = part.get_content_disposition();

        if disposition.disposition == mailparse::DispositionType::Attachment {
            if let Some(name) = disposition.params.get("filename") {
                into.push(name.clone());
            }
        }

        return;
    }

    for sub in &part.subparts {
        attachment_names(sub, into);
    }
}

/// Splits an address header into individual addresses for display.
fn addresses(value: Option<String>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

/// Reads and renders a `.eml` from disk.
#[tauri::command]
pub async fn eml_read(path: String) -> Result<EmlMessage, AppError> {
    // Checked before reading, not after. The window is opened from a shell argument, and an
    // argument naming something that is not a message should fail here rather than producing
    // an empty viewer that looks like a broken one.
    if !std::path::Path::new(&path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("eml"))
    {
        return Err(AppError {
            code: "not-eml".into(),
            message: "That file is not a message.".into(),
        });
    }

    let raw = std::fs::read(&path).map_err(|error| AppError {
        code: "read-failed".into(),
        // The OS message, not a rewrite of it: "Access is denied" and "The system cannot find
        // the file specified" are different problems and the user can act on the difference.
        message: error.to_string(),
    })?;

    let parsed = mailparse::parse_mail(&raw).map_err(|error| AppError {
        code: "parse-failed".into(),
        message: error.to_string(),
    })?;

    let mut html = None;
    let mut plain = None;
    bodies(&parsed, &mut html, &mut plain);

    let mut inline = HashMap::new();
    inline_images(&parsed, &mut inline);

    let mut attachments = Vec::new();
    attachment_names(&parsed, &mut attachments);

    let header = |name: &str| parsed.headers.get_first_value(name);

    Ok(EmlMessage {
        subject: header("Subject").unwrap_or_else(|| "(no subject)".into()),
        from: header("From").unwrap_or_default(),
        to: addresses(header("To")),
        cc: addresses(header("Cc")),
        date: header("Date").unwrap_or_default(),
        // Remote images withheld, with no argument to change that. A file opened from a
        // download is the last place to make a network request on a stranger's say-so, and the
        // count still reaches the viewer so it can say what it is holding back.
        body: render::render(
            html.as_deref(),
            plain.as_deref(),
            &inline,
            false,
            &HashMap::new(),
        ),
        attachments,
    })
}

/// Opens the viewer window. Called by `links::handle_arguments`.
pub async fn open(app: tauri::AppHandle, path: String) -> Result<(), AppError> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let label = format!(
        "eml-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );

    // Percent-encoded, because a Windows path is full of characters a query string treats as
    // structure — a backslash is fine but a `&` or `#` in a filename would truncate it.
    let encoded: String = path
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect();

    WebviewWindowBuilder::new(
        &app,
        &label,
        WebviewUrl::App(format!("index.html?eml={encoded}").into()),
    )
    .title("Message")
    .inner_size(700.0, 620.0)
    .build()
    .map_err(|error| AppError {
        code: "window-failed".into(),
        message: error.to_string(),
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = "From: Ada Lovelace <ada@example.test>\r\n\
To: Grace <grace@example.test>, Alan <alan@example.test>\r\n\
Subject: A note\r\n\
Date: Mon, 3 Mar 2025 09:00:00 +0000\r\n\
Content-Type: text/plain\r\n\
\r\n\
Hello there.\r\n";

    fn parse(raw: &str) -> mailparse::ParsedMail<'_> {
        mailparse::parse_mail(raw.as_bytes()).expect("parse")
    }

    #[test]
    fn a_plain_message_yields_its_text() {
        let mail = parse(SIMPLE);
        let (mut html, mut plain) = (None, None);
        bodies(&mail, &mut html, &mut plain);

        assert!(html.is_none());
        assert!(plain.expect("plain").contains("Hello there"));
    }

    #[test]
    fn several_recipients_are_split() {
        let mail = parse(SIMPLE);
        let to = addresses(mail.headers.get_first_value("To"));

        assert_eq!(to.len(), 2);
        assert!(to[0].contains("grace@example.test"));
    }

    #[test]
    fn html_wins_over_the_plain_alternative() {
        let raw = "Content-Type: multipart/alternative; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nplain version\r\n\
--b\r\nContent-Type: text/html\r\n\r\n<p>html version</p>\r\n--b--\r\n";

        let mail = parse(raw);
        let (mut html, mut plain) = (None, None);
        bodies(&mail, &mut html, &mut plain);

        // Both are found; the renderer prefers HTML. Asserting both are present is the point —
        // an alternative part that lost the plain text would break the fallback path.
        assert!(html.expect("html").contains("html version"));
        assert!(plain.expect("plain").contains("plain version"));
    }

    #[test]
    fn an_attached_text_file_is_not_mistaken_for_the_body() {
        // The failure this guards: a forwarded .txt becoming the message, with the real body
        // discarded, because both are text/plain and only the disposition tells them apart.
        let raw = "Content-Type: multipart/mixed; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nthe actual body\r\n\
--b\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=\"notes.txt\"\r\n\r\nattached notes\r\n--b--\r\n";

        let mail = parse(raw);
        let (mut html, mut plain) = (None, None);
        bodies(&mail, &mut html, &mut plain);

        let body = plain.expect("plain");
        assert!(body.contains("the actual body"), "{body}");
        assert!(!body.contains("attached notes"), "{body}");
    }

    #[test]
    fn attachments_are_listed_by_name() {
        let raw = "Content-Type: multipart/mixed; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nbody\r\n\
--b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\n\r\ndata\r\n--b--\r\n";

        let mail = parse(raw);
        let mut names = Vec::new();
        attachment_names(&mail, &mut names);

        assert_eq!(names, vec!["report.pdf"]);
    }

    #[tokio::test]
    async fn something_that_is_not_a_message_is_refused_before_it_is_read() {
        let error = eml_read("C:/Users/me/notes.txt".into())
            .await
            .expect_err("refused");

        assert_eq!(error.code, "not-eml");
    }

    #[tokio::test]
    async fn a_missing_file_says_so_in_the_operating_system_s_words() {
        let error = eml_read("Z:/definitely/not/here.eml".into())
            .await
            .expect_err("refused");

        assert_eq!(error.code, "read-failed");
        assert!(!error.message.is_empty());
    }
}
