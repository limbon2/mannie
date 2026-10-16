# Credential leases

|              |                                                                                                                                                                                                                                                                                          |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                 |
| Milestone    | M4                                                                                                                                                                                                                                                                                       |
| Last updated | 2026-10-16                                                                                                                                                                                                                                                                               |
| Part of      | [Workers](README.md)                                                                                                                                                                                                                                                                     |
| Related      | [ADR-0010](../../adr/0010-keep-provider-credentials-on-the-master.md), [ADR-0028](../../adr/0028-run-assigned-tasks-under-the-host-ceiling.md), [Workers architecture](architecture.md#model-access-on-workers), [Security](../../architecture/security.md#7-rules-for-handling-secrets) |

## Design

### The profile of a task

A worker stores no provider profile. Each task carries a snapshot of the profile of the reply that delegated it: its ID and revision, the API interface, the base URL, the model, the context window and output limit, the reasoning effort, the quirk profile's server and whether tools are used. The snapshot has no credential, only whether the profile has one. The binding of provider-private data, such as encrypted reasoning, keeps the master's profile and revision, so that data is replayed only to the same profile.

A base URL with a loopback address names the master's own computer, which a worker on another host cannot reach and must not send a credential to. The master refuses such a delegation (`profile_not_reachable`), unless the worker's own address is a loopback address too, as in tests on one machine.

### The lease

1. The master reads the profile's credential from its credential store when the worker has stored the task.
2. It delivers the lease, the task's ID, the profile's binding and the credential, over the link.
3. The worker keeps the lease in its lease vault, in memory, and refuses a lease whose binding is not the task's.
4. A run of the task reads the credential from the vault under the reference `lease/<task>`, as a run on the master reads its profile's credential, and keeps it while it calls the provider.

A run that starts before its lease arrived waits with the activity `waiting_for_lease`, and continues once the vault holds it. While it waits, the run holds its slot and no model slot.

The run that submitted the task and the master's follower of the worker both offer the lease, and the master delivers it once. After a failed delivery, the next state of the task that waits for the lease delivers it again.

### When the worker drops a lease

| Event                        | What happens                                                                             |
| ---------------------------- | ---------------------------------------------------------------------------------------- |
| The task ends                | The vault drops the lease. The master also revokes it, which finds nothing to drop       |
| The master revokes the lease | The vault drops it, and the task's run ends, since the run keeps its copy while it works |
| The worker is unpaired       | Every lease is dropped, and every task is canceled                                       |
| The worker restarts          | Nothing survives in memory. A follow-up gets a new lease                                 |

When a lease is dropped, its memory is cleared, and the log redactor forgets the credential, which it redacted from every log line while the lease lived.

### Where a lease never goes

A lease never reaches the worker's database, its logs, its crash files, a command's environment or arguments, or a transcript. The worker turns off core dumps everywhere, and tracing by other processes of its account on Linux and FreeBSD ([hardening](service.md#hardening)).

### Contracts

- `Leases.Deliver` and `Leases.Revoke`, and in a task's state whether its run waits for a lease.
- The credential vault of the runtime can tell a run when a credential arrives.
- The redactor's scoped registrations: a registration lasts while one of its holders lives.
- The run failures `lease_revoked` and `credential_missing` for a task whose lease never came.

### Threat notes

- T-10: anyone who controls the worker host during a task can read the lease. Leases are per task and memory-only, revoked at the task's end and at unpairing, and the guide recommends scoped provider keys with spending limits for profiles that workers use.
- T-15 on a worker: a credential travels over plain HTTP only to the worker's own loopback or a private address, as on the master, and a loopback URL from the master is refused for another machine.

## Acceptance

- A planted credential delivered as a lease appears in no file of the worker's data folder and in no log line, after a task that ran a command and wrote files (V6; link tests).
- A lease is dropped when its task ends, when the master revokes it, which ends the task's run, and when the worker is unpaired, and the redactor's registrations return to none (V6; link and runtime tests).
- A run that starts before its lease waits, and continues when it arrives (runtime tests).
- The master delivers a task's lease once, also when the worker reports that the task waits for it before the submission's answer arrives, and revokes the lease when the task ends (runtime tests).
- A delegation whose profile points at the master's loopback is refused for a worker on another machine (runtime tests).
