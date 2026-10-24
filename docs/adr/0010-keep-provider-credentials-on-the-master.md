# ADR-0010: Keep provider credentials in the master's credential store and lease them to workers per task

|          |                                                                                                                                                             |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted                                                                                                                                                    |
| Date     | 2026-10-13                                                                                                                                                  |
| Deciders | Owner                                                                                                                                                       |
| Related  | [ADR-0015](0015-run-an-agent-runtime-on-every-machine.md), [ADR-0016](0016-link-machines-over-pinned-mtls-grpc.md), [Security](../architecture/security.md) |

Later changes: [ADR-0047](0047-serve-embeddings-and-reranking-through-provider-profiles.md) refines decisions 1, 3 and 4: the credentials of search profiles are stored, referenced and held as the credentials of chat profiles.

## Context

Verified 2026-10-13:

- Provider API keys are the most sensitive data Mannie handles.
- Worker agents run their own loops ([ADR-0015](0015-run-an-agent-runtime-on-every-machine.md)), so they need model access for the length of a task, including while the master is offline.
- Worker hosts are often headless and may have no credential service. FreeBSD has none in its base system.
- The `keyring` crate 4.x is now a thin wrapper with sample code. Its maintainers recommend that applications depend on `keyring-core` 1.0.1 and the stores they need: `windows-native-keyring-store` 1.1.0 for Windows Credential Manager, and `zbus-secret-service-keyring-store` 1.0.1 for the Linux Secret Service (GNOME Keyring, KWallet).

## Decision

1. Provider credentials are stored only in the master's operating system credential store, through `keyring-core` with the Windows native store or the Secret Service store. Entries are named under the bundle identifier `io.github.limbon2.mannie`.
2. When no credential store is available, Mannie refuses to save the credential and explains how to enable one. It never falls back to a file or to its database.
3. The database and the settings hold only a reference to each credential.
4. In memory, a credential is wrapped in a type that clears its memory when dropped and redacts itself when formatted. It never appears in logs, URLs, command arguments, child process environments, transcripts or error messages.
5. Workers never store provider credentials. For each task, the master leases the credential of the task's provider profile over the link. The worker keeps it in memory until the task ends, is canceled, or the worker is unpaired, and never writes it to disk, to its logs or to its database.
6. The documentation recommends scoped keys with spending limits for provider profiles that workers use.
7. Storing credentials on a worker for unattended work needs its own ADR.

## Consequences

- Positive: keys at rest exist only in stores that the operating system protects, and a worker holds a key only while it works on a task.
- Negative: anyone who controls a worker host during a task can read the leased key (risk R7), and Linux desktops need a running Secret Service.
- Refinement in M4: a lease belongs to one task and lives in the worker's lease vault, which the task's runs read like any credential vault ([credential leases](../../plugins/workers/docs/credential-leases.md)). The vault drops a lease when its task ends, when the master revokes it, which ends the task's run, and when the worker is unpaired, and the log redactor forgets it then. The worker turns off core dumps and, where the system allows it, tracing by other processes of its account.

## Alternatives considered

| Option                                                | Why not chosen                                                               |
| ----------------------------------------------------- | ---------------------------------------------------------------------------- |
| Relay every model call of a worker through the master | Workers would stop whenever the master sleeps, which contradicts decision D5 |
| An encrypted file protected by a password             | Adds a password to manage, and the file still sits on every host             |
| Credentials in the database                           | Plaintext at rest                                                            |

## Validation

V7 covers the credential stores on Windows and Linux, including the behavior without a Secret Service. V6 shows that a leased credential never reaches a worker's disk or logs, and that the worker drops it when the task ends or the worker is unpaired.
