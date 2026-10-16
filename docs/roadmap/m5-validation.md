# M5 validation results

|              |                                                                                                                                                                                                                                                                                                                                                     |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                                                                                                                                                                    |
| Last updated | 2026-10-16                                                                                                                                                                                                                                                                                                                                          |
| Related      | [Milestones](milestones.md#m5-plugins), [M4 validation results](m4-validation.md), [Plugins](../features/plugins.md), [Workers as a plugin](../features/workers/plugin.md), [ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md), [ADR-0031](../adr/0031-move-workers-into-the-first-plugin.md), [Security](../architecture/security.md) |

This document records what the checks and exit criteria of [M5](milestones.md#m5-plugins) showed so far, so that the owner can review the plugins, the move of workers into the first plugin, the refinements of ADR-0030 and ADR-0031, and the defaults chosen while building, and see what remains before v0.3.0. The owner accepted both ADRs before building started ([D28](milestones.md#5-decisions)). Each section names the checks, where they ran, the result, and what is open.

The runs below were made on 2026-10-16 on the owner's Windows 11 workstation, with WebView2 for the end-to-end tests. The changes are not committed yet, and they were tested on top of 8bb5d5a. Nothing of M5 ran in CI, on Linux, on FreeBSD or on WebKitGTK: CI needs pushed branches, and the Linux container of `task test:linux` needs Docker, which was not running. What is not run yet is marked so.

## 1. Summary

| Check or criterion                                                                                                                                        | Result                                                                                                                                                                                                                 | Open                                         |
| --------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| `task ci`                                                                                                                                                 | Passed: every format, documentation and lint check, 788 Rust tests with 7 that need live services ignored, 265 Vitest tests and 117 tests of the scripts, after fixes of what Clippy found (section 9)                 | Linux and FreeBSD                            |
| The host: the order of starts, a failing start, a stop with work in progress, the default for a plugin without a choice, starts after the core's recovery | Passed: 8 tests of the host with fake plugins                                                                                                                                                                          | None                                         |
| A run keeps the tools it started with, a call into a stopped plugin is refused, a new run gets the change                                                 | Passed: 4 tests of plugins' contributions and 14 of task executors, through the agent loop                                                                                                                             | None                                         |
| Turning Workers off and on during a delegation runs nothing twice and loses no report                                                                     | Passed with a fake executor in the runtime and with the Workers plugin's executor in process                                                                                                                           | The same over the link with a real worker    |
| The M4 suites pass from the plugin's folder                                                                                                               | Passed on Windows: 21 delegation tests, 3 crash tests, 8 over the link, 3 with a link that drops, 12 of pairing, the delegated conformance suite, 5 of the contracts' fixtures, and the worker service's tests         | Linux and FreeBSD                            |
| The update from 0.2.0                                                                                                                                     | Passed: worker tasks in every state get the executor's fields, refused calls keep the texts the model read, waiting deletions move into the outbox, the owners' names of workers stay, and each module gets its record | A copy of one of the owner's 0.2.0 databases |
| The core's tests with no plugin in the build                                                                                                              | Passed: no core crate, dev-dependencies included, depends on a plugin's crate, so their tests build and run without one                                                                                                | None                                         |
| cargo-deny and the lint rule refuse a dependency of the core on a plugin                                                                                  | Passed: a planted dependency of `mannie-secrets` on `mannie-workers` fails `cargo deny check bans`, and a planted import of the Workers plugin's state in a core file fails ESLint                                     | None                                         |
| End-to-end: turning Workers off and on from the Plugins page changes the shell without a reload, with axe                                                 | Passed on WebView2 155.0.4283.45: the 3 tests of the Plugins spec and all 17 spec files, after a fix of the Plugins page (section 9)                                                                                   | WebKitGTK                                    |
| ADR-0030 and ADR-0031                                                                                                                                     | Accepted on 2026-10-16 (D28), with refinements in M5                                                                                                                                                                   | None                                         |
| v0.3.0 released                                                                                                                                           | Not yet                                                                                                                                                                                                                | The release                                  |

## 2. Plugins and the host

Checks: `crates/mannie-plugins/tests/host.rs`, 8 tests with fake plugins: a new installation starts its plugins off and keeps that as the choice; a plugin whose default is on starts, and the next launch follows the stored choice; turning a plugin on and off takes effect at once and lasts; a start that fails leaves the plugin failed, and trying again starts it; turning off a failed plugin clears the failure; quitting stops the plugins that are on and keeps the choices; a plugin's work counts its tasks that have not ended; and the host refuses unknown plugins and lists that it cannot run. `apps/desktop/src-tauri/src/credentials.rs` tests that a plugin reaches only its own entries of the credential store, and that a refused entry never reaches the store.

Result: passed.

## 3. Plugins' contributions and task executors

Checks: `crates/mannie-runtime/tests/plugins.rs`, 4 tests: a plugin's tool, agent and section reach the runs that start while it is on; a run keeps what it started with, and its calls into a stopped plugin do not run; a plugin's tool runs while the plugin is on; and the runtime takes all of a plugin's contributions or none. `crates/mannie-runtime/tests/executors.rs`, 14 tests with a fake executor whose tasks outlive their reply: a report returns as the result, Mannie without executors offers their tools to no run, a refusal keeps the executor's text for the model and its code for the owner, background tasks report through `wait_for_tasks`, a follow-up goes to the earlier target, Stop cancels and lets the task go until a wait takes it up, a stopped plugin's tool does not run, arguments that are not valid do not run, turning the plugin off lets a running task go and a later wait takes it up, a wait while the plugin is off does not run, a submission that the plugin's stop cut short lets the task go, the task's conversation is read through its executor, deleting the parent leaves the deletion in the outbox, and a state of another executor or an older one changes nothing.

Result: passed.

## 4. Workers as a plugin

Checks: in `plugins/workers/crates/mannie-workers/tests/`, the M4 suites as they moved: `delegation.rs` (21 tests through the in-process executor), `delegation_crash.rs` (3), `linked_delegation.rs` (8 over the link on loopback), `link_drop.rs` (3), `pairing.rs` (12), `delegated_conformance.rs` (every tool of the worker agent through a delegated task), and `store.rs` (3); `plugin.rs`, 3 tests: without a paired worker Workers starts off and its commands say so, with one it starts on and turning it off ends its parts, and a start without its grants fails while Mannie goes on. `plugins/workers/crates/mannie-link/tests/compatibility.rs` checks the documents of v0.2 that travel on the link, and the worker service's unit and foreground tests run in `plugins/workers/worker/`.

Result: passed on Windows. Open: the same on Linux and FreeBSD, which CI runs once the branch is pushed, and turning Workers off and on during a task with a real worker by hand.

## 5. The store and the update from 0.2.0

Checks: `crates/mannie-store/src/store/tests.rs`: a module that the binary does not know keeps its record and its tables; a module registered after the conversion carries over its migrations; two modules with one name are refused; a database of 0.2.0 gets the version of each module; a database of 0.2.0 names the executor of a worker's task, keeps the texts of its refused calls and keeps the worker's field for 0.3.0's readers; worker tasks of 0.2.0 in every state keep their states and endings, and only the accepted and running ones stay active for the executor; and the store of a module reaches only the modules it opened with. `crates/mannie-store/src/runtime/tests/executor_tasks.rs`, 8 tests of the executors' rows, versions, let-go tasks, follow-ups and the outbox. `plugins/workers/crates/mannie-workers/tests/store.rs` moves the deletions that waited for a worker into the core's outbox and keeps the owners' names of paired workers.

Result: passed. Open: an update of a copy of one of the owner's own 0.2.0 databases.

## 6. Boundaries

Checks: `cargo deny check bans` with a dependency of `mannie-secrets` on `mannie-workers` planted and removed again: it fails with "crate 'mannie-workers' is explicitly banned". ESLint with a core file that imports `state/workers.svelte.js`, planted and removed again: it fails with the rule `mannie/plugin-boundaries`. `tests/scripts/eslint/plugin-boundaries.test.mjs`, 20 cases of the rule. `cargo tree` shows no plugin crate in the graph of any core crate, dev-dependencies included.

Result: passed.

## 7. The interface

Checks: the Vitest suites in `tests/apps/desktop/`: the plugin states and how a plugin's UI follows them, the Plugins page's actions, the slots with plugins that are off, the composition root with the list of plugins, the messages of plugins, chat, tools, child agents and workers with the new codes, and the worker list while Workers turns off and on. svelte-check with warnings as errors, and `svelte-autofixer` on every new or changed component.

Result: passed. The autofixer suggests checking the effect that follows a task conversation again once its plugin is on; it starts a subscription and writes no state, so it stays.

## 8. End-to-end tests

Checks: `tests/e2e/specs/plugins.test.ts`, new: a new installation lists Workers off with none of its parts; turning it on from the keyboard shows its settings page and its command at once; turning it off hides them, and the address of its settings page then says that Workers is off and leads to the Plugins page, without a reload and with axe in both themes. `workers.test.ts` turns Workers on first, `shell.test.ts` expects the Plugins page and no Workers page in the settings of a new installation, and `chat-locale.test.ts` checks the Plugins page and, with Workers on, the Workers page in the pseudo-locale.

Result: passed on WebView2 155.0.4283.45: all 17 spec files, with the 10 tests of the workers spec against a real worker. The first run failed in two places: axe refused the Plugins page, and the workers spec looked for the old attribute of a worker in the task card. Both are fixed (section 9). Open: WebKitGTK.

## 9. Defects found

- Clippy found five functions of the runtime over its limit of lines after the plugin-off branches, a test helper that held a lock through an `if let`, a large error and list update, and lints in new tests. The functions were split around their responsibilities, the list update and the error box their large parts, and the contracts are unchanged.
- The scoped credential store had no test of its refusal, and the update from 0.2.0 had one worker task where ADR-0031 asks for every state. Both tests were added.
- The end-to-end tests found that the items of the Plugins page lacked the role of list items inside the group's list, which axe's rule `aria-required-children` refuses. Each item now has it, as the items of the Tools settings do.
- The workers spec looked for the attribute `data-task-worker` of the task card, which the card replaced with `data-task-target` and `data-task-kind` when it moved to the slot of task kinds; the spec now checks both.

## 10. Decisions for the owner

- ADR-0031 decision 8 changed while building: an executor's document on the core's task row takes the place of the plugin's table `workers_tasks` (ADR-0031, refinement in M5).
- A plugin without a stored choice follows its default once, and the outcome becomes the owner's choice.
- The host starts the plugins that are on in the background after the core's recovery.
- The Plugins page is the last page of the settings.
- WebKitGTK, Linux and FreeBSD run once the branch is pushed to CI, and v0.3.0 needs its release.
