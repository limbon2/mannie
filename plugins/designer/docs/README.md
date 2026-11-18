# Designer

|              |                                                                                                                                                                                                                                                                                                                                 |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                        |
| Plugin       | `designer`                                                                                                                                                                                                                                                                                                                      |
| Last updated | 2026-11-18                                                                                                                                                                                                                                                                                                                      |
| Related      | [Designer ADR-0001](adr/0001-keep-the-projects-of-the-designer-plugin-in-its-own-table.md), [ADR-0035](../../../docs/adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), [ADR-0045](../../../docs/adr/0045-switch-the-sidebar-between-views-with-an-activity-bar.md), [Plugins](../../../docs/features/plugins.md) |

## Intent

The owner wants a design surface in Mannie, in the manner of Figma or of Claude's designer, that works differently from both and is built up step by step. This document describes the current behavior. Proposed additions are linked under Parts.

This step adds the owner's projects: naming one from the sidebar, listing them and switching between them. The main view opens a native graphics viewport for the selected project on Windows.

## Behavior

- Designer is a plugin on the Plugins page with a switch, off by default, like every plugin.
- While Designer is on, the activity bar has a Designer entry with the pen tool icon, after Notebooks and before the Databases of Workers. Selecting it shows the sidebar with the entry's name and its group "Projects", and the main area shows the Designer view.
- The sidebar's group has the heading "Projects" and the plus at its end, as the groups of Notebooks, of the Databases view and of the chat's Projects do.
- The plus opens a dialog that asks for the project's name, with "Create project" and "Cancel". The plugin trims leading and trailing whitespace, accepts up to 100 Unicode code points in the trimmed name and rejects control characters. "Create project" stays disabled while the name is blank.
- Names are unique under SQLite's ASCII case-insensitive comparison. If creation fails, the dialog stays open and explains the error so the owner can correct it or retry.
- A project that the owner creates appears at the top of the group and becomes the active project. Each project has a row with a folder and its name. Selecting another row changes the active project and moves the active mark to that row.
- The main area opens the active project's viewport or shows the empty state when there are no projects. The view's heading in the window is "Designer".
- Projects survive a window reload and an application restart. The newest project is active when the owner has made no selection in the current window.
- Turning Designer off removes its activity, sidebar, view and dialogs. The shell returns to Chat when Designer was active. Turning it on again restores its project list.
- Designer adds no tool, settings section, side panel or mention, and contributes nothing to conversations or the model. It has no way to rename or remove a project yet.

## Fit

Designer registers its activity, view, sidebar and dialogs through the SDK's `navigation`, `views`, `sidebarPanels` and `overlays` slots. The plugin's command and channel use generated IPC bindings, and it keeps its projects through the SDK's `store` service. [ADR-0034](../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), [ADR-0045](../../../docs/adr/0045-switch-the-sidebar-between-views-with-an-activity-bar.md).

The project list follows the shape of the Notebooks plugin: a command that changes the table and a channel that sends full snapshots of the projects. Designer follows the project list while the plugin is on and leases a graphics surface while the selected project's view is mounted. [ADR-0030](../../../docs/adr/0030-split-mannie-into-a-core-and-plugins.md), [Designer ADR-0001](adr/0001-keep-the-projects-of-the-designer-plugin-in-its-own-table.md).

## Parts

- [Native graphics viewport](viewport.md) draws a rectangle whose color the owner changes from the Designer view.

## Design

The manifest `plugin.toml` names the plugin `designer` on SDK 1, with `order = 85`, after Browser, and its Rust entry `mannie-designer`. It declares the services `store` and `graphics`.

The crate `crates/mannie-designer` includes these modules:

- `commands.rs` holds `create_project`, which takes a name and returns the saved project, and `follow_projects`, which delivers a full list on subscription and after changes while Designer is on. It defines the name length limit and the plugin error codes `name_taken` and `storage_failed`.
- `store.rs` holds the migration and statements for the `designer_projects` table in Mannie's shared SQLite database. The table holds a project's ID, name and creation time.
- `plugin.rs` reads the projects when Designer starts and fails the start with `storage_failed` when it cannot read them. It updates the snapshots after a project is saved.

- `viewport.rs` defines the viewport channel, validates project and placement inputs and supplies the rectangle scene to the host.

The UI registers the navigation entry `designer` with `order = 15`, the view `designer`, the sidebar panel `designer.projects` and the overlay `designer.dialogs`. The sidebar shows loading, empty and error states, with a retry when following projects fails. The UI keeps project selection in memory. The catalog `messages/en.json` supplies the view's labels, status and errors.

## Acceptance

- `cargo nextest run --manifest-path plugins/designer/Cargo.toml --workspace --all-features --locked` checks the manifest, IPC interface, input validation, dispatch through the host, delivery of a created project to subscribers and rejection of an ASCII case variant of an existing name.
- The plugin's Vitest tests cover registration and teardown of the UI, following and retrying the project list, selecting a newly created project and keeping the dialog open when the plugin refuses a duplicate name.
- In the desktop app, turn Designer on and create two projects using the sidebar, including `Landing page`. Check that the newest project is at the top and active. Select each row and check that the active mark follows the selection. Try to create `landing PAGE` and check that the dialog stays open and shows the duplicate-name error.
- Reload the window and restart Mannie. Check that both projects are listed and the newest is active each time. With Designer open, turn it off and check that the shell returns to Chat. Turn it back on and check that both projects are still listed. Check the flow with the keyboard and in both themes.
