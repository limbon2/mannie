//! The text of files (docs/features/system-tools/tools-and-results.md, text):
//! UTF-8, and UTF-16 with a byte order mark. A byte order mark is kept, and a
//! file whose lines all end in CRLF is shown with LF and written back with
//! CRLF.

use std::borrow::Cow;

/// The byte order mark of UTF-8.
const UTF8_BOM: [u8; 3] = [0xef, 0xbb, 0xbf];
/// The byte order mark of UTF-16, little-endian.
const UTF16_LE_BOM: [u8; 2] = [0xff, 0xfe];
/// The byte order mark of UTF-16, big-endian.
const UTF16_BE_BOM: [u8; 2] = [0xfe, 0xff];

/// The bytes at the start of a file in which a NUL byte marks it as binary.
const SNIFF_BYTES: usize = 8 * 1024;

/// How a file stores its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Encoding {
    /// UTF-8 without a byte order mark, which new files get.
    Utf8,
    /// UTF-8 with a byte order mark.
    Utf8WithBom,
    /// UTF-16, little-endian, with a byte order mark.
    Utf16Le,
    /// UTF-16, big-endian, with a byte order mark.
    Utf16Be,
}

/// A file's text as the tools show and change it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileText {
    /// The text, with CRLF shown as LF when every line ends in CRLF.
    pub(crate) text: String,
    /// How the file stores it.
    pub(crate) encoding: Encoding,
    /// Whether every line ended in CRLF, so a write turns LF back into CRLF.
    pub(crate) crlf: bool,
}

impl FileText {
    /// The text of a new file: UTF-8 with LF.
    pub(crate) fn new_file(text: String) -> Self {
        Self {
            text,
            encoding: Encoding::Utf8,
            crlf: false,
        }
    }

    /// The bytes to write, in the file's encoding and line endings.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let text: Cow<'_, str> = if self.crlf {
            // Content that already has CRLF keeps it, without doubling the CR.
            Cow::Owned(self.text.replace("\r\n", "\n").replace('\n', "\r\n"))
        } else {
            Cow::Borrowed(&self.text)
        };
        match self.encoding {
            Encoding::Utf8 => text.as_bytes().to_vec(),
            Encoding::Utf8WithBom => [&UTF8_BOM[..], text.as_bytes()].concat(),
            Encoding::Utf16Le => UTF16_LE_BOM
                .into_iter()
                .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
                .collect(),
            Encoding::Utf16Be => UTF16_BE_BOM
                .into_iter()
                .chain(text.encode_utf16().flat_map(u16::to_be_bytes))
                .collect(),
        }
    }
}

/// Why bytes are not text that the tools handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotText {
    /// A NUL byte near the start, outside UTF-16, or a NUL character.
    Binary,
    /// Not valid UTF-8, or not valid UTF-16 after its byte order mark.
    UnsupportedEncoding,
}

/// Decodes a file's bytes.
pub(crate) fn decode(bytes: &[u8]) -> Result<FileText, NotText> {
    let (encoding, body) = if let Some(body) = bytes.strip_prefix(&UTF8_BOM) {
        (Encoding::Utf8WithBom, body)
    } else if let Some(body) = bytes.strip_prefix(&UTF16_LE_BOM) {
        (Encoding::Utf16Le, body)
    } else if let Some(body) = bytes.strip_prefix(&UTF16_BE_BOM) {
        (Encoding::Utf16Be, body)
    } else {
        (Encoding::Utf8, bytes)
    };
    let text = match encoding {
        Encoding::Utf8 | Encoding::Utf8WithBom => {
            if body[..body.len().min(SNIFF_BYTES)].contains(&0) {
                return Err(NotText::Binary);
            }
            String::from_utf8(body.to_vec()).map_err(|_invalid| NotText::UnsupportedEncoding)?
        }
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let text = decode_utf16(body, encoding == Encoding::Utf16Le)?;
            if text.contains('\0') {
                return Err(NotText::Binary);
            }
            text
        }
    };
    let crlf = every_line_ends_in_crlf(&text);
    let text = if crlf {
        text.replace("\r\n", "\n")
    } else {
        text
    };
    Ok(FileText {
        text,
        encoding,
        crlf,
    })
}

