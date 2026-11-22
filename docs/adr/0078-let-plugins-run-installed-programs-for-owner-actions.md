# ADR-0078: Let plugins run installed programs for owner actions in projects

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Accepted (D72)                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Date     | 2026-11-22                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Related  | [ADR-0004](0004-keep-system-access-in-the-rust-core.md), [ADR-0013](0013-journal-every-side-effect.md), [ADR-0014](0014-contain-process-trees.md), [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), [ADR-0038](0038-let-plugins-run-installed-programs-in-the-owners-projects.md), [ADR-0056](0056-let-plugins-work-with-the-owners-sqlite-files-through-the-core.md), [Git panel actions](../../plugins/git/docs/panel-actions.md), [Security](../architecture/security.md) |

## Context

- The owner asked to upgrade the Git side panel into an interactive GitLens-style client where the owner can stage and unstage files and hunks, commit and amend, switch, create, rename and delete branches, manage tags and stashes, fetch, pull and push, merge, rebase, cherry-pick and revert, and resolve or abort conflicts directly from the panel ([decision D72](../roadmap/milestones.md#5-decisions)).
- Earlier decisions frame how this must work:
  - [ADR-0038](0038-let-plugins-run-installed-programs-in-the-owners-projects.md) decision 10 lets a plugin command run a declared installed program only to read, checked against `ReadPlace::Project`, and states that a plugin command that changes a repository or reaches the network needs an ADR of its own.
  - [ADR-0056](0056-let-plugins-work-with-the-owners-sqlite-files-through-the-core.md) decisions 8 and 9 establish the general rule: a plugin command that changes something on the owner's action is an owner operation of the core's journal, committed durably before the side effect starts, marked uncertain by startup recovery if interrupted, and never repeated under the same ID.
  - [ADR-0013](0013-journal-every-side-effect.md) requires every side effect to be journaled before execution, and [ADR-0014](0014-contain-process-trees.md) requires installed programs to run without a shell under the process supervisor.
  - [ADR-0004](0004-keep-system-access-in-the-rust-core.md) treats the webview as untrusted, so commands take a project ID and structured action parameters rather than arbitrary paths or shell strings, and Rust enforces every boundary and confirmation gate.

## Decision

1. The core journals owner-initiated runs of installed programs in a new table `tools_program_operations` owned by the `tools` store module (Migration 8 in `crates/mannie-store`). Each row records:
   - `id`: a UUIDv7 `ProgramOperationId` chosen by the UI per owner action, so duplicate submissions return `AlreadyRun` and never run twice;
   - `plugin`, `project_id`, `program`, and a short stable `action` code (such as `stage`, `commit`, `push`);
   - `arguments_digest`: a SHA-256 hex digest of the canonical operation payload, never raw arguments, commit messages, file contents, or credentials (T-47);
   - `effects`: JSON of redacted `ProgramEffect` entries describing hooks, configured programs, discarded work, rewritten published commits, or remote endpoints;
   - `state`: `'executing'`, `'completed'`, `'failed'`, or `'uncertain'`, plus optional `exit_code`, `failure` code, and timestamps.
     The record commits with `Durability::Full` before any process spawns. Startup recovery in `Runtime::open()` calls `recover_program_operations()` alongside `recover_exports()` and `recover_db_operations()`, marking every `'executing'` row `'uncertain'`. An operation already in the journal never runs again.

2. The `processes` service (`Processes`, major 1, additive) gains two methods on `ProcessesApi` and an optional ticket on `ProgramRun`:
   - `begin_owner_operation(request: OwnerOperationRequest) -> Result<OwnerOperationTicket, ProcessError>`: checks that the program is declared in the plugin's manifest, checks the `action` code syntax, resolves the `project_id` to its canonical folder, validates every folder in `request.folders` (refusing Mannie's protected folders as `InvalidRun` per T-18, allowing folders inside the project's canonical folder, and requiring an existing project or shared write grant for any external folder such as a linked worktree's `.git` dir, refusing with `WriteNotAllowed` otherwise), commits the `'executing'` journal row with `Durability::Full`, and issues an opaque in-memory `OwnerOperationTicket` scoped to `(plugin, program, authorized_folders)`.
   - `ProgramRun::for_owner_operation(ticket)`: binds each program invocation of a single logical owner action (including multi-step git sequences such as snapshot + restore or hash-object + update-index + checkout-index) to the active ticket. `BoundProcesses::run` verifies that the ticket belongs to the calling plugin and declared program and that `working_directory` lies inside the ticket's authorized folders, then runs the program through the existing supervisor with fixed manifest arguments, allowlisted environment, output limits, and timeout.
   - `finish_owner_operation(ticket, outcome)`: revokes the ticket and transitions the journal row to `'completed'`, `'failed'`, or `'uncertain'`. Dropping an unfinished operation session triggers cleanup that marks the row `'uncertain'` and revokes the ticket.

3. Owner actions from a plugin view do not route through agent approval cards, because the owner's click is direct intent (ADR-0056 decision 8). Instead, Rust enforces a two-step preview and confirmation gate for any action with heightened effects:
   - Actions that trigger repository hooks or repository-configured programs (`RunsFolderCode`), discard work (`DiscardsWork`), rewrite published commits (`RewritesPublished`), or delete branches or tags require explicit confirmation in the UI showing the exact hooks, sources, discarded items (and whether a `"Mannie: before ..."` stash snapshot is kept), or rewritten commits.
   - The backend evaluates the repository state and effects under the plugin's per-repository lock (`RepositoryLocks`, shared with agent tools so owner actions and agent tool calls never race). If confirmation is required and the request lacks a matching `confirmed_state` and `effects_digest`, the command returns `NeedsConfirmation` with the fresh preview and executes nothing. If the repository changed between preview and confirmation, the command returns `RepositoryChanged` and executes nothing.
   - Normal local and remote actions that have no hooks, no repository-configured programs, no discarded work, and no published-commit rewrites run directly on explicit owner click while still going through the journal, folder checks, `GIT_ALLOW_PROTOCOL`, and process supervisor.

4. The threats of [security](../architecture/security.md) gain T-119 and T-120: script in the webview attempting to write outside a project, replay a completed owner action, or submit a stale confirmation (with webview confirmation forgery noted as the same section 8 residual risk as agent approvals and database schema confirmations), and a race between an owner action in the panel and a concurrent agent tool call on the same repository.

Earlier decisions that this ADR changes:

| ADR  | Decision | Change                                                                                                                                               |
| ---- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0013 | d1, d2   | Refined: owner-initiated runs of declared installed programs from plugin views are journaled as owner operations in `tools_program_operations`       |
| 0034 | d5       | Refined: `ProcessesApi` gains `begin_owner_operation`, `finish_owner_operation`, and `ProgramRun::for_owner_operation`, plus `ProgramOperationId`    |
| 0038 | d10      | Refined: a plugin command may run a declared installed program to change a project's repository on the owner's action as a journaled owner operation |

## Consequences

- Positive: the Git panel and any future project-tool plugin can offer interactive owner actions without leaving Mannie, under the same process containment, folder checks, secret redaction, hook detection, and durable crash-safe journal as agent tools and Databases. Agent tools and panel actions share `RepositoryLocks`, preventing index lock races.
- Negative: the core gains a journal table, startup recovery step, ticket map in `Runtime`, and two additive methods on `ProcessesApi`. A compromised webview could invoke panel actions and confirm dialogs it receives on open projects (recorded in T-119 and section 8, parallel to T-60, T-68, and T-77).
- Risks: a multi-step owner action that crashes between steps leaves the journal row `'uncertain'` and git in its natural state; the UI refreshes and surfaces the resulting repository state clearly without retrying the operation.

## Alternatives considered

| Option                                                 | Why not chosen                                                                                                                    |
| ------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------- |
| Keep the Git panel read-only and delegate writes to AI | Slow and requires model tokens and turns for routine staging, commits, branch switches, and conflict picks (D72)                  |
| Route panel clicks through the agent approval queue    | Panel actions belong to a project rather than an active agent turn, and owner clicks should not create fake transcript tool calls |
| Let plugins write directly to a plugin-private journal | Side-effect journaling and startup crash recovery belong in the core store (ADR-0013, ADR-0056 decision 9)                        |
| Pass a static list of commands to one `run_batch` call | Multi-step git sequences (such as `stash create` then `stash store`, or `hash-object` then `update-index`) pass step outputs on   |

## Validation

Tests:

- Store tests in `crates/mannie-store` verifying `tools_program_operations` migration, statement preparation, duplicate-ID rejection, and recovery of executing records to `'uncertain'`.
- Runtime integration tests in `crates/mannie-runtime/tests/processes.rs` and `owner_operations.rs` verifying folder boundaries, Mannie protected-folder refusal (T-18), external folder grant checks, ticket validation, duplicate-ID refusal, and crash recovery.
- Git plugin Rust tests and end-to-end tests in `plugins/git/` covering staging, hunk staging, committing with hooks confirmation, stale-state rejection (`repository_changed`), branch/stash/remote/conflict actions, and axe accessibility checks in both themes.
