# ADR-0052: Offer a plugin's tools where its attachments are

|          |                                                                                                                                                                                                                                                                                                                                                                                         |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D44)                                                                                                                                                                                                                                                                                                                                                                          |
| Date     | 2026-10-24                                                                                                                                                                                                                                                                                                                                                                              |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                   |
| Related  | [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), [ADR-0038](0038-let-plugins-run-installed-programs-in-the-owners-projects.md), [ADR-0046](0046-let-plugins-attach-context-to-conversations.md), [ADR-0049](0049-gather-context-for-the-messages-that-ask-and-cite-it-by-links.md), [ADR-0051](0051-let-a-plugin-keep-conversations-of-its-own.md), [Tools](../architecture/tools.md) |

## Context

- D43 gave the model no search tool: a plugin's tool is offered always, in a workspace, or where the workspace holds a marker (ADR-0038, decision 2). A search tool would cost its definition in every request, attached or not.
- In the owner's earlier harness, the model searches and reads the documents of the conversation with three tools: one lists them, one searches them and one reads a passage or pages. The tools are offered only where the conversation can use documents. Its explore and general child agents use the documents of the conversation that started them. Its answers were better ([D44](../roadmap/milestones.md#5-decisions)), and a follow-up that names nothing still finds its passages, because the model writes the query.
- A plugin's tool receives the call's arguments, conversation and workspace (`ToolCall`, `ToolExecution`), but not the conversation's attachments. So a tool cannot know which notebooks to search.

## Decision

1. `ToolOffering` gains `WhereAttached(provider)`. Such a tool is offered in a conversation that holds attachments of that context provider; for a child agent's task, in the conversation that gave the task.
   - The provider must be the plugin's own, by the rule `Owned`, or the registration is refused.
   - The run's `Availability` gains the conversation's attachments, so that the offer is decided for each request, as for workspaces.
2. `ToolCall` and `ToolExecution` gain `attachments`: the keys of that provider in that conversation, in the order of attaching. A tool reaches only what they name, and the core passes none to a tool offered another way.
3. A tool offered this way reads the plugin's own data, so it declares `ToolEffect::ReadsOwnData`, needs no approval, and reaches the explore agent.
4. The plugin Notebooks offers three tools to the agents `conversation`, `general` and `explore`, where notebooks are attached:
   - `list_notebook_sources` lists the attached notebooks' sources.
   - `search_notebooks` searches them and returns passages with cite links (ADR-0049, decision 4).
   - `read_notebook_source` reads a source from a passage, a page range or an offset.

Earlier decisions that this ADR changes:

| ADR  | Decision | Change                                                                                                                        |
| ---- | -------- | ----------------------------------------------------------------------------------------------------------------------------- |
| 0034 | d6       | Refined: a plugin's tool may be offered where the conversation holds the plugin's attachments, and its calls carry their keys |
| 0038 | d2       | Refined: offerings decide by attachments as well as by workspaces                                                             |
| 0046 | Context  | Revised: a tool is offered only where it applies, so the model gets one                                                       |

## Consequences

- Positive: the model searches the attached documents when a question needs it, with its own words, several times in a reply, and reads more than one passage. A conversation without attachments pays nothing for the tools.
- Negative: a conversation with attachments carries the tools' definitions in every request. A model that answers without searching misses what the tools would find; the instructions tell it to search.
- Risks: a tool could reach keys that the conversation does not hold. The core passes only that conversation's keys, and the plugin checks each argument against them.

## Alternatives considered

| Option                                                    | Why not chosen                                                                                            |
| --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Offer the tools always                                    | Every request would carry them, and the model would search notebooks that the owner did not attach        |
| Let the plugin read the conversation's attachments itself | It would need another service, and the core knows them at the call already                                |
| Gathering for every message, without tools                | The owner's conversation showed that searching with the message's own words misses what a follow-up means |

## Validation

M13:

- runtime tests of the offer with and without attachments, and of a child's calls with its parent's keys;
- the plugin's tests of the three tools' scope, of sources that are switched off and of cite links;
- the plugin Notebooks' end-to-end test of a reply that calls `search_notebooks`.
