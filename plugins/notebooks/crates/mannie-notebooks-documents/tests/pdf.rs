//! The text of PDFs (plugin ADR-0001 and ADR-0002): pages with text become
//! sections by their numbers, a document without text is refused, one of
//! many pages reads every page, a page that the parser panics on costs only
//! that page, and a corpus of malformed files fails with a code instead of
//! ending the process (T-64). The PDFs are written here, so the tests carry
//! no binary fixtures.

#![cfg(test)]

use mannie_notebooks_documents::{DocumentKind, ExtractError, Location, extract};

/// What one page of a test PDF draws.
enum Page<'a> {
    /// A line of text in Helvetica.
    Text(&'a str),
    /// A line, and no text.
    Drawing,
    /// Raw operators, such as broken ones.
    Operators(&'a str),
}

/// The bytes of the image `/Im1`, which read as drawing operators as `m`
/// with one of its two operands.
const IMAGE_BYTES: &str = "10 m";

/// A PDF 1.4 with one page for each item, with a correct cross-reference
/// table. Every page can draw the image `/Im1`.
fn pdf(pages: &[Page<'_>]) -> Vec<u8> {
    let first_page = 4;
    let image = first_page + 2 * pages.len();
    let kids: Vec<String> = (0..pages.len())
        .map(|index| format!("{} 0 R", first_page + 2 * index))
        .collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            pages.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    for (index, page) in pages.iter().enumerate() {
        let contents = first_page + 2 * index + 1;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> /XObject << /Im1 {image} 0 R >> >> /Contents {contents} 0 R >>"
        ));
        let stream = match page {
            Page::Text(text) => format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET"),
            Page::Drawing => "10 10 m 100 100 l S".to_owned(),
            Page::Operators(operators) => (*operators).to_owned(),
        };
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ));
    }
    objects.push(format!(
        "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length {} >>\nstream\n{IMAGE_BYTES}\nendstream",
        IMAGE_BYTES.len()
    ));
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).bytes());
    }
    let table = bytes.len();
    bytes.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for offset in offsets {
        bytes.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    bytes.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{table}\n%%EOF\n",
            objects.len() + 1
        )
        .bytes(),
    );
    bytes
}

#[test]
fn pages_with_text_become_sections_by_their_numbers() {
    let bytes = pdf(&[
        Page::Text("The sky is blue."),
        Page::Drawing,
        Page::Text("Bread needs flour."),
    ]);

    let document = extract(DocumentKind::Pdf, bytes).unwrap();

    assert_eq!(document.pages, Some(3));
    let pages: Vec<(Location, &str)> = document
        .sections
        .iter()
        .map(|section| (section.location.clone(), section.text.as_str()))
        .collect();
    assert_eq!(
        pages,
        [
            (Location::Page(1), "The sky is blue."),
            (Location::Page(3), "Bread needs flour."),
        ]
    );
}

#[test]
fn a_page_that_the_parser_panics_on_costs_that_page_and_no_other() {
    // pdf-extract panics on `m` with one of its two operands.
    let bytes = pdf(&[Page::Operators("10 m"), Page::Text("Bread needs flour.")]);

    let document = extract(DocumentKind::Pdf, bytes).unwrap();

    assert_eq!(document.pages, Some(2));
    let pages: Vec<(Location, &str)> = document
        .sections
        .iter()
        .map(|section| (section.location.clone(), section.text.as_str()))
        .collect();
    assert_eq!(pages, [(Location::Page(2), "Bread needs flour.")]);
}

#[test]
fn a_page_keeps_its_text_when_it_draws_an_image_whose_bytes_read_as_broken_operators() {
    let bytes = pdf(&[Page::Operators(
        "BT /F1 12 Tf 72 720 Td (Beside the image.) Tj ET /Im1 Do",
    )]);

    let document = extract(DocumentKind::Pdf, bytes).unwrap();

    assert_eq!(document.sections.len(), 1);
    assert_eq!(document.sections[0].text, "Beside the image.");
}

#[test]
fn a_pdf_whose_every_page_the_parser_panics_on_is_unreadable() {
    let bytes = pdf(&[
        Page::Operators("10 m"),
        Page::Operators("BT /F1 12 Tf Tj ET"),
    ]);

    assert_eq!(
        extract(DocumentKind::Pdf, bytes),
        Err(ExtractError::Unreadable)
    );
}

#[test]
fn a_pdf_without_text_is_refused_as_one_that_holds_none() {
    let bytes = pdf(&[Page::Drawing, Page::Drawing]);

    assert_eq!(extract(DocumentKind::Pdf, bytes), Err(ExtractError::NoText));
}

#[test]
fn a_pdf_of_many_pages_reads_every_page() {
    // More than the 2,000 pages that version 1 refused (plugin ADR-0002).
    let mut pages: Vec<Page<'_>> = (0..2_100).map(|_page| Page::Drawing).collect();
    pages.push(Page::Text("The last page."));

    let document = extract(DocumentKind::Pdf, pdf(&pages)).unwrap();

    assert_eq!(document.pages, Some(2_101));
    assert_eq!(document.sections.len(), 1);
    assert_eq!(document.sections[0].location, Location::Page(2_101));
}

#[test]
fn malformed_files_fail_with_a_code_and_never_end_the_process() {
    let valid = pdf(&[Page::Text("Hello")]);
    let corpus: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("not a PDF", b"This is plain text, not a PDF.".to_vec()),
        ("only the header", b"%PDF-1.7\n".to_vec()),
        ("cut in half", valid[..valid.len() / 2].to_vec()),
        (
            "a cross-reference past the end",
            String::from_utf8_lossy(&valid)
                .replace("startxref\n", "startxref\n999999")
                .into_bytes(),
        ),
        (
            "deep nesting",
            format!("%PDF-1.4\n1 0 obj\n{}\nendobj\n", "[".repeat(100_000)).into_bytes(),
        ),
        (
            "a text operator without its operand",
            pdf(&[Page::Operators("BT /F1 12 Tf Tj ET")]),
        ),
        (
            "a font that does not exist",
            pdf(&[Page::Operators("BT /F9 12 Tf (Hi) Tj ET")]),
        ),
        (
            "random bytes",
            (0..4_096_u32)
                .map(|value| value.wrapping_mul(2_654_435_761).to_be_bytes()[0])
                .collect(),
        ),
    ];

    for (name, bytes) in corpus {
        let outcome = extract(DocumentKind::Pdf, bytes);

        assert!(
            matches!(
                outcome,
                Err(ExtractError::Unreadable | ExtractError::NoText) | Ok(_)
            ),
            "{name}: {outcome:?}"
        );
    }
}

#[test]
fn deep_nesting_is_refused_before_the_parser_reads_it() {
    let bytes = format!("%PDF-1.4\n1 0 obj\n{}\nendobj\n", "<<".repeat(1_000)).into_bytes();

    assert_eq!(
        extract(DocumentKind::Pdf, bytes),
        Err(ExtractError::Unreadable)
    );
}
