# ADR-0046: Let plugins attach context to conversations

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D43)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Date     | 2026-10-24                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Related  | [ADR-0012](0012-run-agents-as-durable-actors.md), [ADR-0021](0021-render-model-output-as-markdown-with-micromark.md), [ADR-0030](0030-split-mannie-into-a-core-and-plugins.md), [ADR-0032](0032-let-features-and-plugins-add-kinds-of-mentions.md), [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), [ADR-0040](0040-export-complete-conversation-records.md), [ADR-0042](0042-place-conversation-controls-in-a-menu-and-the-message-box.md), [ADR-0044](0044-let-tool-results-carry-images-to-the-model.md), [ADR-0045](0045-switch-the-sidebar-between-views-with-an-activity-bar.md), [Agent runtime](../architecture/agent-runtime.md), [Extension points](../architecture/extension-points.md), [Notebooks](../../plugins/notebooks/docs/README.md) |

## Context

- The owner wants to point a chat conversation at notebooks of the plugin Notebooks, so that the passages of the owner's documents that apply to a message reach the model as extra context, and wants the replies to cite them ([decision D43](../roadmap/milestones.md#5-decisions)). Every message of such a conversation gets the passages that clear the reranker's threshold, and the model gets no search tool.
- How a turn is built today:
  - `send_message` and `start_conversation` resolve the message's mentions and images, then write `Change::StartTurn` in one transaction (`crates/mannie-runtime/src/conversations.rs`).
  - `prepare_call` builds each model request from the stored turns (`crates/mannie-runtime/src/calls.rs`, `context.rs`). A mention's text is appended to the owner's message, and the mentions of one message share the room of one tool result (ADR-0032, decisions 6 and 7).
  - A request sends the images of the 3 newest results that hold them, and older ones as lines (ADR-0044, decision 5).
