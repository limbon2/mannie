# Commands

|              |                                                                                                                                                                                                                   |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                          |
| Milestone    | M2                                                                                                                                                                                                                |
| Last updated | 2026-10-14                                                                                                                                                                                                        |
| Part of      | [System tools](README.md)                                                                                                                                                                                         |
| Related      | [ADR-0014](../../adr/0014-contain-process-trees.md), [Tools](../../architecture/tools.md#6-running-commands), [Security](../../architecture/security.md), [Operations and approvals](operations-and-approvals.md) |

## Design

### The shell

- `run_command` runs the command text in the platform's shell, in the working directory, with a timeout of 60 seconds by default and 600 at most.
- Windows: Windows PowerShell 5.1, `powershell.exe` from the system folder, never from `PATH`.
- Linux: `/bin/sh`.
- FreeBSD: `run_command` arrives with workers in M4, together with its validation of V4. Until then the tool is neither built nor registered there.
- One process supervisor in `mannie-tools` starts every command ([ADR-0014](../../adr/0014-contain-process-trees.md)). Its platform modules are the only code in Mannie that uses `unsafe`, and each unsafe block says why it is sound.

### Windows

- The command line is `powershell.exe -NoLogo -NoProfile -NonInteractive -Command` with a fixed bootstrap. The command text arrives through standard input, and the bootstrap reads all of it as UTF-8, sets UTF-8 as the output encoding, turns off progress output, and runs the text as one script block. Mannie never uses `-EncodedCommand`, and the text is parsed as a whole, so blocks and here-strings that span lines work.
- An `exit` in the command sets the exit code. Otherwise the exit code is 0 when the command's last statement succeeded. When it failed, the exit code is that of the last program the command ran, or 1 when that program succeeded or none ran.
- Windows PowerShell's execution policy applies as it does in a PowerShell window: where the policy is `Restricted`, the default of Windows client editions, running a `.ps1` file fails, while commands, `.cmd` files and programs run. The failure reaches the model as the command's output.
- The process starts suspended and without a window, and joins a Job Object of its own before its first thread runs. The job ends every process in it when its last handle closes and suppresses error dialogs, and it lets no process break away. Its handle is not inherited by any process.
- The command ends when the shell exits. The supervisor then terminates the job, which ends every process that the command left behind, including programs it opened (owner decision, [system tools](README.md#decisions)). Stop and the timeout terminate the job at once.
- If Mannie itself ends, the operating system closes the job's handle and ends the tree.

### Linux

- The command line is `/bin/sh -c` with a fixed wrapper, `mannie` as the name of the script and the command text as its first argument. The wrapper runs `(eval "$1") </dev/null 3>&-`, writes the exit status to descriptor 3, closes it, and waits on its standard input.
- Between `fork` and `exec`, the shell gets a new process group and becomes a child subreaper (`PR_SET_CHILD_SUBREAPER`), which `execve` keeps. Every descendant that calls `setsid` or forks twice stays below the shell, because orphans are reparented to the nearest living subreaper.
- The command ends when its status arrives on descriptor 3. The supervisor then ends the rest of the shell's tree, and only then closes the shell's standard input, so the shell exits.
- To end a tree, the supervisor walks the shell's descendants in `/proc` and skips zombies. It sends `SIGTERM` to each, waits up to 500 ms, then stops and kills what is left with `SIGSTOP` and `SIGKILL` until no descendant remains. It signals through process file descriptors and checks the parent again after opening one, so a reused process ID is never signaled.
- Stop and the timeout end the tree the same way, then kill the shell.
- Mannie itself is not a subreaper, so it never adopts the orphans of the webview's processes or of programs that open links.
- This refines decisions 3 and 5 of [ADR-0014](../../adr/0014-contain-process-trees.md), which records the change.

### Environment

A command's environment holds only the variables of an allow list, plus the names the owner adds in the settings, with their values from Mannie's own environment.

| Platform | Variables                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Windows  | `ALLUSERSPROFILE`, `APPDATA`, `CommonProgramFiles`, `CommonProgramFiles(x86)`, `CommonProgramW6432`, `COMPUTERNAME`, `ComSpec`, `HOMEDRIVE`, `HOMEPATH`, `LOCALAPPDATA`, `NUMBER_OF_PROCESSORS`, `OS`, `PATH`, `PATHEXT`, `PROCESSOR_ARCHITECTURE`, `ProgramData`, `ProgramFiles`, `ProgramFiles(x86)`, `ProgramW6432`, `PSModuleAnalysisCachePath`, `PUBLIC`, `SystemDrive`, `SystemRoot`, `TEMP`, `TMP`, `USERDOMAIN`, `USERNAME`, `USERPROFILE`, `windir` |
| Linux    | `HOME`, `LANG`, `LANGUAGE`, the `LC_` variables, `LOGNAME`, `PATH`, `SHELL`, `TMPDIR`, `TZ`, `USER`, `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`, `SSH_AUTH_SOCK`                                                                                                                                                                                                                                                                                          |

- A name the owner adds is refused when one of its parts between underscores is `KEY`, `APIKEY`, `TOKEN`, `SECRET`, `PASSWORD`, `PASSWD` or `CREDENTIAL`, so a credential never reaches a command by mistake.
- `PSModulePath` is left out, so Windows PowerShell finds its own modules even when Mannie was started from PowerShell 7.
- `PSModuleAnalysisCachePath` passes when it is set, so Windows PowerShell reads the module cache that the system prepared instead of analyzing every installed module again for each command. GitHub's Windows runners set it (verified on 2026-10-16), and without it a command there took about 20 seconds to start.

### Output

- Mannie reads standard output and standard error separately. Each keeps its first 256 KiB and its last 768 KiB, and a cut is marked with the number of bytes left out.
- The model gets a head and a tail of each stream within the result limit ([tools and results](tools-and-results.md#results-and-limits)). Everything captured is archived, and both the model and the owner can read it.
- Output is decoded as UTF-8. Bytes that are not valid UTF-8 are replaced, and the result says so. CRLF line endings become LF, so output reads the same on both platforms.
- The result names the exit code, or that the command timed out, was stopped, or was interrupted when Mannie quit, with the time it ran.

### Crash recovery

- Windows: the job ends with Mannie, so nothing is left to clean up.
- Linux: the journal records the shell's process ID and start time when the command starts. After a crash, the recovery looks for that process. When it still runs with the same start time, the recovery ends its tree as Stop does. This is a best effort: a command that finished in the meantime is gone, and its status stays uncertain.

### What containment does not cover

Programs that start processes for a command through the system escape the command's tree: WMI, the Task Scheduler, COM servers, services, `systemd-run`, D-Bus activation and `xdg-open`. A command that the owner approved runs with the owner's rights and can read the owner's credential store, which other processes of the owner can read too. Mannie never hands a credential to a command.

### Errors

Command failures add these codes to the [tool errors](tools-and-results.md#errors): `shell_missing` and `spawn_failed`. A working directory that does not exist, or is not a folder, fails with `not_found` or `not_a_folder` before the owner is asked. A command that runs but fails is not an error: its result carries the exit code and output.

### Platform differences

| Topic              | Windows                       | Linux                                                          |
| ------------------ | ----------------------------- | -------------------------------------------------------------- |
| Shell              | Windows PowerShell 5.1        | `/bin/sh`                                                      |
| Containment        | A Job Object per command      | A process group, with the shell as child subreaper             |
| End of the command | The shell exits               | The wrapper reports the status on descriptor 3                 |
| Stop and timeout   | The job is terminated at once | `SIGTERM`, 500 ms, then `SIGSTOP` and `SIGKILL`                |
| If Mannie crashes  | The job ends with Mannie      | The recovery ends the tree at the next start, as a best effort |

### Tests

V4 runs in `crates/mannie-tools/tests/` on Windows and Linux. The test binary starts itself again in a helper mode to build process trees and write their process IDs to a file:

- Windows: a grandchild started with `Start-Process`, a grandchild in a nested Job Object of its own, and a child that asks to break away from the job.
- Linux: a grandchild that calls `setsid`, a double fork, `nohup`, and a process that ignores `SIGTERM`.

Each shape is ended by Stop, by the timeout and by the command's normal end, and no process ID may remain alive afterwards. Stop ends the tree in less than a second. The conformance suite adds exit codes, separate and cut streams, the working directory, output in other scripts than Latin, multi-line PowerShell, and an environment from which a planted secret variable is absent.

## Acceptance

- V4: Stop, the timeout and the command's normal end leave no process of the tree alive, including processes that detach on purpose, with Job Objects and nested jobs on Windows and with the subreaper on Linux (process tree tests on Windows and Linux).
- Stop ends a running command and its reply within one second (runtime test and end-to-end test).
- A planted secret in Mannie's environment never reaches a command (runtime test).
- Output is captured separately, cut with a mark, archived, and read back (conformance suite).
- The commands of the conformance suite pass on Windows and Linux.
