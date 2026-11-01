# Changelog of the Workers plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- Databases, an optional feature that the owner turns on under Workers on the Plugins page, registers SQLite files on this computer through the system's file dialog, with a name that no other database has, and gives the view commands to list, rename, mark, accept and remove them, read their structures, counts, pages and statements, stop a read and run the owner's changes, which the core journals (ADR-0055, ADR-0056).
- With Databases on, conversations and the general child agent read registered databases with `describe_database` and `query_database` without asking and change them with `change_database` after the owner's approval; the explore agent gets the reading tools, a section lists the databases with their states, and "@" names a database and gives the model its structure.
- The Databases view, in the activity bar while the feature is on: the databases of this computer in the sidebar with their states and menus, "Add database…" with a name and a read-only mark, and a page for each database with the tabs Tables, with row counts on request, Structure, Data, which pages, sorts, filters, edits, inserts, deletes and copies rows as CSV, and SQL, which runs a statement with Stop for reads and asks before a change of the schema. The cards of the database tools say what each call read or changed.

### Fixed

- The end-to-end test of workers expects a task report's result as the runtime now writes it, with the sentence that its evidence comes from the worker's own tool calls.
- A worker that connected while it was being paired, or while it was renamed, could keep showing the status that the command returned, when the list with its newer status arrived before the command's answer. The command's worker now goes in only while no list arrived during the command.

### Security

- tokio 1.53.3, which fixes the advisory RUSTSEC-2026-0386 in the constructors of `AsyncFd` on Unix, which neither the plugin's crates nor their dependencies call.
