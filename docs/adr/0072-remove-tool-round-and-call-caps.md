# ADR-0072: Remove tool-round and tool-call caps

|          |                                                                                                                                                                                                                    |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Accepted (D67)                                                                                                                                                                                                     |
| Date     | 2026-11-17                                                                                                                                                                                                         |
| Deciders | Owner                                                                                                                                                                                                              |
| Related  | [ADR-0012](0012-run-agents-as-durable-actors.md), [ADR-0028](0028-run-assigned-tasks-under-the-host-ceiling.md), [System tools](../features/system-tools/README.md), [Custom agents](../features/custom-agents.md) |

## Context

A reply reached 100 tool rounds while its requested implementation was unfinished. The runtime disabled tools and asked for a final answer, then stored the run as completed. The owner asked to remove the recorded decision and the hard cap.

The system-tools decision of 2026-10-14 imposed 100 rounds and 250 calls. Children, workers and custom agents also had these limits. ADR-0028 decision 2 explicitly assigned them to workers.

## Decision

1. Remove the system-tools decision of 2026-10-14 that caps rounds and calls. No conversation, child, custom, plugin or worker agent stops or loses tools because of the number of tool rounds or calls. This supersedes the tool-round and tool-call limits in ADR-0028 decision 2.
2. Remove the forced final-answer message and tool refusals caused by those limits. Counters remain for progress and logging, and saturate without ending a run.
3. Preserve the retired fields in stored custom-agent drafts, IPC and plugin agent definitions for compatibility under ADR-0002. Their values are ignored, missing draft fields default to zero, and new built-in and custom templates use zero. The editor no longer offers those fields. Historical `over_budget` results remain readable.
4. Keep task-count and concurrency limits, continuation-note limits, context and output limits, timeouts, approvals, policy checks, Stop and shutdown behavior. A model can still finish its reply, and provider or tool failures retain their existing handling.

## Consequences

Long assignments can continue using tools. A model that loops may consume more time and provider usage until it finishes, fails or the owner stops it. The owner retains Stop, and every tool operation still passes its existing checks.

## Alternatives considered

| Option                                   | Why not chosen                                                                        |
| ---------------------------------------- | ------------------------------------------------------------------------------------- |
| Raise the fixed limits                   | Another long assignment would encounter the same forced ending.                       |
| Keep custom-agent limits                 | Saved definitions would continue to stop assignments after the owner removed the cap. |
| Remove the serialized fields immediately | Existing drafts and plugin callers need a compatibility period.                       |

## Validation

Runtime regression coverage runs 101 tool rounds and 303 calls with retired limits set to one, verifies that tools remain available and every call completes, then lets the model finish normally. Draft tests cover old fields, omitted fields and the independent task-count limit. Existing runtime tests cover Stop, approvals, tools-off profiles, children and worker tasks.
