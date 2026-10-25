# Architecture decision records

|              |                                                                                                                                                                                           |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Living index                                                                                                                                                                              |
| Last updated | 2026-10-24                                                                                                                                                                                |
| Related      | [ADR-0001](0001-record-architecture-decisions.md), [Template](0000-template.md), [Decisions](../roadmap/milestones.md#5-decisions), [Validations](../roadmap/milestones.md#4-validations) |

[ADR-0001](0001-record-architecture-decisions.md) describes the process, and new ADRs start from the [template](0000-template.md).

Each ADR has exactly one of these statuses:

- Accepted (Dn): decided by the owner. Decision Dn in the [decision record](../roadmap/milestones.md#5-decisions) holds the owner's reasons.
- Accepted: validated and approved.
- Proposed: the recommended direction. It becomes Accepted when the validations named in its Validation section pass and the owner approves. The milestone that runs an ADR's last validation cannot exit while that ADR is still Proposed.
- Superseded by ADR-NNNN: replaced by a later ADR and kept for its history.
- Deprecated: no longer applies, and nothing replaces it.

| ADR                                                                           | Decision                                                                                      | Status                 | Validation |
| ----------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- | ---------------------- | ---------- |
| [0001](0001-record-architecture-decisions.md)                                 | Record architecture decisions as ADRs                                                         | Accepted               | None       |
| [0002](0002-grow-through-extension-points.md)                                 | Grow the product through features that plug into extension points                             | Accepted (D14)         | None       |
| [0003](0003-build-the-desktop-app-on-tauri-2.md)                              | Build the desktop app on Tauri 2                                                              | Accepted (D3)          | None       |
| [0004](0004-keep-system-access-in-the-rust-core.md)                           | Keep logic and system access in the Rust core and treat the webview as untrusted              | Accepted               | V1         |
| [0005](0005-build-the-ui-with-sveltekit-as-a-static-spa.md)                   | Build the UI with Svelte 5 and SvelteKit as a static single-page app                          | Accepted               | V1         |
| [0006](0006-use-shadcn-svelte-through-its-cli.md)                             | Add and update UI components only through the shadcn-svelte CLI and its documentation         | Accepted (D3)          | None       |
| [0007](0007-generate-ipc-bindings-from-rust.md)                               | Generate IPC bindings from Rust with tauri-specta                                             | Accepted               | V2         |
| [0008](0008-one-workspace-two-binaries.md)                                    | Build a desktop binary and a headless worker from one Cargo workspace                         | Accepted               | V5         |
| [0009](0009-persist-state-in-sqlite.md)                                       | Persist state in SQLite through rusqlite                                                      | Accepted               | V3         |
| [0010](0010-keep-provider-credentials-on-the-master.md)                       | Keep provider credentials in the master's credential store and lease them to workers per task | Accepted               | V7, V6     |
| [0011](0011-own-the-provider-adapters.md)                                     | Call providers through Mannie's own streaming adapters                                        | Accepted               | V8         |
| [0012](0012-run-agents-as-durable-actors.md)                                  | Run agents as durable actors independent of any connection                                    | Accepted               | V3, M1     |
| [0013](0013-journal-every-side-effect.md)                                     | Perform every side effect as a journaled, authorized operation                                | Accepted               | M2         |
| [0014](0014-contain-process-trees.md)                                         | Contain command process trees on every platform                                               | Accepted               | V4         |
| [0015](0015-run-an-agent-runtime-on-every-machine.md)                         | Run an agent runtime on every machine: the master orchestrates, workers own their hosts       | Accepted (D5)          | None       |
| [0016](0016-link-machines-over-pinned-mtls-grpc.md)                           | Connect the master and workers over gRPC with mutually pinned TLS                             | Accepted               | V6         |
| [0017](0017-license-and-dependency-policy.md)                                 | License Mannie under Apache-2.0 and admit dependencies by license policy                      | Accepted               | V9         |
| [0018](0018-release-from-tagged-builds.md)                                    | Release the desktop app and the worker together from tagged builds                            | Accepted               | V10        |
| [0019](0019-keep-visible-text-in-message-catalogs.md)                         | Keep all visible text in message catalogs, with English first                                 | Accepted               | V11        |
| [0020](0020-complete-the-ui-shell-slots.md)                                   | Complete the UI shell slots with panels, overlays, part renderers and settings pages          | Accepted               | M1         |
| [0021](0021-render-model-output-as-markdown-with-micromark.md)                | Render model output as Markdown with micromark and Mannie's own components                    | Accepted               | M1         |
| [0022](0022-add-conversation-controls-to-the-shell.md)                        | Add a slot for conversation controls to the UI shell                                          | Superseded by ADR-0023 | M2         |
| [0023](0023-organize-conversations-into-projects.md)                          | Organize conversations into projects                                                          | Accepted               | M2         |
| [0024](0024-run-child-agent-tasks-in-the-parent-run.md)                       | Run the tasks of child agents from their parent's run                                         | Accepted               | M3         |
| [0025](0025-add-side-panels-to-the-shell.md)                                  | Add side panels beside the main view to the shell                                             | Accepted (D18)         | M3         |
| [0026](0026-add-conversation-controls-back-for-child-agents.md)               | Add conversation controls back to the shell for child agents                                  | Accepted (D19)         | M3         |
| [0027](0027-delegate-tasks-to-workers-through-a-runtime-port.md)              | Delegate tasks to workers through a port of the runtime                                       | Accepted               | M4, V6     |
| [0028](0028-run-assigned-tasks-under-the-host-ceiling.md)                     | Run the tasks that the master assigns under the worker's policy ceiling                       | Accepted               | M4, V6     |
| [0029](0029-let-the-owner-give-conversations-and-hosts-full-access.md)        | Let the owner give conversations and hosts full access                                        | Accepted               | M4         |
| [0030](0030-split-mannie-into-a-core-and-plugins.md)                          | Split Mannie into a core and internal plugins that the owner turns on and off                 | Accepted (D28)         | M5         |
| [0031](0031-move-workers-into-the-first-plugin.md)                            | Move the worker system into the first plugin, behind extension points of the core             | Accepted (D28)         | M5         |
| [0032](0032-let-features-and-plugins-add-kinds-of-mentions.md)                | Let features and plugins add kinds of mentions to the message box                             | Accepted (D30)         | M6         |
| [0033](0033-let-plugins-attach-images-to-the-owners-messages.md)              | Let plugins attach images to the owner's messages                                             | Accepted (D31)         | M7         |
| [0034](0034-build-plugins-on-a-plugin-sdk.md)                                 | Build plugins on a plugin SDK that bridges them to the core                                   | Accepted (D33)         | M5         |
| [0035](0035-find-plugins-at-build-time-in-folders-of-their-own.md)            | Find plugins at build time, each in a folder and a Cargo workspace of its own                 | Accepted (D33)         | M5         |
| [0036](0036-let-plugins-run-commands-on-remote-hosts.md)                      | Let plugins run commands on remote hosts under the master's policy                            | Accepted (D35)         | M8         |
| [0037](0037-run-the-checks-of-task-ci-side-by-side.md)                        | Run the checks of `task ci` side by side, with Clippy in a target folder of its own           | Accepted (D36)         | None       |
| [0038](0038-let-plugins-run-installed-programs-in-the-owners-projects.md)     | Let plugins run installed programs in the owner's projects under the master's policy          | Accepted (D37)         | M9         |
| [0039](0039-let-plugins-describe-their-tool-calls-in-the-transcript.md)       | Let plugins describe their tool calls in the transcript                                       | Accepted (D37)         | M9         |
| [0040](0040-export-complete-conversation-records.md)                          | Let the owner export complete conversation records                                            | Accepted (D38)         | M10        |
| [0041](0041-let-agents-run-commands-in-the-background.md)                     | Let agents run commands in the background, and wake an idle agent when one ends               | Accepted (D40)         | M11        |
| [0042](0042-place-conversation-controls-in-a-menu-and-the-message-box.md)     | Place conversation controls in a menu and in the message box                                  | Accepted (D41)         | None       |
| [0043](0043-let-plugins-offer-the-tools-of-tool-servers.md)                   | Let plugins offer the tools of tool servers that the owner registered, such as MCP servers    | Accepted (D42)         | M12        |
| [0044](0044-let-tool-results-carry-images-to-the-model.md)                    | Let tool results carry images to the model                                                    | Accepted (D42)         | M12        |
| [0045](0045-switch-the-sidebar-between-views-with-an-activity-bar.md)         | Switch the sidebar between views with an activity bar                                         | Accepted (D43)         | M13        |
| [0046](0046-let-plugins-attach-context-to-conversations.md)                   | Let plugins attach context to conversations                                                   | Accepted (D43)         | M13        |
| [0047](0047-serve-embeddings-and-reranking-through-provider-profiles.md)      | Serve embeddings and reranking through provider profiles, and let plugins use them            | Accepted (D43)         | M13        |
| [0048](0048-erase-what-plugins-delete-from-the-store.md)                      | Erase what plugins delete from the store                                                      | Proposed               | M13        |
| [0049](0049-gather-context-for-the-messages-that-ask-and-cite-it-by-links.md) | Gather context for the messages that ask for it, and cite it by links                         | Accepted (D44)         | M13        |
| [0050](0050-let-a-mention-attach-what-it-names.md)                            | Let a mention attach what it names                                                            | Accepted (D44)         | M13        |
| [0051](0051-let-a-plugin-keep-conversations-of-its-own.md)                    | Let a plugin keep conversations of its own                                                    | Accepted (D44)         | M13        |
| [0052](0052-offer-a-plugins-tools-where-its-attachments-are.md)               | Offer a plugin's tools where its attachments are                                              | Accepted (D44)         | M13        |
