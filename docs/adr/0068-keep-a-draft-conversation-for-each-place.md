# ADR-0068: Keep a draft conversation for each place a new conversation starts

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Accepted (D63)                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Date     | 2026-11-14                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Related  | [ADR-0002](0002-grow-through-extension-points.md), [ADR-0023](0023-organize-conversations-into-projects.md), [ADR-0026](0026-add-conversation-controls-back-for-child-agents.md), [ADR-0042](0042-place-conversation-controls-in-a-menu-and-the-message-box.md), [ADR-0046](0046-let-plugins-attach-context-to-conversations.md), [ADR-0051](0051-let-a-plugin-keep-conversations-of-its-own.md), [Chat](../features/chat/README.md) |

## Context

- A conversation exists only after its first message. This was a default chosen when building started and left for the owner's review: "A new conversation is created when the owner sends its first message, so the list never holds empty conversations" ([chat](../features/chat/README.md#decisions), 2026-10-14).
- The controls of a conversation's toolbar, Terminal, Browser and Git, take the conversation's summary ([ADR-0026](0026-add-conversation-controls-back-for-child-agents.md)), and terminals and browser tabs belong to a conversation's ID. The core refuses to open one for an ID that the store does not hold. So the new conversation's pane shows none of them, and the owner sends a message only to open a terminal or a browser.
- The pane of a new conversation shows only the controls of the message box that have a `draft` variant ([ADR-0046](0046-let-plugins-attach-context-to-conversations.md), decision 6). Its draft lives in the window's memory, and the text in the message box is lost when the owner opens another conversation.
- The owner asked on 2026-11-14 that a new conversation work like Codex's: "+" gives a conversation that the list does not show, and its message box text, terminals and browser tabs remain when the owner opens another conversation and comes back through "+".
- The store already accepts a conversation without turns, and `send_message` does not assume earlier turns.

## Decision

1. Each place where a new conversation starts has at most one draft conversation: the general conversations, each project, and each home item of a plugin ([ADR-0051](0051-let-a-plugin-keep-conversations-of-its-own.md)). A draft is a stored conversation with an ID, a profile, an agent, attachments and full access like any other, and it has no turns and an empty title.
2. The store marks a draft with the column `draft` of `runtime_conversations`, added with the default 0, and a partial unique index keeps one draft per place. `ConversationSummary` gains `draft`, which is left out of its serialized form while it is false, so the documents of the link stay the same.
3. The core's command `open_draft_conversation` returns the draft of a place, and creates it when the place has none. It takes the place and what the conversation starts with, attachments and an agent, which it adds to an existing draft through the same checks as `attach_context` and `set_conversation_agent`. One call at a time creates drafts.
4. The first message to a draft makes it an ordinary conversation in the same batch as its turn: the flag clears and the title comes from the message, as `start_conversation` takes it, unless the owner renamed the draft. The next "+" of the place then creates a new draft.
5. The conversation list of the UI keeps drafts in its data, so features can find a draft's project, and leaves them out of the sidebar's groups, as it does for home conversations.
6. The new conversation's view opens the draft of its place and shows it in the conversation's own pane. Every conversation control works on it, the toolbar's included, and the plugin SDK's open conversation is the draft. While it has no turns, the pane shows the hint that the new conversation's pane showed.
7. The message box keeps unsent text, mentions and images for each conversation in the window's memory, so they remain when the owner opens another conversation and comes back.
8. `ConversationControl.draft` and `DraftControlProps` are deprecated. The view no longer renders them, since the controls work on the draft itself. They are removed after the deprecation period ([ADR-0002](0002-grow-through-extension-points.md)).
9. `start_conversation` stays for its callers, such as the tests and the workers' test harness.
10. Outside this decision: removing drafts that the owner left, keeping the message box's text across restarts, and more than one draft for a place.

Earlier decisions that this ADR changes:

| Earlier decision                                                            | Change                                                                                                |
| --------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| [Chat](../features/chat/README.md#decisions), default of 2026-10-14         | Superseded: a draft conversation exists before its first message, and the list does not show it       |
| [ADR-0046](0046-let-plugins-attach-context-to-conversations.md), decision 6 | Superseded: the controls of the message box work on the draft conversation, and `draft` is deprecated |

## Consequences

- Positive:
  - Terminal, Browser, Git and every other control work in a new conversation before its first message.
  - What the owner prepares in a new conversation, its text, terminals, browser tabs and attachments, stays until the owner sends it.
  - One pane shows new and open conversations, and the pane of new conversations goes.
- Negative:
  - The store holds conversations that the owner may never use, at most one per place.
  - A draft's terminals and browser tabs keep running while the owner works elsewhere, as an open conversation's do.
- Risks:
  - A view or a feature that reads the unfiltered list shows a draft. The tests of the list cover the sidebar's groups, and a draft's title falls back to the new conversation's heading.

## Alternatives considered

| Option                                                    | Why not chosen                                                                                               |
| --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Create a listed conversation when a control is first used | The list fills with empty conversations, and the owner asked that it not show them                           |
| Create a listed conversation on "+"                       | The same, on every "+"                                                                                       |
| Key terminals and browser tabs to a draft in the UI       | The core, the terminal and the browser would need a second kind of owner and a handover at the first message |
| Draft variants of every toolbar control                   | Each plugin would build its controls twice, and a terminal still needs a conversation to belong to           |

## Validation

The tests of the runtime and the store for drafts, the tests of the conversation list, and an end-to-end test that opens a terminal in a new conversation, opens another conversation, comes back through "+" and finds the terminal and the text, then sends and finds the conversation in the list.
