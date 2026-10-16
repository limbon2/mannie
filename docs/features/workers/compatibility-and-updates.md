# Compatibility and updates

|              |                                                                                                                                                                                                                                                         |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                |
| Milestone    | M4                                                                                                                                                                                                                                                      |
| Last updated | 2026-10-16                                                                                                                                                                                                                                              |
| Part of      | [Workers](README.md)                                                                                                                                                                                                                                    |
| Related      | [ADR-0016](../../adr/0016-link-machines-over-pinned-mtls-grpc.md), [ADR-0018](../../adr/0018-release-from-tagged-builds.md), [Releasing](../../engineering/releasing.md), [Pairing and the link](pairing-and-link.md), [The worker service](service.md) |

## Design

### Versions

- The master and the worker exchange their versions in `Hello`. The master works with a worker on its own minor version, and on the minor version before it; patch versions do not matter.
- A worker that is older than that, or newer than the master, gets the status `incompatible`: the master shows it with its version and the version to install, and offers it no tasks. A worker newer than the master means the master needs the update.
- The documents of the runtime that travel as JSON, such as task states, have no variant for values they do not know, so the version rule above is what keeps an older master from meeting a newer worker's values.

### Feature levels

Each package of the link has a feature level, 1 for every package in v0.2.0. In `Hello`, each side lists the levels it supports, and the master uses the lower level of each package. A feature checks the agreed level before it uses a call that a later level added. A test checks that every package of `plugins/workers/proto/` has a level.

### Contract checks

- `buf lint` with buf's STANDARD rules checks every change to `plugins/workers/proto/`, and `buf format` formats it.
- `scripts/check-link-contracts.mjs` runs `buf breaking` against the merge base with `origin/main` and against the newest release tag that has the contracts, so a change can only add. CI checks out the whole history for it.
- The JSON documents that travel on the link have fixtures in `plugins/workers/crates/mannie-link/tests/fixtures/documents/`, one folder for each minor version that changed one, starting with `v0.2`. `tests/compatibility.rs` checks that this version writes the newest fixtures exactly, reads every older fixture without losing a field, ignores a field that a later version adds in any object, and knows every refusal code of an older version. A release that changes a document adds a folder with every document; the older folders never change.
- v0.1.0 has no link, so in v0.2.0 a worker one minor version behind is shown by tests that simulate its levels and versions, the contract checks and the fixtures. From v0.3.0 on, a test runs the previous release's worker.

### Updating a worker

The owner downloads the new release's archive for the host's system and its signature (`<archive>.sig`) from the release, and runs, with root or administrator rights:

```sh
mannie-worker update mannie-worker-0.2.1-linux-x86_64.tar.gz
```

1. `update` reads the archive once into a private copy, so the file cannot change between the check and the use, and checks the signature against the release key built into the running program ([releasing](../../engineering/releasing.md#6-signing)). An archive without a valid signature is refused.
2. It unpacks the program with the system's `tar` (bsdtar on Windows and FreeBSD, GNU tar on Linux) into a private folder, runs the new program's `self-check --json`, and reads its version and target. A program for another system, or one that does not run, is refused. A version older than the installed one is installed too, after a warning.
3. It stops the service, copies the database files, puts the new program in the program's folder and keeps the previous one beside it.
4. It starts the service and waits up to 60 seconds for the health marker of the new version, which the program writes once it opened its store and listens. Then it deletes the database copy, which holds the task conversations.
5. When the marker does not come, it stops the service, restores the previous program and the database copy, starts the service again, and says what failed.

`update --check <archive>` does only the first step. The release workflow runs it on every signed archive, which proves that the key in the released program is the release key. `self-check --json` keeps its shape in every version, since an older `update` reads a newer program's output.

### Threat notes

- T-9: an update is installed only with a valid signature of the release key; the owner runs it on the host with administrator rights, and the program's folder belongs to root or Administrators, so the worker's account cannot replace its program.
- The archive is read once, so a change of the file after the check does not reach the installed program.

## Acceptance

- `update` installs a signed build, and restores the previous program and the database when the new build does not start (V6; worker tests with a test key and a stand-in service manager).
- `update` refuses an archive with a wrong or missing signature, and one for another system (worker tests).
- The released archives pass `update --check` with the released program (the release workflow).
- Levels and versions: a worker one minor version behind works at the levels both know, a newer or an older worker is shown as incompatible and gets no task, and the JSON documents match their fixtures (V6; link tests).
