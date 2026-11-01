# Database tools for worker tasks

|              |                                                                                                                                                                    |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Idea                                                                                                                                                               |
| Milestone    | Unscheduled                                                                                                                                                        |
| Last updated | 2026-11-01                                                                                                                                                         |
| Part of      | [Workers](README.md)                                                                                                                                               |
| Related      | [Databases](databases.md), [Host and ceiling](host-and-ceiling.md), [ADR-0057](../../../docs/adr/0057-let-the-master-work-with-the-databases-on-a-workers-host.md) |

## Intent

In the first version of [databases](databases.md), the master's agents reach a worker's databases through the link, and the worker agent that runs a delegated task gets no database tools: it can only run `sqlite3` through `run_command`, under its ceiling. A worker agent could get the same three tools for the databases registered on its own host, so that a delegated task queries and changes them with typed rows and the same approvals.

## Open questions

- Whether the worker agent sees only its own host's databases, as ADR-0015 d5 suggests, since workers do not reach each other.
- How the worker's ceiling decides its own agent's writes, which today ask the owner through the master, and whether a grant of the master may cover them.
- Whether the worker's instruction section lists its databases, and how the master learns which databases a task used.
