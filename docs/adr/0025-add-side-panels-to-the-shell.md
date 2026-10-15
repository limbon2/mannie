# ADR-0025: Add side panels beside the main view to the shell

|          |                                                                                                                                                                                                                                                                                                                                                              |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Accepted (D18)                                                                                                                                                                                                                                                                                                                                               |
| Date     | 2026-10-15                                                                                                                                                                                                                                                                                                                                                   |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                        |
| Related  | [ADR-0020](0020-complete-the-ui-shell-slots.md), [ADR-0006](0006-use-shadcn-svelte-through-its-cli.md), [ADR-0002](0002-grow-through-extension-points.md), [Extension points](../architecture/extension-points.md#37-ui-shell-slots), [Desktop app](../architecture/desktop-app.md#6-frontend-structure), [Child agents](../features/child-agents/README.md) |

## Context

- [Child agents](../features/child-agents/README.md) let the owner open a child's whole conversation. The owner decided on 2026-10-15 that it opens in a side panel docked beside the parent's conversation, and that side panels will grow, as in the Claude app and Codex, so later features can show their own content there ([D18](../roadmap/milestones.md#5-decisions)). Open panels show as tabs, and they belong to the conversation that opened them.
- The shell's slots hold views, sidebar panels, overlays, part renderers, settings sections and commands ([ADR-0020](0020-complete-the-ui-shell-slots.md)). ADR-0020 named the risk that "a panel beside a view" would need a new slot.
- Views are selected by search parameters of the address, and the settings lead back to the main view with the parameters the owner left ([desktop app](../architecture/desktop-app.md#6-frontend-structure)).
- The main window is 1,200 pixels wide by default and at least 800, of which the sidebar takes 256 when it is open.
- shadcn-svelte 1.7.0 has a `resizable` component built on PaneForge (MIT), added through its CLI ([ADR-0006](0006-use-shadcn-svelte-through-its-cli.md)), and bits-ui, which Mannie already uses, has `Tabs` primitives. Verified on 2026-10-15.

## Decision

1. The shell gets a slot of side panel kinds, with this entry type in `apps/desktop/src/lib/types/shell-slots.ts`: an ID, a component, an icon and a default label from the catalog. The component receives the panel's argument, a string such as a conversation's ID, and a function through which it names its tab once it knows what it shows. A feature opens a panel through the shell's side panel state, by kind and argument, optionally with a label to show until the panel names itself.
2. The shell docks the side panel area beside the view, at the end of the main area, below the window's header, in a resizable split from shadcn-svelte's `resizable` that keeps its size in the webview's storage. The area is not modal: the view keeps working while a panel is open. It is a landmark with a heading, and its resize handle works from the keyboard and has a name from the catalog. When the main area is narrower than 720 pixels, the panel covers the view until it closes.
3. Each open panel has a tab with its kind's icon, its label and its Close inside the tab: a component of Mannie's own on bits-ui's `Tabs` primitives, made for panels. Delete closes the focused tab. Open panels stay mounted while their tab is open, so they keep their state. At most 8 panels are open, and opening another closes the oldest.
4. The open panels belong to the main view's address: a search parameter holds each open panel's kind and argument, in the order of the tabs, and another names the visible one. A view that opens other content, such as another conversation, builds a new address without them. The settings lead back to them, and a reload shows them again. Changes to panels replace the address in the history instead of adding to it.
5. Opening a panel from a control moves the focus to the panel's heading; a reload moves no focus. Closing the last panel returns the focus to the control that opened it, or to the main area, and closing a tab while other panels stay open moves the focus to the tab that shows. The command palette closes the visible panel.
6. A kind that no feature registered shows a generic body with Close, and the shell never imports a feature. Like the other slots, the new slot has a registration test with a fake entry.
7. The first kind is the chat feature's conversation panel, which shows a conversation read-only. Child agents open their task conversations in it.

## Consequences

- Positive: content beside a conversation, such as a child's conversation, gets one place that every feature can use, with resizing, tabs, focus handling and the address handled once by the shell. The parent's conversation stays usable while a child's runs beside it.
- Negative: one more slot type to keep stable, a new npm dependency (PaneForge), one more CLI component and a tab component of Mannie's own, and a main area shared between two regions on narrow windows.
- Risks: panels that load heavy content stay mounted while their tab is open, which the limit of 8 bounds; a feature could open panels the owner did not ask for, which the review of registrations, like that of overlays, guards against.

## Alternatives considered

| Option                                                       | Why not chosen                                                                                        |
| ------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------- |
| The child's conversation in the main view                    | The owner wants it beside the parent's conversation, which stays usable                               |
| A sheet over the view, as a modal dialog                     | A modal sheet blocks the parent's conversation, and every later panel would need its own overlay      |
| A panel component in the child agents feature alone          | Later features would each build their own, and the transcript belongs to chat                         |
| Panels that stay open across conversations                   | The owner decided that panels belong to the conversation that opened them                             |
| A menu instead of tabs for several panels                    | The owner chose tabs, as the Claude app and Codex show them                                           |
| shadcn-svelte's `tabs`, with one Close for the visible panel | They look like a segmented control, and the owner asked for tabs made for panels, each with its Close |

## Validation

Decided by the owner (D18). On 2026-10-15, before the change was committed, the owner's review of the interface replaced shadcn-svelte's `tabs` with tabs of Mannie's own, each with its Close (decision 3), which changed where the focus goes when a tab closes (decision 5). M3 validates the mechanism: the registration test with a fake kind, the unit tests of the address's parameters, and the end-to-end tests that open, switch, resize from the keyboard and close panels, with axe in both themes and the pseudo-locale, on WebView2 and WebKitGTK.
