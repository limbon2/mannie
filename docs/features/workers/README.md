# Workers

|              |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Milestone    | M4                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Last updated | 2026-10-16                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Related      | [ADR-0015](../../adr/0015-run-an-agent-runtime-on-every-machine.md), [ADR-0016](../../adr/0016-link-machines-over-pinned-mtls-grpc.md), [ADR-0010](../../adr/0010-keep-provider-credentials-on-the-master.md), [ADR-0027](../../adr/0027-delegate-tasks-to-workers-through-a-runtime-port.md), [ADR-0028](../../adr/0028-run-assigned-tasks-under-the-host-ceiling.md), [Orchestration](../../architecture/orchestration.md), [Child agents](../child-agents/README.md), [Worker setup](../../guides/worker-setup.md) |

## Intent

The owner has several machines, and each host may serve a purpose. A worker runs on each host as an autonomous agent that knows that host. In the owner's words, the master cares about the user's request and workers care about the master's request: the owner talks to the master, the master hands a task to the right worker, and the worker works out how to do it on its host and reports back. The master does not need to know how a FreeBSD host works or which commands to run there.

## Behavior

- Installing: the owner installs the worker on a Windows, Linux or FreeBSD host and runs it as a service (a Windows service, a systemd unit or an rc.d service), following the [setup guide](../../guides/worker-setup.md).
- Pairing: the worker prints its address and a one-time code. The owner adds the worker in the master with both, and from then on the two trust only each other's keys.
- Describing the host: on the host, the owner writes the worker's name, its purpose and its host instructions. The worker discovers its host facts (operating system, version, architecture, shells and tools) and reports them to the master.
- Naming: in the master, the owner can give a worker another name when pairing it or later with "Rename…". Mannie and the conversation agent then use that name in place of the one in the worker's configuration, and an empty name goes back to it.
- Policy ceiling: also on the host, the owner chooses what the worker may do: read-only, ask, or autonomous within the allowed folders and rules. Outside the allowed folders, reads and changes ask the owner through the master, except in read-only mode, which refuses every change and command. A task can narrow the ceiling but never widen it.
- Delegating: the conversation agent sees each worker's name, purpose, status and host facts, and delegates tasks to the worker that fits. Tasks for different workers run in parallel, a task can run in the background, and a follow-up can continue an earlier task's conversation on the same worker.
- Working: the worker agent runs its own loop with its host's tools and knowledge. Its approval requests appear in the master's UI, in the card of the task, and they wait while the master is offline.
- Model access: for each task, the master leases the credential of the task's provider profile to the worker, in memory only. The worker calls the provider itself, so a task can finish while the master is offline.
- Watching: the owner can open any task conversation in the master, live or afterwards, while the worker is reachable. The master shows each worker's status.
- Surviving failures: when the link drops, the worker keeps working, and the master reconnects and catches up. A retried step never starts a second task. Quitting the master leaves worker tasks running, and stopping a reply cancels them.
- Updating: the owner downloads a new release on the host and applies it with `mannie-worker update`, which checks its signature, keeps the previous program and restores it when the new one does not start. The master works with workers one minor version behind and says when a worker needs an update.
- Not included: the master running commands on a worker's host (never), workers delegating to other workers, child agents on workers, the owner chatting with a worker directly, credentials stored on a worker for unattended work, updates pushed by the master, and moving files between machines. Each of these, except the first, would be a feature of its own.

## Fit

- Extension points: link services (introduced by this feature); the task and report contract of [child agents](../child-agents/README.md), which a delegation port of the runtime extends to workers ([ADR-0027](../../adr/0027-delegate-tasks-to-workers-through-a-runtime-port.md)); the worker agent definition and tasks that another machine assigns ([ADR-0028](../../adr/0028-run-assigned-tasks-under-the-host-ceiling.md)); instruction sections for the host and for the workers; the tool registry on the worker (the same tools as [system tools](../system-tools/README.md)) under a policy ceiling; the store on both machines; UI shell slots for the list of workers, the pairing dialog and the settings section of workers; IPC commands and a channel for workers; messages under the `workers_` prefix.
- Follows [ADR-0008](../../adr/0008-one-workspace-two-binaries.md), [ADR-0010](../../adr/0010-keep-provider-credentials-on-the-master.md), [ADR-0015](../../adr/0015-run-an-agent-runtime-on-every-machine.md), [ADR-0016](../../adr/0016-link-machines-over-pinned-mtls-grpc.md), [ADR-0018](../../adr/0018-release-from-tagged-builds.md), [ADR-0027](../../adr/0027-delegate-tasks-to-workers-through-a-runtime-port.md) and [ADR-0028](../../adr/0028-run-assigned-tasks-under-the-host-ceiling.md).
- Changes in other features, which the ADRs name: the card of a task and the toolbar's button of child agents; chat's side panel, read-only footer, status line and failures for a worker's task conversation; the host, decisions, reasons and refusals of the ceiling in the tools' approvals and tool calls.
- Must not change: [system tools](../system-tools/README.md) and [child agents](../child-agents/README.md) on the master behave the same whether or not workers are paired.

