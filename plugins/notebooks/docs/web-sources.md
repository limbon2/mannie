# Web sources

|              |                                                                                                                         |
| ------------ | ----------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                    |
| Milestone    | Unscheduled                                                                                                             |
| Last updated | 2026-10-24                                                                                                              |
| Part of      | [Notebooks](README.md)                                                                                                  |
| Related      | [ADR-0034](../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), [Security](../../../docs/architecture/security.md) |

## Intent

NotebookLM takes a web page or a video by its address. Mannie's notebooks take files and pasted text in version 1. A web source would fetch a page by its address, keep its text, and fetch it again when the owner asks.

## Open questions

- Which part of Mannie fetches the page, since the plugin has no network grant, and how a fetch is limited to the addresses that the owner gave.
- How a page's text is taken from its HTML, and how a page that needs a sign-in or script is refused.
