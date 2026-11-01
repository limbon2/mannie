# Worker terminals

|              |                                                                                                                                                                                                                                                                                                                                                                                                      |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                                                                                                                                                                                                                                                                                                 |
| Milestone    | Unscheduled                                                                                                                                                                                                                                                                                                                                                                                          |
| Last updated | 2026-10-30                                                                                                                                                                                                                                                                                                                                                                                           |
| Part of      | [Terminals](README.md)                                                                                                                                                                                                                                                                                                                                                                               |
| Related      | [ADR-0015](../../../docs/adr/0015-run-an-agent-runtime-on-every-machine.md), [ADR-0028](../../../docs/adr/0028-run-assigned-tasks-under-the-host-ceiling.md), [ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md), [Workers](../../workers/docs/README.md), [Host and ceiling](../../workers/docs/host-and-ceiling.md), [Pairing and the link](../../workers/docs/pairing-and-link.md) |

## Intent

The owner asked for terminals on a worker's host, "so that i can execute commands on a worker machine", then deferred them from the first version ([D45](../../../docs/roadmap/milestones.md#5-decisions)). A worker owns its host, and the master never runs tools there (D5, [ADR-0015](../../../docs/adr/0015-run-an-agent-runtime-on-every-machine.md) d2). A terminal on a worker's host would let the owner, and the master's agent, type into a shell of that host, so it needs an ADR that supersedes ADR-0015 d2 in part, as [ADR-0036](../../../docs/adr/0036-let-plugins-run-commands-on-remote-hosts.md) did for remote hosts.

Such terminals would need:

- a protobuf package of the link for terminals, with its own feature level ([link services](../../../docs/architecture/extension-points.md#310-link-services)), which carries a terminal's bytes, size and end between the master and the worker;
- the host's own consent: the worker's configuration on the host allows terminals, and the master cannot widen it;
- the host's ceiling, which today refuses every terminal command with `no_terminals` ([ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md), decision 5);
- the Workers plugin offering its hosts through the point `terminal-hosts`, as the SSH plugin does;
- the supervisor's terminal mode on FreeBSD.

## Open questions

- Whether what the owner types on a worker's host needs only the host's consent to terminals, since typing is not an operation on the master ([ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md), decision 6), or passes the host's ceiling too.
- Whether an agent's command in a worker terminal is decided by the master's policy, the worker's ceiling or both, and which of them records it in a journal.
- Where the headless emulator runs: on the worker, which then keeps the blocks and their text, or on the master, which then receives every byte over the link.
- What happens to a worker terminal when the link drops, and whether the master's view picks it up again when the link returns.
