# ADR-0048: Erase what plugins delete from the store

|          |                                                                                                                                                                                                                                                                                                                                                                     |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Proposed                                                                                                                                                                                                                                                                                                                                                            |
| Date     | 2026-10-24                                                                                                                                                                                                                                                                                                                                                          |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                               |
| Related  | [ADR-0009](0009-persist-state-in-sqlite.md), [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), [ADR-0046](0046-let-plugins-attach-context-to-conversations.md), [Security](../architecture/security.md#11-local-data), [Extension points](../architecture/extension-points.md#39-store-tables-and-migrations), [Notebooks](../../plugins/notebooks/docs/README.md) |

## Context

- The plugin Notebooks keeps the text of the owner's documents in its tables, and deleting a notebook or a source must leave none of it in the store's files (threat T-67, accepted with [D43](../roadmap/milestones.md#5-decisions)).
- The store's writer overwrites freed pages with zeros (`secure_delete`). In write-ahead log mode, a deleted row's old page stays in the log until a checkpoint copies the log into the database and empties it. Deleting a conversation therefore calls `Store::flush`, a `wal_checkpoint(TRUNCATE)`, after its transaction ([security](../architecture/security.md#11-local-data)).
- A plugin writes only through the SDK's service `store`: a batch of listed statements with a durability, `Full` or `Normal`, as ADR-0009 decision 4 names the two levels ([ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), decision 7). A plugin cannot run a pragma, and its authorizer refuses one, so it cannot empty the log itself.
- The Notebooks plugin's test that plants a text, removes its source and searches the store's files for it failed while the text stayed in the log (`plugins/notebooks/crates/mannie-notebooks/tests/plugin.rs`).

## Decision

1. The SDK's `Durability` gains `Erase`. A batch with `Erase` commits with full durability, and the store then waits for the writes queued before it and empties the write-ahead log, as deleting a conversation does, before the write returns. What the batch deleted is then gone from every file of the store.
2. A log that a running read keeps from being emptied stays until the next checkpoint copies it; the store logs it and the write still succeeds, since its transaction committed.
3. A plugin uses `Erase` for the batches that delete the owner's content, such as a notebook or a source with their text, and `Full` for the others, since emptying the log costs a checkpoint.

Earlier decisions that this ADR changes:

| ADR  | Decision | Change                                                                                                |
| ---- | -------- | ----------------------------------------------------------------------------------------------------- |
| 0009 | d4       | Refined: a plugin's write may also erase, which is full durability and an emptied write-ahead log     |
| 0034 | d7       | Refined: the service `store` lets a plugin's batch erase what it deleted from every file of the store |

## Consequences

- Positive: a plugin that holds the owner's text keeps the promise that the core keeps for conversations, without a pragma or any other statement that the authorizer refuses.
- Negative: each erasing write waits for a checkpoint, which copies the whole log into the database. Deleting a large notebook takes longer by that copy.
- Risks: a read that runs during the checkpoint keeps part of the log until the next checkpoint, as it does after deleting a conversation. A plugin that chooses `Full` for a deletion leaves the old pages in the log until the next checkpoint; the plugin's tests plant and search text to catch it.

## Alternatives considered

| Option                                   | Why not chosen                                                                                  |
| ---------------------------------------- | ----------------------------------------------------------------------------------------------- |
| Let a plugin run `wal_checkpoint` itself | Plugins run no pragmas, and the authorizer refuses them for good reasons                        |
| Empty the log after every plugin write   | Every write would wait for a checkpoint                                                         |
| Overwrite the rows before deleting them  | The overwritten values reach the log as well, and the old pages with the text are already in it |
| Rely on the next automatic checkpoint    | The text stays in the log for an unknown time, and a copy of the files taken meanwhile holds it |

## Validation

M13: the store's test of an erasing batch, and the Notebooks plugin's test that plants a text, removes its source and finds the text in none of the store's files.
