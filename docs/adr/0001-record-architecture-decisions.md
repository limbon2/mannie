# ADR-0001: Record architecture decisions as ADRs

|          |                                                      |
| -------- | ---------------------------------------------------- |
| Status   | Accepted                                             |
| Date     | 2026-10-13                                           |
| Deciders | Owner                                                |
| Related  | [ADR index](README.md), [Template](0000-template.md) |

## Context

The owner builds Mannie together with coding agents, over many separate sessions. An agent starts each session without memory of earlier discussions, and months later the owner will not remember every reason either. Decisions that exist only in chat history, commit messages or someone's memory get argued again or broken without anyone noticing.

## Decision

- Significant decisions are recorded as ADRs in `docs/adr/`, numbered in order and written from the [template](0000-template.md).
- A decision is significant when it affects security properties, a contract (IPC, the link, tool schemas, stored data), an extension point, a dependency on a third-party system, the supported platforms, or anything else that is expensive to reverse.
- Each ADR has one status. Owner mandates are Accepted (Dn), and the [decision record](../roadmap/milestones.md#5-decisions) keeps the owner's reasons. Other ADRs start as Proposed and become Accepted when the validations they name pass and the owner approves.
- Accepted ADRs are not edited, except to mark them superseded. A change of mind is a new ADR that supersedes the old one.
- Code that contradicts an Accepted ADR does not merge until a superseding ADR is accepted.
- A decision that affects only one feature is recorded in that feature's document instead of an ADR.

## Consequences

- Positive: agents and the owner have one place to look up why something is built the way it is, and a deviation shows up in review.
- Negative: each significant decision costs a short document.

## Alternatives considered

| Option                       | Why not chosen                                                                        |
| ---------------------------- | ------------------------------------------------------------------------------------- |
| Design documents only        | Decisions get mixed with details that change often, so it is unclear what still binds |
| Decisions in commit messages | Hard to find later, and they cannot be superseded cleanly                             |

## Validation

None needed. This ADR describes the process itself.
