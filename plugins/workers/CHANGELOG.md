# Changelog of the Workers plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Fixed

- The end-to-end test of workers expects a task report's result as the runtime now writes it, with the sentence that its evidence comes from the worker's own tool calls.
- A worker that connected while it was being paired, or while it was renamed, could keep showing the status that the command returned, when the list with its newer status arrived before the command's answer. The command's worker now goes in only while no list arrived during the command.

### Security

- tokio 1.53.3, which fixes the advisory RUSTSEC-2026-0386 in the constructors of `AsyncFd` on Unix, which neither the plugin's crates nor their dependencies call.
