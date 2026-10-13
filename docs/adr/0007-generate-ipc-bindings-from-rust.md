# ADR-0007: Generate IPC bindings from Rust with tauri-specta

|          |                                                                                                                                                    |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Proposed                                                                                                                                           |
| Date     | 2026-10-13                                                                                                                                         |
| Deciders | Owner                                                                                                                                              |
| Related  | [ADR-0004](0004-keep-system-access-in-the-rust-core.md), [Desktop app](../architecture/desktop-app.md), [Repository](../engineering/repository.md) |

## Context

Verified 2026-10-13:

- The commands and channels between the webview and the core are the desktop app's main internal contract. Hand-written TypeScript types drift away from the Rust types.
- tauri-specta 2.0.0-rc.25, published on 2026-05-08, generates TypeScript bindings for Tauri commands, events and `Channel<T>` together with specta 2.0.0-rc.25. Version 2 has been a release candidate since 2024.
- ts-rs exports Rust types to TypeScript but does not generate command functions.

## Decision

1. Types that cross IPC (command arguments and results, errors, events and channel messages) are defined in `mannie-types`, a crate without Tauri, and derive `specta::Type` and serde.
2. `mannie-desktop` registers commands and events with tauri-specta, which writes the TypeScript bindings (typed command functions and event helpers) to `apps/desktop/src/lib/generated/` during `task gen` and in debug builds. The bindings are never committed.
3. specta and tauri-specta are pinned to exact versions and upgraded only in dedicated changes.
4. Errors cross IPC as a typed enum with a stable code and its arguments. The UI turns the code into a message from its catalog ([ADR-0019](0019-keep-visible-text-in-message-catalogs.md)) and never parses error strings. An English description travels along for logs only.
5. The frontend calls only the generated functions, never `invoke` with a command name string.
6. Streams use `tauri::ipc::Channel<T>`, with `T` defined in `mannie-types`.
7. If V2 fails, or tauri-specta stops being maintained, ts-rs exports the same types and a small typed wrapper per command group replaces the generated functions.

## Consequences

- Positive: the contract has one source of truth, and `svelte-check` catches drift during the build.
- Negative: a release-candidate dependency (risk R2), and a generation step before the frontend builds.

## Alternatives considered

| Option                        | Why not chosen                                                           |
| ----------------------------- | ------------------------------------------------------------------------ |
| Hand-written TypeScript types | They drift from the Rust types                                           |
| ts-rs alone                   | Types only; command calls stay untyped strings                           |
| Protobuf for IPC              | Designed for network contracts and integrates poorly with Tauri commands |

## Validation

V2.
