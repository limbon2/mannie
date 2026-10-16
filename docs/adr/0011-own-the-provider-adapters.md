# ADR-0011: Call providers through Mannie's own streaming adapters

|          |                                                                                                                                                              |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Accepted                                                                                                                                                     |
| Date     | 2026-10-13                                                                                                                                                   |
| Deciders | Owner                                                                                                                                                        |
| Related  | [ADR-0012](0012-run-agents-as-durable-actors.md), [Agent runtime](../architecture/agent-runtime.md), [Technology stack](../architecture/technology-stack.md) |

## Context

Verified 2026-10-13:

- The agent loop is the core of the product. It needs exact control over streaming events, cancellation, error classes, retries, the replay of provider-private reasoning, the integrity of tool calls and the quirks of open-weight model servers.
- The first providers are the OpenAI Responses API and OpenAI-compatible Chat Completions servers ([decision D7](../roadmap/milestones.md#5-decisions)).
- Rust has many young LLM client and agent framework crates, and none of them is clearly established.

## Decision

1. `mannie-provider` defines a `Provider` trait for one streamed model call. It takes a request and returns a stream of typed events that ends in a completed step. There is one adapter per API interface: Responses and Chat Completions.
2. The adapters are thin: reqwest with rustls (ring), a small parser for server-sent events, and serde types for the parts of each API that Mannie uses. No LLM framework or agent library is involved.
3. The adapters classify errors as transient (HTTP 408, 409, 425, 429 and 5xx, and broken streams, honoring `Retry-After`), context overflow, or permanent.
4. The adapters check each stream's integrity: its content type, a finish reason, tool call IDs that are unique and complete, and limits on argument size.
5. Quirk profiles describe the behavior of specific servers and models declaratively, for example tool calls written as text. The adapter normalizes the events before the runtime sees them.
6. Provider-private data, such as encrypted reasoning from the Responses API, is stored with a binding to its provider profile and replayed only to that binding.
7. Tests use recorded server-sent event streams from every supported server. End-to-end tests use a scripted fake provider.
8. Other providers arrive as features that implement the trait.

## Consequences

- Positive: full control, few dependencies and deterministic tests.
- Negative: Mannie has to update the adapters when provider APIs change (risk R5).

## Alternatives considered

| Option                                | Why not chosen                                                                                          |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| An LLM framework crate                | Young APIs that change often and hide the streaming details the loop needs                              |
| An OpenAI client crate (async-openai) | Covers OpenAI's request shapes but not the quirks of other servers; a large dependency for a thin layer |

## Validation

V8.
