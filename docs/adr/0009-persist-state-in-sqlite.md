# ADR-0009: Persist state in SQLite through rusqlite

|          |                                                                                                                                                  |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Proposed                                                                                                                                         |
| Date     | 2026-10-13                                                                                                                                       |
| Deciders | Owner                                                                                                                                            |
| Related  | [ADR-0012](0012-run-agents-as-durable-actors.md), [ADR-0013](0013-journal-every-side-effect.md), [Domain model](../architecture/domain-model.md) |

## Context

Verified 2026-10-13:

- Mannie stores transcripts that grow while models stream, journal entries that must be durable before a side effect starts, settings and registries. A crash or a power loss must not corrupt or lose committed data, and every release must migrate the data of the releases before it.
- Each machine runs one Mannie process: the single-instance plugin on the desktop, and one service per worker host.
- rusqlite 0.40.2 with the `bundled` feature compiles the same SQLite version into every binary, FreeBSD included.
- sqlx is asynchronous, and its compile-time checked queries need a live database or committed query metadata during the build.

## Decision

1. Each machine has one SQLite database in its data directory, in WAL mode.
2. The driver is rusqlite with the `bundled` feature. Only `mannie-store` depends on it, and other crates use its repository interfaces.
3. One writer thread owns the write connection and takes requests over a channel. Reads use a small pool of read-only connections.
4. There are two durability levels. Journal entries and state changes commit with `synchronous=FULL` before the side effect they record. Streamed deltas are batched and committed at least every 250 ms with `synchronous=NORMAL`.
5. Migrations are forward-only, embedded in the binary, and owned by the module whose tables they change. The store copies the database file before migrating. A database whose schema is newer than the binary knows is refused and left untouched.
6. A test prepares every SQL statement against the fully migrated schema. This catches the errors that compile-time checking would catch.
7. IDs are UUIDv7 strings, and timestamps are UTC milliseconds since the Unix epoch.

## Consequences

- Positive: clear behavior after a crash, and one storage engine on every platform.
- Negative: the synchronous API runs on its own thread, and tests check the SQL instead of the compiler.

## Alternatives considered

| Option                                   | Why not chosen                                                                                          |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| sqlx                                     | Asynchronous access brings nothing for one local file, and checked macros need a database at build time |
| JSON files rewritten on every change     | No transactions, and the cost of rewriting grows with the data                                          |
| An embedded key-value store (redb, sled) | No SQL, and migrations are harder                                                                       |

## Validation

V3.
