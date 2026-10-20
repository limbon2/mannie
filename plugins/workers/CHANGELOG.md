# Changelog of the Workers plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Fixed

- The online worker cancellation test uses the normal confirmation grace while the offline cases keep their short deadline, so a busy test machine does not turn its cancellation check into a deadline check.
