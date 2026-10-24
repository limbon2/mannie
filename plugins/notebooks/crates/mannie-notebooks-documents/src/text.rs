//! Text files and the decoding of Markdown and text: UTF-8 with or without a
//! byte order mark, and UTF-16 with one, as Windows' editors may save it.

use crate::{Document, ExtractError, Location, Section, tidy};

/// The text of a file's bytes.
pub(crate) fn decode(bytes: &[u8]) -> Result<String, ExtractError> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).map_err(|_not_utf8| ExtractError::NotUtf8);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, u16::from_be_bytes);
    }
    String::from_utf8(bytes.to_vec()).map_err(|_not_utf8| ExtractError::NotUtf8)
}

fn decode_utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> Result<String, ExtractError> {
    let (pairs, odd) = bytes.as_chunks::<2>();
    if !odd.is_empty() {
        return Err(ExtractError::NotUtf8);
    }
    char::decode_utf16(pairs.iter().map(|pair| unit(*pair)))
        .collect::<Result<String, _>>()
        .map_err(|_unpaired| ExtractError::NotUtf8)
}

/// A text file as one section.
pub(crate) fn extract(text: &str) -> Document {
    let tidied = tidy(text);
    let sections = if tidied.is_empty() {
        Vec::new()
    } else {
        vec![Section {
            location: Location::Whole,
            text: tidied,
        }]
    };
    Document {
        sections,
        pages: None,
    }
}

#[cfg(test)]
mod tests {
    use super::decode;
    use crate::ExtractError;

    #[test]
    fn utf8_decodes_with_or_without_its_mark() {
        assert_eq!(decode("Grüße".as_bytes()).unwrap(), "Grüße");
        let marked = [&[0xEF, 0xBB, 0xBF][..], "Grüße".as_bytes()].concat();
        assert_eq!(decode(&marked).unwrap(), "Grüße");
    }

    #[test]
    fn utf16_decodes_with_its_mark_in_either_order() {
        let little: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("Hé".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        let big: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain("Hé".encode_utf16().flat_map(u16::to_be_bytes))
            .collect();

        assert_eq!(decode(&little).unwrap(), "Hé");
        assert_eq!(decode(&big).unwrap(), "Hé");
    }

    #[test]
    fn other_encodings_are_refused() {
        // "Grüße" in Windows-1252.
        assert_eq!(
            decode(&[0x47, 0x72, 0xFC, 0xDF, 0x65]),
            Err(ExtractError::NotUtf8)
        );
        assert_eq!(decode(&[0xFF, 0xFE, 0x41]), Err(ExtractError::NotUtf8));
        // An unpaired surrogate.
        assert_eq!(
            decode(&[0xFF, 0xFE, 0x00, 0xD8]),
            Err(ExtractError::NotUtf8)
        );
    }
}
