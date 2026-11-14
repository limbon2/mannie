# Browser changelog

Changes to the Browser plugin. Releases collect its entries in the [Mannie changelog](../../CHANGELOG.md).

## Unreleased

### Added

- Add Browser Settings for trusted hosts and saved site credentials. An agent needs consent for each credential in its conversation.
- Open and create tabs from the Browser menu beside Terminal.

### Changed

- Remove the Browser pane's options menu. Trusted hosts and site credentials stay under **Settings > Browser**.

### Fixed

- Use Mannie's primary color for the agent cursor and its click highlight.
- Keep the agent cursor visible during tab status updates.
- Accept bare domains in the Browser address bar and let the owner dismiss browser errors.
- Group Browser pane actions at the end of the toolbar and use the SSH Hosts settings pattern for trusted hosts and site credentials.
