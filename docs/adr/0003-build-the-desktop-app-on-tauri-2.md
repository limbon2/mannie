# ADR-0003: Build the desktop app on Tauri 2

|          |                                                                                                                                                        |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Accepted (D3)                                                                                                                                          |
| Date     | 2026-10-13                                                                                                                                             |
| Deciders | Owner                                                                                                                                                  |
| Related  | [ADR-0004](0004-keep-system-access-in-the-rust-core.md), [ADR-0008](0008-one-workspace-two-binaries.md), [Desktop app](../architecture/desktop-app.md) |

Later changes: [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md) supersedes decision 3 in part: the build generates an inlined Tauri plugin for each of Mannie's plugins. [ADR-0035](0035-find-plugins-at-build-time-in-folders-of-their-own.md) supersedes decision 2 in part: `mannie-desktop-core` and `mannie-desktop` both use Tauri.

## Context

Verified 2026-10-13:

- The owner chose Tauri ([decision D3](../roadmap/milestones.md#5-decisions)).
- Tauri 2.12.3 is the latest stable release of the `tauri` crate and of `@tauri-apps/api`. Tauri 3.0.0-alpha.4 was published on 2026-10-01.
- Tauri renders the UI in the operating system's webview, WebView2 on Windows and WebKitGTK on Linux, and the application core is Rust.

## Decision

1. The desktop app is a Tauri 2 application on the latest 2.x release, with exact versions pinned in `Cargo.lock` and the pnpm lockfile.
2. Only the `mannie-desktop` crate uses Tauri APIs and types. Domain logic lives in crates that do not depend on Tauri, so the worker and the tests build without it.
3. The app uses two plugins, single-instance and opener. Any other plugin needs an ADR.
4. Moving to Tauri 3 needs its own ADR after Tauri 3 is stable, and the migration is planned as separate work.

## Consequences

- Positive: small installers, a Rust core, and webview security fixes that arrive with operating system updates.
- Negative: WebView2 and WebKitGTK behave differently, so the UI is tested on both (risk R4). Tauri 3 will require a migration (risk R1).

## Alternatives considered

| Option                                       | Why not chosen                                                             |
| -------------------------------------------- | -------------------------------------------------------------------------- |
| Electron                                     | Ships its own Chromium and runs a Node main process; the owner chose Tauri |
| A native Rust GUI toolkit (egui, Slint, GTK) | Cuts the UI off from the Svelte component ecosystem it is built on         |
| A local web server opened in a browser       | Adds a loopback HTTP server as an attack surface and has no OS integration |

## Validation

None needed: the owner decided this (D3). V1 checks the stack on both webviews.
