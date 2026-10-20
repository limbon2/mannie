# Git on workers

|              |                                                                                                                                                                                                                           |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                                                                                                                      |
| Milestone    | Unscheduled                                                                                                                                                                                                               |
| Last updated | 2026-10-20                                                                                                                                                                                                                |
| Part of      | [Git](README.md)                                                                                                                                                                                                          |
| Related      | [Workers](../../workers/docs/README.md), [ADR-0028](../../../docs/adr/0028-run-assigned-tasks-under-the-host-ceiling.md), [ADR-0038](../../../docs/adr/0038-let-plugins-run-installed-programs-in-the-owners-projects.md) |

## Intent

A task that runs on a worker would use the same git tools in the worker's folders, instead of running git through the worker's `run_command`.

## Open questions

- A worker refuses every run of an installed program today (`no_programs_on_host`), because its host's ceiling has no rule for one. The ceiling would need to say which programs and effects it allows, such as git's reads in the allowed folders, and never its network operations without the host's own rule.
- Whether a worker runs the plugin's code, which today runs only on the master, or the master's plugin sends git's runs to the worker.
- Which git the worker runs, and how the master learns its version.
