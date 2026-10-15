# ADR-0022: Add a slot for conversation controls to the UI shell

|          |                                                                                                                                                                                                                                                                                                                                                         |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Superseded by ADR-0023                                                                                                                                                                                                                                                                                                                                  |
| Date     | 2026-10-14                                                                                                                                                                                                                                                                                                                                              |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                   |
| Related  | [ADR-0002](0002-grow-through-extension-points.md), [ADR-0020](0020-complete-the-ui-shell-slots.md), [Extension points](../architecture/extension-points.md#37-ui-shell-slots), [System tools](../features/system-tools/README.md), [Conversation view](../features/chat/conversation-view.md), [ADR-0023](0023-organize-conversations-into-projects.md) |

## Context

- A conversation has settings of its own that the owner changes while working in it. Chat added the first one, the provider profile, as a menu in the conversation's toolbar ([conversation view](../features/chat/conversation-view.md#layout)).
- System tools add a second one, the conversation's workspace folder. The owner decided that it belongs in the same toolbar, next to the profile, and in the pane of a new conversation (decision of 2026-10-14 in [system tools](../features/system-tools/README.md#decisions)).
- The toolbar and the new-conversation pane belong to the chat feature. The shell's slots ([ADR-0020](0020-complete-the-ui-shell-slots.md)) have no place there for another feature's controls, and a feature may not import another feature's components ([ADR-0002](0002-grow-through-extension-points.md)).
- Later features will add more of these controls, such as the worker that a conversation delegates to by default.

## Decision

1. The shell gets one more slot, conversation controls. An entry has a component, an ID and an order, and its type lives in `apps/desktop/src/lib/types/shell-slots.ts` with the others.
2. A control receives the ID of the conversation it belongs to, or none in the pane of a new conversation, where it changes what new conversations start with.
3. The conversation view renders the slot's entries in its toolbar, sorted by order, through an outlet component that the shell provides. The new-conversation pane renders the same outlet above its message box.
4. A control is compact: a button or a menu, as the profile menu is. It opens dialogs through overlays, and it never takes the focus by itself.
5. Like the other slots, the new one has a registration test with a fake entry, and the conversation view never imports the feature that registers a control.

## Consequences

- Positive: system tools add the workspace control without editing chat beyond the one outlet, and later features add theirs the same way.
- Negative: one more slot type to keep stable, and the toolbar can grow crowded. Controls must stay compact, and the order numbers decide what comes first.
- Risks: a control that misbehaves breaks the toolbar of every conversation. Controls are reviewed like the other slot components, and the outlet keeps them inside the toolbar's layout.

## Alternatives considered

| Option                                      | Why not chosen                                                                               |
| ------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Edit the chat toolbar to show the workspace | Ties chat to system tools, which [ADR-0002](0002-grow-through-extension-points.md) rules out |
| A sidebar panel for the workspace           | The owner wants it beside the conversation; a panel may sit below a long list                |
| Only a command palette action and a dialog  | The owner would not see which workspace a conversation uses                                  |

## Validation

M2: the system tools feature registers its workspace control through the slot, chat renders the slot without importing system tools, the registration test passes with a fake entry, and the end-to-end tests and axe checks pass with the control in the toolbar.
