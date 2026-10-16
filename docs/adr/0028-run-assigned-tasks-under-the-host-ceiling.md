# ADR-0028: Run the tasks that the master assigns under the worker's policy ceiling

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Proposed                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Date     | 2026-10-15                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Related  | [ADR-0010](0010-keep-provider-credentials-on-the-master.md), [ADR-0013](0013-journal-every-side-effect.md), [ADR-0015](0015-run-an-agent-runtime-on-every-machine.md), [ADR-0024](0024-run-child-agent-tasks-in-the-parent-run.md), [ADR-0027](0027-delegate-tasks-to-workers-through-a-runtime-port.md), [Host and ceiling](../features/workers/host-and-ceiling.md), [Credential leases](../features/workers/credential-leases.md) |

## Context

- A worker runs the same runtime as the master ([ADR-0015](0015-run-an-agent-runtime-on-every-machine.md)). Its tasks come from the master over the link, each with an instruction, a provider profile and possibly a narrower policy ([ADR-0027](0027-delegate-tasks-to-workers-through-a-runtime-port.md)).
- The runtime starts a task from a parent's run on the same machine and keeps the task in a row that names the parent's part ([ADR-0024](0024-run-child-agent-tasks-in-the-parent-run.md)). A worker's task has no parent on the worker.
- Every run uses the conversation agent's definition, and the instruction sections know the owner's computer, not a host with a purpose, host instructions and host facts.
- The tool policy allows, or asks the owner; it never refuses ([operations and approvals](../features/system-tools/operations-and-approvals.md#policy)). The owner decided how a worker's ceiling treats the folders outside the allowed ones ([decision D21](../roadmap/milestones.md#5-decisions)).
- A worker stores no provider profile and no credential; the master leases a credential to each task, in memory only ([ADR-0010](0010-keep-provider-credentials-on-the-master.md)).
- The log redactor keeps every secret it learned until the process ends.

## Decision

1. The runtime accepts tasks that another machine assigns. It assigns a task idempotently by the ID that the master chose, looks one up, streams the states of all its assigned tasks from a version, cancels one, and releases all of them when the worker is unpaired. An assigned task runs in a task conversation without a parent, under the agent definition that the worker registers, is asked once to confirm that it finished before it reports, as a child is, and publishes its state as a child does.
2. A run names its agent definition. The worker registers the definition `worker`: the system tools without `move_to_project`, no task tools, 100 rounds and 250 tool calls. The master never registers it, so the agents that `start_task` offers stay the same.
3. The instruction context gains the host: its name, its purpose, the host instructions and the host facts. The worker assembles its own registry of sections: a base section for a worker agent, tool guidance for the task's mode, the host section and the environment.
4. An assigned task carries a snapshot of its provider profile without the credential. Its runs read the credential from the vault under a reference of the task, and a vault can tell a run when the credential arrives, so a run waits for its lease with the activity `waiting_for_lease`. The worker's vault keeps leases in memory and drops a lease when its task ends, when the master revokes it, which also ends the task's run, and when the worker is unpaired.
5. The tool policy gains a ceiling and a refusal. The ceiling is the mode (read-only, ask or autonomous), the allowed folders and the host's command rules, with the effects that the [workers feature](../features/workers/host-and-ceiling.md#the-ceiling) lists after decision D21. A task can narrow the mode and never widen it. What the ceiling forbids is refused, and the call ends as not run with the reason. An approval request from a worker names the host and offers only Allow once and Deny, and the worker refuses any other choice, whatever the master sends.
6. The log redactor gains scoped registrations: a leased credential is redacted while its lease lives, and the redactor forgets it when the last holder lets it go.
7. The edits in other features: the approvals and tool calls of tools show the host, the ceiling's decisions, its reasons for asking and its refusals; chat shows the activity `waiting_for_lease`.

- Refinement: [ADR-0029](0029-let-the-owner-give-conversations-and-hosts-full-access.md) adds the mode `full_access`, in which every access runs, and lets a conversation in full access on the master answer the worker's requests with Allow once, unless the host's configuration refuses that.

## Consequences

- Positive: a worker reuses the loop, the tools, the journal, the confirmation and the recovery, and the same policy function enforces the ceiling on every call.
- Negative: the runtime has two sources of tasks, and the policy and the tools' interface learn the ceiling.
- Risks: a ceiling that is configured wrong allows more than the owner meant; the configuration is owned by root or Administrators, the worker refuses to start when its own account can change it, and the setup guide explains each mode. A process of the worker's account can read a leased credential from the worker's memory; the worker turns off core dumps and tracing where the system allows it, and the rest is a residual risk of the [security architecture](../architecture/security.md#9-if-a-worker-host-is-compromised).

## Alternatives considered

| Option                                                 | Why not chosen                                                                                                   |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------- |
| A runtime of the worker's own                          | It would copy the loop, the journal and their fixes                                                              |
| A ceiling built only from tool sets                    | Reads and changes inside and outside the allowed folders need a decision for each access                         |
| Provider profiles that the master copies to the worker | The worker would keep the master's configuration and its changes; a snapshot for each task carries what it needs |
| Credentials stored on the worker                       | Ruled out by [ADR-0010](0010-keep-provider-credentials-on-the-master.md)                                         |

## Validation

M4 and V6: runtime tests of assigned tasks and of the ceiling, the lease tests of V6, the link's tests over loopback, and the system tools suite driven by delegated tasks on each system.
