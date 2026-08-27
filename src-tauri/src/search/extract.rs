//! Pulling searchable text out of attachments. docs/06 Phase 9.
//!
//! > attachment *contents* for PDF, DOCX and TXT, extracted at index time on a low-priority
//! > queue.
//!
//! ## Attachments are hostile input, and this is the part that parses them
//!
//! Everywhere else in this app an attachment is bytes that get written to disk or handed to the
//! shell. Here they are *parsed*, by third-party code, on the user's machine, unprompted — and
//! the file came from whoever sent the message. That is a materially different exposure from
//! the rest of the app, and it is why this module is built the way it is:
//!
//! * **A size ceiling before anything is parsed.** A 2GB PDF is not a document anyone needs
//!   indexed; it is a way to make the machine swap.
//! * **A page ceiling.** A PDF can declare hundreds of thousands of pages, and a decompression
//!   bomb costs the sender nothing.
//! * **Every parse is caught.** A panic in a PDF parser is a real possibility, and it must cost
//!   one unindexed attachment rather than the process the user is reading mail in.
//! * **Nothing runs eagerly.** Extraction happens on a queue, off the sync path, so a malformed
//!   file cannot slow down or block mail arriving.
//!
//! ## What is deliberately absent
//!
//! No `.doc`, no `.xls`, no `.rtf`. Each needs a parser for a format designed for an era with
//! different assumptions, and the old Office formats in particular are a long list of parser
//! CVEs. The three the spec names cover what people actually search for.

use std::io::Read;

/// The largest attachment worth reading. Beyond this it is a file to open, not text to index.
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

/// The most text kept from one attachment.
///
/// A 400-page contract contributes its first chunk and no more. The index is for finding the
/// message, not for reading the document, and a single attachment that added ten megabytes to
/// an FTS index would slow every search for everyone.
pub const MAX_TEXT: usize = 256 * 1024;

/// The most PDF pages to walk, however many the file claims.
const MAX_PAGES: u32 = 200;

/// What kind of file this is, decided by extension.
///
/// By extension rather than by content sniffing, and that is the safer choice here: sniffing
/// means handing the bytes to something that inspects them, which is the exposure being
/// limited. A file whose extension lies simply fails to parse and is skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Pdf,
    Docx,
}

impl Kind {
    pub fn of(filename: &str) -> Option<Kind> {
        let extension = filename.rsplit('.').next()?.to_ascii_lowercase();

        match extension.as_str() {
            "txt" | "md" | "csv" | "log" => Some(Kind::Text),
            "pdf" => Some(Kind::Pdf),
            "docx" => Some(Kind::Docx),
            _ => None,
        }
    }
}

/// Extracts searchable text, or `None` when there is nothing to get.
///
/// Never returns an error. Every failure here — an unreadable file, a malformed document, a
/// parser that panics — means one attachment goes unindexed, which is a smaller loss than any
/// of the alternatives.
pub fn text_from(filename: &str, bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return None;
    }

    let kind = Kind::of(filename)?;

    // Caught, because a parser panicking on a malformed file is a real possibility and it must
    // cost one attachment rather than the process the user is reading their mail in.
    let extracted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match kind {
        Kind::Text => from_text(bytes),
        Kind::Pdf => from_pdf(bytes),
        Kind::Docx => from_docx(bytes),
    }));

    let text = match extracted {
        Ok(Some(text)) => text,
        Ok(None) => return None,
        Err(_) => {
            tracing::warn!(filename, "attachment parser panicked; leaving it unindexed");
            return None;
        }
    };

    let trimmed = normalise(&text);
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Collapses whitespace and bounds the length.
///
/// Extracted text is full of the layout artefacts of the format it came from — a PDF gives one
/// line per visual line, a DOCX gives runs split mid-word. None of that helps an index and all
/// of it costs space.
fn normalise(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_TEXT));
    let mut last_was_space = true;

    for character in text.chars() {
        if out.len() >= MAX_TEXT {
            break;
        }

        if character.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
            continue;
        }

        // Control characters carry no meaning to a tokeniser and can upset the terminal a log
        // line is printed to.
        if character.is_control() {
            continue;
        }

        out.push(character);
        last_was_space = false;
    }

    out.trim().to_string()
}

fn from_text(bytes: &[u8]) -> Option<String> {
    // Lossy: a text file with a broken encoding is still worth indexing, and refusing it would
    // mean the ones most likely to matter — old logs, exports from other systems — are skipped.
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn from_pdf(bytes: &[u8]) -> Option<String> {
    let document = lopdf::Document::load_mem(bytes).ok()?;

    let pages: Vec<u32> = document
        .get_pages()
        .keys()
        .copied()
        .take(MAX_PAGES as usize)
        .collect();

    // Page by page rather than in one call, so a document with one unreadable page still
    // contributes the rest — and so a bomb is bounded by the page ceiling above.
    let mut out = String::new();
    for page in pages {
        if out.len() >= MAX_TEXT {
            break;
        }

        if let Ok(text) = document.extract_text(&[page]) {
            out.push_str(&text);
            out.push(' ');
        }
    }

    Some(out)
}

fn from_docx(bytes: &[u8]) -> Option<String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).ok()?;

    // Only this one entry, by name. A docx is a zip, and reading every entry is how a zip bomb
    // gets its leverage — as is trusting an entry's declared path, which is where zip-slip
    // lives. Nothing here is written to disk, and only the one known path is read.
    let mut document = archive.by_name("word/document.xml").ok()?;

    let mut xml = String::new();
    document
        .by_ref()
        .take(MAX_BYTES as u64)
        .read_to_string(&mut xml)
        .ok()?;

    Some(strip_xml(&xml))
}

