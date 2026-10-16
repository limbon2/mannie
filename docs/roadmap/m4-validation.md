# M4 validation results

|              |                                                                                                                                                                                                                                                                                                                                                                                         |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                                                                                                                                                                                                        |
| Last updated | 2026-10-16                                                                                                                                                                                                                                                                                                                                                                              |
| Related      | [Milestones](milestones.md#m4-workers), [M3 validation results](m3-validation.md), [Workers](../features/workers/README.md), [ADR-0016](../adr/0016-link-machines-over-pinned-mtls-grpc.md), [ADR-0027](../adr/0027-delegate-tasks-to-workers-through-a-runtime-port.md), [ADR-0028](../adr/0028-run-assigned-tasks-under-the-host-ceiling.md), [Security](../architecture/security.md) |

This document records what the checks and exit criteria of [M4](milestones.md#m4-workers) showed so far, so that the owner can review ADR-0010, ADR-0014, ADR-0016, ADR-0027, ADR-0028 and ADR-0029 and the defaults of [workers](../features/workers/README.md#decisions), and see what remains before v0.2.0, which carries M2, M3 and M4 ([D23](milestones.md#5-decisions)). The checks stay in the repository and run in CI. Each section names the checks, where they ran, the result, and what is open.

The runs below were made on 2026-10-16 on the owner's Windows 11 workstation (WebView2 155.0.4283.45), in a Linux container with `task test:linux`, which runs Clippy and the Rust tests without the desktop crate, and in the owner's virtual machine with FreeBSD 15.1, with the steps of the nightly workflow ([section 8](#8-freebsd)). The changes are not committed yet, and they were tested on top of 752908c, the commit that added child agents. Nothing of M4 ran in CI, on WebKitGTK, as an installed service on Windows or Linux, or with a live model: CI needs pushed branches, installing a service needs an administrator's terminal, and the evaluations need the owner's keys and endpoints. What is not run yet is marked so.

## 1. Summary

| Check or criterion                                                                              | Result                                                                                                                                                                        | Open                                                                   |
| ----------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| V6: pairing, codes and keys                                                                     | Passed: 10 link tests over loopback, and the end-to-end test with a real worker                                                                                               | None                                                                   |
| V6: a link lost during an operation causes no duplicate side effect                             | Passed: lost answers of Submit, Decide and Cancel, and a link that drops while a task runs, each with one effect and one report                                               | A lost answer of a lease delivery, which is idempotent by construction |
| V6: a worker one minor version behind; `buf breaking` guards contracts                          | Passed: level and version tests, the document fixtures of v0.2, and `scripts/check-link-contracts.mjs` with a planted breaking change                                         | A test with the previous release's worker, from v0.3.0                 |
| V6: `mannie-worker update` installs a signed build and rolls back                               | Passed: unit tests with a test key and a stand-in service manager                                                                                                             | A real update of an installed service; the release workflow's check    |
| V6: a leased credential never reaches a worker's disk or logs                                   | Passed: a planted credential is absent from the worker's data folder and logs, and the vault and the redactor are empty after the task                                        | None                                                                   |
| V4 on FreeBSD                                                                                   | Passed on FreeBSD 15.1: the process tree, conformance and supervisor tests, after a fix of the supervisor (section 13)                                                        | FreeBSD 14.4, in the nightly workflow or `freebsd.yml`                 |
| Delegated tasks pass the system tools suite                                                     | Passed on Windows, in the Linux container and on FreeBSD 15.1: every tool of the worker agent through a delegated task, checked on disk                                       | FreeBSD 14.4 in CI                                                     |
| The ceiling holds against the master's instructions                                             | Passed: the policy's tests of every row of D21, 7 runtime tests through the loop, a read-only host over the link, and a forged choice refused by the worker                   | None                                                                   |
| Full access ([ADR-0029](../adr/0029-let-the-owner-give-conversations-and-hosts-full-access.md)) | Passed on Windows: the policy's tests, 6 runtime tests through the loop and delegation, a link test, and 2 end-to-end tests, after a fix of one of the new tests (section 12) | Linux, FreeBSD and CI                                                  |
| Approval requests wait while the master is offline                                              | Passed: a request stays on the worker while the link is down, and the owner answers it once the master is back                                                                | None                                                                   |
| The service setups pass smoke tests                                                             | Passed on FreeBSD 15.1 with the rc.d service, after a fix of the rc.d script; on Windows the same path ran with a worker in a terminal                                        | `scripts/ci/worker-service-*` in CI on Windows and Linux               |
| End-to-end tests                                                                                | All 16 spec files pass on WebView2, the new workers spec with a real worker, after the settings list of the shell spec gained Workers                                         | WebKitGTK in CI                                                        |
| NFR-PERF-05, an idle worker under 50 MB                                                         | Met on Windows, 16.5 MB of working set and 7.7 MB private, and on FreeBSD 15.1, 27.5 MiB resident                                                                             | Linux, and the reference machine                                       |
| Linux                                                                                           | Passed: `task test:linux`, Clippy and every Rust test without the desktop crate                                                                                               | CI, and the desktop app on WebKitGTK                                   |
| ADR-0010, ADR-0014, ADR-0016, ADR-0027, ADR-0028, ADR-0029                                      | Proposed; the checks above settle them, with FreeBSD run on 15.1                                                                                                              | CI, and the owner's review                                             |
| v0.2.0 released                                                                                 | Not yet                                                                                                                                                                       | v0.1.0 comes first                                                     |

## 2. Pairing and the link

Checks: `crates/mannie-link/tests/pairing.rs`, 10 tests over loopback with a master and a worker in one process, and the unit tests of the keys, the codes, the proofs and the TLS limits:

- A good code pairs, and both sides pin each other's keys; a wrong code is refused, and the sixth attempt ends the code.
- An expired code is refused and deleted, and so is a code made after now, when the clock went back.
- A paired worker refuses another master, and a code works once.
- A relay in the middle with a key of its own never pairs either side.
- A key that is not the pinned master's reaches only pairing.
- Removing a reachable worker unpairs it for another master, and pairing without a credential store is refused.
- A worker whose key changed shows `key_changed` and gets no work. This test found a defect: the error of the TLS verifier arrived wrapped in an `io::Error`, whose source skips the wrapped error, so the master showed such a worker as offline. The walk through the error's sources now reads the wrapped error.
- A handshake that stalls is cut after its time and holds its place among the handshakes only until then.

Result: all pass on Windows and Linux.

## 3. Delegation and lost answers

Checks:

- `crates/mannie-runtime/tests/delegation.rs`, 18 tests through the runtime with an in-process port, 2 of them of full access ([section 12](#12-full-access)), and `delegation_crash.rs`, which ends the master after the part, after the row and after a submission whose answer was lost: no case starts a second task.
- `crates/mannie-link/tests/delegation.rs`, 8 tests over the link: a task reports and its lease leaves no trace, the worker refuses a choice that its request does not offer, the task's conversation is read through the link, the worker stops while the master follows a task, a link that drops during a task comes back and the end arrives once, an approval waits while the master is offline, versions, and a request that a conversation in full access answers ([section 12](#12-full-access)).
- `crates/mannie-link/tests/link_drop.rs`, 3 tests where a relay loses the worker's answers until the call took effect, then drops the link: a lost Submit finds the task that the worker runs, a lost Decide runs its call once, and a lost Cancel ends the task once.

The tests found that a worker's state of a task that ended, which comes from its stored row after a reconnect, reset the task's count of tool calls on the master; a task's count now only grows.

Result: all pass on Windows and Linux.

## 4. The ceiling

Checks: the policy's unit tests of every row of [D21](milestones.md#5-decisions), with narrowing, protected folders, several accesses and metacharacters; `crates/mannie-runtime/tests/ceiling.rs`, 8 tests through the loop, one of them of a host in full access ([section 12](#12-full-access)); and `crates/mannie-link/tests/delegation.rs`, where a read-only host refuses a write of a delegated task whose instruction asks for it, and a choice of "Allow always" from the master is refused by the worker.

Result: all pass on Windows and Linux.

## 5. Credential leases

Checks: `crates/mannie-link/tests/delegation.rs`, where the master's profile holds a planted credential: the worker's run used the lease, and afterwards no file of the worker's data folder holds the credential, the lease vault is empty, and the redactor forgot it. The runtime's `assigned_tasks.rs` covers a run that waits for its lease, a revoked lease that ends the run, and unpairing that drops every lease.

`crates/mannie-runtime/tests/delegation.rs` checks that the master delivers a task's lease once, with a submission whose answer comes only after the worker reported that the task waits for its lease, and that it revokes the lease when the task ends.

Result: all pass on Windows and Linux.

## 6. Compatibility

Checks:

- `crates/mannie-link/src/levels.rs`: both sides use the lower level of each package, every package of the contracts has a level, and a worker of the same minor version or one behind works.
- `crates/mannie-link/tests/delegation.rs`: a master of 0.5.1 works with workers of 0.5.0 and 0.4.9, and shows workers of 0.3.2 and 0.6.0 as incompatible, which get no task.
- `crates/mannie-link/tests/compatibility.rs`, with fixtures of the 15 documents that the link carries, as v0.2 writes them: this version writes them exactly, reads every older version's without losing a field, ignores a field that a later version adds in any object, and knows the refusal codes of every version.
- `scripts/check-link-contracts.mjs`, which `task lint:proto` runs: `buf breaking` against the merge base with `origin/main` and against the newest release that has the contracts. Its tests plant an addition, which passes, and removals before and after a release, which fail. `buf lint` and `buf format` pass.

Result: all pass. The merge base with `origin/main` has no contracts yet, so the check had nothing to compare with in this repository.

Not run yet: a test with the previous release's worker, which v0.3.0 brings.

## 7. The worker service and updates

Checks:

- `apps/worker`, 33 unit tests: the configuration with every key, its defaults, a newer version, unknown keys, values out of range and missing folders; the command line; whether the worker's account can change its configuration, also with an access list on Windows; `self-check --json` and its shape; the systemd unit and the rc.d script; and `update` with a test key and a stand-in service manager: a valid and a changed archive, another key, a missing signature, a new version that starts, one that does not start and is rolled back with its database, a program for another system, and an archive without the program.
- `apps/worker/tests/foreground.rs`: the program starts, logs and stops, and refuses a database from a newer version, a configuration with an unknown key and a configuration that its own account can change.
- The same path as the smoke tests, with a worker in a terminal on Windows: `mannie-worker status` and `pair` against its configuration, then the test master of `crates/mannie-link/examples/smoke-master.rs` paired with the code, delegated a command, allowed it, checked the report and the command's output, and removed the worker, after which `status` said that the worker was not paired. No file of the worker's data folder held the planted credential.
- `self-check --json` of the debug build reports the SHA-256 of the committed release key, which `node scripts/release.mjs fingerprint` computes the same.

Result: all pass on Windows; the unit tests also in the Linux container, with the hardening test there.

Not run yet: `scripts/ci/worker-service-unix.sh` on Linux and `worker-service-windows.ps1`, which install the service with the system's folders and need CI or an administrator's terminal; a real `update` of an installed service; and the release workflow's `update --check` of every signed archive. The Unix script ran on FreeBSD ([section 8](#8-freebsd)). Like the [setup guide](../guides/worker-setup.md), both scripts allow a work folder and restart the service before they pair, since a delegated command runs in an allowed folder.

## 8. FreeBSD

The steps of the nightly workflow ran in the owner's virtual machine: FreeBSD 15.1 on amd64 with 8 processors and 4 GB of memory, and Rust 1.99.0 from rustup, as `rust-toolchain.toml` pins it. They ran the tests of `mannie-worker`, `mannie-store`, `mannie-logging`, `mannie-types` and `mannie-tools`, then those of `mannie-provider`, `mannie-runtime` and `mannie-link` with their test features, built the worker and the test master, and ran `scripts/ci/worker-service-unix.sh`.

Checks:

- V4: `crates/mannie-tools/tests/process_tree.rs`, where a background process, a process that `daemon(8)` puts in a new session, a double fork, `nohup` and a process that ignores `SIGTERM` end with the command at its end, at its timeout and by Stop; the supervisor's unit test that a shell keeps its reaper status across `exec`; and the conformance suite of the system tools.
- The runtime and the link: every suite of `mannie-runtime` and `mannie-link`, among them `delegated_conformance.rs`, which runs every tool of the worker agent through a delegated task, the pairing, delegation and lost-answer tests, and the crash tests.
- The worker: its unit tests, with the hardening test and the tests of the rc.d script, and `tests/foreground.rs`.
- The service: `scripts/ci/worker-service-unix.sh` installed the worker as the rc.d service `mannie_worker` under a new account; checked that the account runs it with a configuration and a program that only root can change and a private data folder; allowed a work folder and restarted the service; paired it with the test master, which delegated a command and checked its output; and removed it.

Result: all 47 suites and the service's smoke test pass, after the four fixes that FreeBSD's first runs called for ([section 13](#13-defects-found), defects 5 to 8). Clippy for FreeBSD with warnings denied passes for `mannie-tools`, `mannie-runtime`, `mannie-link` and `mannie-worker`.

The owner then installed the worker as a service following the [setup guide](../guides/worker-setup.md) and paired it with the desktop app, which delegated tasks to it with a live model. A task's command ran on the worker after the owner allowed it in the card. Two tasks failed as they should while the host had no allowed folder: commands without a working directory had no workspace, and the worker's account could not create a folder under `/`. With `/srv/work` allowed, the account writes there. The service did not stop while the desktop was connected, defect 9; with the fix, `service mannie_worker restart` completed at once with the desktop connected, and the desktop connected again.

Not run yet: FreeBSD 14.4, the oldest supported release, which the nightly workflow and `freebsd.yml` use.

## 9. The interface

Checks: the Vitest tests of the workers' list and actions and of the messages, the updated tests of the task parts and the messages of child agents, and `tests/e2e/specs/workers.test.ts` with a real worker: the empty settings, pairing with a wrong code and with the host's code, the worker's details, a delegated task's card with the worker's name and its request on the worker, allowed in the card, the task's conversation live in a side panel, Stop, a worker that stops and comes back, and removal after asking. axe finds no violation in the sidebar's group, the dialogs, the settings, the card with its request, or the side panel, in either theme, and the pseudo-locale shows only catalog text on the Workers settings.

Result: all pass on WebView2.

## 10. End-to-end tests

The whole suite ran on WebView2 with the validation build, which tries to reach a worker again after at most 2 seconds, and with the worker that `task e2e` now builds beside it.

Result: 15 of the 16 spec files passed at once. The shell's spec failed only because its fixed list of settings pages had no Workers page; with the list updated, it passes too. After the fix of the lease deliveries ([section 13](#13-defects-found)), all 16 spec files, 126 tests, passed in one run.

Not run yet: WebKitGTK, in CI.

## 11. Performance

NFR-PERF-05: an idle worker uses less than 50 MB. The release build ran with a configuration in a temporary folder, unpaired and with no task, and was sampled five times, two seconds apart, after 20 seconds.

Result: on Windows, 16.5 MB of working set and 7.7 MB of private memory in every sample. On FreeBSD 15.1, in the virtual machine of [section 8](#8-freebsd), 28,144 KiB (27.5 MiB) of resident memory in every sample, which includes the pages of the shared libraries.

Not run yet: Linux, and the reference machine.

## 12. Full access

[ADR-0029](../adr/0029-let-the-owner-give-conversations-and-hosts-full-access.md) added full access at the owner's request ([D22](milestones.md#5-decisions)), after the runs of the sections above. Its checks ran on Windows.

Checks:

- The policy's unit tests: full access allows every operation on the master, Mannie's own folders included; a conversation's full access is no input of a worker's ceiling; a host in `full_access` runs every path and command; and a read-only task narrows such a host.
- `crates/mannie-runtime/tests/full_access.rs`, 3 tests through the loop: a conversation that the Tools settings start in full access writes a file and looks at Mannie's database without asking, each call's decision is `full_access`, and its model reads that nobody checks its operations; the switch applies from the next call, and once it is off a change asks again; and a child agent follows its parent's full access and has no switch of its own.
- `crates/mannie-runtime/tests/delegation.rs` and `ceiling.rs`: a conversation in full access answers its worker task's request, which the worker records as `full_access`; a host whose configuration refuses the master's full access waits for the owner; and a host in `full_access` runs everything without asking.
- `crates/mannie-link/tests/delegation.rs`: a conversation in full access answers a worker's request over the link.
- The worker's configuration test of `full_access` and `allow_full_access_from_master`, the settings test that a section from before full access reads with it off, and the Vitest tests of the switch and its dialog.
- `tests/e2e/specs/tools.test.ts`, 2 tests: the switch asks first, then the toolbar and the sidebar mark the conversation, a change and a command run without a request, and once it is off a change asks again; and the Tools setting asks first, then a new conversation starts in full access. axe finds no violation in the dialogs or with the marks, in either theme.

Result: all pass on Windows. In the first end-to-end run, the spec of the system tools failed after the new tests: the second of them left its conversation in full access, so the test after it, which waits for a command's request, saw the command run without one. The test now turns the conversation's full access off at its end, and the spec passes. The other 15 spec files passed in that run.

Not run yet: Linux, FreeBSD and CI.

## 13. Defects found

1. A worker whose key changed showed as offline (section 2). Fixed, with a test.
2. A worker task's count of tool calls fell to zero when the master read the task's end from the worker after a reconnect (section 3). Fixed.
3. A missing host instructions file logged a warning for every task, although a missing file means that there are none. Fixed.
4. The master delivered a task's first lease twice, once from the run that submitted the task and once from the follower of its worker, and did not revoke a lease when its task ended, as [credential leases](../features/workers/credential-leases.md) says it does (section 5). Fixed, with a test that saw three deliveries without the fix. The fix came after the run in the Linux container, so its tests ran on Windows and FreeBSD only.
5. On FreeBSD, the supervisor's tests of Stop and of the timeout panicked. rustix 1.1.5's `reaper_kill` makes a process ID of the kernel's `rk_fpid`, which is -1 when every process was signaled, and a debug build asserts that an ID is not negative; a release build would have reported process -1 as one that could not be signaled. The supervisor now calls `procctl(PROC_REAP_KILL)` through libc, and logs a process that really could not be signaled (section 8).
6. On FreeBSD, the worker did not turn off tracing, so another process of its account, such as a command that the owner approved, could have read a lease from its memory. rustix 1.1.5's `set_dumpable_behavior` passes the value where procctl(2) expects a pointer to it, and the call fails with `EFAULT`, which the worker only logged. The worker now calls `procctl(PROC_TRACE_CTL)` through libc. The hardening test found it (section 8).
7. The rc.d service did not start. The script named the account `mannie_worker_user`, which rc.subr reads as the account to run the command itself as, through su(1), so daemon(8) could neither write its pid file nor start the worker with `-u`. daemon(8) also kept the pipes that `service start` inherited, so `service install` waited forever. The account is now `mannie_worker_account`, daemon(8) gets `-f`, and a test refuses the old name (section 8).
8. The crash tests counted the command's marker right after they killed Mannie, although on Linux and FreeBSD a command that outlives Mannie runs until the recovery ends it at the next start, as [commands](../features/system-tools/commands.md#crash-recovery) says. On FreeBSD the command wrote its marker after the count, and `task_crash.rs` reported a second run that did not happen; `tool_crash.rs` had the same race. Both now count once the command's shell is gone.
9. With a master connected, the worker never stopped: `service mannie_worker stop` and `restart` waited forever, and so would `update` and `service uninstall`, on every system. The link's graceful stop waited for the streams that a master keeps open, its watch of the tasks and its subscriptions, which end only when the runtime stops, and the worker stopped the runtime only after the link. The link now ends those streams when it stops, and the worker waits at most 10 seconds for the link's calls before it closes the connections. A link test stops the worker while a master follows a task, and failed before the fix (section 8).

Defects 5 and 6 are in rustix 1.1.5 and are not reported to its project yet.

## 14. Decisions for the owner

- The defaults recorded in the [workers feature](../features/workers/README.md#decisions), among them the port, the folders, the choices of a worker's requests, the budgets, and Stop and quit for worker tasks.
- ADR-0010, ADR-0014, ADR-0016, ADR-0027, ADR-0028 and ADR-0029, for acceptance once CI ran; FreeBSD ran on 15.1 ([section 8](#8-freebsd)).
- The release order of v0.1.0 and v0.2.0, and pushing the branches, so that CI, the FreeBSD jobs and the service smoke tests can run.
