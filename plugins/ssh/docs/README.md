# SSH hosts

|              |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Plugin       | `ssh`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Milestone    | M8                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Last updated | 2026-11-09                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Related      | [ADR-0036](../../../docs/adr/0036-let-plugins-run-commands-on-remote-hosts.md), [ADR-0041](../../../docs/adr/0041-let-agents-run-commands-in-the-background.md), [ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md), [Terminal](../../terminal/docs/README.md), [speak SSH with russh and pinned host keys](adr/0001-speak-ssh-with-russh-and-pinned-host-keys.md), [ADR-0013](../../../docs/adr/0013-journal-every-side-effect.md), [ADR-0015](../../../docs/adr/0015-run-an-agent-runtime-on-every-machine.md), [Plugins](../../../docs/features/plugins.md), [Operations and approvals](../../../docs/features/system-tools/operations-and-approvals.md), [Workers](../../workers/docs/README.md) |

## Intent

The owner wants agents to work on computers that do not run Mannie. In the owner's words: "I set up SSH hosts manually, like IP, port, username, password (defer SSH keys for the first version), then I can point an agent to use the SSH channel to do something on the remote host." Workers stay the way to give a computer an agent of its own; an SSH host is reached directly from the master, one approved command at a time.

## Behavior

### Turning it on

