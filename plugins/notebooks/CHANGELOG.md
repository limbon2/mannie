# Changelog of the notebooks plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- Add a self-management provider exposing notebook and indexing diagnostics and chat-driven actions for notebooks, text sources, source status, and search preferences ([ADR-0081](../../docs/adr/0081-let-agents-inspect-and-configure-mannie.md)).
