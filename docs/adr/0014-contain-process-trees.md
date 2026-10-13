# ADR-0014: Contain command process trees on every platform

|          |                                                                                  |
| -------- | -------------------------------------------------------------------------------- |
| Status   | Proposed                                                                         |
| Date     | 2026-10-13                                                                       |
| Deciders | Owner                                                                            |
| Related  | [ADR-0013](0013-journal-every-side-effect.md), [Tools](../architecture/tools.md) |

## Context

- Commands start child processes, and those can start more. Stopping a command, or a timeout, has to end the whole tree, including children that detach on purpose with `setsid` or a double fork.
- `std::process::Child::kill` ends only the direct child.
- Antivirus products treat some ways of starting shells as suspicious, such as PowerShell's `-EncodedCommand`.

## Decision

1. One process supervisor in `mannie-tools` starts every command, and later every terminal.
2. Windows: each command runs in its own Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and stopping the command terminates the job.
3. Linux: each command runs in a new process group, and the supervisor is a child subreaper (`PR_SET_CHILD_SUBREAPER`), so detached descendants are re-parented to it. Stopping sends `SIGTERM` to the group, then `SIGKILL` after a grace period, and reaps the descendants.
4. FreeBSD: the supervisor becomes a reaper with `procctl(PROC_REAP_ACQUIRE)`, and stopping uses `PROC_REAP_KILL`.
5. On Windows, commands run in PowerShell with `-NoLogo -NoProfile -NonInteractive`, and the command text arrives through standard input or a temporary script file, never through `-EncodedCommand`. On Linux and FreeBSD they run in `/bin/sh -c`.
6. A child's environment is built from an allow list, so credentials never reach child processes.
7. Commands time out after 60 seconds by default and after 600 seconds at most. Output is capped per stream, and a cut is marked as truncated.

## Consequences

- Positive: stop and timeout end everything a command started.
- Negative: three platform-specific code paths, each tested on its own platform.

## Alternatives considered

| Option                                     | Why not chosen                            |
| ------------------------------------------ | ----------------------------------------- |
| Killing only the direct child              | Detached descendants keep running         |
| A separate helper executable for processes | Another binary to build, sign and talk to |

## Validation

V4.