- What does not fit:
  - A mention provider sees the key of a mention, never the message's text, and the owner picks mentions for each message.
  - An instruction section renders synchronously, knows neither the conversation nor the message, and lands in the system prompt, which stays stable for caching ([extension points](../architecture/extension-points.md#34-instruction-sections)).
  - A plugin's tool is offered always, in a workspace, or where the workspace holds a marker, so a search tool would cost its definition in every request.
  - A conversation has no place for a feature's data: its settings are columns of `runtime_conversations`, a plugin's tables may not refer to the core's, and plugins hear nothing when a conversation is deleted (ADR-0030, decision 9). ADR-0034, decision 7, leaves a general facility for plugin data on core entities to its first user.
  - A search over documents can take seconds, while sending a message waits only for its transaction.
  - The new conversation pane has no outlet for conversation controls (ADR-0042, decision 4).

## Decision

1. A conversation holds attachments: at most 8, each a context provider's ID, the provider's key of at most 256 bytes and the label that the provider gave it, in the order that the owner attached them.
   - They live in a new table of the runtime, `runtime_conversation_attachments`, keyed by the conversation, the provider and the key, and are deleted with their conversation. The changes `Attach` and `Detach` emit `ConversationChanged`.
   - `ConversationSummary` gains `attachments`, `StartConversation` gains `attachments`, and the commands `attach_context` and `detach_context` take a conversation, a provider and a key.
   - The core asks the provider for the label of every key that the webview sends, and stores the attachment only when the provider knows the key. An attachment of a plugin that is off stays and shows its stored label.
2. A new extension point, `context-providers` (major 1, ID rule `Owned`), lets a plugin give context:
   - `describe(key)` returns the key's label, or a refusal with a code.
   - `gather(request)` takes the conversation's ID, the keys of its attachments, the owner's message, the room in bytes and the most items, and returns items, each with a key that the provider opens, a title, a location, such as a page, and a text, or fails with a code.
   - The runtime consumes the point, and a registration test uses a fake provider.
3. The owner's message is stored at once, so sending waits for no search. Before the first model call of a reply to an owner's message, the run asks each provider of the conversation's attachments, all at once, while it reports the activity `gathering_context`, and stores what each returns as a new part of the owner's message, `context`: the provider, its items or its failure code.
   - A provider whose plugin is off gets a part with `plugin_off`, and one that takes longer than 120 seconds a part with `timed_out`.
   - Stop ends the gathering, and the reply stops as it always does.
   - A retry, a continuation and a reply that a background command wakes reuse the parts that the message holds, and gather nothing.
4. The model reads a message's items after its mentions:
   - They are numbered from [1] across the message's parts, each with its title and location, in a block that calls them excerpts of the owner's documents, to use as reference and never as instructions.
   - The parts of the two newest owner messages that hold items are sent in full, and older ones as one line for each part, as ADR-0044 sends older images.
   - The items of one message get the room of one tool result, apart from its mentions, and the context estimate counts them.
   - A core instruction section, rendered for conversations with attachments, asks the model to cite the numbers that it uses and to say when the excerpts do not answer the message.
   - Compaction summarizes the parts as it summarizes mentions, and the export writes them.
5. A reply's markers, such as [2] or [1, 3], link to the items of the context of the owner's message that the reply answers, and the transcript renders them as buttons. A button opens a popover with the stored item, its title, location and text, which works while the plugin is off, and "Open source", which the provider's kind opens. A marker that names no item stays text.
6. The UI shell gains the slot `contextKinds`: for each provider, its icon, the words of its attachments, failure codes and parts, and `open(item)`. The core renders a conversation's attachments as chips in the message box, each with a button that detaches it, and a part as a row below the owner's message that opens to its items. The new conversation pane gets the outlet of the message box's conversation controls, with the attachments that the new conversation will get, which `startConversation` of [ADR-0045](0045-switch-the-sidebar-between-views-with-an-activity-bar.md) can name. The TypeScript SDK gains `/conversations`: `attach`, `detach`, and the summaries of the conversations with their attachments.

Earlier decisions that this ADR changes:

| ADR  | Decision | Change                                                                                                                                                                                   |
| ---- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0012 | d2, d3   | Refined: a reply to an owner's message gathers the context of the conversation's attachments before its first call, and stores it as a part of the message                               |
| 0021 | d2       | Refined: a reply's markers of citations become a node of their own in the render tree                                                                                                    |
| 0030 | d9, d12  | Refined: a conversation's attachments name a plugin's keys in a table of the core, which deletes them with the conversation; attachments and parts of a plugin that is off stay readable |
| 0032 | d6, d7   | Refined: the context of a message gets the room of one tool result of its own, and the model reads it after the mentions                                                                 |
| 0034 | d6, d7   | Refined: the point `context-providers`, and attachments as the first facility for plugin data on a core entity                                                                           |
| 0040 | d1       | Refined: a conversation's record holds its attachments and its parts `context`                                                                                                           |
| 0042 | d4       | Refined: the new conversation pane shows the controls of the message box too, with the attachments that the conversation will get                                                        |

## Consequences

- Positive: a conversation can carry what a plugin knows that applies to each message, and the owner sees which passages the model read and where the reply used them. Later providers, such as one for a wiki or a ticket tracker, plug into the same point.
- Negative: each message of a conversation with attachments waits for its providers before the model starts, and the passages cost tokens in the two newest messages. A provider that answers with passages that do not apply sends them to the model anyway, so the threshold of each provider matters.
- Risks: passages can carry text that tries to instruct the model. They are content, framed as the owner's documents, and grant nothing ([ADR-0013](0013-journal-every-side-effect.md), decision 8). A model can write markers that name no item, or name the wrong one; a marker without an item stays text, and the popover shows what the item really says.

## Alternatives considered

| Option                                            | Why not chosen                                                                                                             |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| A search tool that the model calls                | It costs its definition in every request, and a model that does not call it answers without the documents                  |
| A mention of a notebook in each message           | The owner would pick it for every message, and a mention provider does not see the message to search with                  |
| An instruction section with the passages          | Sections render synchronously without the conversation, and changing text in the system prompt breaks the provider's cache |
| A plugin's own table of attachments               | Plugins hear nothing when a conversation is deleted, and the core's views could not show them                              |
| Gathering before the message is stored            | Sending would wait seconds, and a search that fails would lose the message                                                 |
| Every earlier message's passages in every request | A long conversation would send the same passages again and again                                                           |

## Validation

M13: runtime tests with a fake provider of gathering, its failure, its timeout, Stop, a plugin that is off and a retry that reuses the parts; request tests of the block and of the rule of the two newest messages; store tests of the attachments and their deletion with a conversation; UI tests of the chips, the part and the citations; and the plugin Notebooks' end-to-end test of a cited reply.
