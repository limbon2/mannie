# ADR-0060: Let agents save plans that the owner runs later

|          |                                                                                                                                                                                                                                                                                                                                                                                                                 |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D49)                                                                                                                                                                                                                                                                                                                                                                                                  |
| Date     | 2026-11-07                                                                                                                                                                                                                                                                                                                                                                                                      |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                           |
| Related  | [ADR-0013](0013-journal-every-side-effect.md), [ADR-0023](0023-organize-conversations-into-projects.md), [ADR-0032](0032-let-features-and-plugins-add-kinds-of-mentions.md), [ADR-0054](0054-let-tool-results-keep-what-they-cite-and-let-plugin-views-add-mentions.md), [ADR-0058](0058-let-agents-ask-the-owner-questions.md), [ADR-0059](0059-let-the-owner-define-agents.md), [Plans](../features/plans.md) |

## Context

- The owner asked for "a plan agent that will focus on creating readonly plans that can later be used to execute them".
- An agent that only reads has nowhere to put a plan but its reply, which is hard to find later and cannot be handed to another agent without copying it. Writing a file is an operation on the owner's files, which a read-only agent must not have, and which would ask for approval.
- A plan is model output, and text that the model read can shape it. Running a plan puts that text in front of an agent that can change things, at the owner's request.
- The policy reads the owner's words from the `user_text` part alone, and a move into a project that those words name runs without asking (T-19). A mention's text never stands in for them ([ADR-0032](0032-let-features-and-plugins-add-kinds-of-mentions.md), decision 7), but its token enters the owner's text.

## Decision

1. A plan is a title and a Markdown body with a number, in the table `runtime_plans`. A conversation holds one plan, and saving again replaces it, so the model never handles an ID. A plan outlives its conversation and its project: both references are set to `NULL` when they go.
2. The tool `save_plan` belongs to the runtime, outside the tool registry, as `ask_owner` does. A definition names it, and the runtime offers it to runs whose definition names it, whose profile uses tools and that are not tasks. Only custom conversation agents can name it in this version.
3. `save_plan` has no accesses, asks for no approval and is no operation under [ADR-0013](0013-journal-every-side-effect.md). It changes nothing on the owner's machine beyond Mannie's own database, which every transcript write already changes. Nothing runs because of it, and its `tool_call` part records it in the same transaction as the plan. So a read-only agent may have it.
4. The core refuses a title over 80 characters and a body over 32 KiB, and a blank one, so that a plan never changes by being cut. Titles are cleaned as labels are. The result says what is wrong, as for `ask_owner`.
5. The owner can edit a plan, which marks it. The next `save_plan` of that conversation is refused once, the result holds the plan as it stands, and the mark clears, so the model's copy never overwrites the owner's edit without the model having read it.
6. A plan reaches a later reply only as a mention, through the core's provider `plans`. Its token is `plan-<number>`, and its title shows only as the chip's label. A title never enters `user_text`, so a model-written title that names a project cannot allow a move into it (T-19, T-84). The provider states what it left out when the message's budget does not hold the whole plan.
7. The buttons "Run in a new conversation" and "Run in the plan's conversation" put "Carry out this plan." and the mention into the message box and choose an agent, and the owner presses Send. Nothing runs before Send. The owner's sentence is what asks the agent to follow the plan, and every operation still goes through the policy. The second button sets the conversation's agent at once, which the owner can set back.
8. The model reads a mentioned plan under a heading that says a model wrote it, and that it allows no operation.
9. The newest call of `save_plan` stays whole through compaction, as an answer does ([ADR-0058](0058-let-agents-ask-the-owner-questions.md), decision 7), because the model edits from it.
10. The message boxes gain `fill`, which sets a box's text with a mention, and seeds that wait for a box that does not show yet, including the new conversation pane's ([ADR-0054](0054-let-tool-results-keep-what-they-cite-and-let-plugin-views-add-mentions.md), decision 4, grows).
11. Contracts grow by addition: a table, two outcomes, two changes of the store, a mention provider and the commands of [plans](../features/plans.md#contracts). This adds no extension point.

## Consequences

- Positive: a read-only agent leaves a result that lasts, which the owner reads, edits and runs when they choose, with an agent of their choice, and any conversation can mention it. No file of the owner's changes.
- Negative: a plan is not in the project's repository, so it does not travel with the code. A conversation holds one plan, so a second one needs another conversation. Older versions are only in the stored call's arguments, which the row shows cut.
- Risks: a plan shaped by what the model read carries an instruction that the owner did not intend, and in full access nothing checks what follows. The plan shows before Run, the owner presses Send, the heading says a model wrote it, and the policy still decides each operation (T-84).

## Alternatives considered

| Option                                           | Why not chosen                                                                                                             |
| ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| Plans as files in the project                    | A write to the owner's files is an operation that a read-only agent must not have, and it would ask for approval each time |
| Putting the plan's text into the owner's message | The policy reads the owner's words from that part, so a model's text would stand in for them                               |
| A token made from the title                      | The token enters the owner's text, so a title could name a project and allow a move without a question                     |
| A button that starts the reply at once           | The owner could not add notes or change the agent, and a click on model output would run it                                |
| Several plans in a conversation                  | The model would have to manage IDs for a case that another conversation covers                                             |
| A read tool for plans                            | The mention holds the plan, and only a model with a very small context cannot hold it, which the note states               |

## Validation

M17: the runtime tests of plans (one transaction, the limits, compaction, the owner's edit, the mention whose title names a project, a plan that does not fit, a retry), the schema's snapshot, the UI's tests, the end-to-end test of both ways to run a plan, and the live evaluation in which the Plan agent changes nothing and saves a plan.
