# ADR-0026: Add conversation controls back to the shell for child agents

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D19)                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Date     | 2026-10-15                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Related  | [ADR-0002](0002-grow-through-extension-points.md), [ADR-0020](0020-complete-the-ui-shell-slots.md), [ADR-0022](0022-add-conversation-controls-to-the-shell.md), [ADR-0023](0023-organize-conversations-into-projects.md), [ADR-0025](0025-add-side-panels-to-the-shell.md), [Extension points](../architecture/extension-points.md#37-ui-shell-slots), [Child agents](../features/child-agents/README.md), [Conversation view](../features/chat/conversation-view.md) |

## Context

- On 2026-10-15 the owner reviewed the interface of child agents and asked for a button that shows the running child agents ([D19](../roadmap/milestones.md#5-decisions)). So far a child's conversation opens only from its task's card, which can stand far up in the transcript.
- A conversation's toolbar belongs to the chat feature. The shell's slots ([ADR-0020](0020-complete-the-ui-shell-slots.md)) have no place there for another feature's controls, and a feature does not edit another feature's modules ([ADR-0002](0002-grow-through-extension-points.md)).
- [ADR-0022](0022-add-conversation-controls-to-the-shell.md) added a slot for conversation controls. [ADR-0023](0023-organize-conversations-into-projects.md) removed it when its only control left, and said that a later control near the conversation would come back with an ADR of its own.
- The conversation view holds the conversation's summary and the turns it has loaded. The task parts of child agents are among those turns, and running children always belong to the latest turn, since a reply does not end while its children work ([tasks](../features/child-agents/tasks.md#stopping)).

## Decision

1. The shell gets the slot of conversation controls again. An entry has an ID, a component and an order, and its type lives in `apps/desktop/src/lib/types/shell-slots.ts` with the others.
2. A control receives the conversation's summary and the turns that the conversation view has loaded, oldest first, read-only. It works out what it shows from them, so it needs no subscription of its own.
3. The conversation view renders the controls in its toolbar, before the context meter, sorted by order, through an outlet that the shell provides. The pane of a new conversation and the read-only conversation of a side panel have no toolbar and show no controls.
4. A control is compact, a button or a menu, and renders nothing when it has nothing to offer. It never takes the focus by itself, and it shows more through menus, overlays or side panels.
5. Like the other slots, the slot has a registration test with a fake entry, and chat never imports a feature that registers a control.
6. The first control is the child agents' button. It lists the conversation's child agents, says how many of them work, and opens a child's conversation in a side panel.

## Consequences

- Positive: features add controls beside the conversation without editing chat, and the owner reaches a child's conversation without looking for its card.
- Negative: one more slot type to keep stable, and less room in the toolbar as controls arrive; the order numbers decide what comes first.
- Risks: a control that misbehaves breaks the toolbar of every conversation, so controls are reviewed like the other slot components. A control sees only the loaded turns, so the child agents' button lists no child from a turn the view has not loaded; the running ones are always there.

## Alternatives considered

| Option                                     | Why not chosen                                                                                                                                                               |
| ------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A "Child agents" group in the sidebar      | It needs a core command that lists a conversation's tasks and keeps the list current, or a second subscription to the conversation, while the toolbar's view holds the turns |
| A command palette action that opens a list | The owner asked for a button in view                                                                                                                                         |
| Chat's toolbar showing the child agents    | It ties chat to child agents, which [ADR-0002](0002-grow-through-extension-points.md) rules out                                                                              |

## Validation

Decided by the owner (D19). M3 validates it: child agents register the button through the slot, chat renders the slot without importing child agents, the registration test passes with a fake entry, and the end-to-end tests open a child's conversation from the button, with axe in both themes and the pseudo-locale.
