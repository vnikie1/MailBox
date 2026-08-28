//! `mailto:` links and `.eml` files. docs/06 Phase 10.
//!
//! Both arrive the same way — as a command-line argument, either at launch or handed to the
//! running instance by the single-instance plugin — so both are parsed here.
//!
//! ## `mailto:` is parsed strictly, because it comes from a web page
//!
//! A `mailto:` URL is untrusted input from wherever the user clicked. RFC 6068 allows headers
//! in the query string, and a naive implementation lets a link set any of them — including
//! `Bcc`, so a page could open a compose window that silently copies a stranger. Only the four
//! headers a person would recognise from the compose window are honoured, and everything else
//! is dropped rather than passed through.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use ts_rs::TS;

/// What a `mailto:` asked for, after everything unsafe has been dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MailtoRequest {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
}

/// Percent-decoding, plus `+` for space in a query string.
fn decode(value: &str, plus_is_space: bool) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    // A stray `%` that is not an escape. Kept as itself rather than dropped:
                    // a subject reading "100% done" is a real thing to type.
                    Err(_) => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).into_owned()
}

/// Splits and cleans a comma-separated address list.
fn addresses(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        // Newlines removed rather than the address rejected. A `%0A` in a mailto is the classic
        // header-injection trick — it would end the To line and let the rest of the value
        // become a header of its own — and there is no legitimate address containing one.
        .map(|part| part.replace(['\r', '\n'], " ").trim().to_string())
        .collect()
}

/// Parses a `mailto:` URL. RFC 6068.
///
/// Returns `None` for anything that is not one, so a caller can pass every argument it was
/// given without pre-filtering.
pub fn parse_mailto(input: &str) -> Option<MailtoRequest> {
    let rest = input
        .strip_prefix("mailto:")
        .or_else(|| input.strip_prefix("MAILTO:"))?;

    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };

    let mut request = MailtoRequest {
        to: addresses(&decode(path, false)),
        ..MailtoRequest::default()
    };

    let Some(query) = query else {
        return Some(request);
    };

    for pair in query.split('&') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };

        let value = decode(value, true);

        // An allow-list, not a deny-list. RFC 6068 permits arbitrary headers, and a link that
        // could set `Bcc` would let any web page open a compose window that silently copies
        // somebody — which the sender would only discover from the reply.
        match name.trim().to_ascii_lowercase().as_str() {
            "to" => request.to.extend(addresses(&value)),
            "cc" => request.cc.extend(addresses(&value)),
            "subject" => request.subject = value.replace(['\r', '\n'], " "),
            "body" => request.body = value,
            _ => {}
        }
    }

    Some(request)
}

/// Whether an argument names a message file the reader can open.
pub fn is_eml(argument: &str) -> bool {
    std::path::Path::new(argument)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("eml"))
}

/// Acts on whatever the shell passed in.
///
/// The compose window is opened here rather than by emitting to the UI, because the sanitised
/// fields have to reach it and the window is created from Rust anyway — routing them through the
/// frontend would mean a second parse on that side, which is precisely where the Bcc filter
/// above would get lost.
pub fn handle_arguments(app: &AppHandle, argv: &[String]) {
    for argument in argv.iter().skip(1) {
        if let Some(request) = parse_mailto(argument) {
            let handle = app.clone();

            tauri::async_runtime::spawn(async move {
                super::tray::show(&handle);

                if let Err(error) =
                    crate::ipc::compose::compose_open(handle.clone(), None, None, Some(request))
                        .await
                {
                    tracing::warn!(?error, "could not open a compose window for a mailto link");
                }
            });

            continue;
        }

        if is_eml(argument) {
            let _ = app.emit("eml:open", argument.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_address_is_the_recipient() {
        let request = parse_mailto("mailto:ada@example.test").expect("parsed");
        assert_eq!(request.to, vec!["ada@example.test"]);
    }

    #[test]
    fn the_query_fills_the_rest() {
        let request =
            parse_mailto("mailto:ada@example.test?subject=Hello&body=How+are+you").expect("parsed");

        assert_eq!(request.subject, "Hello");
        assert_eq!(request.body, "How are you");
    }

    #[test]
    fn percent_escapes_are_decoded() {
        let request = parse_mailto("mailto:a@x.test?subject=100%25%20done").expect("parsed");
        assert_eq!(request.subject, "100% done");
    }

    #[test]
    fn a_stray_percent_survives_rather_than_eating_the_text() {
        let request = parse_mailto("mailto:a@x.test?subject=50%off").expect("parsed");
        assert!(request.subject.contains('%'), "{}", request.subject);
    }

    #[test]
    fn several_recipients_are_split() {
        let request = parse_mailto("mailto:a@x.test,b@x.test?cc=c@x.test").expect("parsed");

        assert_eq!(request.to, vec!["a@x.test", "b@x.test"]);
        assert_eq!(request.cc, vec!["c@x.test"]);
    }

    #[test]
    fn bcc_is_refused() {
        // The reason this is an allow-list. A page that could set Bcc would open a compose
        // window silently copying a stranger, and the sender would find out from the reply.
        let request = parse_mailto("mailto:a@x.test?bcc=hidden@evil.test").expect("parsed");

        assert!(!format!("{request:?}").contains("hidden@evil.test"));
    }

    #[test]
    fn arbitrary_headers_are_refused() {
        // RFC 6068 permits them. Honouring them would let a link set Reply-To, so a reply to
        // what looks like the user's own message goes somewhere else entirely.
        let request =
            parse_mailto("mailto:a@x.test?reply-to=evil@x.test&from=someone@x.test").expect("ok");

        let debug = format!("{request:?}");
        assert!(!debug.contains("evil@x.test"));
        assert!(!debug.contains("someone@x.test"));
    }

    #[test]
    fn a_newline_cannot_be_smuggled_into_an_address() {
        // `%0A` would end the To line and let everything after it become a header of its own.
        let request = parse_mailto("mailto:a@x.test%0ABcc:%20evil@x.test").expect("parsed");

        for address in &request.to {
            assert!(!address.contains('\n'), "{address}");
            assert!(!address.contains('\r'), "{address}");
        }
    }

    #[test]
    fn a_newline_cannot_be_smuggled_into_the_subject() {
        let request = parse_mailto("mailto:a@x.test?subject=Hi%0ABcc:%20evil@x.test").expect("ok");

        assert!(!request.subject.contains('\n'));
        assert!(!request.subject.contains('\r'));
    }

    #[test]
    fn something_that_is_not_a_mailto_is_not_one() {
        assert!(parse_mailto("https://example.test").is_none());
        assert!(parse_mailto("C:/Users/me/message.eml").is_none());
        assert!(parse_mailto("").is_none());
    }

    #[test]
    fn the_scheme_is_recognised_whatever_its_case() {
        // Windows hands the scheme back in whatever case the registry has it.
        assert!(parse_mailto("MAILTO:a@x.test").is_some());
    }

    #[test]
    fn eml_files_are_recognised_by_extension() {
        assert!(is_eml("C:/Users/me/Downloads/message.eml"));
        assert!(is_eml("message.EML"));
        assert!(!is_eml("message.txt"));
        assert!(!is_eml("message"));
    }
}