/// The text content of an XML document, tags removed.
///
/// Deliberately not an XML parser. The only thing wanted is the character data, the input is
/// one known file from one known producer, and a full parser here would be a second untrusted
/// surface for no gain. Paragraph tags become spaces so words do not run together.
fn strip_xml(xml: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    let mut tag = String::new();

    for character in xml.chars() {
        match character {
            '<' => {
                inside = true;
                tag.clear();
            }
            '>' => {
                inside = false;

                // `</w:p>` ends a paragraph and `<w:br/>` a line. Without this the last word of
                // one paragraph and the first of the next become a single nonsense token.
                let name = tag.trim_start_matches('/');
                if name.starts_with("w:p") || name.starts_with("w:br") || name.starts_with("w:tab")
                {
                    out.push(' ');
                }
            }
            _ if inside => tag.push(character),
            _ => out.push(character),
        }
    }

    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_come_from_the_extension() {
        assert_eq!(Kind::of("notes.txt"), Some(Kind::Text));
        assert_eq!(Kind::of("Report.PDF"), Some(Kind::Pdf));
        assert_eq!(Kind::of("contract.docx"), Some(Kind::Docx));
        assert_eq!(Kind::of("photo.jpg"), None);
        assert_eq!(Kind::of("noextension"), None);
    }

    #[test]
    fn an_old_office_format_is_not_indexed() {
        // Deliberate. `.doc` and `.xls` need parsers for formats with a long history of parser
        // CVEs, and the three the spec names cover what people search for.
        assert_eq!(Kind::of("old.doc"), None);
        assert_eq!(Kind::of("sheet.xls"), None);
        assert_eq!(Kind::of("page.rtf"), None);
    }

    #[test]
    fn plain_text_comes_through() {
        let text = text_from("notes.txt", b"the quarterly figures").expect("text");
        assert_eq!(text, "the quarterly figures");
    }

    #[test]
    fn whitespace_is_collapsed() {
        // Extracted text is full of its format's layout artefacts, none of which helps an index.
        let text = text_from("notes.txt", b"one\n\n\ttwo   three\r\n").expect("text");
        assert_eq!(text, "one two three");
    }

    #[test]
    fn an_oversized_file_is_skipped_before_it_is_parsed() {
        // The ceiling exists so a huge file cannot make the machine swap. Checked before the
        // parser sees anything.
        let huge = vec![b'a'; MAX_BYTES + 1];
        assert_eq!(text_from("notes.txt", &huge), None);
    }

    #[test]
    fn extracted_text_is_bounded() {
        let long = vec![b'a'; MAX_TEXT * 2];
        let text = text_from("notes.txt", &long).expect("text");
        assert!(text.len() <= MAX_TEXT, "{} bytes", text.len());
    }

    #[test]
    fn an_empty_file_yields_nothing() {
        assert_eq!(text_from("notes.txt", b""), None);
        assert_eq!(text_from("notes.txt", b"   \n\t "), None);
    }

    #[test]
    fn a_file_whose_extension_lies_is_skipped_rather_than_forced() {
        // Not a PDF. It fails to parse and contributes nothing, which is the whole point of
        // deciding by extension and then letting the parser refuse.
        assert_eq!(text_from("fake.pdf", b"this is not a pdf at all"), None);
    }

    #[test]
    fn a_malformed_archive_is_skipped() {
        assert_eq!(
            text_from("fake.docx", b"PK\x03\x04 and then nonsense"),
            None
        );
    }

    #[test]
    fn docx_paragraphs_do_not_run_words_together() {
        // Without a space at the paragraph boundary, "figures" and "Regards" become one token
        // that matches neither search.
        let xml = "<w:p><w:r><w:t>the figures</w:t></w:r></w:p>\
                   <w:p><w:r><w:t>Regards</w:t></w:r></w:p>";

        let text = strip_xml(xml);
        assert!(text.contains("figures"), "{text}");
        assert!(text.contains("Regards"), "{text}");
        assert!(!text.contains("figuresRegards"), "{text}");
    }

    #[test]
    fn xml_entities_are_decoded() {
        assert_eq!(
            strip_xml("<w:t>Marks &amp; Spencer</w:t>").trim(),
            "Marks & Spencer"
        );
    }

    #[test]
    fn control_characters_are_dropped() {
        let text = text_from("notes.txt", b"before\x00\x07after").expect("text");
        assert_eq!(text, "beforeafter");
    }
}
