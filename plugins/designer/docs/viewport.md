# Native graphics viewport

|              |                                                                                                                                                                                                                                                                                                                                           |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                  |
| Milestone    | Unscheduled                                                                                                                                                                                                                                                                                                                               |
| Last updated | 2026-11-20                                                                                                                                                                                                                                                                                                                                |
| Part of      | [Designer](README.md)                                                                                                                                                                                                                                                                                                                     |
| Related      | [Designer ADR-0001](adr/0001-keep-the-projects-of-the-designer-plugin-in-its-own-table.md), [ADR-0004](../../../docs/adr/0004-keep-system-access-in-the-rust-core.md), [ADR-0034](../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), [ADR-0064](../../../docs/adr/0064-give-agents-an-isolated-browser-beside-the-conversation.md) |

## Intent

The owner wants a native 2D graphics viewport inside Designer. In the owner's sketch, the gray area is Mannie's shell, the blue area holds Designer's graphics controls, and the red rectangle is the viewport. The gray and blue areas share Mannie's Svelte webview. The red area presents live graphics directly through native OpenGL, while controls in blue change what it draws.

## Behavior

- Selecting a project opens its viewport in the Designer main view. With no selected project, the view shows its empty state and the project creation action is available in the sidebar.
- The viewport follows its rectangle as the window and layout resize, including when a sidebar moves or the window crosses displays with different scale factors. It stays inside the visible part of the Designer view.
- The viewport draws the project's rectangle on a charcoal background. The webview's Rectangle color slider previews the fill during a drag and saves it on release or keyboard edits. The native viewport accepts no input. The [persisted document](persisted-document.md) supplies the saved hue and geometry.
- Shell overlays, including the command palette, menus, dialogs and the side panel on narrow windows, remain visible and interactive where they overlap the viewport. A covered viewport yields pointer input and keyboard focus to the covering UI. Closing the overlay restores the current scene.
- Leaving Designer or switching projects releases the previous surface. Turning Designer off or reloading the window releases it. Drawing pauses while the viewport is hidden.
- The view shows a loading state while the graphics surface opens. If initialization or rendering fails, the view explains what failed and offers retry for recoverable errors. The owner can still use the shell and manage projects.

## Design

The manifest declares the desktop `graphics` service. Designer validates the project before opening its status channel. The channel owns a lease on the main window's native surface. Commands send the requested placement or a replacement scene under that lease. The host rejects expired leases and checks placement against its own client area. [ADR-0073](../../../docs/adr/0073-host-native-graphics-surfaces-for-desktop-plugins.md).

The webview uses the shell's floating layer information, visible bounds and device pixel ratio to place the child window. The Windows host draws into its OpenGL framebuffer on the window thread. Linux shows the platform availability message and keeps project management available.

When the viewport opens, Designer reads the project's confirmed rectangle. At identity zoom its document units map to CSS pixels. The host draws the complete scene and clips it to the visible viewport. A resize changes which part of the drawing is visible. [Persisted document](persisted-document.md), [graphics scenes](../../../docs/adr/0075-draw-bounded-graphics-scenes-in-document-coordinates.md).

## Acceptance

- Create two projects. Change the first project's rectangle color during a drag and with the arrow keys. Switch projects, reload and restart. Check that each project restores its own color.
- On Windows, compare the child window's bounds with the visible viewport after a resize, a sidebar change and a move between displays of different scale. Run the [viewport test](../tests/e2e/viewport.test.ts) with `MANNIE_E2E_SHOW_WINDOW=1` to sample the rectangle and background directly from the visible desktop during a drag and after a keypress.
- Open and dismiss the command palette, project dialog, menus and a narrow side panel. The shell receives input and closing an overlay restores the current scene. Leaving the view, reloading and disabling the plugin release the child.
- Check loading, recoverable failure and retry. On Linux, create and select projects and check the viewport availability message.
