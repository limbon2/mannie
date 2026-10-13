# Changelog

All notable changes to Mannie are recorded in this file. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/). Each change adds one line under Unreleased, and a release turns that section into its notes.

## Unreleased

### Added

- Documentation baseline: vision, goals and scope, the feature protocol, architecture, architecture decision records, milestones, engineering standards and agent rules.
- Documentation checks, run with `task docs:check`.
- Decisions on the interface language, closing the window, code signing and release hosting, with the window and tray feature and ADR-0019 for message catalogs.
- Typed IPC bindings generated from Rust (`task gen`), and channel streams in batches paced by the display's frames.
- The SQLite store, with migrations, a copy of the database before each migration, and refusal of a database from a newer version.
- Credentials in Windows Credential Manager and the Linux Secret Service; without a Secret Service, Mannie stores none.
- Logs in daily rotating files, with secrets redacted, and crash files.
- Mannie's application icon.
