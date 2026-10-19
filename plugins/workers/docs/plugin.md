# Workers as a plugin

|              |                                                                                                                                                                                                                                                                        |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                               |
| Milestone    | M5                                                                                                                                                                                                                                                                     |
| Last updated | 2026-10-16                                                                                                                                                                                                                                                             |
| Part of      | [Workers](README.md)                                                                                                                                                                                                                                                   |
| Related      | [ADR-0030](../../../docs/adr/0030-split-mannie-into-a-core-and-plugins.md), [ADR-0031](../../../docs/adr/0031-move-workers-into-the-first-plugin.md), [Plugins](../../../docs/features/plugins.md), [Delegation](delegation.md), [Worker activity](worker-activity.md) |

## Intent

Not every owner works across machines, so the worker system becomes the first plugin ([decision D26](../../../docs/roadmap/milestones.md#5-decisions)). An owner who needs workers turns Workers on in the Plugins settings, and an owner who does not never sees them. While it is on, workers behave as they did in 0.2.0.

## Behavior

- Workers is listed on the Plugins page with a switch. Its description says that it lets the conversation agent give tasks to Mannie's workers on other Windows, Linux and FreeBSD computers.
- While Workers is on, everything in the [workers feature](README.md) works as in 0.2.0: pairing, the Workers group in the sidebar, the Workers settings, "Pair a worker…" in the command palette, `delegate_task`, the cards of worker tasks, their conversations in the side panel and their approval requests.
- While Workers is off, Mannie opens no connection to any worker, and the conversation agent is not offered `delegate_task` or the list of workers. The Workers group, the Workers settings and "Pair a worker…" are gone. Cards of earlier worker tasks stay in their conversations with the worker's name, the task's title, its last known state and its report when it had one. "Open conversation", "Stop" and the answers to a worker's approval say that Workers is off, with a link to the Plugins page.
- Turning Workers off while worker tasks run asks first: the tasks keep running on their workers, their approval requests wait there, and Mannie follows them again when Workers is back on. A reply that is running when Workers goes off keeps going: Mannie stops following its worker tasks, as when Mannie quits, and the conversation agent reads that each task may still be running on its worker. Calls of `delegate_task`, and of `follow_up_task` and `wait_for_tasks` for a worker task, then do not run, and the conversation agent reads that Workers is off.
- Turning Workers on connects to the paired workers and catches up as after a restart: Mannie compares its worker tasks with each worker's, a task that the worker does not know ends as interrupted, and `wait_for_tasks` reads the report of a task that ran on while Workers was off.
- Deleting a conversation while Workers is off deletes it in Mannie at once, and the deletion of its worker tasks' conversations reaches each worker once Workers is on and the worker is connected.
- Workers keeps its paired workers, their names and its records while it is off. Turning it on needs no new pairing.
- When Workers cannot start, for example because it cannot read its list of paired workers, the Plugins page shows it as failed, with what failed and "Try again", and the rest of Mannie works. In 0.2.0, that failure kept Mannie from starting.
- After the update from 0.2.0, Workers is on when at least one worker is paired, and off otherwise. A new installation starts with Workers off.
- The worker service itself does not change. A worker of 0.2.0 works with Mannie 0.3.0, and the owner updates it with `mannie-worker update` as before.

## Fit

- Follows [ADR-0030](../../../docs/adr/0030-split-mannie-into-a-core-and-plugins.md) and [ADR-0031](../../../docs/adr/0031-move-workers-into-the-first-plugin.md).
- Extension points: the plugin definition and its start and stop; task executors, for `delegate_task` and worker tasks; instruction sections, for the list of workers; the UI's task kinds, for a worker task's card, footer, status and messages; UI shell slots, for the sidebar group, the dialogs, the command and the settings page, which show only while Workers is on; IPC commands and a channel through the host's dispatch, which refuses them with `plugin_off` while Workers is off; the store, for the plugin's tables; messages under `workers_`.
- The worker service keeps using the core's assigned tasks, ceiling, credential vault and scoped redaction, which stay in the core.
- Must not change: what workers do while the plugin is on, the link's contracts, the worker's configuration and folders, and what child agents and system tools do with or without Workers.

## Open questions

None.

## Decisions

- 2026-10-16: the owner decided that the worker system becomes the first plugin and that its code lives in `plugins/workers/` ([D26](../../../docs/roadmap/milestones.md#5-decisions)).
- 2026-10-16: the owner approved ADR-0030, ADR-0031 and these defaults, which were chosen when shaping, and building started ([D28](../../../docs/roadmap/milestones.md#5-decisions)):
  - Turning Workers off lets running worker tasks go and cancels none of them.
  - After the update from 0.2.0, Workers is on when at least one worker is paired.
  - The crates, the service and the link's packages keep their names.

## Design

### The plugin

`mannie-workers` defines the plugin `workers` ([ADR-0031](../../../docs/adr/0031-move-workers-into-the-first-plugin.md)):

- Its store module `workers` holds the paired workers and carries over the migrations 19, 20 and 22 that 0.2.0 numbered in its single list. Its migration 28 moves the deletions that waited for a worker into the core's outbox. Its code runs the six statements that the module lists through the SDK's service `store` ([ADR-0034](../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), decision 7), and a pin that another paired worker has fails the pairing with `already_added`.
- Its default is on when the store holds a paired worker, and off otherwise. The host keeps that outcome as the owner's choice at the first start of 0.3.0.
- Its manifest, `plugins/workers/plugin.toml`, declares four services of the plugin SDK ([ADR-0034](../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), decision 5): the executor host and the credentials, both bound to the executor kind `workers`, its module of the store, and its entries of the credential store, where the master's key of the link stays under the name that 0.2.0 gave it, `link/master-key`. Its start runs the hub, which connects to every paired worker and compares their tasks with the master's, and returns its executor, whose kind is `workers` and whose tool is `delegate_task`, with the section that lists the workers. A host without one of the services fails the start with `service_unavailable` before any of Workers' code runs, and a store that cannot be read fails it with `storage_failed`.
- Validation builds try a worker again after two seconds at most, which the host profile tells Workers, and every other build after a minute at most.
- Its work in progress is its tasks that have not ended, as one item with the code `active_tasks`, which the question before turning it off words.
- Its stop comes after the host took its executor out of the runtime, so each run has let its worker tasks go. It ends the followers of the tasks and every link, and cancels nothing.

### Contracts

- `delegate_task`, its schema, its texts for the model and the link's contracts are unchanged. The executor refuses a call with the codes `worker_offline`, `worker_unknown`, `worker_incompatible`, `profile_not_reachable` and `still_running`, with the worker's name as the target.
- The host refuses the workers' IPC commands with `plugin_off` while Workers is off, and ends the channel of the workers' list with that reason ([ADR-0034](../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), decision 8).

### Data

- A worker task's row keeps its worker as the executor's target and the version of its last state as the executor's version, and the executor's document holds the leased profile, whether the worker stored the task, and a queued cancel. The worker's old columns stay unused until the release after ([ADR-0031](../../../docs/adr/0031-move-workers-into-the-first-plugin.md), refinement in M5).
- The plugin's tables stay while Workers is off, and `workers_deletions` stays empty until the release after drops it.

### Interface

- The plugin's UI registration adds the Workers group of the sidebar, the dialogs, "Pair a worker…" and the Workers settings, and the task kind `workers`, with the server icon and the messages of a worker task's card, footer, status line and refusals under `workers_task_` and `workers_refusal_`.
- The list of workers follows the core only while Workers is on, and the dialogs close as Workers turns off.
- Messages: `workers_plugin_name`, `workers_plugin_description`, `workers_plugin_off_question` and `workers_plugin_failure_storage_failed`, with the texts of other features that named workers moved under `workers_`. The host's refusals of its commands use the core's messages under `plugins_host_`.

### Threat notes

- The plugin reaches the credential store only in its own entries and borrows a provider credential from the core for each task, through the vault as before. The core lends only for an active Workers task whose run uses the profile, so a document that names another profile, or a task that ended, gets nothing ([security](../../../docs/architecture/security.md#5-threats-and-mitigations), T-30).
- Turning Workers off closes every link, so no connection to a worker stays open while the owner sees Workers as off (T-31).

## Acceptance

- The M4 suites pass from the plugin's folder: delegation through the in-process executor (`plugins/workers/crates/mannie-workers/tests/delegation.rs`, `delegation_crash.rs`), the link over loopback (`linked_delegation.rs`, `link_drop.rs`, `pairing.rs`, `delegated_conformance.rs`), the contracts' fixtures (`plugins/workers/crates/mannie-link/tests/compatibility.rs`), the worker service (`plugins/workers/worker/`), and the end-to-end tests with a real worker (`plugins/workers/tests/e2e/workers.test.ts`).
- Without a paired worker, Workers starts off and its commands say so; with one, it starts on, and turning it off ends its parts; its manifest declares its four services and its legacy entry, and a start on a host without a service fails and Mannie goes on (`plugins/workers/crates/mannie-workers/tests/plugin.rs`).
- Turning the plugin off lets a running task go and a later wait takes it up, a submission that the stop cut short lets the task go, and a deletion waits in the outbox for the executor (`crates/mannie-runtime/tests/executors.rs`).
- A database of 0.2.0 names the executor of its worker tasks in every state, keeps the texts of refused calls, moves the waiting deletions into the outbox and keeps the owners' names of workers (`crates/mannie-store/src/store/tests.rs`, `plugins/workers/crates/mannie-workers/tests/store.rs`).
