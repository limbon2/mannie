//! The text of the documents of the plugin Notebooks (plugin ADR-0001): the
//! pages of a PDF through pdf-extract, the sections of a Markdown file through
//! a scanner of its headings, and a text file as one section, each with its
//! place in the document, and the passages that the plugin embeds.
//!
//! The files come from anywhere, so their structure can be hostile (T-64).
//! No size, page count or length of text is refused (plugin ADR-0002): a PDF
//! is read on a thread of its own with a large stack, each page inside
//! `catch_unwind`, so that a parser that panics costs the page it read. The
//! crate depends on no part of Mannie, and its callers run it on a blocking
//! thread.

mod chunk;
mod markdown;
mod pdf;
mod text;

pub use chunk::{CHUNKER_VERSION, ChunkOptions, Passage, passages};

/// What a document is, which decides how its text is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocumentKind {
    /// A PDF, read page by page.
    Pdf,
    /// A Markdown file, read section by section.
    Markdown,
    /// A text file, read as one section.
    Text,
}

impl DocumentKind {
    /// The kind of a file by the extension of its name, without regard to
    /// case: `pdf`, `md` or `markdown`, and `txt`.
    pub fn from_file_name(name: &str) -> Option<Self> {
        let (_stem, extension) = name.rsplit_once('.')?;
        match extension.to_ascii_lowercase().as_str() {
            "pdf" => Some(Self::Pdf),
            "md" | "markdown" => Some(Self::Markdown),
            "txt" => Some(Self::Text),
            _ => None,
        }
    }

    /// The kind's name, as the plugin stores it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Markdown => "markdown",
            Self::Text => "text",
        }
    }

    /// The kind with this name.
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Pdf, Self::Markdown, Self::Text]
            .into_iter()
            .find(|kind| kind.as_str() == name)
    }
}

/// Where a section stands in its document.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Location {
    /// A page of a PDF, counting from 1.
    Page(u32),
    /// A section of a Markdown file, named by the path of its headings, which
    /// is empty before the first heading.
    Headings(Vec<String>),
    /// The whole of a text.
    Whole,
}

/// A page or a section of a document and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Where it stands.
    pub location: Location,
    /// Its text, with its white space tidied and never empty.
    pub text: String,
}

/// The text of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Its pages or sections with text, in their order.
    pub sections: Vec<Section>,
    /// How many pages a PDF has, those without text included.
    pub pages: Option<u32>,
}

/// Why a document's text could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExtractError {
    /// The document holds no text, such as a scanned PDF.
    #[error("the document holds no text")]
    NoText,
    /// The PDF is encrypted with a password.
    #[error("the PDF is encrypted")]
    Encrypted,
    /// The file is not a PDF that can be read, or the parser failed on every
    /// page.
    #[error("the document could not be read")]
    Unreadable,
    /// A Markdown or text file is neither UTF-8 nor UTF-16 with a byte order
    /// mark.
    #[error("the text is not UTF-8")]
    NotUtf8,
}

impl ExtractError {
    /// The error's stable code, which the plugin stores and the UI words.
    pub fn code(self) -> &'static str {
        match self {
            Self::NoText => "no_text",
            Self::Encrypted => "encrypted",
            Self::Unreadable => "unreadable",
            Self::NotUtf8 => "not_utf8",
        }
    }
}

/// Reads the text of a document. It blocks: a PDF is read on a thread of its
/// own, so the caller runs this on a blocking thread.
pub fn extract(kind: DocumentKind, bytes: Vec<u8>) -> Result<Document, ExtractError> {
    let document = match kind {
        DocumentKind::Pdf => pdf::extract(bytes)?,
        DocumentKind::Markdown => markdown::extract(&text::decode(&bytes)?),
        DocumentKind::Text => text::extract(&text::decode(&bytes)?),
    };
    if document.sections.is_empty() {
        return Err(ExtractError::NoText);
    }
    Ok(document)
}

/// Text with its white space tidied: line endings as `\n`, no trailing white
/// space on a line, at most one empty line in a row, runs of spaces and tabs
/// as one space, and no white space at the start or the end.
pub(crate) fn tidy(text: &str) -> String {
    let mut tidied = String::with_capacity(text.len());
    let mut blank_lines = 0;
    for line in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        let mut words = line
            .split([' ', '\t', '\u{a0}'])
            .filter(|word| !word.is_empty());
        let Some(first) = words.next() else {
            blank_lines += 1;
            continue;
        };
        if !tidied.is_empty() {
            tidied.push_str(if blank_lines > 0 { "\n\n" } else { "\n" });
        }
        blank_lines = 0;
        tidied.push_str(first);
        for word in words {
            tidied.push(' ');
            tidied.push_str(word);
        }
    }
    tidied
}

#[cfg(test)]
mod tests {
    use super::{DocumentKind, ExtractError, Location, extract, tidy};

    #[test]
    fn kinds_follow_the_extension_without_regard_to_case() {
        assert_eq!(
            DocumentKind::from_file_name("Paper.PDF"),
            Some(DocumentKind::Pdf)
        );
        assert_eq!(
            DocumentKind::from_file_name("notes.md"),
            Some(DocumentKind::Markdown)
        );
        assert_eq!(
            DocumentKind::from_file_name("guide.markdown"),
            Some(DocumentKind::Markdown)
        );
        assert_eq!(
            DocumentKind::from_file_name("read.me.txt"),
            Some(DocumentKind::Text)
        );
        assert_eq!(DocumentKind::from_file_name("image.png"), None);
        assert_eq!(DocumentKind::from_file_name("pdf"), None);
        for kind in [
            DocumentKind::Pdf,
            DocumentKind::Markdown,
            DocumentKind::Text,
        ] {
            assert_eq!(DocumentKind::parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn white_space_is_tidied_and_paragraphs_kept() {
        assert_eq!(
            tidy("  First   line\t here  \r\n\r\n\r\n\nSecond\u{a0}\u{a0}line \n  third  "),
            "First line here\n\nSecond line\nthird"
        );
        assert_eq!(tidy(" \n\t\n "), "");
    }

    #[test]
    fn a_text_of_any_length_keeps_all_of_it() {
        // Larger than the 16 MiB that version 1 kept (plugin ADR-0002).
        let large = "word ".repeat(4 * 1024 * 1024);

        let document = extract(DocumentKind::Text, large.clone().into_bytes()).unwrap();

        assert_eq!(document.sections[0].text, large.trim_end());
    }

    #[test]
    fn an_empty_text_holds_no_text() {
        assert_eq!(
            extract(DocumentKind::Text, b" \n \n".to_vec()),
            Err(ExtractError::NoText)
        );
        assert_eq!(
            extract(DocumentKind::Text, b"Hello".to_vec())
                .unwrap()
                .sections[0]
                .location,
            Location::Whole
        );
    }

    #[test]
    fn every_error_has_a_code_of_its_own() {
        let errors = [
            ExtractError::NoText,
            ExtractError::Encrypted,
            ExtractError::Unreadable,
            ExtractError::NotUtf8,
        ];
        let codes: std::collections::BTreeSet<&str> =
            errors.iter().map(|error| error.code()).collect();

        assert_eq!(codes.len(), errors.len());
    }
}
