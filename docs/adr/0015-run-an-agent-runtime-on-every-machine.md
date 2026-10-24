# ADR-0015: Run an agent runtime on every machine: the master orchestrates, workers own their hosts

|          |                                                                                                                                                                                                                          |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Accepted (D5)                                                                                                                                                                                                            |
| Date     | 2026-10-13                                                                                                                                                                                                               |
| Deciders | Owner                                                                                                                                                                                                                    |
| Related  | [Orchestration](../architecture/orchestration.md), [ADR-0010](0010-keep-provider-credentials-on-the-master.md), [ADR-0016](0016-link-machines-over-pinned-mtls-grpc.md), [Workers](../../plugins/workers/docs/README.md) |

Later changes: [ADR-0036](0036-let-plugins-run-commands-on-remote-hosts.md) supersedes decision 2 in part: an agent's tools also act on a remote host that the owner registered with a plugin, through a remote-command access that the master's policy authorizes. The master still never runs tools on a worker's host. [ADR-0043](0043-let-plugins-offer-the-tools-of-tool-servers.md) refines decision 2: an agent's tool also acts where a tool server that the owner registered acts, through an external-tool access that the master's policy authorizes.

## Context

- The owner's direction ([decision D5](../roadmap/milestones.md#5-decisions)): the master takes care of the owner's request and orchestrates. Workers are autonomous agents, each running on a host that may serve a particular purpose. A worker takes care of the master's request and of its host's environment, so the master does not need to know how a FreeBSD host works or which commands to run there.
- Running every agent loop on the master, with workers executing single tool calls, was considered. It would keep credentials on the master and need only one runtime. It was rejected because the master would have to know every host's environment, and workers could not act on their own.

## Decision

1. The same agent runtime runs in the desktop app and in every worker.
2. An agent runs on exactly one machine, and its tools act only on that machine. The master never runs tools on a worker's host.
3. The master's conversation agent answers the owner, uses tools on the master's machine, starts child agents there and delegates tasks to workers.
4. A worker agent receives a task, which is an instruction with its constraints. It works out how to do the task with its host's tools, host facts and host instructions, and returns a confirmed report.
5. Child agents run on the same machine as the agent that started them. Workers do not delegate to other workers.
6. Each worker stores the conversations of its tasks. The master shows them live and afterwards by subscribing over the link ([ADR-0012](0012-run-agents-as-durable-actors.md), [ADR-0016](0016-link-machines-over-pinned-mtls-grpc.md)).
7. Each worker's policy ceiling is configured on its host. The master can narrow it for a task but cannot widen it.
8. The master holds the owner's conversations, the provider profiles with their credentials ([ADR-0010](0010-keep-provider-credentials-on-the-master.md)) and the worker registry.

## Consequences

- Positive: knowledge about a host stays on that host, and a worker can finish a task while the master is offline.
- Negative: the runtime runs on three operating systems. The master and a worker can run different versions of it (risk R15), and transcripts are spread across machines.
- Refinement in M5: workers become the first plugin, which the owner turns on and off, and these decisions hold while it is on ([ADR-0031](0031-move-workers-into-the-first-plugin.md)).

## Alternatives considered

| Option                                                            | Why not chosen                                                                                |
| ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| All loops on the master, with workers executing single tool calls | The master would need to know each host's environment, and workers could not act on their own |
| Workers as plain remote shells over SSH                           | No autonomy on the host and no knowledge of it                                                |

## Validation

None needed: the owner decided this (D5). The [workers feature](../../plugins/workers/docs/README.md) validates the mechanisms in M4, including V6.
