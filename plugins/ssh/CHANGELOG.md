# Changelog of the SSH plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- The tool `start_ssh_command` starts a command on a registered host in the background, such as `journalctl -f`, which `read_command_output` reads and `stop_command` ends with TERM and KILL. Its approval says that it runs in the background, its lines never show the host's password, and editing or removing the host, turning SSH off and quitting end it.

### Changed

- The host dialog's Password and Note for agents fields explain themselves behind a "(?)" button beside their labels, as the other fields do.
