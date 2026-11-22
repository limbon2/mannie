# Changelog of the network plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- Add the Network Client plugin (`network`) with Postman-style HTTP collections, OpenAPI v2 (Swagger 2.0) and v3 (3.0 and 3.1) JSON and YAML import, global, collection and environment variables, headers, query and path parameters, cookie jar management, owner workbench execution and redacted agent tools under per-endpoint approvals ([ADR-0075](../../docs/adr/0075-let-plugins-send-network-requests-from-collections.md)).

### Changed

- Replace the sidebar quick-action bar with a vertical overflow menu, prevent menu label wrapping, add tooltips for truncated sidebar and overview names, separate Query and Path tabs with URL synchronization, add `{{variable}}` autocomplete across inputs and code editors, use the SDK CodeMirror 6 editor for request bodies, response bodies and OpenAPI paste views ([ADR-0076](../../docs/adr/0076-provide-a-code-editor-through-the-plugin-sdk.md)), keep dialogs bounded and steady across tabs, and smooth vertical workbench resizing.
- Visually differentiate `{{variable}}` references, `{path}` placeholders and `?query=value` parameters across the URL editor, request bodies, read-only URLs, parameter/variable tables and variable inputs, dismiss resolved OpenAPI import notes (such as missing base URL and cookie warnings) as soon as they are fixed, and enable per-activity shell sidebar resizing for Network Client ([ADR-0077](../../docs/adr/0077-let-the-shell-sidebar-resize-per-activity.md)).

### Fixed

- Wait for the collection editor to finish opening before comparing tab heights in the end-to-end test.
- Lock agent request execution to the owner-configured origin, extract secret query parameters from saved URLs, include a full-body SHA-256 digest in approval summaries, honor RFC 6265 host-only cookies, honor OpenAPI v2 and v3 document and per-operation `security` rules, stage secret updates by slot before SQLite commits, distinguish post-dispatch `response_lost` failures, cap expanded YAML bytes before cloning aliases, allow large OpenAPI and body IPC payloads, refuse ambiguous request names, support nested server-variable interpolation, bound tool archive size, track concurrent workbench runs per request, surface environment switch failures, preserve cookie expiry on edit and add an end-to-end test.
