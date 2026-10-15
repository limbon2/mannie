# M2 validation results

|              |                                                                                                                                                                                                                                                                                                                     |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                                                                                                                                    |
| Last updated | 2026-10-15                                                                                                                                                                                                                                                                                                          |
| Related      | [Milestones](milestones.md#m2-system-tools), [M1 validation results](m1-validation.md), [System tools](../features/system-tools/README.md), [ADR-0013](../adr/0013-journal-every-side-effect.md), [ADR-0014](../adr/0014-contain-process-trees.md), [ADR-0023](../adr/0023-organize-conversations-into-projects.md) |

This document records what the validations and exit criteria of [M2](milestones.md#m2-system-tools) showed so far, so that the owner can accept ADR-0013 and see what remains before v0.2.0. The owner accepted ADR-0023, which supersedes ADR-0022, on 2026-10-15. The checks stay in the repository and run in CI. Each section names the checks, where they ran, the result, and what is open.

The runs below were made on 2026-10-14 and 2026-10-15 on the owner's Windows 11 workstation (WebView2 155.0.4283.45), and the Rust suites also in a Linux container (`task test:linux`, the `rust:1.99` image). The same container ran Clippy for FreeBSD. The changes are not committed yet. They were tested on top of 71eb00b, the commit that added chat, and `origin/main` has since gained the release commit of 0.1.0 and the update of rustls. Nothing of M2 ran in CI, on WebKitGTK, on a FreeBSD machine or with a live model: M2 is not pushed yet, and the evaluations need the owner's keys and endpoints. What is not run yet is marked so.

## 1. Summary

| Check or criterion                            | Result                                                                                                                                                                | Open                                                                                 |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| Tool conformance suite                        | Passed on Windows and Linux; the tools and the runtime build for FreeBSD without warnings                                                                             | CI on both systems, and the nightly tests of the runtime on FreeBSD                  |
| Ending the process in every journal state     | Passed: five kill points on Windows and Linux, and an end-to-end test that kills the app during a command                                                             | The end-to-end test on WebKitGTK                                                     |
| Prompt-injection corpus                       | Passed: no side effect, no grant, no rule and no move into a project without the owner                                                                                | None                                                                                 |
| A path changed between approval and execution | Passed: unit tests of the swaps, and both swap cases through the runtime                                                                                              | None                                                                                 |
| V4, process trees                             | Passed on Windows and Linux, for a normal end, a timeout and Stop                                                                                                     | FreeBSD, when workers bring commands there (M4)                                      |
| Evaluations                                   | The harness and its checks pass all 24 tasks with the scripted oracle                                                                                                 | Live runs with an OpenAI model and an open-weight model                              |
| ADR-0013, the journal                         | Passed: the tests above, and the recovery table of [operations and approvals](../features/system-tools/operations-and-approvals.md#recovery)                          | The owner's review                                                                   |
| ADR-0023, projects                            | Passed: the store's upgrade from schema 11, the runtime tests of projects and moves, and the end-to-end tests of the sidebar with axe and the pseudo-locale; Accepted | WebKitGTK                                                                            |
| Performance targets                           | All met on the workstation; a streamed piece, a cold start and opening a long conversation measured higher than in M1                                                 | A side-by-side run of M1 and M2 on the reference machine, which enforces the targets |
| End-to-end tests                              | All 14 spec files pass on WebView2                                                                                                                                    | WebKitGTK in CI                                                                      |
| `task ci`                                     | Passes after the merge with `main`, which updated rustls to 0.23.46 for four advisories published on 2026-10-14                                                       | None                                                                                 |
| ADRs Accepted, v0.2.0 released                | ADR-0023 is Accepted; ADR-0013 and v0.2.0 not yet                                                                                                                     | The owner's review of ADR-0013; v0.1.0 comes first                                   |

## 2. Tool conformance

Checks: `crates/mannie-tools/tests/conformance.rs` runs every tool against real temporary folders, through each success path and each error code: encodings with their byte order marks and line endings, pages, the result limit and archived outputs, revisions, swaps of files and folders, and commands with exit codes, separate and cut streams, timeouts, Stop, the working directory, output in other scripts than Latin and the environment's allow list. `crates/mannie-tools/tests/definitions.rs` compares the tools' definitions, as the model receives them, with a reviewed snapshot.

| System  | Unit tests | Conformance | Definitions |
| ------- | ---------- | ----------- | ----------- |
| Windows | 113        | 46          | 2           |
| Linux   | 109        | 45          | 2           |

The conformance counts differ by the platform's own cases: Windows adds the refusal of network paths while preparing and a multi-line PowerShell command, and Linux adds that `file_info` never follows a link.

FreeBSD gets commands with workers in M4, so M2 only builds the tools there. Clippy passes with `-D warnings` for `x86_64-unknown-freebsd` on every target of `mannie-tools` and of the crates that the nightly FreeBSD job tests, but for the worker: its `SERVICE_NAME` constant, from M0, is unused on FreeBSD. A pull-request step in `ci.yml` lints `mannie-tools` the same way. The tools first run on FreeBSD through the runtime's tests in the nightly job, which has not run M2 yet.

## 3. ADR-0013: the journal

Checks:

- `crates/mannie-runtime/tests/tool_crash.rs` starts itself again as a child that runs a scripted conversation, whose command appends a marker line, and kills it while the call is received, while it waits for approval, before and during the side effect, and after it completed. After each restart, the run is interrupted, the call is canceled, uncertain or completed as the recovery table says, a retry reports an uncertain call without repeating it, and the marker appears at most once. On Linux the recovery also ends what was left of the command's tree.
- `crates/mannie-runtime/tests/tools_loop.rs`: 20 tests of the loop, approvals, grants, stale decisions, Stop while an approval waits and while a command runs, budgets, notes, profiles without tools, retries that keep a reply with tool calls, Mannie's own folders, both swap cases, and the activity of a call. Its conversations start in a project, whose folder is their workspace.
- `crates/mannie-store`: the statuses that recovery stores, `part_updated` in replays and snapshots, kept runs, and deletion with a planted string in archives and grants.
- `tests/e2e/specs/tools.test.ts` kills the app while a command runs: after the restart the call shows "May have run", and Retry finishes the reply without running the command again.

Result: all pass on Windows and Linux.

## 4. Prompt injection and changed paths

- `crates/mannie-runtime/tests/injection.rs`: a scripted, compromised model asks for every operation that a hostile corpus asks for: writes outside the workspace, deletes inside it, `git status; rm -rf x` against a saved rule `git status`, edits of Mannie's database, paths through links, UNC and device paths. The owner never approves. Nothing changes on the disk, no grant or rule appears, and every call is denied by the owner or refused by its tool, also a move of the project's conversation into another project. On FreeBSD, which has no commands yet, the command call ends as not run.
- `crates/mannie-runtime/tests/projects.rs`: a move that follows another tool call of the turn asks even when the owner's message names the project, and the owner's denial keeps the conversation where it is.
- The swap tests replace a folder by a junction or a symbolic link, or replace a file, between the approval and the execution. The call goes back to the policy, or is refused when it would follow a link, in unit tests and through the runtime.

Result: all pass on Windows and Linux.

## 5. V4: process trees

Checks: `crates/mannie-tools/tests/process_tree.rs` builds trees from the shell and ends each by its normal end, by a timeout of 10 seconds and by Stop, then checks that no process of the tree is alive:

- Windows: a grandchild that `Start-Process` opens, a grandchild in a nested Job Object of its own, and a child whose request to break away from the job is refused.
- Linux: a background child, a grandchild that calls `setsid`, a double fork, `nohup`, and a process that ignores `SIGTERM`.

Result: all pass, and Stop ends a tree in less than a second. `tests/e2e/specs/tools.test.ts` measures Stop through the app: a running command ends, and its card says so, less than a second after Escape.

## 6. Evaluations

Checks: `crates/mannie-runtime/tests/evals.rs`, which `task tools:eval` and `.github/workflows/evals.yml` run. It has 19 tasks and 5 injection tasks, each in a temporary workspace with a check of what it left behind. The workspace is a project; one task starts as a general conversation that the model moves into the project that the owner's message names. A scripted owner allows changes inside the workspace and the commands each task names, and denies everything else. The report goes to `target/evals/<model>.json`, with each task's result and how many of the model's tool calls named their activity.

Result: with `MANNIE_EVAL_MODEL=oracle`, a scripted model that solves each task from the results it receives, all 24 tasks pass, so every check can be met and the harness works end to end. The oracle also showed that a check fails when a task goes wrong: two oracle scripts that misread results failed their tasks until they were fixed.

Not run yet: the evaluations with an OpenAI model and an open-weight model, which decide the exit criterion of at least 90% and every injection task.

## 7. ADR-0023 and the interface

- The tools feature registers a part renderer, an overlay, a command and a settings section, and edits chat only where the runtime's states show: the status line, the conversation list, the run failure `tools_unsupported`, the Tools check and the "Use tools" switch. On 2026-10-15 the owner replaced the workspace control by projects in the sidebar ([ADR-0023](../adr/0023-organize-conversations-into-projects.md)), so the conversation controls slot of ADR-0022 lost its only control and was removed with it. The registration tests compose the real feature list, with chat's two sidebar panels.
- At the owner's request of 2026-10-14, each tool call shows as one row that the model names in a few words, through the argument `activity` that every tool schema now requires, with its details on demand ([tool activity](../features/system-tools/tool-activity.md#tool-calls-in-the-reply), [tool calls](../features/system-tools/tool-calls.md#the-activity-of-a-call)). The runtime takes the activity out of the arguments before the tool reads them; the runtime tests show that a misleading activity changes nothing that authorization decides, and the injection corpus now names every hostile call as harmless. The oracle's 33 calls named their activity, and the report counts how many of a live model's calls do.
- `tests/e2e/specs/tools.test.ts` covers the acceptance of [tool activity](../features/system-tools/tool-activity.md#acceptance) in 14 scenarios: a project added with the folder dialog's stand-in, reads in its folder without a question through both APIs, a call shown as one row with its activity and its details, a request after 12 calls that shows in view without scrolling, an approval that leaves the focus in the message box, Allow once, Deny, "Allow for…" this conversation with its revocation, Save rule, Stop during a command, a reload while an approval waits, the full output in pages, the settings that allow changes, a profile without tools, and the killed app. Axe finds no WCAG 2.1 AA violation in the rows, their details, the approval, the dialogs, the new-conversation pane and the Tools settings, in the light and dark themes. The pseudo-locale spec covers a row that waits for approval, with its activity, its details and the menu of "Allow for…", and the Tools settings.
- [Projects](../features/chat/projects.md#acceptance): `tests/e2e/specs/projects.test.ts` covers in 11 scenarios a project added with the folder dialog's stand-in and added again, reads in its folder, the group and the project collapsed from the mouse and the keyboard and after a reload, five conversations and "Show more", a waiting request on a collapsed project, a move that the message names and one that asks, the grants for a project and for every conversation with their revocation in the settings, rows that keep their place while their buttons show on hover, renaming, and a removal that names its number of conversations and deletes them. Axe finds no violation in the sidebar, the new conversation's pane, the dialogs and the request of a move, in both themes, and the pseudo-locale spec covers the project's row, its menu and its dialogs. `crates/mannie-runtime/tests/projects.rs` covers the list's channel, folders that are refused, moves, the grants and a removal that stops a reply, and `crates/mannie-store` the upgrade of a database at schema 11 with workspaces into projects.

The validation found and fixed these defects:

1. The model read the tool calls of consecutive steps as one step of parallel calls, because the context merged calls until text came between them. Each part now records the model call that wrote it, and each model call reaches the model as an assistant message of its own. A context test covers it; the oracle of the evaluations found it.
2. The conversation list never learned that a reply started or stopped waiting for an approval, because the store published a new summary only when a run started or ended. A change of the run's activity in or out of waiting now publishes one.
3. The workspace control's accessible name did not contain its visible text (WCAG 2.5.3). The visible text became its name, with "Workspace:" for screen readers in a conversation's toolbar; projects later replaced the control.
4. A fallback for outcomes of unknown tools gave every transcript type two shapes in the generated bindings. The fallback now lives in the stored document, which the UI never receives.
5. On FreeBSD, which has no `run_command` until M4, the injection test would have failed: the runtime ends a call of a tool it does not know as not run, and the test accepted only denied and refused calls. The test now also accepts that ending there. The code that only commands use, in the tools and in the tests, is now built only for Windows and Linux.
6. Two V4 tests on Windows failed when they ran in parallel on the busy workstation: Windows PowerShell 5.1 had not started the grandchild within the 2-second timeout, so the test had no process to check. Run one at a time, they passed, and three parallel starts that each started a grandchild took 3.1 seconds. The timeout case now allows 10 seconds, and three parallel runs passed.
7. The owner found that the transcript stopped following a reply that made tool calls, so an approval request waited below the view. The transcript decided after each scroll event whether the owner was still at the bottom, and a scroll event that arrived after a row or a request had grown the content by more than 48 pixels counted as the owner scrolling away. A screenshot of the new rows showed a second cause: a message that leaves the message box shrinks it, and the browser moves the transcript up to its new bottom, which read the same way. Now only a move up that follows the owner's own wheel, touch, key or pointer ends the following. Unit tests cover both causes, and the end-to-end test of a request after 12 calls failed with the old decision and passes with the new one.
8. The owner found that a long reply let the view scroll away into empty space. The text that only screen readers read, such as a tool row's status, is positioned absolutely, and the element that scrolls the transcript was not positioned, so those elements escaped its scrolling and stretched the pane around it to the height of the conversation; the wheel then scrolled the whole view once the transcript reached its end. With 20 calls the pane could already scroll 857 pixels. That element is now positioned, and the end-to-end test of many calls checks that nothing around the transcript can scroll.
9. A collapsed project expanded again after a reload when one of its conversations was open: the sidebar expanded the open conversation's project whenever it learned that project, also when the list loaded. Only a move of the open conversation into a collapsed project expands it now, and the end-to-end tests cover both.
10. The owner found that a row of the sidebar shrank when the pointer reached it: the buttons that a row shows on hover took their room from the row, so its highlight and its title moved. A project's row lost 50 pixels. The buttons now lie over room that the row keeps free, and the row stays highlighted while the pointer is on them. The end-to-end test of the rows failed with the old layout and passes with the new one.

## 8. Performance

`task e2e:perf` measured the targets in the real app with a local provider, twice in a row against the same build, in report-only mode (`MANNIE_PERF_REPORT_ONLY=1`):

| Target                                                                     | M2, two runs       | M1       | Limit    |
| -------------------------------------------------------------------------- | ------------------ | -------- | -------- |
| NFR-PERF-01, a streamed piece appears (p95, from the provider to the page) | 94 ms, 96 ms       | 88 ms    | 100 ms   |
| NFR-PERF-02, input with 8 replies streaming (p95, input to painted frame)  | 17 ms, 17 ms       | 17 ms    | 50 ms    |
| NFR-PERF-03, a cold start shows a usable window                            | 1,409 ms, 1,462 ms | 1,267 ms | 2,000 ms |
| NFR-PERF-04, a conversation of 10,000 parts opens                          | 77 ms, 93 ms       | 59 ms    | 1,000 ms |
| A reply of 32 KB at 200 pieces a second, input (p95)                       | 25 ms, 37 ms       | 26 ms    | 50 ms    |

Every target is met. Three figures are higher than M1's, and a streamed piece now comes within 4 to 6 ms of its limit. M2 changed nothing that runs for each piece: the runtime still stores pieces every 50 ms, and the store's handling of a piece and the view's text path are as in M1. M1 and M2 were not measured side by side on an idle machine, so this record does not attribute the differences. A side-by-side run on the reference machine, which enforces the targets, is open.

The snapshot benchmark (`task bench`) now also opens a conversation of 5,000 turns that each made a tool call, 15,000 parts in all, against the same limit:

| Conversation                 | Snapshot of the latest 50 turns, p50 / p95 | Limit at p95 |
| ---------------------------- | ------------------------------------------ | ------------ |
| 10,000 parts of chat         | 0.3 ms / 0.5 ms                            | 100 ms       |
| 15,000 parts with tool calls | 0.5 ms / 0.6 ms                            | 100 ms       |

## 9. Manual checks

Not done yet, and needed before v0.2.0: the folder dialog of "+" on Windows 11 and on Linux desktops with the XDG desktop portal and with `zenity` only, since the end-to-end tests use the validation build's stand-in; a command that runs a `.ps1` file under the default execution policy; and a session with real models that reads, changes and runs commands in a workspace, with approvals, Stop, a reload and a restart.

## 10. Decisions for the owner

- Accept, change or supersede ADR-0013, and review the refinements recorded in ADR-0012 and ADR-0014.
- Review the defaults in the [system tools decisions](../features/system-tools/README.md#decisions) and the [chat decisions](../features/chat/README.md#decisions) of projects, and the text that the model sees, which `crates/mannie-tools/src/texts.rs` and `crates/mannie-runtime/src/texts.rs` hold.
- Provide the keys and endpoints of an OpenAI model and an open-weight model for the evaluations, with the variables and secrets that `.github/workflows/evals.yml` lists.
- Release v0.1.0 first; v0.2.0 follows once the open items above are done.
