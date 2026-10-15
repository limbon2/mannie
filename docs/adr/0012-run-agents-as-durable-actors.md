# ADR-0012: Run agents as durable actors independent of any connection

|          |                                                                                                                                               |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Proposed                                                                                                                                      |
| Date     | 2026-10-13                                                                                                                                    |
| Deciders | Owner                                                                                                                                         |
| Related  | [ADR-0009](0009-persist-state-in-sqlite.md), [ADR-0013](0013-journal-every-side-effect.md), [Agent runtime](../architecture/agent-runtime.md) |

## Context

- Windows reload, the owner closes them, and links to workers drop. If a run belongs to the connection that started it, every disconnect interrupts a reply, and recovery logic spreads through the code.
- More than one subscriber can follow a run: the UI, and the master following a worker's task.

## Decision

1. Each active run is an actor, a tokio task inside the agent runtime, with a mailbox for commands such as stop. It runs to completion whether or not anyone is subscribed. An approval decision reaches the run through the approval it waits on, a one-shot answer registered for that request, so decisions never fill the mailbox that stop relies on.
2. Each step makes one model call. The runtime owns tools, budgets, retries, compaction and quirk handling, and the provider layer only streams that one call.
3. The transcript is an append-only list of typed parts. Every change is an event with a per-conversation sequence number, stored before it is published.
4. Subscribers (the UI through Tauri channels, the master through the link) receive the events after a cursor. A subscriber that reconnects replays from its last cursor without gaps.
5. A step is stored before its tool calls run.
6. At startup, runs that were active become interrupted, and the owner can retry them. Operations follow [ADR-0013](0013-journal-every-side-effect.md).
7. Token counts reported by the provider are the truth for budgets. Estimates are used only before a conversation's first response.
8. Compaction is a projection: a checkpoint replaces covered parts in the model's context, and the transcript itself is never edited.
9. A subscriber without a cursor, or with a cursor far behind, first receives a snapshot of the conversation with the sequence number it reflects, and then the events after that number.
10. When a part finishes, its full text is stored with the part, and the events that streamed its deltas are dropped in the same transaction. The event that finishes the part carries the whole text, so applying the stored events after any cursor still gives the current state. Sequence numbers only increase, and may skip the numbers of dropped deltas.

## Consequences

- Positive: reloads, crashes and dropped links are ordinary events with one recovery path, shared by the UI and the master.
- Negative: the actor model needs care around ordering and shutdown, and each conversation's event log keeps growing, by its lifecycle events once the deltas of finished parts are dropped.

## Alternatives considered

| Option                                     | Why not chosen                                                          |
| ------------------------------------------ | ----------------------------------------------------------------------- |
| Runs tied to the request that started them | Every disconnect interrupts the run                                     |
| Event sourcing for all state               | Settings and registries are simple tables; only transcripts need events |

## Validation

V3 measures the store and channel throughput that this design needs. M1 proves the behavior: reloading the webview during a reply loses nothing, killing the process leaves an intact transcript and a run the owner can retry, and a property test shows that subscribers who replay from any cursor, or start from a snapshot, end with the same state as a fresh read.
