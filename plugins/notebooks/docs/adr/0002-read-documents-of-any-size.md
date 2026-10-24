# ADR-0002: Read documents of any size, page by page

|          |                                                                                                                                                                                                                                                                                                  |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Accepted (D44)                                                                                                                                                                                                                                                                                   |
| Date     | 2026-10-24                                                                                                                                                                                                                                                                                       |
| Deciders | Owner                                                                                                                                                                                                                                                                                            |
| Related  | [ADR-0001](0001-extract-documents-with-pdf-extract-and-a-markdown-scanner.md), [Notebooks](../README.md), [Security](../../../../docs/architecture/security.md), [M13 validation results](../../../../docs/roadmap/m13-validation.md), [D44](../../../../docs/roadmap/milestones.md#5-decisions) |

## Context

- ADR-0001, decision 2, refuses a file of more than 32 MiB and a PDF of more than 2,000 pages, keeps at most 16 MiB of text, and reads a PDF inside one `catch_unwind`. The plugin also refuses a notebook's 201st source and pasted text over 1 MiB, and a dropped file travels as one base64 string through one command of at most 44 MiB.
- The owner's administration guides broke both rules ([D44](../../../../docs/roadmap/milestones.md#5-decisions)):
  - Two Delinea guides of 53 and 55 MB were refused as too large. The owner wants no hard limits.
  - The BeyondTrust guide of 130 pages failed as `unreadable`. pdf-extract 0.12.1 reads every XObject that a page draws as drawing operators, images included. The bytes of an image on page 77 read as `m` with one of its two operands, and pdf-extract panics when it indexes the second. One panic failed the whole document.
- Facts verified on 2026-10-24 on the owner's seven PDFs, with the images emptied:
  - Every page of each one reads, from BeyondTrust, 130 pages in 0.6 s, to Delinea Secret Server, 1,546 pages in 27 s.
  - A second image panic, on page 167 of the Delinea Platform guide, goes away too.
  - PDF.js reads the same text from English and Russian pages of those guides.
- A JavaScript string ends near 1 GB in WebView2's V8, so a large file cannot travel as one base64 string.

## Decision

1. The plugin limits neither a file's size, nor a PDF's pages, nor the text it keeps, nor a notebook's sources, nor pasted text. The limits of names and titles stay.
2. A PDF is read page by page:
   - It loads once with pdf-extract's lopdf.
   - The content of every image stream is emptied, since an image holds no text and pdf-extract would read it as operators.
   - Each page is read with `pdf_extract::output_doc_page` inside its own `catch_unwind`. A page that fails or panics costs that page.
   - The document fails as `unreadable` only when no page could be read.
   - The thread with a stack of 256 MiB and the scan of nesting stay as ADR-0001 decided.
3. A dropped file travels in parts:
   - `start_upload` names the notebook and the file, whose type it checks at once.
   - `add_upload_part` sends parts of at most 4 MiB in order.
   - `finish_upload` adds the source as the file dialog does.
   - `cancel_upload` drops it.
   - The plugin keeps the parts in memory, and drops an upload whose part comes out of order or that receives no part for 5 minutes.
   - `add_dropped_source` goes.
4. A file from the dialog is read through the service `file-dialog` without a limit.

Earlier decisions that this ADR changes:

| ADR  | Decision | Change                                                                                                                  |
| ---- | -------- | ----------------------------------------------------------------------------------------------------------------------- |
| 0001 | d2       | Changed: no limits on size, pages or text, and each page of a PDF is read on its own with the images emptied            |
| 0001 | d4       | Unchanged: the plugin still keeps the text and never a copy, so a large file costs the database its text, not its bytes |

## Consequences

- Positive: the owner's large guides read, and a page with a broken drawing costs that page, while the text of the others stays.
- Negative: memory and time grow with the document. pdf-extract holds the whole PDF in memory while it reads it, and an upload holds its parts in memory until it finishes. A huge source takes long to read and embed, and the source shows its progress meanwhile.
- Risks: a hostile file is bounded only by the scan of nesting, the reader's own thread and the isolation of pages (T-64). The owner chose the file, and removes a source that never finishes. A panic that a page causes writes a crash file, as every panic does, and the log names the page's document.

## Alternatives considered

| Option                                                      | Why not chosen                                                                                                   |
| ----------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Higher limits                                               | The owner wants none, and any number refuses a guide that someone has                                            |
| PDF.js, as the earlier harness used                         | Its text matched pdf-extract's on the owner's guides, and it needs JavaScript outside the webview                |
| Removing the operators that pdf-extract panics on, by count | Emptying the images removes the cause that the owner's guides showed, and the isolation of pages covers the rest |
| One base64 string for a drop, with a larger command limit   | V8 cannot hold the string of a file near 1 GB                                                                    |

## Validation

M13: the documents crate's tests of a page that panics, of a page that draws an image whose bytes read as a broken operator, and of a document whose every page panics; the plugin's tests of uploads; and the owner's seven PDFs ([M13 validation results](../../../../docs/roadmap/m13-validation.md)).
