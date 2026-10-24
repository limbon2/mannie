# ADR-0050: Let a mention attach what it names

|          |                                                                                                                                                                                                                                                                                                                       |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D44)                                                                                                                                                                                                                                                                                                        |
| Date     | 2026-10-24                                                                                                                                                                                                                                                                                                            |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                 |
| Related  | [ADR-0032](0032-let-features-and-plugins-add-kinds-of-mentions.md), [ADR-0046](0046-let-plugins-attach-context-to-conversations.md), [ADR-0049](0049-gather-context-for-the-messages-that-ask-and-cite-it-by-links.md), [Mentions](../features/chat/mentions.md), [Notebooks](../../plugins/notebooks/docs/README.md) |

## Context

- ADR-0046 attaches context through a conversation control in the message box. The plugin Notebooks added a button there that lists notebooks with checkboxes.
- The owner wants regular chats to attach notebooks with "@" instead of a button ([D44](../roadmap/milestones.md#5-decisions)).
- In the owner's earlier harness, "@" names a notebook in a message. That message gets the notebook's passages, and later messages can search it with tools ([ADR-0049](0049-gather-context-for-the-messages-that-ask-and-cite-it-by-links.md)).
- A mention today names something whose content the provider resolves for the message it is in (ADR-0032, decisions 2 and 6). The provider never sees the message, so it cannot search with it, and the mention says nothing to later messages.

## Decision

1. A mention attaches what it names when its mention provider has the ID of a context provider, which ADR-0034's rule `Owned` keeps to one plugin. Its key is then a key of that context provider. No field of `MentionEntry` changes, so the "@" list and the stored mentions keep their shape.
2. Choosing such an entry puts a mention into the message, with its token and chip, as for a file.
3. When the message is sent, or starts a conversation, the core handles each mention that attaches:
   - It attaches the key to the conversation as `attach_context` does, unless the key is attached already, with the label that the provider describes.
   - The message then gathers from that attachment (ADR-0049, decision 1).
   - The message stores the mention with the variant `attachment` and the summary code `attached`, which the plugin's kind of mentions words, so its chip stays in the transcript. The model reads a line that says the owner attached it. The core never calls `resolve` for it.
   - An attachment that fails, such as one beyond the 8 that a conversation keeps, refuses the message, as a mention's failure does.
4. Later messages of the conversation do not gather from the attachment unless they name it again. The model searches it with the tools of its plugin (ADR-0052). The attachment's chip in the message box detaches it.
5. The plugin Notebooks adds a mention provider `notebooks` beside its context provider `notebooks`, which lists the notebooks whose names hold the query, and removes its control from the message box. The outlet of ADR-0046, decision 6, stays for other plugins.

Earlier decisions that this ADR changes:

| ADR  | Decision | Change                                                                                                            |
| ---- | -------- | ----------------------------------------------------------------------------------------------------------------- |
| 0032 | d2, d6   | Refined: an entry may attach what it names instead of resolving it, and the core then never resolves it           |
| 0046 | d1, d6   | Refined: a mention can attach a key as the commands do, and the message box's outlet is no longer needed for that |

## Consequences

- Positive: attaching joins the gesture that the owner already uses for files, and the message that names a notebook gets its passages at once.
- Negative: a mention now does one of two things, which the provider's ID decides, so a plugin cannot offer mentions of its context provider's keys that resolve instead. Removing an attachment still needs its chip.
- Risks: an entry that attaches with a key that its plugin's context provider does not know is refused when the message is sent, and the message stays in the box.

## Alternatives considered

| Option                                                    | Why not chosen                                                                                 |
| --------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Attach at once when the entry is chosen, with no mention  | The message would not show what it asked for, and it would not gather for the message          |
| A mention whose provider resolves the notebook's passages | A mention provider never sees the message to search with, and the attachment would not persist |
| Keep the button and add "@"                               | The owner wants one way, and the button takes the message box's room                           |

## Validation

M13:

- runtime tests of a message that attaches through a mention, of one that names an attached key again, and of a refused attachment;
- UI tests of the chip;
- the plugin Notebooks' end-to-end test of "@" in a regular chat.
