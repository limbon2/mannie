# ADR-0020: Complete the UI shell slots with panels, overlays, part renderers and settings pages

|          |                                                                                                                                                                                                                                                                                                                                                |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted                                                                                                                                                                                                                                                                                                                                       |
| Date     | 2026-10-14                                                                                                                                                                                                                                                                                                                                     |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                          |
| Related  | [ADR-0002](0002-grow-through-extension-points.md), [ADR-0006](0006-use-shadcn-svelte-through-its-cli.md), [Extension points](../architecture/extension-points.md#37-ui-shell-slots), [Desktop app](../architecture/desktop-app.md#6-frontend-structure), [Chat](../features/chat/README.md), [Window and tray](../features/window-and-tray.md) |

Later changes: [ADR-0039](0039-let-plugins-describe-their-tool-calls-in-the-transcript.md) refines decision 1: the slot of tool kinds lets a plugin describe its tool calls inside the core's `tool_call` renderer. [ADR-0045](0045-switch-the-sidebar-between-views-with-an-activity-bar.md) refines decisions 1 and 6: navigation entries show in the activity bar, sidebar panels in the content sidebar of the entry that they name, and the settings sidebar takes the content sidebar's place while the activity bar stays.

## Context

- The UI shell slots are an extension point ([extension points](../architecture/extension-points.md#37-ui-shell-slots)). The catalog names navigation entries, views, panels, settings sections, command palette actions and part renderers. M0 built navigation entries, views, settings sections and commands, because they were the ones the shell itself needed. Panels and part renderers are named but have no shape yet.
- The first features need more than the built slots offer. Chat lists conversations in the sidebar, and a navigation entry is a fixed icon and label. Its transcript holds parts whose kinds later features extend, starting with the tool calls of M2. Window and tray shows dialogs that do not belong to any view: the first-close question and the quit warning can appear while the settings are open.
- Some palette commands only make sense in a state, such as "Compact the conversation" while a conversation is open.
- The shell's layout lets the page grow with its content, so a view cannot keep a message box at the bottom. The shell also puts its `<main>` inside the `<main>` that the shadcn-svelte sidebar's inset renders.
- A view's title is a function that returns text. It can already read reactive state, so a view can title itself after the conversation it shows.
- The settings view stacks every section on one page, beside the conversation list, and grows with each feature that adds a section. The owner asked for the settings as a page of their own, with the groups of settings as pages in a sidebar.

## Decision

1. The shell gets three more slots, with these entry types in `apps/desktop/src/lib/types/shell-slots.ts`:
   - Sidebar panels: a component, an ID and an order. The sidebar renders them after the navigation group, sorted by order. The navigation group is rendered only when it has entries.
   - Overlays: a component and an ID. The shell mounts each overlay once, beside the command palette, for the whole life of the window. An overlay renders nothing until it has something to show, such as a dialog.
   - Part renderers: a component and the part kind it renders, as the ID. The shell's `PartOutlet` picks the renderer for a part's kind, and its `GenericPart` renders kinds that no feature registered, so a transcript never fails on a kind it does not know ([extension points](../architecture/extension-points.md#35-transcript-parts-and-run-events)). A renderer receives the part, the conversation and whether the part belongs to the latest turn.
2. A command may declare when it is available, with a function that the palette calls when it opens. A command without one is always available. A view may say that its title is content, such as a conversation's title, so that the catalog check of the end-to-end tests skips it.
3. The shell's layout gives the main area a fixed height: the window is the viewport's height, `id="main"` moves to the sidebar inset's `<main>`, and the inner element becomes a scrolling region. A view either lets that region scroll or fills it and scrolls parts of itself, such as a transcript above a message box.
4. Part renderers never insert raw HTML. Content goes through text bindings or through the Markdown renderer of [ADR-0021](0021-render-model-output-as-markdown-with-micromark.md).
5. Like the other slots, each new slot has a registration test with a fake entry, and the shell never imports a feature.
6. The settings are a page of their own. There, the shell replaces the main sidebar with the settings sidebar: a link back to the main view as the owner left it, and an entry per settings section with the section's icon and title. Each section is a page, which the `section` search parameter selects; without one, the settings open at the section with the lowest order. A settings section therefore names an icon.

## Consequences

- Positive: chat, window and tray, and the tools of M2 plug into the shell without editing it or each other. Unknown part kinds degrade to a generic card instead of an error. A feature's settings get a page of their own, and the settings grow by one sidebar entry per section.
- Negative: three more slot types to keep stable. An overlay that misbehaves can cover the whole window, so overlays are reviewed like the shell. A section is the smallest page of the settings, so a feature with two groups of settings registers two sections.
- Risks: the panel shape may be too narrow for a later feature, such as a panel beside a view. Such a feature extends the slot through a new ADR.

## Alternatives considered

| Option                                               | Why not chosen                                                                                       |
| ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Edit the shell for chat and window and tray directly | Ties the shell to features, which [ADR-0002](0002-grow-through-extension-points.md) rules out        |
| A conversation list inside the chat view             | Two sidebars next to each other                                                                      |
| Dialogs inside each view                             | The first-close question must appear in every view, including the settings                           |
| A route per conversation                             | The shell selects views by search parameters; a second mechanism for one feature adds a special case |
| Every settings section on one page                   | The page grows with every feature, and the conversation list beside it serves no task there          |
| A slot for settings pages that group sections        | A second registration for the same thing: a section already is one feature's group of settings       |

## Validation

M1: chat and window and tray are built only through these slots, the registration tests pass with fake entries, and the end-to-end tests and axe checks pass with the new layout and on the settings pages, on WebView2 and WebKitGTK.
