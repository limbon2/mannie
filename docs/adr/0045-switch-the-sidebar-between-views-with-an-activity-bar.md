# ADR-0045: Switch the sidebar between views with an activity bar

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D43)                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Date     | 2026-10-24                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Related  | [ADR-0006](0006-use-shadcn-svelte-through-its-cli.md), [ADR-0020](0020-complete-the-ui-shell-slots.md), [ADR-0025](0025-add-side-panels-to-the-shell.md), [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), [ADR-0046](0046-let-plugins-attach-context-to-conversations.md), [Desktop app](../architecture/desktop-app.md), [Extension points](../architecture/extension-points.md#37-ui-shell-slots), [Notebooks](../../plugins/notebooks/docs/README.md) |

## Context

- The owner wants an icon-only bar at the far left of the window, before the main sidebar, whose first item shows Chat and whose second shows the notebooks of the plugin Notebooks ([decision D43](../roadmap/milestones.md#5-decisions)).
- The shell today (`apps/desktop/src/lib/components/app/shell/`):
  - `ShellSidebar.svelte` is one `Sidebar.Root collapsible="icon"`: a header with the app's name, the group of `navigation` entries, which shows only when it has entries and has none, every `sidebarPanels` entry in order, and a footer with "Find a command" and "Settings". Collapsed, it keeps the header, the entries and the footer as icons and hides every panel.
  - The `view` search parameter of `/` selects the main area's view, and chat is the only view. A navigation entry links to `?view=<view>` alone, which drops the open conversation and the side panels.
  - A sidebar panel names no view, so every panel would show beside every view.
  - The settings page replaces the sidebar with a sidebar of its own (ADR-0020, decision 6).
- shadcn-svelte's block sidebar-09 composes an icon rail and a second sidebar inside one `Sidebar.Root collapsible="icon"`, as two nested `Sidebar.Root collapsible="none"`, and shows the rail's tooltips while the sidebar is expanded with `tooltipContentProps={{ hidden: false }}` (verified 2026-10-24). A second `Sidebar.Provider` would shadow the first one's context and bind a second Ctrl+B handler.
- `Sidebar.Provider` sets `--sidebar-width` and `--sidebar-width-icon` in its element's `style`, after which it appends the `style` that it receives. Svelte sets a dynamic `style` through the CSS object model, which the Content Security Policy allows, while it refuses the static `style` attributes of markup (verified 2026-10-24 in Svelte 5.57's `set_style`).
- A plugin can register views and navigation entries, but its UI can open neither a view nor a conversation and cannot read the address: the TypeScript SDK's `/navigation` opens only side panels and settings, and a plugin's UI may not import `$app` (ADR-0034, decision 11).

## Decision

1. The shell's sidebar holds two columns inside its one `Sidebar.Root collapsible="icon"`, as the block sidebar-09 composes them: the activity bar, a nested `Sidebar.Root collapsible="none"` as wide as the sidebar's icon width, and beside it the content sidebar, a second nested `Sidebar.Root collapsible="none"` that takes the rest. The sidebar grows by the bar's width through the `style` that the shell gives the provider. Ctrl+B collapses the sidebar to the bar, and the content sidebar then leaves the page, so the focus cannot reach it. The bar is a composition in `components/app/shell/` of installed components, the sidebar's parts and their tooltips, and nothing in `components/ui/` changes.
2. The bar shows the `navigation` entries in their order as icon buttons in a `nav` landmark named "Views". Each button's label is its tooltip and its accessible name, and the selected one has `aria-current="page"`. The app's icon heads the bar, and "Find a command" and "Settings" stand at its foot. Chat registers the first entry, `chat`, with order 0, for its view.
3. A sidebar panel names the navigation entry that it belongs to in a new optional field, `activity`. A panel that names none belongs to the entry with the lowest order, so the panels of today, Projects, Conversations and the plugin Workers' panel, stay where they are. The content sidebar shows the selected entry's panels under a header with the entry's label. A panel that names an entry that nothing registered shows nowhere.
4. The selected entry is the shell's state, kept in the webview's storage, and the main area keeps its own address:
   - Selecting an entry shows the address that the main area last had while that entry was selected, which the shell keeps for each entry in the webview's storage, or the entry's view without parameters the first time.
   - Navigating inside the main area keeps the selected entry, so a conversation that the notebooks' sidebar opens shows in the chat view while their sidebar stays.
   - The command palette gets a command for each entry, "Show {label}".
   - When the selected entry's plugin stops, the first entry is selected, with the address it last had. A stored entry that no longer exists at a start does the same.
5. The settings page keeps the bar, shows its own sidebar in the content column, and leads back to the main area's address as the owner left it.
6. The TypeScript SDK's `/navigation` gains `openView(view, parameters)`, which shows a view with its parameters and selects the view's entry, `openConversation(id)`, `startConversation(options)`, which opens the new conversation pane with what the options name, such as attachments ([ADR-0046](0046-let-plugins-attach-context-to-conversations.md), decision 6), and `viewParameters()`, the reactive parameters of the main area's address for a plugin's view. Its components gain `progress`, added with the pinned CLI.
7. Like every slot, the bar has a registration test with fake entries, and the shell's end-to-end test checks the bar, the content sidebar and Ctrl+B.

Earlier decisions that this ADR changes:

| ADR  | Decision | Change                                                                                                                                      |
| ---- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| 0020 | d1       | Refined: sidebar panels show in the content sidebar of the navigation entry that they name, and navigation entries show in the activity bar |
| 0020 | d6       | Refined: the settings sidebar takes the place of the content sidebar, and the activity bar stays                                            |
| 0034 | d11      | Refined: the TypeScript SDK opens views and conversations, reads a view's parameters and offers a progress bar                              |

## Consequences

- Positive: features and plugins get a place of their own beside chat, with their own sidebar, and the owner switches between them with one click or the palette. Each entry keeps its place in the main area.
- Negative: the sidebar is wider by the bar. A view can show under an entry that is not its own, such as a conversation under Notebooks, so the selected entry says what the sidebar shows, and the main area's title says what the main area shows. A plugin that adds a sidebar panel to another entry than the first must name it.
- Risks: the nested sidebars rely on the sidebar component as the block composes it, which a later version of shadcn-svelte could change; the review of an update of the components and the shell's end-to-end test find it.

## Alternatives considered

| Option                                                  | Why not chosen                                                                                                 |
| ------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| The selected entry follows the main area's view         | A notebook's conversation would need the chat view inside the plugin's view, which a plugin's UI cannot import |
| A second `Sidebar.Provider` for the bar                 | It shadows the first provider's context and binds a second Ctrl+B handler                                      |
| A bar outside the sidebar component                     | The sidebar's fixed container would cover it, and collapsing to the bar would need code of the shell's own     |
| Navigation entries in the sidebar's top group, as today | Every entry's panels would share one list, and the owner asked for a bar of icons                              |
| "Find a command" and "Settings" in the content sidebar  | They would hide with it, and the bar is the column that always shows                                           |

## Validation

M13: the registration tests of the slots with fake entries, Vitest tests of the selection and of the addresses that it keeps, and the shell's end-to-end test with axe in both themes.
