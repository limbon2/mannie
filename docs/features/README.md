# Feature protocol

|              |                                                                                                                                                                                    |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                   |
| Last updated | 2026-10-19                                                                                                                                                                         |
| Related      | [ADR-0002](../adr/0002-grow-through-extension-points.md), [Extension points](../architecture/extension-points.md), [Template](template.md), [Milestones](../roadmap/milestones.md) |

Mannie grows one feature at a time. The owner says what they want in their own words, often roughly, and this protocol turns that into a feature that fits the architecture without breaking anything else. It is deliberately small: one document per feature, and nothing else to fill in.

## How a request becomes a feature

1. Capture. Restate the request in plain terms. Check `docs/features/`, and each plugin's `plugins/<id>/docs/`, for an existing feature that the request changes, and use the terms from the [glossary](../glossary.md).
2. Size it.

   | Size          | Test                                                                                                                                        | What to write                              |
   | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------ |
   | Tweak         | No new or changed contract, stored data, dependency, trust boundary or UI surface                                                           | The change, its tests and a changelog line |
   | Feature       | Adds or changes a contract (IPC, the link, a tool schema, text the model sees), stored data, a dependency, a trust boundary or a UI surface | A feature document                         |
   | Architectural | Needs a new or changed extension point, or conflicts with an Accepted ADR                                                                   | An ADR first, approved by the owner        |

3. Ask only the questions whose answers change the outcome. Write the other assumptions into the document instead of asking about them.
4. Shape. Write the feature document from the [template](template.md) with status Shaped. A request phrased as "do X" means build it now. A request phrased as "maybe later we want Y" means record it, as Idea or Shaped with milestone Unscheduled. When it is unclear which one the owner means, ask. An architectural change always waits for the owner to approve its ADR.
5. Build. Add the Design and Acceptance sections as the work starts, and build the feature as a slice through the [extension points](../architecture/extension-points.md). If the slice needs edits inside unrelated modules, stop: an extension point is missing. Propose it in an ADR, build it in a general form, then build the feature on it. A plugin builds only on the plugin SDK: when it needs something that the SDK does not provide, the SDK changes first, with an ADR for a new or changed extension point or service, and the plugin then builds on it ([ADR-0034](../adr/0034-build-plugins-on-a-plugin-sdk.md), decision 14).
6. Ship. When the acceptance checks pass, set the status to Shipped, add one line to the changelog, or to the plugin's own changelog for a plugin, and update the feature's line in the roadmap.

## Feature documents

- A feature lives in `docs/features/<slug>.md`, where the slug is a short noun phrase in kebab case, such as `system-tools`.
- A big feature becomes a folder, `docs/features/<slug>/`, when its document outgrows about a page or has parts that can be designed separately. Its `README.md` holds the status, Intent, Behavior, Fit and a Parts section that links each part document. Each part document has the same status table plus a "Part of" row, and its own Design and Acceptance. A part that can ship on its own becomes a separate feature, linked through Related.
- A plugin's feature document is `plugins/<id>/docs/README.md`, with a Plugin row that names the plugin's folder, and its part documents lie beside it ([ADR-0035](../adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), decision 11). Its milestone row is optional, and it needs no line in the roadmap, so adding a plugin changes no file outside its folder. Its changes go into `plugins/<id>/CHANGELOG.md`, which a release moves into Mannie's changelog under the plugin's name.
- A changed feature keeps its document. Update the Behavior and record the decision.
- A dropped idea keeps its document with status Dropped and the reason, so nobody proposes it again without knowing the history.

| Status   | Meaning                                                                  |
| -------- | ------------------------------------------------------------------------ |
| Idea     | Recorded, not yet shaped. A few lines are enough.                        |
| Shaped   | Intent, Behavior and Fit are written, and the open questions are listed. |
| Accepted | The owner agreed, and the feature has a milestone.                       |
| Building | Work has started. Design and Acceptance are filled in.                   |
| Shipped  | Released. The document describes the current behavior.                   |
| Dropped  | Not going to happen. The document says why.                              |

A feature document is not a task list, a test plan or a copy of the architecture. It links to the architecture where needed and describes only the feature.

## What else changes when a feature lands

| Artifact             | Change                                                                                  |
| -------------------- | --------------------------------------------------------------------------------------- |
| The feature document | Created or updated                                                                      |
| The roadmap          | One line linking the feature from its milestone, once it is Accepted; none for a plugin |
| The changelog        | One line when it ships, in the plugin's changelog for a plugin                          |
| An ADR               | Only for an architectural change                                                        |
| The glossary         | Only when a new term is used by more than one feature                                   |

Nothing else changes. Architecture documents describe extension points and rules, so they do not change because a feature was added. If you find yourself editing one to list a feature, stop and check whether an extension point is missing instead.