## Open questions

None.

## Decisions

- 2026-10-15: the owner decided:
  - Worker updates are applied on the host with `mannie-worker update`, and the master pushes no builds ([D20](../../roadmap/milestones.md#5-decisions)).
  - Outside the allowed folders, a worker's reads and changes ask the owner through the master ([D21](../../roadmap/milestones.md#5-decisions)).
  - Workers listen on port 7445 by default, and keep their configuration, host instructions and data where each system expects them: `/etc/mannie-worker` and `/var/lib/mannie-worker` on Linux, `/usr/local/etc/mannie-worker` and `/var/db/mannie-worker` on FreeBSD, and `%ProgramData%\Mannie\worker` with a `data` subfolder on Windows. The configuration is owned by root or Administrators.
- 2026-10-16: the owner decided that a worker can be named in the master, when pairing it and with "Rename…" in its menu and in the Workers settings, and that the name replaces the configured one everywhere in Mannie, the conversation agent's list of workers and `delegate_task` included ([worker activity](worker-activity.md#naming-a-worker)).
- 2026-10-15: the defaults below were chosen when building started. Each one waits for the owner's review, and the part documents give the details.
  - A worker's approval requests offer Allow once and Deny.
  - The master works with workers of its own minor version and one minor version behind.
  - Stop cancels a reply's worker tasks and waits up to 10 seconds for the worker; quitting the master leaves worker tasks running, and `wait_for_tasks` takes up a task that runs on.
  - A delegated task uses the provider profile of the reply that delegated it, without the owner's global instructions; a profile with a loopback address is refused for a worker on another machine.
  - Deleting a conversation deletes the conversations of its worker tasks on the workers, queued while a worker is offline.
  - `service install` copies the program into a folder of root or Administrators, and the worker refuses to start when its own account can change its configuration.
  - The pairing code has 20 characters of Crockford base32, valid 10 minutes, for 5 attempts.
  - The link uses raw public keys (RFC 7250) in place of certificates.
  - A worker runs 4 tasks at once, from 1 to 16; the worker agent has 100 rounds and 250 tool calls; `delegate_task` counts against the reply's 20 tasks.
  - The link reconnects with a jittered delay from 1 to 60 seconds.

## Parts

- [The worker service](service.md): the configuration, the command line, the folders, the service of each system, and hardening.
- [Pairing and the link](pairing-and-link.md): keys, pins, pairing, the services and their feature levels, the connection and its statuses.
- [Host and ceiling](host-and-ceiling.md): host facts, purpose and host instructions, the worker agent and the text it reads, the policy ceiling and approvals.
- [Delegation](delegation.md): `delegate_task`, worker tasks and their conversations on the master, following tasks, stopping, recovery and deletion.
- [Credential leases](credential-leases.md): how a task gets its provider's credential, and when the worker drops it.
- [Compatibility and updates](compatibility-and-updates.md): versions and feature levels, contract checks, and `mannie-worker update`.
- [Worker activity](worker-activity.md): workers in the master's interface, the pairing dialog, worker tasks in a conversation and their states.

## Acceptance

- Workers on Windows, Linux and FreeBSD pass the system tools suite on their own hosts, driven by delegated tasks.
- V6 passes: pairing refuses wrong, expired and reused codes and a substituted key; a link lost during an operation causes no duplicate side effect; a worker one minor version behind still works; `mannie-worker update` installs a signed build and rolls back one that does not start; a leased credential never reaches the worker's disk or logs.
- A worker's ceiling holds even against instructions from the master. Approval requests reach the owner and wait while the master is offline.
- The documented service setups pass smoke tests on each operating system.
