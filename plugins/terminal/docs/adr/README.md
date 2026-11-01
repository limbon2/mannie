# The terminal plugin's decision records

|              |                                                                                                                                    |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Living index                                                                                                                       |
| Last updated | 2026-10-30                                                                                                                         |
| Related      | [Terminals](../README.md), [Mannie's ADR index](../../../../docs/adr/README.md), [Template](../../../../docs/adr/0000-template.md) |

The terminal plugin's own decisions, numbered on their own ([ADR-0035](../../../../docs/adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), decision 11). A decision that changes the SDK or the core lives in [Mannie's index](../../../../docs/adr/README.md), such as [ADR-0053](../../../../docs/adr/0053-let-plugins-offer-terminals.md) and [ADR-0054](../../../../docs/adr/0054-let-tool-results-keep-what-they-cite-and-let-plugin-views-add-mentions.md), which this plugin builds on.

| ADR                                                               | Decision                                                                        | Status         | Validation |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------- | -------------- | ---------- |
| [0001](0001-mark-commands-with-shell-integration.md)              | Mark commands with shell integration, with the terminal's nonce in every marker | Accepted (D45) | M14        |
| [0002](0002-keep-a-headless-emulator-of-each-terminal-in-rust.md) | Keep a headless emulator of each terminal in Rust, with vt100                   | Accepted (D45) | M14        |
