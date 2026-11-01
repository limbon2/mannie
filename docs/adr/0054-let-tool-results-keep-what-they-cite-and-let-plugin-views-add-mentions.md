# ADR-0054: Let tool results keep what they cite, and let plugin views add mentions

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D45)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Date     | 2026-10-30                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Related  | [ADR-0021](0021-render-model-output-as-markdown-with-micromark.md), [ADR-0032](0032-let-features-and-plugins-add-kinds-of-mentions.md), [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), [ADR-0036](0036-let-plugins-run-commands-on-remote-hosts.md), [ADR-0046](0046-let-plugins-attach-context-to-conversations.md), [ADR-0049](0049-gather-context-for-the-messages-that-ask-and-cite-it-by-links.md), [ADR-0053](0053-let-plugins-offer-terminals.md), [Terminals](../../plugins/terminal/docs/README.md), [Mentions](../features/chat/mentions.md), [Extension points](../architecture/extension-points.md) |

## Context

- The owner wants a reply to point at the evidence of a command that its agent ran, such as when the owner asks to "Find occupied ports", and wants to point an agent at a command and its output directly ([decision D45](../roadmap/milestones.md#5-decisions)). An agent reads only the output of its own commands and what the owner mentions ([ADR-0053](0053-let-plugins-offer-terminals.md)).
- Terminals live in memory: a terminal's output is gone once the terminal ends, and no terminal comes back after a restart ([Terminals](../../plugins/terminal/docs/README.md)).
- Facts of this repository, verified on 2026-10-30:
  - A link `cite:<provider>/<key>` becomes a citation when the slot `contextKinds` has an entry for its provider or a stored part `context` holds its key (`MarkdownCitation.svelte`). Without a stored item, the popover (`CitationButton.svelte`) says that the model read the item with a tool, and `ContextOpen.run(key)` opens the item where it comes from. `cite_link` lives in `crates/mannie-plugin-sdk/src/points/context.rs` (ADR-0049 d4).
  - A cite key names a whole item, and nothing names lines of it.
  - The guidance that tells the model to copy cite links renders only for conversations with attachments (`crates/mannie-runtime/src/texts.rs`), so a plugin whose tool returns cite links in other conversations says in its own instruction section how to cite them, as the terminal plugin's does.
  - A plugin tool returns one final `ToolResult` with `output`, up to 4 archives, `data` for its card and `failure` ([ADR-0036](0036-let-plugins-run-commands-on-remote-hosts.md) d5).
  - Mention providers resolve their keys when the message is sent, and a message holds at most 20 mentions, which share one budget ([ADR-0032](0032-let-features-and-plugins-add-kinds-of-mentions.md) d5 and d6). Nothing lets a panel put a mention into the message box (`Composer.svelte`, `state/message-mentions.svelte.ts`).
- So a citation of a terminal's output would show nothing once the terminal ended, could not mark the lines that a reply relied on, and the owner could not hand a command from a terminal's panel to the message box.

## Decision

1. A tool result keeps the items that it cites (refines ADR-0049 d4):
   - `ToolResult` gains `cited`: up to 8 items, each with a key, a label, a location and a text of at most 64 KiB.
   - The core stores them in the call's part, and they are deleted with it.
   - `citedItems(turns)` reads them as it reads the items of parts `context`, and a later item for the same key replaces an earlier one.
   - The popover shows them, so a citation keeps showing its text after its source is gone.
   - The conversation export leaves them out, since the result already holds the same text.
2. A cite key may end with `#L<first>-L<last>`, which names lines of its item. The popover finds the item by the part of the key before `#` and marks those lines, and `ContextOpen.run` receives the whole key.
3. An entry of `contextKinds` whose ID names no context provider serves citations only, as the terminal plugin's `terminal` does.
4. A plugin's view may add a mention to a message box (refines ADR-0032 d1 and ADR-0034 d11):
   - `@mannie/plugin-sdk/message-box` offers `mention(conversationId, entry)`, which puts a mention's token and chip into that conversation's message box.
   - It returns `not_shown` when that conversation is not open, and `full` when the message already holds 20 mentions.
   - The mention counts as chosen from the list, and its provider checks the key again when the message is sent (ADR-0032 d5).

Earlier decisions that this ADR changes:

| ADR  | Decision | Change                                                                                                                                                              |
| ---- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0032 | d1       | Refined: a plugin's view may put a mention into a conversation's message box, which counts as chosen from the list                                                  |
| 0034 | d6, d11  | Refined: a plugin tool's result may keep the items that it cites, and the TypeScript SDK puts mentions into a message box                                           |
| 0046 | d6       | Refined: an entry of the slot of context kinds whose ID names no context provider serves citations only                                                             |
| 0049 | d4       | Refined: the popover shows the items that a tool's result keeps, a cite key may name lines of its item, and a context kind without a provider serves citations only |

## Consequences

- Positive: a reply's evidence stays readable after its terminal ended and after a restart, and a citation marks the lines that the reply relied on. Any plugin tool can keep what it cites, and any plugin view can hand the owner's choice to the message box, where the owner still decides what to send.
- Negative: a call's part may store up to 512 KiB of cited text that its result repeats, so the database grows with each call that cites. The export leaves the items out, since the result holds their text.
- Risks: a model can garble a line fragment as it can garble a link (ADR-0049); the part of the key before `#` still finds the item. A view that names a conversation that is not open gets `not_shown`, and nothing reaches a message box.

## Alternatives considered

| Option                                             | Why not chosen                                                                                                                          |
| -------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| A context provider for terminals                   | A provider gathers for the owner's messages from a conversation's attachments; a terminal is no attachment, and its output ends with it |
| Storing a terminal's output in the plugin's tables | Terminals live in memory, and the plugin keeps nothing of a terminal except what reaches the model                                      |
| The popover reading the call's result              | A result is text for the model, cut to the result's limit with the rest in archives, and it holds no keys                               |
| Pasting a command's output into the message box    | Pasted text never becomes a mention (ADR-0032 d1), and the model would not learn the command, the host or the exit code                 |

## Validation

M14:

- Runtime tests: a tool's cited items are stored in its call's part, read back with the conversation, and deleted with it.
- UI tests: citations with stored items and with line fragments, and `mention()` with an open conversation, one that is not open and a full message box.
- The terminal plugin's end-to-end test: a reply's citation shows the cited lines in its popover, and Show in terminal marks them; the owner mentions a command and a selection, and the model reads them.
