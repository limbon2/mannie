# ADR-0014: Contain command process trees on every platform

|          |                                                                                  |
| -------- | -------------------------------------------------------------------------------- |
| Status   | Accepted                                                                         |
| Date     | 2026-10-13                                                                       |
| Deciders | Owner                                                                            |
| Related  | [ADR-0013](0013-journal-every-side-effect.md), [Tools](../architecture/tools.md) |

Later changes: [ADR-0036](0036-let-plugins-run-commands-on-remote-hosts.md) refines decisions 1 and 8: a command on a remote host is outside the supervisor, and Stop and timeout ask the host to end it. [ADR-0038](0038-let-plugins-run-installed-programs-in-the-owners-projects.md) refines decisions 1, 5 and 7: the supervisor also runs an installed program without a shell, with its output as bytes.

## Context

- Commands start child processes, and those can start more. Stopping a command, or a timeout, has to end the whole tree, including children that detach on purpose with `setsid` or a double fork.
- `std::process::Child::kill` ends only the direct child.
- Antivirus products treat some ways of starting shells as suspicious, such as PowerShell's `-EncodedCommand`.

## Decision

1. One process supervisor in `mannie-tools` starts every command, and later every terminal.
2. Windows: each command runs in its own Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and stopping the command terminates the job.
3. Linux: each command runs in a new process group, under a shell that is a child subreaper (`PR_SET_CHILD_SUBREAPER`, set between `fork` and `exec`, which keeps it), so detached descendants are reparented to that shell and stay in the command's tree. Stopping sends `SIGTERM` to every process of the tree, then `SIGSTOP` and `SIGKILL` after a grace period until none is left, and then ends the shell. Mannie itself is not a subreaper, so it never adopts the orphans of processes that are not commands, such as the webview's.
4. FreeBSD: the supervisor becomes a reaper with `procctl(PROC_REAP_ACQUIRE)`, and stopping uses `PROC_REAP_KILL`.
5. On Windows, commands run in Windows PowerShell with `-NoLogo -NoProfile -NonInteractive`, and the command text arrives through standard input, where a fixed bootstrap reads it and runs it as one script block, never through `-EncodedCommand`. On Linux and FreeBSD they run in `/bin/sh -c`, through a fixed wrapper that reports the command's status on a descriptor of its own, so the supervisor learns that the command ended while the shell still holds its tree.
6. A child's environment is built from an allow list, so credentials never reach child processes.
7. Commands time out after 60 seconds by default and after 600 seconds at most. Output is capped per stream, and a cut is marked as truncated.
8. When a command ends on its own, the processes it leaves behind end too, on every platform, so nothing a command starts outlives it.

## Consequences

- Positive: stop and timeout end everything a command started.
- Negative: three platform-specific code paths, each tested on its own platform. A command cannot start a server or an application that keeps running after it.
- Refinement in M2: decisions 3 and 5 were made concrete while V4 was built. Making Mannie itself the subreaper, as first proposed, would have made it adopt every orphan of its webview and of the programs that open links.
- Refinement in M4: decision 4 is made concrete as decision 3 is. The shell becomes the reaper between `fork` and `exec`, which keeps the status, so the worker never adopts orphans that are not a command's, and stopping sends the signals of decision 3 to the shell's reaped descendants through `PROC_REAP_KILL`. V4 runs on FreeBSD in the nightly workflow and for every pull request that changes the supervisor ([M4 validation results](../roadmap/m4-validation.md)).

## Alternatives considered

| Option                                     | Why not chosen                            |
| ------------------------------------------ | ----------------------------------------- |
| Killing only the direct child              | Detached descendants keep running         |
| A separate helper executable for processes | Another binary to build, sign and talk to |

## Validation

V4.
