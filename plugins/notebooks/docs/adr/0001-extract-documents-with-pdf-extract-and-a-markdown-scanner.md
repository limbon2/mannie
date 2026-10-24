# ADR-0001: Extract the text of documents with pdf-extract and a Markdown scanner

|          |                                                                                                                                                                                                                                                                                                                                                                                                      |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D43)                                                                                                                                                                                                                                                                                                                                                                                       |
| Date     | 2026-10-24                                                                                                                                                                                                                                                                                                                                                                                           |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                |
| Related  | [Notebooks](../README.md), [ADR-0017](../../../../docs/adr/0017-license-and-dependency-policy.md), [ADR-0034](../../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), [ADR-0035](../../../../docs/adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), [Security](../../../../docs/architecture/security.md), [Technology stack](../../../../docs/architecture/technology-stack.md) |

## Context

- The plugin Notebooks reads the owner's PDF, Markdown and text files, splits their text into passages and keeps the passages with their place in the document, a page or a heading, so that a citation can say where a passage stands ([Notebooks](../README.md)).
- The files can come from anywhere, so their structure can be hostile: a PDF with cycles of references, huge streams or broken fonts (T-64). The plugin Images meets the same threat for images with a size cap, a pure-Rust decoder and a blocking thread of its own (T-32).
- Facts verified on 2026-10-24:
  - pdf-extract 0.12.1 (MIT) extracts the text of a PDF from memory, page by page, with `extract_text_from_mem_by_pages`. It depends on lopdf 0.42 without lopdf's default features, and on adobe-cmap-parser, cff-parser, encoding_rs, euclid, postscript, type1-encoding-parser, unicode-normalization and log.
  - pdf_oxide 0.3.78 (MIT or Apache-2.0) extracts text and Markdown, but depends on `image`, which `plugins/images/plugin.toml` makes the Images plugin's own (`[crates] exclusive`).
  - pdfium-render binds Google's PDFium, a C++ library that would have to ship beside the desktop app.
- Markdown needs only its structure for passages: headings, fenced code, which may hold lines that look like headings, and front matter.
- The SDK's file dialog reads a file once, by its handle, and a drop reaches the plugin as bytes (ADR-0034, decision 6), so the plugin never holds a path.

## Decision

1. The plugin reads PDFs with pdf-extract, pinned at `=0.12.1`, page by page from the bytes of the file. Only this plugin may depend on pdf-extract and lopdf, which its manifest's `[crates]` records.
2. An extraction runs on a thread of its own with a stack of 256 MiB, inside `catch_unwind`, so that a panic of the parser fails its source and nothing else. A stack that overflows ends the process, which `catch_unwind` cannot stop, so a PDF whose arrays and dictionaries nest deeper than 256 levels outside its streams is refused before the parser reads it. An extraction refuses a file of more than 32 MiB before reading it, and a PDF of more than 2,000 pages, and keeps at most 16 MiB of text. A PDF that is encrypted with a password fails with `encrypted`, one that pdf-extract cannot read with `unreadable`, and one whose pages hold no text, such as a scan, with `no_text`.
3. Markdown is split by a scanner of the plugin's own, line by line: ATX and setext headings outside fenced code make the sections, each named by the path of its headings, and front matter between `---` lines at the top is left out. Text files are one section. Both must be UTF-8, with or without a byte order mark, and fail with `not_utf8` otherwise.
4. The plugin keeps the text of each page or section, and never a copy of the file. Removing a source deletes its text.

## Consequences

- Positive: the plugin's extraction is Rust, with no native library to ship, and a hostile file costs its own source at most. The text that a citation shows is the text that the model read.
- Negative: pdf-extract reads the text in the order of the page's content, which can mix columns and tables, and finds nothing in a scanned page. The owner sees the extracted text in the source viewer.
- Risks: a file that makes the parser loop never returns its thread. The size and page limits bound the work, and the source shows as Reading until the plugin stops; the owner removes it. Nesting inside a compressed stream is out of the scan's sight: the large stack bounds it, and a deeper one still ends the process (T-64).

## Alternatives considered

| Option                                    | Why not chosen                                                                                 |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------- |
| pdf_oxide                                 | It depends on `image`, which only the plugin Images may use                                    |
| pdfium-render with PDFium                 | A C++ library beside the app, with its memory safety and its builds for each platform          |
| A Markdown parser, such as pulldown-cmark | Passages need only headings, fences and front matter, which a scanner of a hundred lines finds |
| Keeping a copy of each file               | The database would grow by every file, and the text is what search and citations need          |

## Validation

M13: the plugin's tests of extraction with fixture PDFs and Markdown files and with a corpus of malformed files, and the spike of step 1 on the owner's own PDFs ([M13 validation results](../../../../docs/roadmap/m13-validation.md)).
