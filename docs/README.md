# Mannie documentation

|              |                                                                                                                       |
| ------------ | --------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                      |
| Last updated | 2026-10-15                                                                                                            |
| Related      | [AGENTS.md](../AGENTS.md), [README](../README.md), [ADR index](adr/README.md), [Feature protocol](features/README.md) |

This is the project's documentation: the vision, scope, architecture, decisions and working rules, and the results of the validations that settle the decisions.

## Reading order

| #   | Document                                             | What it answers                                                                                                                                                                                                                                    |
| --- | ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | [Vision](product/vision.md)                          | What Mannie is, for whom, and the principles behind its trade-offs                                                                                                                                                                                 |
| 2   | [Goals and scope](product/goals-and-scope.md)        | Goals, non-goals, quality requirements, supported platforms and the release model                                                                                                                                                                  |
| 3   | [Feature protocol](features/README.md)               | How a request becomes a feature, and how features are documented                                                                                                                                                                                   |
| 4   | [Milestones](roadmap/milestones.md)                  | The milestones, the validations, the decision record and the risk register                                                                                                                                                                         |
| 5   | [Architecture overview](architecture/overview.md)    | Components, process model, key flows, data placement and failure behavior                                                                                                                                                                          |
| 6   | [Extension points](architecture/extension-points.md) | Where features plug in, and the rules that keep growth additive                                                                                                                                                                                    |
| 7   | [Domain model](architecture/domain-model.md)         | Concepts, lifecycles and invariants                                                                                                                                                                                                                |
| 8   | [Desktop app](architecture/desktop-app.md)           | IPC, streaming, webview security and the frontend structure                                                                                                                                                                                        |
| 9   | [Agent runtime](architecture/agent-runtime.md)       | Runs, the agent loop, providers, model quirks and context management                                                                                                                                                                               |
| 10  | [Tools](architecture/tools.md)                       | The tool contract, operations, approvals, file safety and running commands                                                                                                                                                                         |
| 11  | [Orchestration](architecture/orchestration.md)       | Child agents, workers, delegation, pairing and the link                                                                                                                                                                                            |
| 12  | [Security](architecture/security.md)                 | Objectives, trust boundaries, threats and the rules for secrets                                                                                                                                                                                    |
| 13  | [Technology stack](architecture/technology-stack.md) | How each building block is used, with verified versions                                                                                                                                                                                            |
| 14  | [Engineering standards](engineering/standards.md)    | The Definition of Done and the standards for code, contracts, data and tests                                                                                                                                                                       |
| 15  | [Repository](engineering/repository.md)              | The layout, the crates and their rules, generated artifacts and tasks                                                                                                                                                                              |
| 16  | [Releasing](engineering/releasing.md)                | When releases happen and how they are built                                                                                                                                                                                                        |
| -   | [M0 validation results](roadmap/m0-validation.md)    | What the M0 validations showed, and the decisions they leave for the owner                                                                                                                                                                         |
| -   | [M1 validation results](roadmap/m1-validation.md)    | What the M1 validations and exit criteria showed so far, and what remains before v0.1.0                                                                                                                                                            |
| -   | [M2 validation results](roadmap/m2-validation.md)    | What the M2 validations and exit criteria showed so far, and what remains before v0.2.0                                                                                                                                                            |
| -   | [M3 validation results](roadmap/m3-validation.md)    | What the M3 checks and exit criteria showed so far, and what remains before v0.3.0                                                                                                                                                                 |
| -   | [Architecture decision records](adr/README.md)       | The decisions behind all of the above                                                                                                                                                                                                              |
| -   | [Features](features/README.md)                       | One document per feature: [chat](features/chat/README.md), [window and tray](features/window-and-tray.md), [system tools](features/system-tools/README.md), [child agents](features/child-agents/README.md), [workers](features/workers/README.md) |
| -   | [Glossary](glossary.md)                              | Shared vocabulary                                                                                                                                                                                                                                  |
| -   | [Setting up a worker](guides/worker-setup.md)        | How the owner installs, pairs, updates and removes a worker on a Windows, Linux or FreeBSD host                                                                                                                                                    |

## Status

- Every document is a draft for review.
- ADRs with the status Accepted (Dn) record decisions the owner has made. Proposed ADRs are recommendations that the validations in the [milestones](roadmap/milestones.md#4-validations) settle.
- No decision in the [decision record](roadmap/milestones.md#5-decisions) is open.

## Conventions

- Rules for coding agents live in [AGENTS.md](../AGENTS.md). `CLAUDE.md` only imports it.
- Every document starts with a status table: status, last updated and related documents. Feature documents add a milestone row.
- Product, roadmap, architecture and engineering documents number their second-level headings.
- Diagrams are Mermaid, embedded in the Markdown.
- Facts about third-party components carry the date they were verified. Check them again when upgrading.
- An accepted ADR is not edited. A change of mind is a new ADR that supersedes it.
- Architecture documents describe extension points and rules, never lists of features.
- `task docs:check` checks formatting, Markdown rules, spelling, links and the consistency of the documents.
