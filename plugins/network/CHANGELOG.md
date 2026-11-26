# Changelog of the network plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- Add a self-management provider exposing redacted inspection and chat-driven actions for collections, folders, requests, environments, global variables, and cookies ([ADR-0081](../../docs/adr/0081-let-agents-inspect-and-configure-mannie.md)).

### Fixed

- Align OpenAPI YAML node counting with JSON by counting value nodes and charging mapping keys against the expanded-size limit, allowing large specifications such as GitLab's `openapi_v3.yaml` to import within the ADR-0075 safety bounds.
