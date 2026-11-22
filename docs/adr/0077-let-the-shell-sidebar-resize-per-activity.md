# ADR-0077: Let the shell sidebar resize per activity

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D71)                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Date     | 2026-11-22                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Related  | [ADR-0004](0004-keep-system-access-in-the-rust-core.md), [ADR-0006](0006-use-shadcn-svelte-through-its-cli.md), [ADR-0020](0020-complete-the-ui-shell-slots.md), [ADR-0025](0025-add-side-panels-to-the-shell.md), [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), [ADR-0045](0045-switch-the-sidebar-between-views-with-an-activity-bar.md), [Desktop app](../architecture/desktop-app.md), [Extension points](../architecture/extension-points.md#37-ui-shell-slots) |

## Context

- The desktop shell places the activity bar beside a fixed-width (`16rem`, `256px`) content sidebar (`apps/desktop/src/lib/components/app/shell/Shell.svelte` and `ShellSidebar.svelte`; [ADR-0045](0045-switch-the-sidebar-between-views-with-an-activity-bar.md), decision 1).
- Activities that show deep trees or long names, such as Chat projects and conversations, Agents and the Network Client plugin's API collections, folders and endpoint names, truncate labels at `256px` while leaving no way for the owner to widen the sidebar.
- Other activities, and the Settings sidebar, hold short flat lists whose fixed width stays predictable and compact.
- `Sidebar.Provider` and `Sidebar.Root` in `apps/desktop/src/lib/components/ui/sidebar/` already size both `[data-slot="sidebar-gap"]` and `[data-slot="sidebar-container"]` from `--sidebar-width`, which `Shell.svelte` updates through its CSS custom property (permitted under `style-src 'self'`, [ADR-0004](0004-keep-system-access-in-the-rust-core.md)). Keeping resize logic in `components/app/shell/` avoids any edit inside `components/ui/` ([ADR-0006](0006-use-shadcn-svelte-through-its-cli.md)).
- The owner requested resizable main shell sidebars with an opt-in capability for plugins, enabled for Network Client, and approved this ADR in advance on 2026-11-22 (D71).

## Decision

1. Extend `NavigationEntry` in `apps/desktop/src/lib/types/shell-slots.ts` (exported through `@mannie/plugin-sdk/slots`) with an optional `sidebarResizable?: boolean | SidebarResizeOptions` field:
   - `SidebarResizeOptions` carries optional `defaultWidth`, `minWidth` and `maxWidth` in CSS pixels for the content sidebar column beside the activity bar.
   - Omitting `sidebarResizable` or setting `false` keeps the fixed `16rem` (`256px`) content width and renders no resize handle.
   - Passing `true` enables resizing with the shell's default bounds (`defaultWidth: 256`, `minWidth: 200`, `maxWidth: 480`). Passing a `SidebarResizeOptions` object customizes those bounds within the shell's safe limits (`minWidth` at least `180px`, `maxWidth` at most `560px`).
2. Built-in main shell activities (`chat` and `agents`) opt in with `sidebarResizable: true`, and the Network Client plugin (`network`) opts in with `sidebarResizable: true`. Settings (`/settings`) stays fixed at `16rem`.
3. Persist each resizable activity's content width in `localStorage` under `mannie.shell.sidebar-widths` (`apps/desktop/src/lib/state/sidebar-widths.svelte.ts`), keyed by the navigation entry's `id`. Missing, unreadable or invalid storage falls back cleanly to the activity's `defaultWidth` without affecting other activities.
4. Clamp every effective content width dynamically against the current window width so the main area keeps at least `360px` (and the content sidebar stays at most `45%` of the window width whenever the window allows `minWidth`).
5. Render a pointer-captured, `requestAnimationFrame`-throttled, keyboard-operable window splitter (`role="separator"`, `aria-orientation="vertical"`, `ArrowLeft`/`ArrowRight`/`Home`/`End`, double-click to reset to default width) on the trailing edge of `[data-shell-sidebar="content"]` inside `ShellSidebar.svelte`. Hide the handle while collapsed (`Ctrl+B`), on narrow mobile sheets, on Settings and for non-resizable activities.
6. While dragging the handle, toggle `data-shell-sidebar-resizing` on `document.documentElement`. `apps/desktop/src/app.css` sets `cursor: ew-resize` and `transition-duration: 0ms` on `[data-slot="sidebar-gap"]` and `[data-slot="sidebar-container"]` during the drag so resizing tracks the pointer without lag while preserving the `200ms` collapse/expand transition for `Ctrl+B`.

Earlier decisions that this ADR refines:

| ADR  | Decision | Change                                                                                                                               |
| ---- | -------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| 0034 | d11      | Additive: `@mannie/plugin-sdk/slots` exports `SidebarResizeOptions` and optional `NavigationEntry.sidebarResizable`                  |
| 0045 | d1       | Refined: the shell's content sidebar width follows the selected resizable activity's clamped width instead of staying fixed at 16rem |

## Consequences

- Positive: owners can widen the main shell sidebar in Chat, Agents and opt-in plugins such as Network Client, with each activity remembering its own width across reloads and restarts.
- Positive: existing plugins that do not opt in keep their current `16rem` sidebar width unchanged, and no file in `components/ui/` is modified.
- Negative: switching between activities with different saved widths changes the sidebar width; smooth CSS transitions apply outside active drags.
- Risks: narrow windows could leave little room for side panels alongside a widened sidebar; dynamic clamping to the current viewport width keeps the main area usable and side panels fall back to overlay mode below `720px` ([ADR-0025](0025-add-side-panels-to-the-shell.md)).

## Alternatives considered

| Option                                                   | Why not chosen                                                                                             |
| -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| One shared sidebar width across every activity           | Wide collection trees in Network Client would force Chat and Settings sidebars to stay overly wide         |
| Replace `Sidebar.Provider` with an outer PaneForge split | Rebuilds shadcn-svelte's sidebar collapse, mobile sheet and `Ctrl+B` mechanics and adds resize lag         |
| Enable resizing unconditionally for every plugin         | Some plugins design compact fixed sidebars; opt-in on `NavigationEntry` lets each plugin declare its range |

## Validation

- Unit tests in `tests/apps/desktop/src/lib/state/sidebar-widths.test.ts` cover option normalization, viewport clamping, per-activity persistence, invalid storage recovery and reset.
- End-to-end tests in `tests/e2e/specs/shell.test.ts` and `plugins/network/tests/e2e/network.test.ts` verify keyboard resizing, reload persistence, independent per-activity widths, fixed Settings width, zero CSP violations and axe WCAG 2.1 AA compliance in both themes.
