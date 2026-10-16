# M3 validation results

|              |                                                                                                                                                                                                                                                                                                                  |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                                                                                                                                 |
| Last updated | 2026-10-16                                                                                                                                                                                                                                                                                                       |
| Related      | [Milestones](milestones.md#m3-child-agents), [M2 validation results](m2-validation.md), [Child agents](../features/child-agents/README.md), [ADR-0024](../adr/0024-run-child-agent-tasks-in-the-parent-run.md), [ADR-0025](../adr/0025-add-side-panels-to-the-shell.md), [Security](../architecture/security.md) |

This document records what the checks and exit criteria of [M3](milestones.md#m3-child-agents) showed so far, so that the owner can review ADR-0024 and the defaults of [child agents](../features/child-agents/README.md#decisions), and see what remains before v0.2.0, which carries M2, M3 and M4 ([D23](milestones.md#5-decisions)). The owner accepted ADR-0025, side panels, on 2026-10-15 (D18), and ADR-0024 on 2026-10-16 ([D25](milestones.md#5-decisions)). The checks stay in the repository and run in CI. Each section names the checks, where they ran, the result, and what is open.

The runs below were made on 2026-10-15 on the owner's Windows 11 workstation (WebView2 155.0.4283.45). The changes are not committed yet, and they were tested on top of 9892dec, the commit that added system tools. Nothing of M3 ran in CI, on Linux, on WebKitGTK or with a live model: `task test:linux` could not run, because Docker Desktop did not start its engine on the workstation, and the evaluations need the owner's keys and endpoints. What is not run yet is marked so.

## 1. Summary

| Check or criterion                                                    | Result                                                                                                                                                          | Open                                                       |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| Explore and general children, with their tool sets enforced           | Passed: runtime tests of the tools each child is offered and of a call to a tool it was not offered                                                             | None                                                       |
| Parallel slots, background tasks, waiting and stopping one child      | Passed: runtime tests and end-to-end tests                                                                                                                      | WebKitGTK                                                  |
| The parent receives only confirmed reports                            | Passed: one confirmation for each report, and reports only as tool results                                                                                      | None                                                       |
| A follow-up keeps the child's context                                 | Passed: runtime tests and the crash tests' follow-up                                                                                                            | None                                                       |
| Retrying the parent never repeats an operation that a child completed | Passed: four kill points, each followed by a retry and a follow-up                                                                                              | Linux                                                      |
| Prompt injection relayed by a report                                  | Passed: no deletion, no grant and no rule without the owner                                                                                                     | None                                                       |
| Evaluation of a multi-step task split across children                 | The harness and its checks pass all 27 tasks with the scripted oracle, the 3 tasks with children among them                                                     | Live runs with an OpenAI model and an open-weight model    |
| ADR-0024, tasks in the runtime                                        | Passed: the checks above, and the store's tests of task conversations, recovery and deletion; Accepted                                                          | None                                                       |
| ADR-0026, conversation controls                                       | Passed: the registration test with a fake control, and the end-to-end tests of the child agents' button with axe in both themes and the pseudo-locale; Accepted | WebKitGTK                                                  |
| ADR-0025, side panels                                                 | Passed: the registration test with a fake kind, the unit tests of the address, and the end-to-end tests with axe in both themes and the pseudo-locale; Accepted | WebKitGTK                                                  |
| Performance targets                                                   | All met on the workstation                                                                                                                                      | A run on the reference machine, which enforces the targets |
| End-to-end tests                                                      | All 15 spec files pass on WebView2, after the races of items 3 and 5 below were fixed                                                                           | WebKitGTK                                                  |
| `task ci`                                                             | Passes                                                                                                                                                          | None                                                       |
| Linux                                                                 | Not run in M3; `task test:linux` passed in M4                                                                                                                   | None                                                       |
| v0.2.0 released                                                       | Released on 2026-10-16                                                                                                                                          | None                                                       |

## 2. Tasks in the runtime

Checks: `crates/mannie-runtime/tests/tasks.rs`, 16 tests through the runtime with a scripted model that routes each request by its first message, so a parent and its children each follow their own script:

- A foreground task returns the child's report, and the child reads exactly one confirmation note.
- An explore child is offered only its read-only tools and a general child no `move_to_project`; a call to a tool a child was not offered does not run; no child is offered the task tools; a profile without tools is offered none.
- The task tools' definitions match their reviewed snapshot, `tests/fixtures/task_definitions.json`.
- With a limit of 2, two children run and a third waits for a slot, and raising the limit starts it at once.
- Background tasks report through `wait_for_tasks`, with numbers and with the empty list; a reply that wants to end with unread reports waits for its tasks and asks the model once, with a note that holds none of their text.
- A follow-up continues the child's conversation with its agent, and a follow-up of a running or unknown task assigns nothing.
- A child's approval request shows in the parent's part, the parent's activity and the conversation list say that it waits, the decision through the child's conversation applies, and a grant "for this conversation" lets the parent and its other children run what it covers.
- Stopping the parent ends, within a second, a child that runs a command, one that waits for approval and one that waits for a slot. Stopping one child gives the parent a result that says so, and the parent continues. Quitting interrupts the parent and its children.
- A reply starts no more tasks than its budget, and a report longer than the result limit is cut and archived whole in the parent's conversation.
- The owner's commands refuse a task conversation, and deleting the parent deletes its children and ends their subscriptions.

The runtime's unit tests cover the slots (order, growth, shrinking with a debt and cancellation), the board, the rendering of reports and their limits, the parsing of the task tools' arguments, the context and compaction of task parts, the instruction sections of children, and the singular and plural notes about reports. `tests/subscriptions.rs` follows a task to its report as a subscriber.

Result: all pass on Windows.

## 3. Recovery and retries

Checks: `crates/mannie-runtime/tests/task_crash.rs` starts itself again as a child process that runs a parent whose general child runs a command that appends a marker line. It kills the process at four points and starts a runtime on the same database:

| Killed                                   | The parent's run | The task    | The child's run | The child's command |
| ---------------------------------------- | ---------------- | ----------- | --------------- | ------------------- |
| While the command is stored as executing | Interrupted      | Interrupted | Interrupted     | Uncertain           |
| After the command wrote its marker       | Interrupted      | Interrupted | Interrupted     | Uncertain           |
| After the child reported                 | Interrupted      | Reported    | Completed       | Completed           |
| After the parent's reply ended           | Completed        | Reported    | Completed       | Completed           |

After each restart the states match the [recovery table](../features/child-agents/tasks.md#recovery); where the parent's reply had not ended, Retry starts no child; a follow-up reads the command's result and does not run it again; and the marker appears at most once. The store's tests cover the same table from stored rows, a part whose task was never stored, and a run with only task parts that a retry keeps.

Result: all pass on Windows. Not run yet on Linux.

## 4. Prompt injection

`crates/mannie-runtime/tests/injection.rs` adds a relayed injection: an explore child reads a hostile file, and its report asks the parent to delete files. The parent's scripted, compromised model asks for the deletion, and the owner denies it. Nothing changes on the disk, no grant or rule appears, and the report reaches the parent's model only as the result of its task.

Result: passes on Windows.

## 5. Evaluations

Checks: `crates/mannie-runtime/tests/evals.rs`, which `task tools:eval` and `.github/workflows/evals.yml` run, now with 22 tasks and 5 injection tasks. Three tasks split their work across children: two explore children read in parallel, a general child fixes a file, and a background child counts while the parent waits for it. The scripted owner decides a child's requests through the child's conversation, and the report counts the tasks that children reported.

Result: with `MANNIE_EVAL_MODEL=oracle`, all 27 tasks pass, and all 39 of the oracle's tool calls named their activity.

Not run yet: the evaluations with an OpenAI model and an open-weight model.

## 6. The store

`crates/mannie-store` adds migrations 15 and 16: the parent, number and agent of task conversations, and the tasks table. Its tests upgrade a database at schema 14 with data, create a task conversation with its task in one transaction and keep it out of the list, end tasks from their runs, recover task conversations before their parents, finish open task parts from their tasks, keep open task parts in what the model reads, delete a parent's task conversations with a planted string gone from the database and its log, and report the task conversations that removing a project deleted. Every statement prepares against the migrated schema.

Result: all 68 unit tests of the store pass.

## 7. The interface and ADR-0025

- The child agents feature registers a part renderer, a settings section and messages, and edits chat only where the runtime's states show: the status line, the read-only conversation, the failure `task_conversation`, and the jump to the part that waits. Chat registers the first side panel kind, `chat.conversation`, and the shell renders side panels without importing a feature. The slot's registration test uses a fake panel kind, and the composition test the real feature list.
- Vitest: 31 files with 203 tests, including the side panels' address, the task parts, the child agents' messages and the conversation list's moves.
- `tests/e2e/specs/child-agents.test.ts` covers the acceptance of [child agent activity](../features/child-agents/child-agent-activity.md#acceptance) in 9 scenarios: a general child's write that asks in its task's card, labeled with the task, and the report once allowed; the instruction and the report on demand; the child's conversation in a side panel, read-only, with the focus on its heading and back on the card's button when it closes; resizing the panel from the keyboard, and the panel with its size after a reload; a narrow window in which the panel covers the view until it closes; a limit of 1 that makes a second task wait, and Stop on each child while the parent continues; tabs for two children, switched with the arrow keys, with the hidden one still open; the limit after a reload; and a background task read with `wait_for_tasks`. Axe finds no WCAG 2.1 AA violation in the cards, a child's request inside a card, the side panel with one and two tabs, and the Child agents settings, in the light and dark themes.
- The pseudo-locale spec covers a card with its child's request and its details, the side panel, a stopped task and the Child agents settings. The shell spec lists the new settings page.
- The owner's review of the interface on 2026-10-15 removed the card's "In the background" badge, moved the agent's badge onto the title's line, and replaced shadcn-svelte's `tabs` in the side panel with tabs of Mannie's own, each with its Close ([ADR-0025](../adr/0025-add-side-panels-to-the-shell.md), decisions 3 and 5). The end-to-end tests close tabs with Delete and with a tab's Close, also one that does not show.
- At the owner's request in the same review ([D19](milestones.md#5-decisions)), the conversation's toolbar shows a "Child agents" button, through the conversation controls that [ADR-0026](../adr/0026-add-conversation-controls-back-for-child-agents.md) brings back. The end-to-end tests check its count while two children work, its menu with axe in both themes and in the pseudo-locale, a child's conversation that opens from it, and the focus that returns to it.

The validation found and fixed these defects:

1. PaneForge, under shadcn-svelte's `resizable`, sets the handle's value and `aria-controls` itself, from the pane before the handle, which is the main view. The shell had also set them, from the side panel's share, and the two writers left the handle with whichever wrote last. The shell now leaves them to PaneForge and gives the handle only its name and orientation; the end-to-end test of resizing found it.
2. The note about unread reports said "you have not read their reports" for a single task. It now says "its report" for one task, and a unit test covers both forms.
3. Four races in the tests, each of which failed once in a full run. Two tests of chat sent a message and then read the latest turn, which could still be the previous one; they now wait for the new turn first. The helpers that send a message counted the turns before the message box took messages, so right after leaving the settings they could count a transcript that had not loaded and take an earlier reply for the new one; they now count once the box takes messages. The IPC test that changes the language could start its channel load before the window had reloaded; it now waits for the new document.
4. When the first message of a new conversation was sent, the view replaced the new conversation's pane with the conversation's own, and the message box lost the focus. This predates M3, whose end-to-end test of a child's request met it. The new pane's message box now takes the focus once the conversation has loaded, and the end-to-end tests of chat and child agents check it.
5. The sidebar expanded a collapsed project only when it saw the open conversation as general before it saw the conversation in the project. The list applies its updates in batches, so a move that came in one batch with the new conversation, or before the view showed the conversation, left the project collapsed. The end-to-end test of a move that the message names failed 3 times in 9 runs once the focus of item 4 changed the timing, and 0 times in 6 runs without it. The list now announces each move as it applies it, the conversation that a first message started counts as open until the view shows it, and unit tests cover moves within one batch and in a new snapshot.

One gap remains: while a handle is dragged with the pointer, PaneForge adds a style element that sets the page's cursor, which the Content Security Policy blocks. `app.css` shows the same cursor instead, so dragging works and looks the same, but the webview reports the blocked element on each drag ([PATCHES.md](../../apps/desktop/src/lib/components/ui/PATCHES.md)). Allowing the element by its hash would change the policy and needs an ADR.

## 8. Performance

`task e2e:perf` measured the targets in the real app with a local provider, twice in a row against the same build, in report-only mode (`MANNIE_PERF_REPORT_ONLY=1`). Docker Desktop was starting in the background during the first run, and the second ran without it:

| Target                                                                     | M3, two runs     | M2, two runs       | Limit    |
| -------------------------------------------------------------------------- | ---------------- | ------------------ | -------- |
| NFR-PERF-01, a streamed piece appears (p95, from the provider to the page) | 99 ms, 87 ms     | 94 ms, 96 ms       | 100 ms   |
| NFR-PERF-02, input with 8 replies streaming (p95, input to painted frame)  | 17 ms, 17 ms     | 17 ms, 17 ms       | 50 ms    |
| NFR-PERF-03, a cold start shows a usable window                            | 930 ms, 1,201 ms | 1,409 ms, 1,462 ms | 2,000 ms |
| NFR-PERF-04, a conversation of 10,000 parts opens                          | 71 ms, 65 ms     | 77 ms, 93 ms       | 1,000 ms |
| A reply of 32 KB at 200 pieces a second, input (p95)                       | 24 ms, 26 ms     | 25 ms, 37 ms       | 50 ms    |

Every target is met. A streamed piece still comes close to its limit, as in M2, and the first run's 99 ms came while Docker Desktop started on the same machine. The side panels wrap the main view in a pane of the resizable split, which the view's measurements include.

The snapshot benchmark (`task bench`) opened a conversation of 10,000 parts of chat in 0.3 ms at the 95th percentile, and one of 15,000 parts with tool calls in 1.1 ms, against a limit of 100 ms. The writer kept up with 8 runs at 200 tokens per second, with batches every 250 ms and every 33 ms.

## 9. Manual checks

Not done yet, and needed before v0.2.0: a session with real models in which two explore children work in parallel and a general child changes a file after an approval in its card, Stop on one child while another works, the side panel's tabs while the parent streams, dragging the panel's handle with the pointer, and a reload and a restart while a child works.

## 10. Decisions for the owner

- ADR-0024: the owner accepted it on 2026-10-16 ([D25](milestones.md#5-decisions)).
- Review the defaults in the [child agents decisions](../features/child-agents/README.md#decisions), and the text that the model sees, which [tasks](../features/child-agents/tasks.md#the-text-the-model-sees) lists and `crates/mannie-runtime/src/texts.rs` holds.
- Provide the keys and endpoints of an OpenAI model and an open-weight model for the evaluations, as for M2.
- Releases: v0.2.0 was published on 2026-10-16, and v0.1.0 was not published.