- SSH is a plugin on the Plugins page, with a switch. It starts off on a new installation, like every plugin ([D28](../../../docs/roadmap/milestones.md#5-decisions)).
- While SSH is off, agents have no tool for hosts, the system prompt lists no hosts and "@" offers none. The hosts and their passwords stay.

### Registering a host

- The settings section SSH hosts lists the hosts and has "Add SSH host". The command palette has "Add SSH host…".
- A host has a name, an address, a port, a user name, a password and a note for agents:
  - The name is how the owner and the agents call the host, such as `build-box`: 1 to 64 letters, digits, dots, underscores and hyphens, starting with a letter or a digit, and unique without regard to case.
  - The address is a host name, an IPv4 address or an IPv6 address, without a user, a port or a scheme. The port is 22 unless the owner changes it.
  - The user name has no white space or control characters. `user@domain` and `DOMAIN\user` work.
  - The password has at most 1,024 bytes. Mannie keeps it in the system's credential store and never shows it again. Editing a host leaves the password field empty, which keeps the stored one.
  - The note tells agents what the host is for, in at most 500 characters. Agents read it as information from the owner, and it grants nothing.
- Once a host is added, its key dialog opens and reads the host's key without logging in. The owner compares the fingerprint with the one that the host prints, such as with `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`, and chooses "Trust this key". Mannie then logs in once to test the password and runs nothing.
- Each host shows its status: Ready, Host key not checked, Host key changed, or Password missing. Its row offers "Check key…", "Test login", "Edit…" and "Remove…".
- Changing a host's address or port lets go of its trusted key, since the key belongs to the server that was there. Changing its name, user, password or note keeps the key.
- Removing a host asks first, then deletes its password and forgets it. Permissions and rules for its commands stay in the Tools settings until the owner revokes them, and they match no other host.
- A computer that runs a worker is reached through the worker and its ceiling, and is not registered here as well.

### Host keys

- Mannie accepts Ed25519 and ECDSA host keys, and RSA host keys of at least 2048 bits signed with SHA-2 (`rsa-sha2-512` or `rsa-sha2-256`). It prefers Ed25519 and ECDSA when a host has both. A host that signs its RSA key only with SHA-1 (`ssh-rsa`), or whose RSA key is shorter, is refused before Mannie logs in ([D53](../../../docs/roadmap/milestones.md#5-decisions), [ADR-0002](adr/0002-accept-rsa-host-keys.md)).
- Mannie compares the key that a host shows with the trusted key before it logs in. A host that shows another key gets no password: its status becomes Host key changed, and its key dialog shows the trusted and the new fingerprint with a warning, until the owner trusts the new key.

### Running commands

- Agents run commands with the tool `run_ssh_command`, which takes the host's name, the command and a timeout. The conversation agent and the `general` child agent have it; the `explore` child agent, which only reads, does not.
- Each command asks the owner like a local command. The request shows the command, the host's name, its address with the user and the port, and the fingerprint of its key. The owner can:
  - allow it once;
  - allow it for this conversation, this project or every conversation, which covers the same command on the same host at the same address;
  - save a rule of its leading words, which applies only on that host;
  - deny it.
- A command with shell operators or a backslash offers only "Allow once" and "Deny". Full access runs commands on hosts without asking, as it runs local ones.
- The host runs the command with the account's shell, without a terminal, starting in the account's home folder. The result holds the exit status or the signal that ended the command, how long it ran, and its standard output and standard error. A long stream shows its start and its end, and is archived for `read_tool_output` as `run_command` archives it: whole, or its first 256 KiB and its last 768 KiB when it is longer. The host's password is replaced wherever the output shows it, when it has at least 8 characters.
- A command ends after its timeout, 60 seconds unless the call names 1 to 600. Then, and when the owner stops the reply, Mannie asks the host to end the command, first with TERM and then with KILL, and closes its channel. Processes that the command started may keep running on the host, and the result says so. A command that the owner stopped while it ran is marked uncertain.
- Turning SSH off while a command runs waits up to 5 seconds for it; a command that still runs is then marked uncertain.
- Agents start a command on a host in the background with the tool `start_ssh_command`, which takes the host's name and the command. It asks as `run_ssh_command` does and says that the command runs in the background, and a grant or a rule covers it only when it came from a command in the background (D40). The command has no timeout and keeps running after the call until it ends on the host, the agent or the owner stops it, its conversation is deleted, its task ends, SSH goes off, the owner edits or removes the host, or Mannie quits. The agent reads its output with `read_command_output` and stops it with `stop_command`, as it does for a local command in the background ([background commands](../../../docs/features/system-tools/background-commands.md)), and its session stays open while it runs.

### Terminals

- While the plugin [Terminal](../../terminal/docs/README.md) is on, its menu "New terminal" lists the hosts. A host that is not Ready shows its status and opens nothing.
- A terminal logs in with the host's password, only to a host that shows the trusted key, as "Test login" does, and runs the account's login shell on a pseudo terminal of the panel's size. What the owner types reaches the shell as typed ([ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md), decision 6).
- The first terminal on a host reads the account's login shell, so that the terminal plugin can mark its commands: bash, zsh, fish, sh (also dash and ash) and PowerShell. Another shell, or a host that does not say, gives a plain terminal, where agents run no commands. Changing the host's address, port, user or trusted key reads the shell again.
- The host's password is replaced in the terminal's output as it arrives, when it has at least 8 characters. Output that could be the start of the password waits for the next read, at most 50 milliseconds, so a password that the host's writes cut is still replaced.
- A terminal that an agent's approved command opens checks the host's endpoint and key against the approval first, and a host where either changed opens nothing.
- A terminal keeps the host's session open while it runs. Editing, re-trusting or removing the host, and turning SSH off, close the session and end its terminals, which then say that the connection was lost.

### What the model knows

- In runs that have the tool, the system prompt lists the hosts with their names, addresses and notes, and says which ones cannot run commands yet.
- The owner can point an agent at a host by typing "@" and choosing it, such as "@build-box". The model then reads which host the owner means and its address. A mention reaches nothing on the host.

### What it does not do

- It transfers no files and uses no SSH keys; both are [ideas](#parts) for later.
- It forwards no ports, agents or X11 connections, passes no environment variables to a terminal's shell, and uses no jump hosts or host certificates.

## Fit

- Builds on the extension points of [ADR-0036](../../../docs/adr/0036-let-plugins-run-commands-on-remote-hosts.md): a tool with side effects whose calls are remote-command accesses that the core's policy decides and its journal records, archives in a tool's result, and keyed secret families. It also adds an instruction section, a mention provider, a settings section, an overlay, a command of the palette and a kind of mention.
- Follows [Plugins](../../../docs/features/plugins.md): a switch, a start and a stop that take effect at once, the plugin's code in `plugins/ssh/`, its UI files named `ssh` and its messages under `ssh_`.
- Opens its own sockets under the manifest's `[native] sockets` grant, since the SDK has no network service.
- Hands its commands in the background to the core's registry through the SDK's service `background-commands` ([ADR-0041](../../../docs/adr/0041-let-agents-run-commands-in-the-background.md), decision 11).
- Offers its hosts to terminals through the point `terminal-hosts`, which the core consumes ([ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md), decision 3). The terminal plugin never reads a host's record or its password.
- Must not change: local commands, their rules and grants, which never match a command on a host, and Workers.

## Open questions

None.

## Decisions

- 2026-10-19: the owner decided ([D35](../../../docs/roadmap/milestones.md#5-decisions)):
  - Every conversation and the `general` child agent can use every registered host, and "@" points an agent at one.
  - Commands on a host get the same approval choices as local commands, with grants and rules bound to the host, and full access runs them without asking.
  - Version 1 only runs commands. File transfer and SSH keys come later.
  - Hosts whose only key is RSA are refused.
  - M8 ships in v0.4.0, a release of its own.
  - Approving the plan accepts ADR-0036 and the plugin's first ADR, [speak SSH with russh and pinned host keys](adr/0001-speak-ssh-with-russh-and-pinned-host-keys.md).
- 2026-11-09: the owner decided ([D53](../../../docs/roadmap/milestones.md#5-decisions)):
  - Hosts whose only key is RSA work, with SHA-2 signatures and keys of at least 2048 bits, which replaces the refusal of D35.
  - The owner accepts the plugin's [ADR-0002](adr/0002-accept-rsa-host-keys.md), and `deny.toml` ignores RUSTSEC-2023-0071, because the plugin only verifies signatures with public keys.
- 2026-10-19: the defaults below were chosen when shaping, and wait for the owner's review:
  - Reaching a host and its handshake take at most 15 seconds, and a login at most 30.
  - An open session sends a keepalive after 30 seconds of silence and closes after 3 unanswered ones, and after 5 minutes without a command.
  - A command has at most 100,000 bytes, and its output keeps the first 256 KiB and the last 768 KiB of each stream.
  - The password is replaced in output from 8 bytes on, as the log redactor does.
- 2026-10-21: the owner decided (D40, [ADR-0041](../../../docs/adr/0041-let-agents-run-commands-in-the-background.md)): agents start commands on hosts in the background, through the core's background commands, and a grant or a rule covers a command in the background only when it came from a command in the background.
- 2026-10-30: the owner decided ([D45](../../../docs/roadmap/milestones.md#5-decisions), [ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md)): terminals open on the registered hosts.
- 2026-10-30: the defaults below were chosen when building, and wait for the owner's review:
  - The login shell is read with `printf '%s' "$SHELL"` on a channel of its own, beside the terminal's.
  - The host has 10 seconds to give a terminal its pseudo terminal and shell, and to say its login shell.
  - Output that could start the password waits at most 50 milliseconds.

## Parts

- [SSH keys](ssh-keys.md): logging in with a key instead of a password. An idea.
- [File transfer](file-transfer.md): copying files to and from a host. An idea.

## Design

### The plugin

`plugins/ssh/crates/mannie-ssh` defines the plugin `ssh`:

- Its manifest, `plugins/ssh/plugin.toml`, declares the services `store`, `secrets` and `background-commands`, the keyed secret family `host`, the crate `russh` that only it may use, and the native grant `sockets`. Its default is off.
- Its start reads the hosts and contributes the tools `run_ssh_command` and `start_ssh_command`, offered to the agents `conversation` and `general` with side effects, the section `ssh-hosts` at position 320, the mention provider `ssh` and the source `ssh` of terminal hosts. A task of its start closes idle sessions every minute, and its stop closes every session.
- Its UI registers a card for each tool, with the server icon and the sentence of the failure code `not_started`, and the core names each call by its remote command.
- The test-only crate `mannie-ssh-testing` runs an SSH server in a test's process, and its example program `ssh-test-server` serves the end-to-end tests.

### Contracts

- Commands, which the host's dispatch answers with `plugin_off` while SSH is off ([ADR-0034](../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), decision 8):
  - `add_host` and `update_host` take the name, the address, the port, the user name, the password and the note, and return the host's view; `update_host` takes the host's ID and keeps the password when none is given.
  - `remove_host`, `scan_host_key` and `test_host` take the host's ID. `scan_host_key` returns the key's algorithm, fingerprint and time, and how it compares with the trusted key: `not_pinned`, `same` or `changed`.
  - `trust_host_key` takes the host's ID and the fingerprint of the key that the owner checked.
  - The channel `follow_hosts` sends the whole list of hosts, then again after every change.
  - A host's view has its ID, name, note, address, port, user name, endpoint, whether its password is stored, its trusted key, the key that it showed last when that differs, and its status: `ready`, `key_not_checked`, `key_changed` or `password_missing`. It never holds the password.
  - Errors: `name_taken`, `host_not_found`, `unreachable`, `timed_out`, `handshake_failed`, `host_key_unsupported`, `host_key_changed`, `host_key_not_trusted`, `scan_outdated`, `auth_failed`, `auth_method_unsupported`, `password_missing`, `credential_store_unavailable` and `storage_failed`. A field that breaks a rule is the host's `invalid_input` with the field's name.
- The tool `start_ssh_command` takes `host` and `command`. Its preparation returns the access of `run_ssh_command` with `background`. Its execution opens the channel, runs the command and hands the service `background-commands` the command's lines and its stop, under the ticket of its execution; its result names the command's number, the host and the endpoint. The manifest declares the service.
- The tool `run_ssh_command` takes `host`, `command` and `timeout_seconds`. Its preparation returns one remote-command access with the host's ID as the target, its name as the label, `user@address:port` as the endpoint, the fingerprint of its trusted key as the identity, and the command.
- The mention provider `ssh` lists hosts with the variant `host` and the warning `not_ready`, and refuses a removed host with `host_not_found`. A mentioned host's summary code is `host`.
- The source `ssh` of the point `terminal-hosts` lists each host with its ID as the key, its name as the label, `user@address:port` as the endpoint, the fingerprint of its trusted key as the identity, and the login shell once a terminal read it:
  - A host that is not ready is listed as unavailable with its status: `key_not_checked`, `key_changed` or `password_missing`.
  - Opening refuses an unknown host and one whose endpoint or key is not the approved one. Other failures carry the code of the SSH error, such as `unreachable`, `auth_failed` or `host_key_changed`, or `pty_refused`, `shell_refused` and `timed_out`.
  - A terminal ends with the shell's exit status, as closed when the core closed it or the host closed it without a status, as lost with `session_closed` when Mannie closed its session, and as lost with `connection_lost` when the connection broke.

### Data

- The plugin's migration 1 creates `ssh_hosts`: the ID, the name, unique without regard to case, the note, the address, the port, the user name, whether a password is stored, the trusted key and the key seen last, each as its algorithm, its OpenSSH form, its fingerprint and its time, and the times of creation and change. A CHECK keeps each key complete or absent.
- Each host's password lives in the credential store as `ssh/host/<host ID>`. Adding a host stores its row without a password, then the password, then marks the row, and a password that cannot be kept takes the row out again. Removing a host deletes the password first.
- Trusting a key copies the key seen last into the trusted key, only when its fingerprint is the one that the owner checked. Changing the address or the port clears both keys in the same transaction.

### Connections

- The plugin resolves the address and opens the TCP connection itself, then speaks SSH through russh 0.64.1 on ring, which offers Ed25519 and ECDSA host keys only ([speak SSH with russh and pinned host keys](adr/0001-speak-ssh-with-russh-and-pinned-host-keys.md)).
- The client compares the full public key that the host shows with the trusted one right after the key exchange, before any authentication. A scan records the key and stops there.
- Login tries the password, then keyboard-interactive login that answers exactly one hidden question with the password, which is how FreeBSD's sshd takes passwords through PAM. Anything else fails with `auth_method_unsupported`.
- A command in the background holds its session's lease for as long as it runs, so idle closing skips the session. Editing, re-trusting or removing the host, and the plugin's stop, close the session and end the commands in it; the core asks them to end first when SSH goes off or Mannie quits.
- One session for each host is reused by later calls while its endpoint and trusted key stay the same. A reused session that the host closed meanwhile, such as while the computer slept, is replaced once before anything runs. A call holds its session while it runs, so a long command is never closed for being idle.
- The tool opens a channel without a terminal, runs the command, closes the command's input and reads both streams until the channel closes. A guard that the call's future drops asks the host to end the command when the core lets the call go.
- A terminal opens a channel on the host's session, asks for a pseudo terminal of type `xterm-256color` and the account's shell, and holds the session's lease while it runs. Writes and size changes go to the channel in order. Closing it ends the channel, which hangs up the shell, and waits for the host to close it within the core's grace.

### The text the model sees

| Case                                | What the model reads                                                                                                                                      |
| ----------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The section, with hosts             | An introduction, then one line for each host: its name, its endpoint, whether it cannot run commands yet, and its note, all cleaned of control characters |
| The section, without hosts          | "SSH hosts: no SSH host is registered. The owner adds hosts in the SSH settings."                                                                         |
| A result                            | "Ran on {name} ({endpoint})." with the exit status or signal, the duration and both streams                                                               |
| A start in the background           | "Command {n} started in the background on {name} ({endpoint}).", with how to read its output and stop it                                                  |
| A call that did not reach the shell | "Not run: {reason} Nothing ran on the host."                                                                                                              |
| An unknown host                     | "There is no SSH host named …", with the registered hosts' names                                                                                          |
| A mention                           | "The owner mentioned the SSH host {name} ({endpoint}).", with how to run commands there and the owner's note                                              |

### Interface

- The settings section SSH hosts lists the hosts with their status, endpoint, note and fingerprint, and the actions of each. The dialogs to add or edit a host, to check its key and to remove it are an overlay of the plugin.
- The mention kind `ssh` shows hosts under SSH hosts with the server icon.
- Messages live in `plugins/ssh/messages/en.json` under `ssh_`.

### Loading, empty and error states

| View                | Loading                   | Empty                               | Error                                                                     |
| ------------------- | ------------------------- | ----------------------------------- | ------------------------------------------------------------------------- |
| The SSH settings    | Placeholders of two hosts | "No SSH hosts", with "Add SSH host" | "The SSH hosts could not be loaded" with "Try again"                      |
| The host dialog     | "Saving the host…"        | Not applicable                      | What failed, with the refused field marked                                |
| The key dialog      | "Reading the host key…"   | Not applicable                      | What failed while reading, trusting or testing, with "Read the key again" |
| A host's login test | "Testing the login to …"  | Not applicable                      | "The login to … failed:" with the reason                                  |

### Threat notes

- A prompt injection can make an agent try a destructive command on a host. Every command is an operation that the owner allows unless a grant, a rule or full access covers it, grants and rules cover one host at one endpoint, and the explore agent never gets the tool (T-39).
- An attacker can pose as a host to catch its password. The password goes only to a host that shows the trusted key, and the owner trusts a key after checking its fingerprint (T-40).
- The password never reaches the database, logs, IPC output or a child process, and output that shows it is redacted. Password login hands it to the server, which is the reason SSH keys come later (T-41).
- A terminal's output passes the password's redaction as it arrives, with a carry-over across reads (T-71). What the owner types is the owner's own input and is not redacted.
- Processes that a command started can outlive Stop and the timeout (T-42). A command in the background keeps running until it ends or is stopped (T-52), and its lines pass the password's redaction one by one, with a carry-over across reads (T-41). A compromised master can reach every registered host (T-43).
- Logs at `info` name hosts by ID and calls by their outcome. Commands are logged only at `debug`.
- A compromised webview could make Mannie connect to an address of its choice through `add_host` and `scan_host_key`, as with pairing ([security](../../../docs/architecture/security.md#8-if-the-webview-is-compromised)). Through the terminal plugin it could also open a terminal on a registered host and type into it (T-68); the plugins start off, and script that can do that can already turn on full access.

### Platform differences

- OpenSSH on Linux takes passwords directly; FreeBSD's default sshd takes them through keyboard-interactive login.
- OpenSSH Server on Windows runs commands in `cmd.exe` unless its configuration names another shell. `cmd.exe` reads `^` and `%`, which the check for shell operators does not refuse, so a rule for a command there may match more than its words.
- On OpenSSH Server for Windows, `printf` does not run in `cmd.exe`, so a terminal there is plain.
- Signals reach commands on OpenSSH 7.9 and later. A server that ignores them only loses the channel.

## Acceptance

- The rules of hosts, the capture and rendering of output, the redaction of the password, the section, the mentions and the schema of the tool pass their unit tests in `plugins/ssh/crates/mannie-ssh/src/`.
- Against the SSH server in the test's process (`plugins/ssh/crates/mannie-ssh/tests/plugin.rs`): adding a host keeps its password only in the credential store and connects to nothing; a name is unique without regard to case; a scan sends no password; trusting a key that the last scan did not see pins nothing; a test login refuses a wrong password; keyboard-interactive login works; a changed key stops before the password is sent; an RSA-only host is scanned, trusted and logged in to; a host that signs its RSA key only with SHA-1, one whose RSA key is under 2048 bits and an unreachable one are refused; a host with both an Ed25519 and an RSA key shows its Ed25519 key; another address or port lets go of the trusted key; removing a host deletes its password; and the plugin passes the SDK's conformance check.
- Through the core's tool loop (`plugins/ssh/crates/mannie-ssh/tests/tool.rs`): a command asks with the host and runs once allowed; a grant for the conversation runs the same command again on its host only; a timeout sends TERM; the password never shows in the output; long output is archived for `read_tool_output`; Stop sends TERM and leaves the call uncertain; a host without a trusted key is refused before anyone is asked; and the model reads the hosts and the tool.
- Against the SSH server in the test's process: `start_ssh_command` starts a command that writes lines over time, `read_command_output` reads them as they arrive, `stop_command` ends it with TERM and KILL and says how it ended, the password never shows in its lines, its session stays open past the idle limit, editing the host ends it without a wake-up, and turning SSH off ends it.
- Terminals against the SSH server in the test's process (`plugins/ssh/crates/mannie-ssh/tests/terminals.rs`): a terminal runs the login shell on a pseudo terminal of the requested size and takes a new size, the login shell is listed once the first terminal read it, the password never shows when the host's writes cut it, a long typed line reaches the shell whole, a host whose key changed opens nothing and gets no password, an approval whose endpoint changed opens nothing, and editing the host or turning SSH off ends the terminal with `session_closed`.
- The plugin's UI passes its Vitest tests in `plugins/ssh/tests/ui/`.
- The end-to-end test (`plugins/ssh/tests/e2e/ssh.test.ts`) turns SSH on, adds a host of a real SSH server, checks and trusts its key, runs a command after the owner allows it, and removes the host, with axe finding no WCAG 2.1 AA violation.
- The owner's checks against OpenSSH on Linux, on FreeBSD with keyboard-interactive login, and on Windows.
