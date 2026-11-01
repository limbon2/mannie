# ADR-0001: Mark commands with shell integration

|          |                                                                                                                                                                                                                                                                                                                                                                                                    |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D45)                                                                                                                                                                                                                                                                                                                                                                                     |
| Date     | 2026-10-30                                                                                                                                                                                                                                                                                                                                                                                         |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                              |
| Related  | [Terminals](../README.md), [keep a headless emulator](0002-keep-a-headless-emulator-of-each-terminal-in-rust.md), [ADR-0053](../../../../docs/adr/0053-let-plugins-offer-terminals.md), [M14 validation results](../../../../docs/roadmap/m14-validation.md), [Goals and scope](../../../../docs/product/goals-and-scope.md#41-performance), [Security](../../../../docs/architecture/security.md) |

## Context

- An agent runs a command in a terminal and needs to know where the command's output starts and ends, its exit code, and the user and folder of the shell that runs it ([ADR-0053](../../../../docs/adr/0053-let-plugins-offer-terminals.md), decision 5). A terminal's stream holds all of it at once: prompts, what the owner types, the echo of commands and their output. The owner's command list needs the same places for the owner's own commands.
- VS Code's shell integration marks these places with OSC 633 sequences that a setup of the shell writes (verified 2026-10-30): the prompt's start (`A`) and end (`B`), the command's start (`C`), its end with an optional exit code (`D`), the command line (`E`, with an optional nonce that keeps output from spoofing it), and properties such as the folder (`P`).
- A setup runs at every prompt and every command. NFR-PERF-10 allows it under 50 ms at a shell's start and under 1 ms at a prompt, and 100 ms and 5 ms for PowerShell.
- On an SSH host, the plugin gets the account's login shell from the host's own server, and no file of Mannie's lies on the host.
- S1 on the owner's workstation found ([M14 validation results](../../../../docs/roadmap/m14-validation.md)):
  - The built-in ConPTY keeps the markers in order with the output: every line of 1, 50 and 3,000 lines arrived between the command's start and its end in Windows PowerShell and Git Bash. cmd marks only the end, through its prompt variable.
  - After Ctrl+C, PowerShell's prompt reported the stale `$LASTEXITCODE`, 3, of an earlier command.
  - bash sets a flag for a command without a fork through an arithmetic expansion in `PS0`, `${__mn:0:$((__m_ran=1,0))}`. The variable must be set, since bash does not evaluate the length of a substring of an unset variable.
  - The PowerShell setup cost about 5 ms at start and nothing measurable at a prompt: an empty command's round trip took 1.8 ms (p50) without it and 1.7 ms with it.

## Decision

1. Each terminal gets a random nonce, and its setup writes OSC 633 markers that each carry it:
   - the prompt's start and end;
   - the command line;
   - the command's start;
   - the command's end with its exit code;
   - the user and the folder.

   A marker without the terminal's nonce is plain output (T-69).

2. What each shell marks:
   - bash, zsh, fish and PowerShell mark all of them. PowerShell marks through PSReadLine and its prompt function.
   - sh, such as dash and FreeBSD's sh, and cmd mark their prompts through their prompt variables, which is enough to end a command. cmd gives no exit code.
   - A shell whose markers never arrive is plain. Agents cannot run commands there, and the panel says so.
3. Where the setup comes from:
   - A shell of this machine gets the setup at its start ([ADR-0053](../../../../docs/adr/0053-let-plugins-offer-terminals.md), decision 1).
   - On an SSH host, the plugin types the setup after the first prompt, for the login shell that the SSH plugin reports. It hides the setup's echo until the first marker, and removes the setup from the shell's history where the shell allows it.
4. A setup starts no process for a prompt or a command:
   - bash marks through `PS0` and `PROMPT_COMMAND` with builtins only, never through a `DEBUG` trap, which would slow every command of a loop. It sets its flag for a command that ran through an arithmetic expansion inside `PS0` (S1).
   - zsh marks through `precmd` and `preexec`, fish through its events, and PowerShell through its prompt function and a PSReadLine handler.
   - Where a shell cannot report the owner's command line without a new process, the plugin reads it from the screen, between the prompt's end and the command's start.
   - PowerShell's exit code comes from `$?` and from whether `$LASTEXITCODE` changed during the command, since an interrupted command leaves the old value (S1).
5. An agent types as follows:
   - Only at an idle prompt, after the terminal's state matches the approval.
   - Then the line and Enter.
   - Ctrl+C at the timeout or on Stop, from a drop guard. If no end marker arrives within 5 seconds, the result says that the command still runs.

## Consequences

- Positive: a command's block comes from the shell's own account of it, so an agent's result holds exactly its command's output and exit code, and the command list shows the owner's commands with their output and exit codes too. A forged marker changes no block. A prompt starts no process, so the setup costs nothing that the owner notices.
- Negative: sh and cmd mark only their prompts, so their blocks take the command line from the screen, and cmd's blocks have no exit code. A plain shell gets no blocks, and agents cannot use it. A shell that cannot drop a line from its history keeps the typed setup there.
- Risks: a prompt framework, such as starship, oh-my-zsh or oh-my-posh, or custom PSReadLine settings can replace the prompt or its hooks and drop the markers (R44), and the typed setup can fail on unusual hosts (R46). S4 checks the common ones, and a shell whose markers never arrive stays usable as a plain terminal.

## Alternatives considered

| Option                                                 | Why not chosen                                                                                                         |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------- |
| A `DEBUG` trap in bash                                 | It runs before every simple command, so it slows every command of a loop                                               |
| A process at each prompt that reports the command line | A process start at every prompt breaks the budget of 1 ms at a prompt (NFR-PERF-10)                                    |
| Markers without a nonce                                | Output could forge a command's end, its exit code or its output (T-69)                                                 |
| Commands read from the screen alone                    | Prompts differ between shells and configurations, and the screen shows neither the exit code nor where the output ends |

## Validation

M14: the plugin's tests against real bash, zsh, fish and sh on Linux and PowerShell and cmd on Windows, which check blocks, forged markers and setups that start no process at a prompt; a scripted shell that speaks the markers; S1, which ran; and S4: the setups injected locally and typed through a pseudo terminal of russh to OpenSSH on Ubuntu, FreeBSD and Windows, with starship, oh-my-zsh, oh-my-posh and custom PSReadLine settings, and a clean history ([M14 validation results](../../../../docs/roadmap/m14-validation.md)).
