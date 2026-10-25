//! The pages of a PDF through pdf-extract (plugin ADR-0001, decisions 1 and
//! 2). pdf-extract panics on some malformed files, and a hostile one can
//! nest deeply, so a PDF is read on a thread of its own with a large stack,
//! and each page inside `catch_unwind`: a page that the parser panics on
//! costs that page, and the document fails only when no page could be read.
//!
//! pdf-extract 0.12 reads every `XObject` that a page draws as drawing
//! operators, images included, and the bytes of an image can read as an
//! operator without its operands, which it panics on. An image holds no
//! text, so the images are emptied before any page is read.
//!
//! A stack that overflows ends the process, which `catch_unwind` cannot
//! stop, so a PDF whose arrays and dictionaries nest deeper than
//! [`MAX_NESTING`] outside its streams is refused before the parser reads it.
//! Nesting inside a compressed stream is out of the scan's sight, and the
//! large stack is what bounds it (T-64).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread;

use pdf_extract::{Object, PlainTextOutput};

use crate::{Document, ExtractError, Location, Section, tidy};

/// The stack of the thread that reads a PDF. Only what the parser uses of
/// it is committed.
const STACK_BYTES: usize = 256 * 1024 * 1024;

/// The deepest nesting of arrays and dictionaries outside streams. Real
/// documents nest a few levels.
const MAX_NESTING: usize = 256;

/// The text of a PDF's pages, read on a thread of its own.
pub(crate) fn extract(bytes: Vec<u8>) -> Result<Document, ExtractError> {
    if !nesting_is_shallow(&bytes) {
        return Err(ExtractError::Unreadable);
    }
    let worker = thread::Builder::new()
        .name("notebooks-pdf".to_owned())
        .stack_size(STACK_BYTES)
        .spawn(move || catch_unwind(AssertUnwindSafe(|| read(&bytes))))
        .map_err(|_no_thread| ExtractError::Unreadable)?;
    match worker.join() {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(_panic)) | Err(_panic) => Err(ExtractError::Unreadable),
    }
}

fn read(bytes: &[u8]) -> Result<Document, ExtractError> {
    let mut document =
        pdf_extract::Document::load_mem(bytes).map_err(|_unreadable| ExtractError::Unreadable)?;
    let pages = document.get_pages();
    // A PDF that is encrypted without a password opens with the empty one,
    // as pdf-extract opens it.
    if document.is_encrypted() && document.decrypt("").is_err() {
        return Err(ExtractError::Encrypted);
    }
    empty_images(&mut document);
    let mut any_page_read = false;
    let mut sections = Vec::new();
    for &page in pages.keys() {
        let Some(text) = page_text(&document, page) else {
            continue;
        };
        any_page_read = true;
        let tidied = tidy(&text);
        if !tidied.is_empty() {
            sections.push(Section {
                location: Location::Page(page),
                text: tidied,
            });
        }
    }
    if !any_page_read {
        return Err(ExtractError::Unreadable);
    }
    Ok(Document {
        sections,
        pages: u32::try_from(pages.len()).ok(),
    })
}

/// Empties the content of every image, which pdf-extract would otherwise
/// read as drawing operators when a page draws it.
fn empty_images(document: &mut pdf_extract::Document) {
    for object in document.objects.values_mut() {
        let Object::Stream(stream) = object else {
            continue;
        };
        let subtype = stream.dict.get(b"Subtype").and_then(Object::as_name);
        if subtype.is_ok_and(|name| name == b"Image") {
            stream.set_plain_content(Vec::new());
        }
    }
}

/// The text of a page, or `None` when pdf-extract fails or panics on it.
fn page_text(document: &pdf_extract::Document, page: u32) -> Option<String> {
    let mut text = String::new();
    // The parser only reads the document, so a page that panics leaves
    // nothing half changed for the next one.
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let mut output = PlainTextOutput::new(&mut text);
        pdf_extract::output_doc_page(document, &mut output, page)
    }));
    matches!(outcome, Ok(Ok(()))).then_some(text)
}

/// Whether the arrays and dictionaries of a PDF nest at most [`MAX_NESTING`]
/// levels outside its strings, comments and streams, which the scan skips.
fn nesting_is_shallow(bytes: &[u8]) -> bool {
    let mut depth: usize = 0;
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let next = bytes.get(index + 1).copied();
        match byte {
            b'%' => index = end_of_line(bytes, index),
            b'(' => index = end_of_literal_string(bytes, index),
            b'<' if next == Some(b'<') => {
                depth += 1;
                index += 2;
            }
            b'<' => index = after(bytes, index, b">"),
            b'>' if next == Some(b'>') => {
                depth = depth.saturating_sub(1);
                index += 2;
            }
            b'[' => {
                depth += 1;
                index += 1;
            }
            b']' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            b's' if bytes[index..].starts_with(b"stream") => {
                index = after(bytes, index + b"stream".len(), b"endstream");
            }
            _ => index += 1,
        }
        if depth > MAX_NESTING {
            return false;
        }
    }
    true
}

/// The index after the end of the line that starts at or after `index`.
fn end_of_line(bytes: &[u8], index: usize) -> usize {
    bytes[index..]
        .iter()
        .position(|&byte| byte == b'\n' || byte == b'\r')
        .map_or(bytes.len(), |at| index + at + 1)
}

/// The index after the literal string that opens at `index`: its
/// parentheses balance, and a backslash escapes the next byte.
fn end_of_literal_string(bytes: &[u8], index: usize) -> usize {
    let mut open: usize = 0;
    let mut at = index;
    while let Some(&byte) = bytes.get(at) {
        match byte {
            b'\\' => at += 1,
            b'(' => open += 1,
            b')' => {
                open = open.saturating_sub(1);
                if open == 0 {
                    return at + 1;
                }
            }
            _ => {}
        }
        at += 1;
    }
    bytes.len()
}

/// The index after the first `marker` at or after `index`, or the end.
fn after(bytes: &[u8], index: usize, marker: &[u8]) -> usize {
    bytes
        .get(index..)
        .and_then(|rest| {
            rest.windows(marker.len())
                .position(|window| window == marker)
        })
        .map_or(bytes.len(), |at| index + at + marker.len())
}

#[cfg(test)]
mod tests {
    use super::{MAX_NESTING, nesting_is_shallow};

    #[test]
    fn nesting_counts_arrays_and_dictionaries_outside_strings_and_streams() {
        let deep = "[".repeat(MAX_NESTING + 1);
        let at_the_limit = format!("{}{}", "<<".repeat(MAX_NESTING), ">>".repeat(MAX_NESTING));
        let in_a_string = format!("({})", "[".repeat(10_000));
        let in_a_stream = format!("stream\n{}\nendstream", "[".repeat(10_000));
        let in_a_comment = format!("%{}\n[]", "[".repeat(10_000));
        let in_hex = format!("<{}>", "5B".repeat(10));

        assert!(!nesting_is_shallow(deep.as_bytes()));
        assert!(nesting_is_shallow(at_the_limit.as_bytes()));
        assert!(nesting_is_shallow(in_a_string.as_bytes()));
        assert!(nesting_is_shallow(in_a_stream.as_bytes()));
        assert!(nesting_is_shallow(in_a_comment.as_bytes()));
        assert!(nesting_is_shallow(in_hex.as_bytes()));
        assert!(nesting_is_shallow(br"(a \) b ( c ) d) [ ]"));
    }
}
