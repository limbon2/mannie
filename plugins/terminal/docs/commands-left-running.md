# Commands left running

|              |                                                                                                                                                                                                                                                                                                    |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                                                                                                                                                                                               |
| Milestone    | Unscheduled                                                                                                                                                                                                                                                                                        |
| Last updated | 2026-10-30                                                                                                                                                                                                                                                                                         |
| Part of      | [Terminals](README.md)                                                                                                                                                                                                                                                                             |
| Related      | [ADR-0041](../../../docs/adr/0041-let-agents-run-commands-in-the-background.md), [ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md), [Background commands](../../../docs/features/system-tools/background-commands.md), [Wake-ups](../../../docs/features/system-tools/wake-ups.md) |

## Intent

`run_in_terminal` waits for its command to end, 600 seconds at most, and interrupts it at the timeout or on Stop ([Terminals](README.md#agents-commands)). An agent would also start a command in a terminal and leave it running after its call, such as a development server or a log that the owner watches, and read its later output, as background commands let an agent do with `run_command` ([ADR-0041](../../../docs/adr/0041-let-agents-run-commands-in-the-background.md)).

## Open questions

- How an agent reads the later output of its command in a terminal, and how it learns that the command ended.
- Whether such a command needs an approval of its own, with grants and rules kept apart from those of commands that end within their call, as D40 keeps the grants of background commands apart.
- How the toolbar's count, the quit warning and wake-ups treat it.
- Whether an agent may type into the same terminal again while its command runs, and how it then tells its new output from the old.
