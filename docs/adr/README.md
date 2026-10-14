# Architecture decision records

|              |                                                                                                                                                                                           |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Living index                                                                                                                                                                              |
| Last updated | 2026-10-14                                                                                                                                                                                |
| Related      | [ADR-0001](0001-record-architecture-decisions.md), [Template](0000-template.md), [Decisions](../roadmap/milestones.md#5-decisions), [Validations](../roadmap/milestones.md#4-validations) |

[ADR-0001](0001-record-architecture-decisions.md) describes the process, and new ADRs start from the [template](0000-template.md).

Each ADR has exactly one of these statuses:

- Accepted (Dn): decided by the owner. Decision Dn in the [decision record](../roadmap/milestones.md#5-decisions) holds the owner's reasons.
- Accepted: validated and approved.
- Proposed: the recommended direction. It becomes Accepted when the validations named in its Validation section pass and the owner approves. The milestone that runs an ADR's last validation cannot exit while that ADR is still Proposed.
- Superseded by ADR-NNNN: replaced by a later ADR and kept for its history.
- Deprecated: no longer applies, and nothing replaces it.

| ADR                                                            | Decision                                                                                      | Status         | Validation |
| -------------------------------------------------------------- | --------------------------------------------------------------------------------------------- | -------------- | ---------- |
| [0001](0001-record-architecture-decisions.md)                  | Record architecture decisions as ADRs                                                         | Accepted       | None       |
| [0002](0002-grow-through-extension-points.md)                  | Grow the product through features that plug into extension points                             | Accepted (D14) | None       |
| [0003](0003-build-the-desktop-app-on-tauri-2.md)               | Build the desktop app on Tauri 2                                                              | Accepted (D3)  | None       |
| [0004](0004-keep-system-access-in-the-rust-core.md)            | Keep logic and system access in the Rust core and treat the webview as untrusted              | Proposed       | V1         |
| [0005](0005-build-the-ui-with-sveltekit-as-a-static-spa.md)    | Build the UI with Svelte 5 and SvelteKit as a static single-page app                          | Proposed       | V1         |
| [0006](0006-use-shadcn-svelte-through-its-cli.md)              | Add and update UI components only through the shadcn-svelte CLI and its documentation         | Accepted (D3)  | None       |
| [0007](0007-generate-ipc-bindings-from-rust.md)                | Generate IPC bindings from Rust with tauri-specta                                             | Proposed       | V2         |
| [0008](0008-one-workspace-two-binaries.md)                     | Build a desktop binary and a headless worker from one Cargo workspace                         | Proposed       | V5         |
| [0009](0009-persist-state-in-sqlite.md)                        | Persist state in SQLite through rusqlite                                                      | Proposed       | V3         |
| [0010](0010-keep-provider-credentials-on-the-master.md)        | Keep provider credentials in the master's credential store and lease them to workers per task | Proposed       | V7, V6     |
| [0011](0011-own-the-provider-adapters.md)                      | Call providers through Mannie's own streaming adapters                                        | Proposed       | V8         |
| [0012](0012-run-agents-as-durable-actors.md)                   | Run agents as durable actors independent of any connection                                    | Proposed       | V3, M1     |
| [0013](0013-journal-every-side-effect.md)                      | Perform every side effect as a journaled, authorized operation                                | Proposed       | M2         |
| [0014](0014-contain-process-trees.md)                          | Contain command process trees on every platform                                               | Proposed       | V4         |
| [0015](0015-run-an-agent-runtime-on-every-machine.md)          | Run an agent runtime on every machine: the master orchestrates, workers own their hosts       | Accepted (D5)  | None       |
| [0016](0016-link-machines-over-pinned-mtls-grpc.md)            | Connect the master and workers over gRPC with mutually pinned TLS                             | Proposed       | V6         |
| [0017](0017-license-and-dependency-policy.md)                  | License Mannie under Apache-2.0 and admit dependencies by license policy                      | Proposed       | V9         |
| [0018](0018-release-from-tagged-builds.md)                     | Release the desktop app and the worker together from tagged builds                            | Proposed       | V10        |
| [0019](0019-keep-visible-text-in-message-catalogs.md)          | Keep all visible text in message catalogs, with English first                                 | Proposed       | V11        |
| [0020](0020-complete-the-ui-shell-slots.md)                    | Complete the UI shell slots with panels, overlays, part renderers and settings pages          | Proposed       | M1         |
| [0021](0021-render-model-output-as-markdown-with-micromark.md) | Render model output as Markdown with micromark and Mannie's own components                    | Proposed       | M1         |
