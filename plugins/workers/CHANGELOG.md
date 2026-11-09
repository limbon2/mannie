# Changelog of the Workers plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- Databases, an optional feature that the owner turns on under Workers on the Plugins page, registers SQLite files on this computer through the system's file dialog, with a name that no other database has, and gives the view commands to list, rename, mark, accept and remove them, read their structures, counts, pages and statements, stop a read and run the owner's changes, which the core journals (ADR-0055, ADR-0056).
- With Databases on, conversations and the general child agent read registered databases with `describe_database` and `query_database` without asking and change them with `change_database` after the owner's approval; the explore agent gets the reading tools, a section lists the databases with their states, and "@" names a database and gives the model its structure.
- The Databases view, in the activity bar while the feature is on: the databases of this computer in the sidebar with their states and menus, "Add database…" with a name and a read-only mark, and a page for each database with the tabs Tables, with row counts on request, Structure, Data, which pages, sorts, filters, edits, inserts, deletes and copies rows as CSV, and SQL, which runs a statement with Stop for reads and asks before a change of the schema. The cards of the database tools say what each call read or changed.
- Databases reach the paired workers' hosts (ADR-0057): the owner registers a SQLite file on a worker by its path, which the worker checks under its ceiling once its `worker.toml` sets `enabled = true` under `[databases]`. The view groups the databases by machine and says when a worker is offline, needs an update or does not allow databases, and the master keeps each worker's list to show it while the worker is offline. Agents read a worker's databases without asking and change them under the master's policy and the worker's ceiling; both machines journal each write, and one whose answer the link lost ends uncertain.
- The worker serves the link's optional package `databases`, runs each request under its ceiling, journals each write under its operation ID before it runs, and forgets the host's registrations when it is unpaired. A worker without the package stays connected, and its databases say that it needs an update.

### Changed

- The "Workers" group of the sidebar collapses from its heading, as "Projects" does.
- The Databases view lists each database's tables under it in the sidebar, which choose the table of the Data and Structure tabs in place of the Tables tab and the tabs' table pickers. "Count rows" moves to the Structure tab.
- The Data tab puts the order, the conditions' buttons, the pages and the actions on rows in one row, with Insert, Delete and Copy as icons whose names show on hover.
- The row dialog writes the kind of value that a column's declared type gives and offers the kind only for a column without one. Each field starts with the row's value, or with the column's default for a new row, can be set to NULL and can be undone, in place of the choices "Unchanged" and "The column's default".

### Fixed

- The end-to-end test of workers expects a task report's result as the runtime now writes it, with the sentence that its evidence comes from the worker's own tool calls.
- A worker that connected while it was being paired, or while it was renamed, could keep showing the status that the command returned, when the list with its newer status arrived before the command's answer. The command's worker now goes in only while no list arrived during the command.

### Security

- tokio 1.53.3, which fixes the advisory RUSTSEC-2026-0386 in the constructors of `AsyncFd` on Unix, which neither the plugin's crates nor their dependencies call.
