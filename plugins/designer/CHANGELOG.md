# Changelog of the designer plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- Adds the optional Designer activity, with a sidebar for creating and selecting projects that persist across restarts.
- Draws a rectangle in a native Windows OpenGL viewport and changes its color as the owner moves a slider.
- Saves each project's rectangle geometry and fill across reloads and restarts and restores the confirmed color when a save fails.
