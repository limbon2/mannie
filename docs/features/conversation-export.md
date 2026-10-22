# Conversation export

|              |                                                                                                                        |
| ------------ | ---------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                               |
| Milestone    | M10                                                                                                                    |
| Last updated | 2026-10-21                                                                                                             |
| Related      | [Chat](chat/README.md), [ADR-0040](../adr/0040-export-complete-conversation-records.md), [Feature protocol](README.md) |

## Intent

The owner wants to export a conversation as Markdown, including its full current record and its child conversations. Images and complete archived tool outputs travel with the documents in one folder. The export remains useful when a worker cannot be reached and identifies everything that is missing.

## Behavior

- The conversation toolbar and command palette offer "Export conversation…", including in a side panel. The native folder picker chooses the destination parent. Canceling creates nothing.
- Each export creates a new folder. It holds `conversation.md`, child documents under `tasks/`, stored images under `images/`, and archived output files under `outputs/`. Links between these files are relative.
- The documents include metadata, capture times and sequences, messages, stored reasoning, reply and call states, complete tool and task inputs, authorization decisions, results, resolved mention snapshots and checkpoint summaries. They exclude discarded and superseded attempts, except earlier tool activity retained in the current transcript.
- Active conversations continue. Their unfinished content is labeled as it was captured. Each worker record has its own capture time and sequence.
- Offline workers, disabled plugins, older workers and missing attachments produce an "Unavailable content" section and a visible warning. Available parent reports remain. Failure to read the selected conversation or write the destination fails the export.
- Export reads only saved records. It does not reread mentioned files, fetch external media, contact providers or execute tools. Provider credentials, global settings and opaque provider-internal records are excluded.
- The UI shows work in progress and the saved location. Only one export can run at a time. Failure names what failed and offers another attempt.

## Fit

The feature uses IPC commands, conversation-control, command and overlay slots, message catalogs, and a store module. [ADR-0040](../adr/0040-export-complete-conversation-records.md) adds complete executor reads and the owner-initiated operation journal. The owner accepted the written ADR on 2026-10-20 (D38). Existing tool authorization, transcript rendering, provider behavior and worker host ownership stay as they are.

## Open questions

None.

## Decisions

- 2026-10-20: the owner chose Markdown, a full current record, a folder with linked assets and children, available data with explicit omissions for offline workers, and complete worker export through a new read-only interface.
- 2026-10-20: the owner approved the written ADR-0040 (D38).
- 2026-10-20: the owner requested implementation of the reviewed plan. M10 records the feature; version changes remain part of release preparation.

## Design

- `export_conversation(ConversationRef)` opens the native picker in Rust and returns a typed canceled or saved result with its destination and warnings. Typed failures distinguish storage, missing conversations, dialogs and writes. The webview never supplies a destination or export body.
- A versioned `ConversationRecord` is a full read contract, distinct from shortened transcript views. The store captures complete current parts, their saved documents, current and retained reply metadata, committed streaming deltas, checkpoints, images, archives and child summaries consistently. The runtime dispatches remote reads through the optional executor method of ADR-0040.
- The record also lists the conversation's background commands with how each ended, and the Markdown names each command with the call that started it, whose archive holds its output ([ADR-0041](../adr/0041-let-agents-run-commands-in-the-background.md), [background commands](system-tools/background-commands.md)).
- The events link package advances to level 2 and gains a streaming complete-record read with chunks of at most 256 KiB. Existing level-1 reads remain available. Worker ownership checks and the authenticated link cover export reads as they cover transcript reads.
- A dedicated store module journals export metadata. An executing record is durable before file creation; completion follows flushes; startup marks interrupted execution uncertain. No export repeats automatically.
- The writer uses exclusive creation, an incomplete marker, validated ID filenames and escaped generated Markdown. Cleanup touches only paths created by the current export. Transcript content and filenames never authorize access. Unknown parts have a generic representation.
- UI state uses Svelte 5 runes. Existing shadcn-svelte components provide controls, progress and result feedback. Messages use the `export_` prefix, including generated document labels. Dates and numbers in the UI use `Intl`.

## Acceptance

- Full records include turns beyond view limits, complete inputs, resolved mentions, retained tool runs, checkpoints and committed streaming text, with discarded attempts excluded.
- Images match stored bytes, archived outputs remain complete, relative links resolve, and Unicode and embedded Markdown fences remain readable.
- Worker reads pass authenticated loopback tests of large chunked records, ownership, disconnects, shutdown, unsupported peers and partial export warnings.
- Native-picker cancellation writes nothing; collisions preserve existing files; injected write failures clean up owned files or retain the incomplete marker; startup recovery never retries writes.
- End-to-end coverage checks main and side-panel export, keyboard access, focus, announcements, both themes, pseudo-localization and accessibility on Windows and Linux.
- Bindings and catalogs regenerate; the Svelte autofixer, formatting, relevant tests, documentation checks and `task ci` pass. Machine-specific checks are reported only when run.

## Verification results

- 2026-10-20, Windows 11 and WebView2: the six native export checks pass. They cover picker cancellation, all 60 seeded turns beyond the initial page, keyboard access, focus in the result dialog, status announcements, both themes, the command palette, separate export folders, destination failures and retry, side panels, pseudo-localization and axe checks. The ten existing child-agent and side-panel regression checks also pass.
- 2026-10-20, Linux: `task test:linux` passes, including Clippy and the core and plugin Rust tests over the authenticated worker link.
- Validation builds reject the normal app identifier before startup opens owner data.
- 2026-10-21, GitHub-hosted Ubuntu 22.04 and WebKitGTK: all 23 desktop test files pass, including the six export checks, and the performance checks pass. On Ubuntu 24.04, all 14 checks of `task ci` pass.
- 2026-10-21, FreeBSD 14.4: the native core and worker tests pass. Worker builds for FreeBSD and static Linux, worker service checks on Windows and Linux, and all three credential-store checks also pass.
