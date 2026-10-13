# Workers

|              |                                                                                                                                                                                                                                                                                                         |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Accepted                                                                                                                                                                                                                                                                                                |
| Milestone    | M4                                                                                                                                                                                                                                                                                                      |
| Last updated | 2026-10-13                                                                                                                                                                                                                                                                                              |
| Related      | [ADR-0015](../../adr/0015-run-an-agent-runtime-on-every-machine.md), [ADR-0016](../../adr/0016-link-machines-over-pinned-mtls-grpc.md), [ADR-0010](../../adr/0010-keep-provider-credentials-on-the-master.md), [Orchestration](../../architecture/orchestration.md), [Child agents](../child-agents.md) |

## Intent

The owner has several machines, and each host may serve a purpose. A worker runs on each host as an autonomous agent that knows that host. In the owner's words, the master cares about the user's request and workers care about the master's request: the owner talks to the master, the master hands a task to the right worker, and the worker works out how to do it on its host and reports back. The master does not need to know how a FreeBSD host works or which commands to run there.

## Behavior

- Installing: the owner installs the worker on a Windows, Linux or FreeBSD host and runs it as a service (a Windows service, a systemd unit or an rc.d service), following the documentation.
- Pairing: the worker prints its address and a one-time code. The owner adds the worker in the master with both, and from then on the two trust only each other's keys.
- Describing the host: on the host, the owner writes the worker's name, its purpose and its host instructions. The worker discovers its host facts (operating system, version, architecture, shells and tools) and reports them to the master.
- Policy ceiling: also on the host, the owner chooses what the worker may do: read-only, ask, or autonomous within the allowed folders and rules. A task can narrow the ceiling but never widen it.
- Delegating: the conversation agent sees each worker's name, purpose, status and host facts, and delegates tasks to the worker that fits. Tasks for different workers run in parallel, a task can run in the background, and a follow-up can continue an earlier task's conversation on the same worker.
- Working: the worker agent runs its own loop with its host's tools and knowledge. In ask mode, its approval requests appear in the master's UI, and they wait while the master is offline.
- Model access: for each task, the master leases the credential of the task's provider profile to the worker, in memory only. The worker calls the provider itself, so a task can finish while the master is offline.
- Watching: the owner can open any task conversation in the master, live or afterwards. The master shows each worker's status.
- Surviving failures: when the link drops, the worker keeps working, and the master reconnects and catches up. A retried step never starts a second task.
- Updating: the master pushes signed worker builds. The worker verifies a build, installs it, restarts, and rolls back if the new build fails its self-check. The master works with workers one minor version behind.
- Not included: the master running commands on a worker's host (never), workers delegating to other workers, the owner chatting with a worker directly, credentials stored on a worker for unattended work, and moving files between machines. Each of these, except the first, would be a feature of its own.

## Fit

- Extension points: link services (introduced by this feature), the task and report contract from [child agents](../child-agents.md), the worker agent definition, instruction sections for the host instructions and host facts, the tool registry on the worker (the same tools as [system tools](../system-tools.md)), the store on the worker, UI shell slots for the list of workers, pairing and the delegated task card, and the settings section for workers.
- Follows [ADR-0008](../../adr/0008-one-workspace-two-binaries.md), [ADR-0010](../../adr/0010-keep-provider-credentials-on-the-master.md), [ADR-0015](../../adr/0015-run-an-agent-runtime-on-every-machine.md), [ADR-0016](../../adr/0016-link-machines-over-pinned-mtls-grpc.md) and [ADR-0018](../../adr/0018-release-from-tagged-builds.md).
- Must not change: [system tools](../system-tools.md) and [child agents](../child-agents.md) on the master behave the same whether or not workers are paired.

## Open questions

- Where a worker keeps its data, its configuration and its host instructions on each operating system, by default.
- The worker's default port.

## Decisions

None yet.

## Parts

This feature is big enough to be a folder. The part documents are written when M4 starts:

- the worker service and its setup on each operating system;
- pairing and the link;
- host purpose, host instructions and the policy ceiling;
- delegation and live task conversations;
- credential leases;
- compatibility and signed updates.

## Acceptance

- Workers on Windows, Linux and FreeBSD pass the system tools suite on their own hosts, driven by delegated tasks.
- V6 passes: pairing refuses wrong, expired and reused codes and a substituted key; a link lost during an operation causes no duplicate side effect; a worker one minor version behind still works; a signed update installs and rolls back; a leased credential never reaches the worker's disk or logs.
- A worker's ceiling holds even against instructions from the master. Approval requests reach the owner and wait while the master is offline.
- The documented service setups pass smoke tests on each operating system.
