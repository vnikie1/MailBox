//! IMAP's modified UTF-7, for mailbox names. RFC 3501 §5.1.3.
//!
//! A mailbox name on the wire is 7-bit: printable ASCII stands for itself, `&` is written `&-`,
//! and everything else is UTF-16 in a base64 variant between `&` and `-`. So a folder the user
//! calls "Entwürfe" is `Entw&APw-rfe` to the server.
//!
//! Neither `async-imap` nor `imap-proto` converts in either direction — `LIST` hands back the
//! encoded bytes and `CREATE` sends whatever it is given — which is why `mailbox.remote_path` is
//! stored raw and has to stay raw: it is the name the server knows. This module is the only
//! place the two spellings meet. Encoding happens when a name the user typed becomes a path, and
//! decoding when a path becomes a label.
//!
//! Before this existed the sidebar showed the wire form. Every folder with an accent in its name
//! read like `Re&AOc-us`, and the name heuristics in `mailboxes` compared that against words like
//! "envoyés" that it could never equal.

use base64::alphabet::IMAP_MUTF7;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::DecodePaddingMode;
use base64::Engine;

/// Base64 with `,` for `/` and no padding, which is the whole of the "modified" part.
///
/// Trailing bits are tolerated on the way in. A server that writes them non-zero is wrong, and
/// refusing its folder name would leave the user with a raw path rather than a slightly lenient
/// reading of it — standing rule 13.
const ENGINE: GeneralPurpose = GeneralPurpose::new(
    &IMAP_MUTF7,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone)
        .with_decode_allow_trailing_bits(true),
);

/// A name as the server spells it.
pub fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut shifted: Vec<u16> = Vec::new();

    for ch in name.chars() {
        if (' '..='~').contains(&ch) {
            flush(&mut out, &mut shifted);

            if ch == '&' {
                out.push_str("&-");
            } else {
                out.push(ch);
            }
        } else {
            let mut units = [0u16; 2];
            shifted.extend_from_slice(ch.encode_utf16(&mut units));
        }
    }

    flush(&mut out, &mut shifted);
    out
}

/// Writes one run of non-ASCII characters as a single shifted section.
///
/// One section per run rather than per character: `&AOkA6Q-` and `&AOk-&AOk-` decode alike, but
/// RFC 3501 requires the first, and a server comparing names byte for byte would treat the
/// second as a different mailbox.
fn flush(out: &mut String, shifted: &mut Vec<u16>) {
    if shifted.is_empty() {
        return;
    }

    let bytes: Vec<u8> = shifted.iter().flat_map(|unit| unit.to_be_bytes()).collect();

    out.push('&');
    out.push_str(&ENGINE.encode(bytes));
    out.push('-');

    shifted.clear();
}

/// A wire name as a person would write it, or `None` if it is not valid modified UTF-7.
pub fn decode(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;

    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);

        let after = &rest[start + 1..];
        let end = after.find('-')?;
        let section = &after[..end];

        if section.is_empty() {
            out.push('&');
        } else {
            let bytes = ENGINE.decode(section).ok()?;
            if bytes.len() % 2 != 0 {
                return None;
            }

            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();

            out.push_str(&String::from_utf16(&units).ok()?);
        }

        rest = &after[end + 1..];
    }

    out.push_str(rest);
    Some(out)
}

/// A wire name for display: decoded where it can be, and shown as it is where it cannot.
///
/// Servers exist that send raw UTF-8, or a stray `&`, instead of the encoding. A name that does
/// not decode is still a folder with mail in it, so it is shown rather than hidden.
pub fn display(raw: &str) -> String {
    decode(raw).unwrap_or_else(|| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ascii_is_left_alone() {
        for name in ["INBOX", "Receipts 2019", "[Gmail]/Sent Mail", "a.b-c_d~e"] {
            assert_eq!(encode(name), name);
            assert_eq!(decode(name).as_deref(), Some(name));
        }
    }

    #[test]
    fn an_ampersand_is_written_as_an_empty_shift() {
        assert_eq!(encode("Tom & Jerry"), "Tom &- Jerry");
        assert_eq!(decode("Tom &- Jerry").as_deref(), Some("Tom & Jerry"));
        assert_eq!(encode("&"), "&-");
    }

    #[test]
    fn the_rfc_example_round_trips() {
        // RFC 3501 §5.1.3, verbatim: "~peter/mail/台北/日本語".
        let wire = "~peter/mail/&U,BTFw-/&ZeVnLIqe-";
        let text = "~peter/mail/\u{53F0}\u{5317}/\u{65E5}\u{672C}\u{8A9E}";

        assert_eq!(decode(wire).as_deref(), Some(text));
        assert_eq!(encode(text), wire);
    }

    #[test]
    fn accented_names_are_the_ones_real_servers_send() {
        // Gmail's French and German folder names, as LIST really returns them.
        assert_eq!(
            decode("[Gmail]/Messages envoy&AOk-s").as_deref(),
            Some("[Gmail]/Messages envoyés")
        );
        assert_eq!(decode("Entw&APw-rfe").as_deref(), Some("Entwürfe"));
        assert_eq!(encode("Reçus"), "Re&AOc-us");
    }

    #[test]
    fn a_run_of_non_ascii_is_one_section() {
        // Two sections would decode the same, but RFC 3501 forbids them, and a server comparing
        // names byte for byte would see another mailbox.
        assert_eq!(encode("éé"), "&AOkA6Q-");
    }

    #[test]
    fn characters_outside_the_basic_plane_survive() {
        // A surrogate pair, split across the base64 boundary.
        assert_eq!(encode("😀"), "&2D3eAA-");
        assert_eq!(decode("&2D3eAA-").as_deref(), Some("😀"));
    }

    #[test]
    fn anything_typed_comes_back_as_it_was_typed() {
        for name in [
            "Projects",
            "Rechnungen & Belege",
            "Été 2026",
            "日本語のメール",
            "Ünïcödé — mixed ASCII",
            "Family 👪 photos",
            "trailing é",
            "é leading",
        ] {
            assert_eq!(decode(&encode(name)).as_deref(), Some(name), "{name}");
        }
    }

    #[test]
    fn a_malformed_name_is_refused_rather_than_guessed() {
        // Unterminated, not base64, and an odd number of bytes: none has a right reading.
        assert_eq!(decode("Re&AOc"), None);
        assert_eq!(decode("&!!-"), None);
        assert_eq!(decode("&AA-"), None);
    }

    #[test]
    fn a_malformed_name_is_still_displayed() {
        // It is a folder with mail in it. Hiding it would hide the mail.
        assert_eq!(display("Q&A notes"), "Q&A notes");
        assert_eq!(display("Re&AOc-us"), "Reçus");
    }
}
