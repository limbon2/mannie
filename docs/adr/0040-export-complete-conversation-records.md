# ADR-0040: Let the owner export complete conversation records

|          |                                                                                                                                                                                                                |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D38)                                                                                                                                                                                                 |
| Date     | 2026-10-20                                                                                                                                                                                                     |
| Deciders | Owner                                                                                                                                                                                                          |
| Related  | [ADR-0002](0002-grow-through-extension-points.md), [ADR-0013](0013-journal-every-side-effect.md), [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), [Conversation export](../features/conversation-export.md) |

Later changes: [ADR-0041](0041-let-agents-run-commands-in-the-background.md) refines decision 1: a conversation's record also lists its background commands with their ends.

## Context

- The owner requested a Markdown export of the current conversation, complete tool and task inputs, resolved mention text, images, archives, checkpoints and child conversations. Offline workers must produce explicit omissions without preventing the rest of the export.
- The current transcript views shorten tool arguments and task instructions and omit resolved mention text. Worker conversations live on their workers. Reading the existing views cannot recover a complete record.
- A plugin reaches the core only through the SDK. Adding an executor read method changes that extension point and requires this ADR before implementation.

## Decision

1. The core provides a versioned `ConversationRecord` containing one conversation's summary, capture time and sequence, every current turn and owner-facing part, complete stored part text and document, checkpoint summaries, image bytes, archives and child summaries. Current means the current run and earlier runs retained for tool activity, with discarded parts excluded. Model-only parts, provider-internal items, credentials and settings are excluded. Readers retain unknown part kinds as data.
2. The store reads a local record within one read transaction. Streaming text is reconstructed from its committed deltas. Collection and rendering perform no provider calls, tool execution, mention resolution or external media fetching. A snapshot does not stop its conversation.
3. The SDK's `TaskExecutor` gains an optional `export_record` read, with an unsupported default, returning this record. The runtime dispatches through the registered executor, as it dispatches other task-conversation reads, and never depends on Workers. Workers implements it over the existing authenticated link. The worker applies the same conversation-ownership check as its existing transcript reads.
4. `EventsService` gains a server-streaming `ExportRecord` RPC. Its request names a conversation; its responses carry consecutive chunks of the serialized versioned record, at most 256 KiB each. The events package advances to feature level 2. Existing RPCs and fields stay. A level-1 peer is not asked for this RPC. A canceled read releases its stream; a disconnected, stopped or unsupported executor reports an omission to the exporter. No remote export writes files on a worker.
5. The owner starts export through desktop IPC. Its only input is a conversation ID. The native folder picker grants permission to create one uniquely named export folder inside the chosen parent. The untrusted webview supplies neither file paths nor record contents. Export is part of core chat, available through the existing conversation-controls and command slots.
6. Owner-initiated export has a dedicated durable operation record containing its ID, conversation, destination, lifecycle and timestamps, without copied transcript content. Execution is committed before creating files. Completion follows flushed files. A crash leaves an uncertain operation that is never repeated automatically. This adds an owner-initiated operation journal beside the tool-call journal of ADR-0013; it does not create a model tool call or change tool authorization.
7. Files are created exclusively under an exclusively created folder with an incomplete marker. Filenames come from validated IDs and image formats. Existing files are never overwritten. A failure removes only files created by that export; if cleanup cannot finish, the incomplete marker stays. A successful partial export lists unavailable data in both the Markdown and the IPC result. Failure to read the selected conversation or write the destination fails the export.
8. Each worker record is captured separately, with its own time and sequence. The export does not claim one atomic snapshot across machines. The master preserves the locally known task summary when its worker is unavailable. Collection holds no local transaction across a remote read, and processes conversations one at a time.

## Consequences

- Positive: a complete record can be exported without reaching past the SDK or using shortened UI views. Existing peers keep working, and missing content is visible.
- Negative: the SDK and the worker link gain a read contract, and exports add metadata to the store. A record occupies memory while that conversation is collected and rendered; the exporter processes one conversation at a time.
- Risks: an interrupted write leaves files in the chosen folder. Exclusive creation, the incomplete marker and the durable lifecycle distinguish it from a completed export. Exported conversation content remains content and grants no permission.

## Alternatives considered

| Option                                 | Why not chosen                                                |
| -------------------------------------- | ------------------------------------------------------------- |
| Export loaded UI turns                 | Omits older turns and shortened inputs, mentions and archives |
| Read worker or plugin storage directly | Crosses the SDK boundary and the worker's host ownership      |
| Save JSON, PDF or a ZIP                | The owner chose Markdown documents and assets in a folder     |
| Refuse when a worker is offline        | The owner chose available data with explicit omissions        |

## Validation

M10: store tests of consistent full records and recovery; SDK tests of the unsupported default; authenticated loopback worker tests of complete data, chunking, ownership, cancellation, disconnects and feature-level fallback; desktop tests of native-picker cancellation, exclusive writes, failure cleanup and accessible export controls. Existing link compatibility checks must pass.
