# Changelog of the SSH plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- Add a self-management provider exposing secret-free host inspection and chat-driven actions to add, update, remove, scan, trust, and test SSH hosts ([ADR-0081](../../docs/adr/0081-let-agents-inspect-and-configure-mannie.md)).
