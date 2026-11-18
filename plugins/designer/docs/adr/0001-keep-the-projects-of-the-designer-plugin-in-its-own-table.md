# ADR-0001: Keep the projects of the Designer plugin in its own table

|          |                                                                                                                                                                                                                                                                       |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted                                                                                                                                                                                                                                                              |
| Date     | 2026-11-18                                                                                                                                                                                                                                                            |
| Deciders | Owner                                                                                                                                                                                                                                                                 |
| Related  | [Designer](../README.md), [ADR-0002](../../../../docs/adr/0002-grow-through-extension-points.md), [ADR-0034](../../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), [ADR-0035](../../../../docs/adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md) |

## Context

The first Designer increment lets the owner create named projects, switch between them and find them again after restarting Mannie. A Designer project has a lifecycle independent of the directory-backed projects that set the working folder for conversations. [Designer](../README.md), [Chat projects](../../../../docs/features/chat/projects.md).

The SDK provides a plugin-scoped service for tables in Mannie's shared SQLite database. The host registers the module and its migrations independently of whether the plugin is on. The shell provides slots for the Designer activity, sidebar, main view and dialogs. [ADR-0034](../../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), [ADR-0045](../../../../docs/adr/0045-switch-the-sidebar-between-views-with-an-activity-bar.md).

## Decision

1. Keep each project's ID, display name and creation time in the plugin-owned `designer_projects` table. Designer registers its migration and accesses the table through the SDK's `store` service. Design documents, content and history need their own storage decision. Any future schema changes follow Mannie's additive migration rules. [ADR-0002](../../../../docs/adr/0002-grow-through-extension-points.md).
2. Trim surrounding whitespace before storing the name. The trimmed name contains 1 to 100 Unicode code points and input containing control characters is refused. Enforce uniqueness in SQLite using its built-in collation, which folds ASCII case, and report a constraint conflict as `name_taken`. Verified 2026-11-18. [SQLite collation documentation](https://www.sqlite.org/datatype3.html#collating_sequences).
3. Expose `create_project` and `follow_projects` through generated SDK bindings. The command returns the project after a durable write. The channel sends a full project list on subscription and fresh snapshots after changes while the plugin runs. Full snapshots let a client recover by resubscribing and reading the current state.
4. Keep the current selection in the window. Use the newest project when there is no valid selection. Selection is a view concern, and switching projects needs no store write. The shell slots expose the project management UI while the plugin is on.

## Consequences

- Designer owns its schema while sharing the existing database and migration machinery. Turning the plugin off removes its UI and ends its subscriptions. The persisted projects are available when the owner turns it on again.
- A window reload or application restart returns the view to the newest project. Renaming and removal need their own commands and UI.
- The cost of each channel snapshot grows with the number of projects. Revisit this choice if project lists outgrow full snapshots. Names using differently cased non-ASCII letters can coexist.

## Alternatives considered

| Option                            | Why not chosen                                                                               |
| --------------------------------- | -------------------------------------------------------------------------------------------- |
| Keep a separate Designer database | Adds a file lifecycle and migration path for one table.                                      |
| Reuse chat projects               | Couples design work to the chat's working folders and exposes chat projects through the SDK. |
| Send list changes as deltas       | Requires snapshot and replay rules when a window falls behind.                               |
| Persist the selected project      | Adds a write for each switch and shares a preference currently private to the window.        |

## Validation

M5 established the SDK baseline for plugin store modules, IPC and lifecycle. Apply those checks to Designer and verify that its command persists valid names, the channel sees created projects, and ASCII case variants of an existing name are refused. [M5 plugin SDK validation](../../../../docs/roadmap/m5-validation.md#11-plugin-sdk-spikes-step-0).

Use Designer's UI tests to check subscription recovery, project selection and duplicate-name handling. Use the feature's manual checks to confirm persistence across restarts and the shell's response to toggling the plugin. [Designer acceptance](../README.md#acceptance).