/// Decodes UTF-16 code units.
fn decode_utf16(body: &[u8], little_endian: bool) -> Result<String, NotText> {
    let (pairs, rest) = body.as_chunks::<2>();
    if !rest.is_empty() {
        return Err(NotText::UnsupportedEncoding);
    }
    let units = pairs.iter().map(|pair| {
        if little_endian {
            u16::from_le_bytes(*pair)
        } else {
            u16::from_be_bytes(*pair)
        }
    });
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_unpaired| NotText::UnsupportedEncoding)
}

/// Whether the text has line breaks and every one of them is CRLF.
fn every_line_ends_in_crlf(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut breaks = 0_usize;
    for (index, _) in text.match_indices('\n') {
        if index == 0 || bytes[index - 1] != b'\r' {
            return false;
        }
        breaks += 1;
    }
    breaks > 0
}

/// The lines of a text without their line breaks. A final line break ends the
/// last line instead of starting an empty one, as editors count lines.
pub(crate) fn lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.split('\n').collect()
}

#[cfg(test)]
mod tests {
    use super::{Encoding, FileText, NotText, decode, lines};

    #[test]
    fn utf8_text_reads_as_it_is() {
        let text = decode("first\nsecond\n".as_bytes()).unwrap();

        assert_eq!(text, FileText::new_file("first\nsecond\n".to_owned()));
        assert_eq!(text.encode(), b"first\nsecond\n");
    }

    #[test]
    fn crlf_files_show_lf_and_write_crlf() {
        let text = decode(b"first\r\nsecond\r\n").unwrap();

        assert_eq!(text.text, "first\nsecond\n");
        assert!(text.crlf);
        let changed = FileText {
            text: "first\nchanged\r\nthird\n".to_owned(),
            ..text
        };
        assert_eq!(changed.encode(), b"first\r\nchanged\r\nthird\r\n");
    }

    #[test]
    fn mixed_line_endings_stay_as_they_are() {
        let text = decode(b"first\r\nsecond\nthird\r\n").unwrap();

        assert_eq!(text.text, "first\r\nsecond\nthird\r\n");
        assert!(!text.crlf);
        assert_eq!(text.encode(), b"first\r\nsecond\nthird\r\n");
    }

    #[test]
    fn byte_order_marks_are_kept() {
        let utf8 = decode(b"\xef\xbb\xbfhello\n").unwrap();
        assert_eq!(utf8.encoding, Encoding::Utf8WithBom);
        assert_eq!(utf8.text, "hello\n");
        assert_eq!(utf8.encode(), b"\xef\xbb\xbfhello\n");

        let little = decode(b"\xff\xfeh\x00i\x00\r\x00\n\x00").unwrap();
        assert_eq!(little.encoding, Encoding::Utf16Le);
        assert_eq!(little.text, "hi\n");
        assert_eq!(little.encode(), b"\xff\xfeh\x00i\x00\r\x00\n\x00");

        let big = decode(b"\xfe\xff\x00h\x00i").unwrap();
        assert_eq!(big.encoding, Encoding::Utf16Be);
        assert_eq!(big.text, "hi");
        assert_eq!(big.encode(), b"\xfe\xff\x00h\x00i");
    }

    #[test]
    fn binary_and_foreign_bytes_are_refused() {
        assert_eq!(decode(b"PK\x03\x04\x00\x00"), Err(NotText::Binary));
        assert_eq!(decode(b"caf\xe9\n"), Err(NotText::UnsupportedEncoding));
        assert_eq!(decode(b"\xff\xfeh\x00i"), Err(NotText::UnsupportedEncoding));
        // An unpaired surrogate.
        assert_eq!(
            decode(b"\xff\xfe\x00\xd8"),
            Err(NotText::UnsupportedEncoding)
        );
        assert_eq!(decode(b"\xff\xfeh\x00\x00\x00"), Err(NotText::Binary));
    }

    #[test]
    fn only_the_first_8_kib_decide_whether_a_file_is_binary() {
        let mut bytes = vec![b'a'; 9 * 1024];
        bytes.push(0);

        assert!(decode(&bytes).is_ok_and(|text| text.text.ends_with('\0')));
    }

    #[test]
    fn lines_count_like_an_editor() {
        assert_eq!(lines(""), Vec::<&str>::new());
        assert_eq!(lines("one"), vec!["one"]);
        assert_eq!(lines("one\n"), vec!["one"]);
        assert_eq!(lines("one\n\n"), vec!["one", ""]);
        assert_eq!(lines("one\r\ntwo"), vec!["one\r", "two"]);
    }
}
