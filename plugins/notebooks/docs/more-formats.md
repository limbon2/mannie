# More formats

|              |                                                                                            |
| ------------ | ------------------------------------------------------------------------------------------ |
| Status       | Idea                                                                                       |
| Milestone    | Unscheduled                                                                                |
| Last updated | 2026-10-24                                                                                 |
| Part of      | [Notebooks](README.md)                                                                     |
| Related      | [extract documents](adr/0001-extract-documents-with-pdf-extract-and-a-markdown-scanner.md) |

## Intent

Version 1 reads PDF, Markdown and text files. Notebooks could also read Word documents, HTML pages saved to disk, EPUB books, and the source files of programs, each split by its own structure.

## Open questions

- Which formats matter first to the owner.
- Which crates read them under the license policy and without a native library, and whether each needs a decision of its own.
