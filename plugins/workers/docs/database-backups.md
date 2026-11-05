# Database backups and replicas

|              |                                                                                                                                                                                   |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                                                                              |
| Milestone    | Unscheduled                                                                                                                                                                       |
| Last updated | 2026-11-01                                                                                                                                                                        |
| Part of      | [Workers](README.md)                                                                                                                                                              |
| Related      | [Databases](databases.md), [ADR-0057](../../../docs/adr/0057-let-the-master-work-with-the-databases-on-a-workers-host.md), [D46](../../../docs/roadmap/milestones.md#5-decisions) |

## Intent

When shaping [databases](databases.md), the owner asked whether rqlite, or something of Mannie's own, should spread a database over several machines, and chose one home for each database for the first version ([D46](../../../docs/roadmap/milestones.md#5-decisions)). A registered database could later get copies: a backup on another machine on a schedule or on the owner's action, or a read-only replica that a worker keeps current, so that its data stays readable while its home is offline.

## Open questions

- Whether a copy is a backup, which SQLite's backup API writes in one step, or a replica that follows the changes, such as with the session extension's changesets over the link.
- Where a copy lives, since workers never connect to each other: on the master, or on another worker through the master.
- What the owner may do with a copy while its home is offline, and how a replica catches up when the home returns.
- Whether a consensus such as rqlite's Raft is ever worth its cost for one owner's databases, given the reasons of ADR-0057.
