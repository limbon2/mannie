# Feature name

|              |                                                         |
| ------------ | ------------------------------------------------------- |
| Status       | Idea · Shaped · Accepted · Building · Shipped · Dropped |
| Milestone    | Mn · Unscheduled                                        |
| Last updated | YYYY-MM-DD                                              |
| Related      | ADRs, architecture documents, other features            |

A part document of a folder feature adds one more row to the table: "Part of", linking the feature's `README.md`.

## Intent

The owner's request restated in two to five sentences: who wants what, and why. Quote the owner's own words when they carry a nuance.

## Behavior

What the owner, or an agent, can do once this ships. Observable behavior only, including failure cases and what the feature deliberately does not do.

## Fit

The extension points the feature uses, the behavior of other features that must not change, the ADRs it follows, and any ADR it needs.

## Open questions

Only questions whose answers change the outcome. Remove each one once it is answered and put the answer into Behavior or Decisions.

## Decisions

Dated decisions that affect only this feature. Decisions that affect the whole project go to the [decision record](../roadmap/milestones.md#5-decisions).

## Parts

Folder features only: one line per part document, with a link.

## Design

Added when building starts: contracts (IPC, link, tool schemas, text the model sees), data (tables and migrations), the message keys the feature adds, threat notes when the feature touches a trust boundary, and platform differences.

## Acceptance

The checks that prove the feature works, including failure paths, and how each one is tested.
