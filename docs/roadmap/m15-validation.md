# M15 validation results

|              |                                                                                                                                                                                                                                                                                                                                                                                         |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                                                                                                                                                                                                        |
| Last updated | 2026-11-01                                                                                                                                                                                                                                                                                                                                                                              |
| Related      | [Milestones](milestones.md#m15-databases), [ADR-0055](../adr/0055-let-plugins-have-optional-features.md), [ADR-0056](../adr/0056-let-plugins-work-with-the-owners-sqlite-files-through-the-core.md), [ADR-0057](../adr/0057-let-the-master-work-with-the-databases-on-a-workers-host.md), [Databases](../../plugins/workers/docs/databases.md), [Security](../architecture/security.md) |

This document records what the spikes and checks of [M15](milestones.md#m15-databases) showed, so that the owner can review Databases and see what remains before the release. The owner accepted ADR-0055, ADR-0056 and ADR-0057 (D47). Each section names the checks, where they ran, the result, and what is open.

The spikes S1 to S5 and S9 ran on 2026-11-01 on the owner's workstation, Windows 11 build 28000, in a scratch folder outside the repository, with Rust 1.99.0 and the SQLite that Mannie bundles: rusqlite 0.40.2 and libsqlite3-sys 0.38.2, which build SQLite 3.53.2.

## 1. Summary

| Spike or check                                | Result                                                                                                                                                                                           | Open                                                                       |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------- |
| S1: the authorizer's codes                    | Every statement that the engine refuses reports an action that the authorizer can deny, or fails under the attach limit at run time; `readonly()` alone does not tell a read (section 2)         | None                                                                       |
| S2: opening other programs' files             | WAL files open read-only while their program holds them; `file:` paths are URIs even without the flag, and `SQLITE_OPEN_NOFOLLOW` opens a link on Windows, so the engine checks both (section 3) | Folders without write rights and network file systems on Linux and FreeBSD |
| S3: interrupts, limits and memory             | An interrupt ends a statement within 0.2 ms, a progress handler costs nothing measurable, and a large sort spills to temporary files (section 4)                                                 | None                                                                       |
| S4: contention with the owner's applications  | An open read blocks a rollback journal's writer and stalls a WAL checkpoint; a page read and dropped releases its lock at once (section 5)                                                       | Linux and FreeBSD                                                          |
| S5: file identity                             | The volume and file index stay through writes in place and change when a program saves by replacing the file (section 6)                                                                         | OneDrive, and the device and inode on Linux and FreeBSD                    |
| S6: the view's components and grid            | Not run                                                                                                                                                                                          | All of it, with the view                                                   |
| S7: feature generations under load            | Not run                                                                                                                                                                                          | All of it, with the runtime's tests of plugin features                     |
| S8: the optional package with an older worker | Not run                                                                                                                                                                                          | All of it, with the link                                                   |
| S9: schemas with thousands of tables          | 5,000 tables and 5,000 indexes parse in 33 ms, and all 50,000 columns read in one statement in 17 ms (section 7)                                                                                 | None                                                                       |

## 2. The authorizer (S1)

The spike prepared each statement on a database with a table, a child table with a foreign key, an index, a trigger, a view and an FTS5 table, under an authorizer that recorded every action, and ran some of them.

| Statements                                                                | Actions that the authorizer reports                                                             | `readonly()` |
| ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | ------------ |
| `SELECT` on tables, views, FTS5 tables, `sqlite_schema` and `dbstat`      | `Select` and `Read` of each column, with the view's name as the accessor for its reads          | true         |
| `INSERT`, `UPDATE`, `DELETE`, `REPLACE`, with `WITH` or `RETURNING`       | `Insert`, `Update` or `Delete`, with the trigger's name as the accessor for what a trigger does | false        |
| `CREATE` and `DROP` of tables, indexes, views and triggers, `ALTER TABLE` | Their own code, such as `CreateTable` or `AlterTable`, with writes of `sqlite_master`           | false        |
| `REINDEX` and `ANALYZE`                                                   | `Reindex`, and for `ANALYZE` the creation of `sqlite_stat1` and `sqlite_stat4` with `Analyze`   | false        |
| `CREATE TEMP TABLE`                                                       | `CreateTempTable` on the database `temp`                                                        | false        |
| `CREATE VIRTUAL TABLE`, and `DROP TABLE` of one                           | `CreateVtable` and `DropVtable` with the module's name                                          | false        |
| `BEGIN`, `COMMIT`, `ROLLBACK`                                             | `Transaction`                                                                                   | true         |
| `BEGIN IMMEDIATE`                                                         | `Transaction`                                                                                   | false        |
| `SAVEPOINT`, `RELEASE`                                                    | `Savepoint`                                                                                     | true         |
| `ATTACH`, `DETACH`                                                        | `Attach` with the file's name, `Detach`                                                         | true         |
| `PRAGMA` statements                                                       | `Pragma` with the pragma's name and its value or argument                                       | varies       |
| `PRAGMA writable_schema = 1`                                              | `Pragma`                                                                                        | true         |
| `PRAGMA journal_mode` and `PRAGMA user_version = 5`                       | `Pragma`                                                                                        | false        |
| Table-valued pragmas, such as `pragma_table_info('t')`                    | `Read` of a table named after the pragma, such as `pragma_table_info`, and no `Pragma`          | true         |
| `SELECT load_extension(...)`, `SELECT fts5_source_id()`                   | `Function` with the function's name                                                             | true         |
| `VACUUM`, `VACUUM INTO`                                                   | Nothing when prepared; at run time, `Attach` of the target or a temporary database              | false        |
| `EXPLAIN` and `EXPLAIN QUERY PLAN`                                        | The actions of the statement that they explain; `is_explain()` is 1 or 2                        | true         |

Further results:

- With `SQLITE_LIMIT_ATTACHED` at 0, `ATTACH`, `VACUUM` and `VACUUM INTO` fail with "too many attached databases - max 0", and `VACUUM INTO` creates no file. With the limit at 10 and an authorizer that denies `Attach`, `VACUUM` fails with "authorization denied".
- rusqlite's `prepare` refuses a second statement with "Multiple statements provided" and accepts a trailing comment or white space after the first.
- `SQLITE_DBCONFIG_DEFENSIVE` refuses a change of `sqlite_schema` under `writable_schema` ("table sqlite_master may not be modified"). A trigger that calls `load_extension` fails with "unsafe use of load_extension()", with trusted schema off and on, since SQLite allows the function only in top-level SQL.
- An in-memory copy of the schema, made by running the file's `CREATE` statements in order, classified an insert, an update, a delete through a view and an insert into an FTS5 table as the file did. The copy fails to create the FTS5 table's shadow tables, which its `CREATE VIRTUAL TABLE` already made, and its actions for the FTS5 insert lack the reads of the shadow configuration, with the same class.

What this means for the engine:

- A statement's class comes from its actions. Any creation, removal or change of a schema object, `REINDEX` or `ANALYZE` makes it a schema change. Otherwise any `Insert`, `Update` or `Delete` outside `sqlite_master` makes it a write. A read is a statement with neither, whose `readonly()` is true.
- The authorizer denies `Attach`, `Detach`, `Transaction` and `Savepoint` of every operation, the temporary database's objects, `CreateVtable` and `DropVtable`, and the function `load_extension`. It allows a `Pragma` only by name from a list of pragmas that read: `table_info`, `table_xinfo`, `index_list`, `index_info`, `index_xinfo`, `foreign_key_list`, `foreign_key_check`, `collation_list`, `page_count`, `page_size`, `freelist_count`, `encoding`, `user_version`, `application_id`, `schema_version`, `data_version`, `quick_check` and `integrity_check`, and only without a value, except the table or index that the first six name. Reads of table-valued pragmas pass, since SQLite offers that form only for pragmas without side effects.
- `readonly()` is true for transactions, savepoints, `ATTACH`, `DETACH` and `PRAGMA writable_schema = 1`, so a read passes the authorizer's denials as well as `readonly()`.
- The attach limit at 0 and the denial of `Attach` together refuse every form of `VACUUM`.

## 3. Opening other programs' files (S2)

| Check                                                                 | Result                                                                                                |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| A read-only open of a WAL database that another connection holds open | Opens, reads, and sees the other connection's later commits                                           |
| A read-only open of a WAL database that no connection holds           | Opens and reads, and creates `-wal` and `-shm`, which stay after it closes until a writer next closes |
| `file:<path>` without `SQLITE_OPEN_URI`                               | Read as a URI: the bundled build sets `SQLITE_USE_URI`, so the file of the path was created           |
| An extended-length path, `\\?\D:\...`                                 | Opens                                                                                                 |
| A symbolic link to a database with `SQLITE_OPEN_NOFOLLOW`, on Windows | Opens: Windows' layer of SQLite does not refuse links                                                 |
| A text file                                                           | Opens, and fails with "file is not a database" at the first read                                      |
| `PRAGMA foreign_keys` on a new connection                             | 1: the bundled build sets `SQLITE_DEFAULT_FOREIGN_KEYS`                                               |

What this means for the engine:

- It passes only absolute, canonical paths, refuses any path that starts with `file:`, and keeps `SQLITE_OPEN_URI` off.
- It refuses links itself on every system, by the link's metadata and by comparing the canonical path with the registered one, since `SQLITE_OPEN_NOFOLLOW` covers only Linux and FreeBSD.
- It reads SQLite's 16-byte header itself before it registers a file.
- It enforces foreign keys, as SQLite's default in this build does, and says so in the view: a write that breaks a foreign key fails, even where the file's own program turns them off.
- Reading a database in WAL mode can leave its `-wal` and `-shm` files in the owner's folder, as any reader of SQLite does, which the feature document says.

## 4. Interrupts, limits and memory (S3)

| Check                                                                         | Result                                                     |
| ----------------------------------------------------------------------------- | ---------------------------------------------------------- |
| An interrupt from another thread, 20 times, to an endless recursive statement | Ended in 19 µs at the median and 123 µs at most            |
| A statement of 5,000,000 steps, with and without a progress handler           | 565 ms and 574 ms: no cost that the run could measure      |
| A progress handler with a deadline 100 ms ahead                               | The endless statement ended after 100.1 ms, "interrupted"  |
| A sort of 1,000,000 rows of 100 bytes                                         | 986 ms, with 4 MiB of SQLite's heap at the high-water mark |

The engine bounds a statement by time, with a progress handler every 10,000 steps and an interrupt when its call is dropped, and by the rows and bytes that it returns. SQLite's sorter spills to temporary files, so a heavy statement costs time and disk more than memory, and the process-wide heap limit stays off.

## 5. Contention with the owner's applications (S4)

An application's connection, with a busy timeout of 500 ms, and Mannie's connection worked on one database in each journal mode.

| Check                                                          | Rollback journal                  | WAL                                                     |
| -------------------------------------------------------------- | --------------------------------- | ------------------------------------------------------- |
| The application writes while Mannie's read is open             | "database is locked" after 612 ms | Commits in 2 ms                                         |
| The application checkpoints while Mannie's read is open        | Not applicable                    | Busy: 533 of 534 frames moved, so the WAL cannot shrink |
| Mannie writes while the application holds a write transaction  | "database is locked" after 619 ms | "database is locked" after 630 ms                       |
| The application writes after Mannie read a page and dropped it | Commits                           | Not run                                                 |

What this means for the engine:

- A read returns one page and finishes its statement in the same call. Pages follow keys, so the next page needs no open statement.
- A write waits at most its busy timeout, then fails with `busy`, which the view and the tools word as another program writing to the file.

## 6. File identity (S5)

On Windows, the identity is the volume's serial number and the file's index, from `GetFileInformationByHandle`. It stayed the same after a write in place and changed after a save that wrote a copy and renamed it over the file. A file that a program saves this way shows as changed, and an approval whose identity no longer matches goes back to the policy, as ADR-0056 decides.

## 7. Large schemas (S9)

A database of 5,000 tables of 10 columns, each with an index, took 33 ms from the read-only open to the end of the first statement, which parses the schema. One statement that joins `sqlite_schema` with `pragma_table_info` read all 50,000 columns in 17 ms. The engine reads a database's schema in one statement, and the view counts rows only on request.

## 8. Open

- S6, the view's components and grid on WebView2 and WebKitGTK, runs with the view.
- S7, feature generations while a run uses a feature's tool, runs with the runtime's tests of plugin features.
- S8, the optional package with a worker of the previous release, runs with the link.
- The parts of S2, S4 and S5 on Linux and FreeBSD wait for the owner's machines: WAL databases in folders without write rights, network file systems, and the device and inode as identity.
