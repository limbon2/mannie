# Changelog of the Workers plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Changed

- The documents of tool calls on the link gain the field `interruption` (ADR-0036), which a master or a worker one minor version behind ignores; their fixtures for v0.4 are in `crates/mannie-link/tests/fixtures/documents/v0.4/`.
