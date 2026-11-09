# Changelog of the MCP plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Fixed

- A server that started while it was being added, turned on or signed out could keep showing the status that the command returned, when the list with its newer status arrived before the command's answer. The command's server now goes in only while no list arrived during the command.
