# Changelog

All notable changes to Mannie are recorded in this file. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/). Each change adds one line under Unreleased, and a release turns that section into its notes.

## Unreleased

### Added

- Documentation baseline: vision, goals and scope, the feature protocol, architecture, architecture decision records, milestones, engineering standards and agent rules.
- Documentation checks, run with `task docs:check`.
- Decisions on the interface language, closing the window, code signing and release hosting, with the window and tray feature and ADR-0019 for message catalogs.
- The Cargo workspace on a pinned Rust 1.99 toolchain, with the `mannie-types`, `mannie-store`, `mannie-logging` and `mannie-secrets` crates, the desktop app and the worker.
- The desktop shell: Tauri 2 with the security baseline of ADR-0004, a SvelteKit single-page app built from shadcn-svelte components, a sidebar, a command palette (Ctrl+K), the settings view (Ctrl+,), light and dark themes, and one running instance.
- Typed IPC bindings generated from Rust (`task gen`), and channel streams in batches paced by the display's frames.
- The SQLite store, with migrations, a copy of the database before each migration, and refusal of a database from a newer version.
- Credentials in Windows Credential Manager and the Linux Secret Service; without a Secret Service, Mannie stores none.
- Logs in daily rotating files, with secrets redacted, and crash files.
- The worker: a headless service that starts, logs and opens its store, and installs itself as a Windows service, a systemd unit or an rc.d service with `mannie-worker service install`.
- The interface language as a setting, with English message catalogs and a pseudo-locale that the end-to-end tests check.
- Mannie's application icon.
- CI on Windows and Linux, nightly worker tests on FreeBSD, and a release workflow that builds, checks, signs and drafts a release from a version tag.
- The tasks `gen`, `dev`, `build`, `lint`, `test`, `e2e` and `release:prepare`.
- The shadcn-svelte agent skill and the Svelte MCP server for coding agents.
