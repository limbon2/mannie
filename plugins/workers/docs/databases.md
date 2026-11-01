# Databases

|              |                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Milestone    | M15                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Last updated | 2026-11-02                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Part of      | [Workers](README.md)                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Related      | [ADR-0055](../../../docs/adr/0055-let-plugins-have-optional-features.md), [ADR-0056](../../../docs/adr/0056-let-plugins-work-with-the-owners-sqlite-files-through-the-core.md), [ADR-0057](../../../docs/adr/0057-let-the-master-work-with-the-databases-on-a-workers-host.md), [Plugins](../../../docs/features/plugins.md), [Host and ceiling](host-and-ceiling.md), [Pairing and the link](pairing-and-link.md), [Tools](../../../docs/architecture/tools.md) |

## Intent

The owner wants "first class support for an sqlite database for the harness", so that "mannie will have something of adminer in it", as "an optional subplugin for workers" ([D46](../../../docs/roadmap/milestones.md#5-decisions)). Databases is an optional feature of the Workers plugin with a switch of its own ([ADR-0055](../../../docs/adr/0055-let-plugins-have-optional-features.md)). The owner registers SQLite files that already exist, on this computer or on a paired worker, browses and edits them in a view like Adminer's, and agents query and change them under the master's policy and, on a worker, under the host's ceiling. Each database stays where it is: the owner considered rqlite and a replication of Mannie's own, and chose one home for each database instead.

## Behavior

### Turning it on

- The Plugins page lists Databases under Workers, with a sentence on what it adds, what it may use and a switch. It is off in every installation until the owner turns it on, and its switch is disabled while Workers is off.
- Turning it on takes effect at once: Databases appears in the activity bar, and replies that start from then on get the database tools and the list of databases. Turning it off removes them at once, asks first while a statement runs, and keeps every registration. Turning Workers off turns Databases off with it and keeps the owner's choice.
- Databases on this computer work with no paired worker.

### Registering a database

- "Add database…" asks where the database is. For this computer, "Choose file…" opens the system's file dialog, and only a file chosen there is registered. For a worker, the owner types the file's absolute path on that host.
- The owner names the database and may mark it read-only. A name is unique among all registered databases, ignoring case, so agents and the owner can name a database without its machine. A file that the owner leaves unnamed takes its file's name, with a number after it when another database has that name.
- Mannie refuses a file without SQLite's header, a file in Mannie's own folders or a worker's own folders, a network path, a link, and the companion files that end in `-wal`, `-shm` or `-journal`. A worker also refuses a file outside its allowed folders, unless the host is in full access, and every database while its configuration does not allow databases. Each refusal says what failed and what the owner can do, such as "This worker does not allow databases. Set `[databases] enabled = true` in its `worker.toml`."
- "Rename…" and "Remove" act on a registration. Removing a registration never touches the file.

### The view

- The sidebar of the Databases view lists the databases grouped by machine, this computer first and then each worker by its name. Each shows its state: available, worker offline, worker needs an update, file missing, file changed, or refused, with the reason.
- A database opens in the main area with its name, machine, path, size and read-only mark, and four tabs:
  - Tables lists the tables and views. "Count rows" counts a table's rows on request and stops after 10 seconds.
  - Structure shows a table's columns with their types, nullability, defaults and primary key, its indexes, foreign keys and triggers, and its CREATE statement.
  - Data shows a table in pages of 100 rows, in the order of its key or of the column that the owner sorts by. A filter of conditions on columns becomes a parameterized statement. Edit and Insert open a dialog with a field for each column and the kind of value it writes; saving writes only the changed columns, checks the row's old values and says when the row changed since it was read. Delete removes the selected rows after a confirmation. "Copy page as CSV" copies the page.
  - SQL runs one statement that the owner writes, with Ctrl+Enter or Run, and Stop while a read runs; a change runs to its end, within 30 seconds. A read shows its rows, a write the number of rows that it changed, and a schema statement runs after a confirmation that names it. The tab keeps the statements of the session.
- A read-only database shows no edit, insert, delete or write, and its SQL tab runs reads only.
- Mannie enforces foreign keys on every write, so a write that breaks one fails, even where the file's own program turns them off. A file that another program is writing fails with "busy" after 2 seconds, and Retry tries again. Reading a database in WAL mode can leave its `-wal` and `-shm` files beside it, as any reader of SQLite does ([M15 validation results](../../../docs/roadmap/m15-validation.md#3-opening-other-programs-files-s2)).
- Every part of the view has its loading, empty and error states, works from the keyboard and meets WCAG 2.1 AA. Long text shows cut and blobs with their size, and a cell opens its whole value in a dialog.

### Agents

- Every conversation and the general child agent see every registered database in an instruction section: its name, its machine, its state and whether it is read-only. The explore child agent, which gets the reading tools, reads the same list. "@" points an agent at a database and gives the model its schema summary.
- `describe_database` reads a database's schema, and `query_database` runs one read with parameters. Both run without asking. The model reads up to 200 rows within the result limit, and `read_tool_output` reads the whole result from its archive.
- `change_database` runs 1 to 16 statements with parameters in one transaction. A write asks, unless full access allows it or a grant covers the same statement on the same database. A schema change always asks, and no grant covers it. The approval shows the database, its machine, each statement and its parameters.
- The explore child agent gets the two reading tools. Each call shows in the transcript with its database, its statement and its result ([ADR-0039](../../../docs/adr/0039-let-plugins-describe-their-tool-calls-in-the-transcript.md)).
- On a worker, the host's ceiling decides too: a read-only host refuses every write, and an ask host needs the owner's Allow once for each write, which no grant replaces ([host and ceiling](host-and-ceiling.md)).

### Databases on workers

- A worker hosts databases only when its configuration allows them, with `[databases] enabled = true` in `worker.toml`. The master cannot change that.
- The owner's own changes from the view run on a worker unless the host is read-only. The worker records each statement in its own journal, and the master records the owner's operation or the agent's call.
- While a worker is offline, its databases say so and nothing runs there. A worker one minor version behind stays connected, and its databases say that it needs an update.

### What it does not do

- Mannie never creates a database, and never replicates, backs up or moves one ([backups and replicas](database-backups.md)).
- No export to files and no import from them in this version ([export and import](database-export-and-import.md)).
- The agents of worker tasks get no database tools ([database tools for worker tasks](database-tools-for-worker-tasks.md)).
- Only SQLite: other engines, SQL highlighting and statements that span several databases stay out.

## Fit

- Extension points: plugin features ([ADR-0055](../../../docs/adr/0055-let-plugins-have-optional-features.md)); the service `sqlite-files` and the database access ([ADR-0056](../../../docs/adr/0056-let-plugins-work-with-the-owners-sqlite-files-through-the-core.md)); tools, instruction sections and mention providers; link services, with the package `databases` ([ADR-0057](../../../docs/adr/0057-let-the-master-work-with-the-databases-on-a-workers-host.md)); IPC commands and channels; UI shell slots for the activity bar, the view, its sidebar panel, the dialogs and the tool kinds; the store for the copy of the workers' lists; messages under `workers_databases_`.
- Follows [ADR-0013](../../../docs/adr/0013-journal-every-side-effect.md) for the journal, [ADR-0028](../../../docs/adr/0028-run-assigned-tasks-under-the-host-ceiling.md) for the ceiling and [ADR-0029](../../../docs/adr/0029-let-the-owner-give-conversations-and-hosts-full-access.md) for full access.
- Must not change: what Workers does with Databases off, the link for workers without the package, and what system tools, delegation and the ceiling do with other accesses.

## Open questions

None.

## Decisions

- 2026-11-01: the owner decided ([D46](../../../docs/roadmap/milestones.md#5-decisions)):
  - Databases are SQLite files that already exist and that the owner registers. Mannie creates none.
  - Each database has one home, this computer or a paired worker, and nothing replicates it.
  - Databases is an optional feature inside Workers, with its switch under Workers on the Plugins page.
  - Every conversation and the general child agent see every registered database, and their reads run without asking.
  - The first version changes rows and schema. The view runs a schema statement after a confirmation, and an agent's schema change always asks with no grant that covers it.
  - The documents follow M14's commit, and building follows M14's release.
- 2026-11-01: chosen while shaping, for the owner's review:
  - On this computer, a database is registered only through the system's file dialog.
  - Names are unique among all registered databases.
  - Data pages hold 100 rows, the model reads up to 200 rows of a query, and `change_database` takes up to 16 statements.
  - Full access runs an agent's schema change, as it runs every operation ([ADR-0056](../../../docs/adr/0056-let-plugins-work-with-the-owners-sqlite-files-through-the-core.md), decision 7).
- 2026-11-01: the owner accepted ADR-0055, ADR-0056 and ADR-0057 and the defaults above, and asked to continue, so building started ([D47](../../../docs/roadmap/milestones.md#5-decisions)).

## Design

### The feature

- Workers' manifest declares `[features.databases]` with the service `sqlite-files`, so the service is bound only while Databases is on. The feature starts on Workers' running hub, which knows the paired workers and their links.
- On this computer, every call goes through `sqlite-files`. For a worker's database, the feature calls the worker over the package `databases`, and the worker uses the engine and the journal of `mannie-host-kit`.
- The feature's work in progress is its running statements, as the item `databases_running`, which the question before turning it off words.

### Contracts

- Tools, offered to conversations and the general child agent, with the two reading tools also for the explore agent:
  - `describe_database { database }`, with the effect `ReadsDatabases`.
  - `query_database { database, sql, parameters? }`, with the effect `ReadsDatabases`.
  - `change_database { database, statements: [{ sql, parameters? }] }`, with the effect `SideEffects`.
  - Each access is a database access with the class read, write or schema. The refusals carry codes, such as `unknown_database`, `worker_offline`, `worker_needs_update`, `read_only_database`, `not_one_statement`, `busy` and `row_changed`, which the tool kinds word.
- IPC, as feature commands of Workers: choose, register, rename and forget a database; follow the list of databases; read a schema; browse a table; count rows; run a read; and the owner's writes, a statement, a row edit, an insert and a deletion. Integers and keys travel as text. Their names settle when building starts.
- The link: the package `mannie.link.databases.v1` of [ADR-0057](../../../docs/adr/0057-let-the-master-work-with-the-databases-on-a-workers-host.md), optional, with its documents and fixtures, and `WorkerDetails.databases` in `Hello`.

### Data

- This computer's registrations and the owner operations live in the core's engine module, `dbfiles` of `mannie-store`, whose tables `dbfiles_registrations` and `dbfiles_operations` no plugin can reach, since the prefix is reserved ([ADR-0056](../../../docs/adr/0056-let-plugins-work-with-the-owners-sqlite-files-through-the-core.md)). A worker keeps its own in its store.
- Workers' table `workers_databases` holds the copy of each worker's list: the worker, the key, the name, the path for display, the file's identity, the read-only mark, the last known state and when it was read. Removing a worker removes its rows.
- The owner's choice is `features.workers.databases` in the core's settings section `plugins`.

### Interface

- The activity bar's Databases entry opens the view, whose sidebar panel lists the databases. The view's tabs, dialogs and confirmations use `table` and `checkbox`, which the pinned shadcn-svelte CLI adds, with the components already installed.
- Messages: `workers_feature_databases_name` and `workers_feature_databases_description` on the Plugins page, and `workers_databases_` for the view, the dialogs, the tool kinds and the refusals.

### Threat notes

- T-74 to T-81 in [security](../../../docs/architecture/security.md): a hostile database file meets a hardened engine; prompt injection can make an agent read out a registered database, since reads run without asking, so the owner registers only files that agents may read and marks the others read-only or leaves them out; a grant covers other parameters of the same statement; script in the webview can write through the view as the owner can, journaled as owner operations; statements can lock the owner's applications out of their files; a compromised master reaches every consenting worker's databases; a worker's forged rows are untrusted content; and script in the webview can register a worker's path only within the host's consent and allowed folders.

### Platform differences

- A file's identity is the volume and file index on Windows, and the device and inode on Linux and FreeBSD. Network paths are UNC paths on Windows and network file systems on Linux and FreeBSD, as spike S2 of M15 settles.
- Reading a database in WAL mode needs its `-shm` file, which a folder without write rights can lack (spike S2).

## Acceptance

- The Plugins page shows Databases under Workers, off in a new installation; turning it on shows the view and gives new replies the tools at once, and turning it off or turning Workers off removes them (`tests/e2e/specs/plugins.test.ts`, the plugin's end-to-end tests).
- Registering refuses each kind of file listed above, on this computer and on a worker (the engine's tests, link tests over loopback).
- The view browses, sorts, filters, edits with an old-value check, inserts, deletes after a confirmation and runs SQL, with a schema statement after a confirmation, from the keyboard with axe passing in both themes on WebView2 and WebKitGTK (the plugin's end-to-end tests against a fixture database).
- An agent's read runs without asking, a write asks and a grant covers the same statement, a schema change asks despite a grant, and a read-only database refuses writes (runtime and policy tests).
- On a worker: every row of the ceiling's table, a forged authority refused, a write that a dropped link interrupted runs once, and a worker of the previous release stays connected with its databases needing an update (link and compatibility tests, the owner's checks on Linux and FreeBSD).
- The owner's writes are journaled before they run and become uncertain after a crash (the engine's tests).
