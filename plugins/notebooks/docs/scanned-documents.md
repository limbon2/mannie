# Scanned documents

|              |                                                                                            |
| ------------ | ------------------------------------------------------------------------------------------ |
| Status       | Idea                                                                                       |
| Milestone    | Unscheduled                                                                                |
| Last updated | 2026-10-24                                                                                 |
| Part of      | [Notebooks](README.md)                                                                     |
| Related      | [extract documents](adr/0001-extract-documents-with-pdf-extract-and-a-markdown-scanner.md) |

## Intent

A scanned PDF holds pictures of its pages and no text, so version 1 refuses it with "This PDF has no text, it may be a scan". Text recognition would read such pages, and images of documents too.

## Open questions

- Whether recognition runs on a server, as the models of search do, or inside Mannie, and which engine.
- How the owner sees the quality of the recognized text before it reaches the model.
