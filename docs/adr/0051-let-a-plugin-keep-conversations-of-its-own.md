# ADR-0051: Let a plugin keep conversations of its own

|          |                                                                                                                                                                                                                                                                                                                                                                                                                   |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D44)                                                                                                                                                                                                                                                                                                                                                                                                    |
| Date     | 2026-10-24                                                                                                                                                                                                                                                                                                                                                                                                        |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                             |
| Related  | [ADR-0030](0030-split-mannie-into-a-core-and-plugins.md), [ADR-0045](0045-switch-the-sidebar-between-views-with-an-activity-bar.md), [ADR-0046](0046-let-plugins-attach-context-to-conversations.md), [ADR-0049](0049-gather-context-for-the-messages-that-ask-and-cite-it-by-links.md), [ADR-0052](0052-offer-a-plugins-tools-where-its-attachments-are.md), [Notebooks](../../plugins/notebooks/docs/README.md) |

## Context

- A notebook's "New chat" starts a regular conversation with the notebook attached. It shows under the notebook and in the Chat list (D43), and runs like any chat.
- The owner wants notebooks to have chats of their own, apart from the regular ones, because their purpose differs: a notebook's chat answers about its documents ([D44](../roadmap/milestones.md#5-decisions)). The notebook's chats belong in the Notebooks sidebar, and its page shows only its sources.
- In the owner's earlier harness, a chat belongs to its notebook through a field of the conversation:
  - It never belongs to a project, and its sidebar of chats leaves it out.
  - Its instructions say to answer from the notebook's documents first.
  - Every message gets passages. It keeps the usual tools, and gets the tools that search the notebook.
  - Deleting the notebook deletes its chats.
- The core knows no conversation that a plugin owns. Its lists show every conversation that is not a task's (ADR-0030, decision 9), and a plugin hears nothing when a conversation is deleted.

## Decision

1. A conversation may have a home: one attachment that it belongs to.
   - `StartConversation` gains `home`. The core asks the home's provider to describe it, attaches it first, and keeps it in `runtime_conversations` as `home_provider` and `home_key`.
   - A conversation with a home has no project. A start that names both is refused, and a move into a project is refused with `home_conversation`.
2. `ConversationSummary` gains `home`, left out when there is none, so the documents of the Workers link stay as they are. `detach_context` refuses the home with `home_attachment`, and the home's chip in the message box has no button that removes it.
3. The lists of the Chat activity, conversations and projects, leave out conversations with a home. The TypeScript SDK's `listConversations()` returns them with their homes, so the home's plugin lists them where it shows its keys.
4. Every message of a conversation with a home gathers from the home (ADR-0049, decision 1). The conversation runs the core's conversation agent, with the usual tools and the tools that its attachments' plugins offer (ADR-0052). The instruction section of attachments says that the conversation belongs to its home.
5. The TypeScript SDK changes so that the home's plugin can manage these conversations:
   - `startConversation({ home })` prepares a new conversation with its home. The Chat activity's "New chat" prepares one without.
   - `/conversations` gains `deleteConversation(id)`, which a plugin calls for the conversations of a key that it deletes, as Notebooks does for a notebook.
6. While the home's plugin is off, its conversations are kept and readable by their address. No list shows them, and their messages gather nothing but the failure `plugin_off`.

Earlier decisions that this ADR changes:

| ADR  | Decision | Change                                                                                                                       |
| ---- | -------- | ---------------------------------------------------------------------------------------------------------------------------- |
| 0030 | d9       | Refined: a conversation can belong to a plugin's key, and the core's lists leave it to that plugin                           |
| 0045 | d6       | Refined: `startConversation` names a home, and the SDK can delete a conversation                                             |
| 0046 | d1       | Refined: an attachment can be a conversation's home, which cannot be detached, and a conversation with a home has no project |

## Consequences

- Positive: a notebook's chats live with the notebook and leave the Chat list to regular chats. Every message in them reaches the notebook, and the model keeps the tools that a question may need beyond the documents.
- Negative: the core now has two kinds of owner's conversations, and every list and every move must respect the home. A home conversation whose plugin is off is out of sight until the plugin is on again.
- Risks: a plugin that deletes a key without deleting its conversations leaves conversations that no list shows. Notebooks deletes them in the same step, and the conversation stays reachable by its address.

## Alternatives considered

| Option                                                        | Why not chosen                                                                                                           |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| A plugin agent without tools for notebook chats               | The owner's earlier harness kept the usual tools, and answers that need a command or a file would have none              |
| A notebook chat as a view of its own, with its own transcript | It would duplicate the conversation view, which the plugin cannot embed, and the owner's notes keep chats in the sidebar |
| Marking the attachment and leaving lists unchanged            | The Chat list would still show notebook chats, which the owner wants apart                                               |

## Validation

M13:

- store tests of the home's columns, of the summaries and of the migration;
- runtime tests of a start with a home, of a refused detach and of a refused project;
- UI tests of the lists without home conversations and of the chip;
- the plugin Notebooks' end-to-end test of a notebook chat that shows under its notebook and not under Chat.
