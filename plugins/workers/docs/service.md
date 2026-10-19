# The worker service

|              |                                                                                                                                                                                                                                                                                             |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                    |
| Milestone    | M4                                                                                                                                                                                                                                                                                          |
| Last updated | 2026-10-16                                                                                                                                                                                                                                                                                  |
| Part of      | [Workers](README.md)                                                                                                                                                                                                                                                                        |
| Related      | [ADR-0008](../../../docs/adr/0008-one-workspace-two-binaries.md), [ADR-0016](../../../docs/adr/0016-link-machines-over-pinned-mtls-grpc.md), [Pairing and the link](pairing-and-link.md), [Compatibility and updates](compatibility-and-updates.md), [Worker setup](guides/worker-setup.md) |

## Design

### The configuration

The owner configures a worker in one TOML file, owned by root or Administrators and readable by the service's account:

```toml
version = 1
name = "build-01"
purpose = "Builds and tests the backend services"
listen = "0.0.0.0:7445"
data_dir = "/var/lib/mannie-worker"
host_instructions = "/etc/mannie-worker/host-instructions.md"
max_parallel_tasks = 4

[ceiling]
mode = "ask"
allowed_folders = ["/srv/app"]
rules = [
  { command = "git status" },
  { command = "cargo test", folder = "/srv/app" },
]
```

| Key                                     | Rule                                                                                                                                                   |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `version`                               | 1. A newer version is refused with the version it names, so an older worker never misreads a newer file                                                |
| `name`                                  | Required: 1 to 64 characters without control characters. The master and the conversation agent see it, unless the owner names the worker in the master |
| `purpose`                               | Optional, at most 1,000 characters                                                                                                                     |
| `listen`                                | The address and port to listen on; `0.0.0.0:7445` by default                                                                                           |
| `data_dir`                              | The data folder; the system's default below when left out                                                                                              |
| `host_instructions`                     | A Markdown file of at most 64 KiB; `host-instructions.md` beside the configuration when left out. A missing file means no host instructions            |
| `max_parallel_tasks`                    | From 1 to 16, 4 by default                                                                                                                             |
| `ceiling.mode`                          | `read_only`, `ask` (the default), `autonomous` or `full_access` ([host and ceiling](host-and-ceiling.md#the-ceiling))                                  |
| `ceiling.allow_full_access_from_master` | Whether a conversation in full access on the master answers the host's requests; `true` by default ([host and ceiling](host-and-ceiling.md#approvals)) |
| `ceiling.allowed_folders`               | Absolute folders that exist when the worker starts; empty by default                                                                                   |
| `ceiling.rules`                         | Command prefixes of whole words, each optionally limited to a folder, as the master's rules are                                                        |

Unknown keys are refused, and every error names the key and what is wrong with it. The worker reads the configuration when it starts, so a change applies after a restart. It reads the host instructions again for every task.

The worker refuses to start when its own account can open the configuration for writing, since such an account could widen its own ceiling. `--allow-writable-config` lets a worker in a terminal or a test run with a configuration of its own account; the service definitions never pass it.

### Folders

| System  | Program                                                              | Configuration and host instructions | Data                                |
| ------- | -------------------------------------------------------------------- | ----------------------------------- | ----------------------------------- |
| Linux   | `/usr/local/libexec/mannie-worker/`, with a link in `/usr/local/bin` | `/etc/mannie-worker/`               | `/var/lib/mannie-worker/`           |
| FreeBSD | `/usr/local/libexec/mannie-worker/`, with a link in `/usr/local/bin` | `/usr/local/etc/mannie-worker/`     | `/var/db/mannie-worker/`            |
| Windows | `%ProgramFiles%\Mannie\Worker\`                                      | `%ProgramData%\Mannie\worker\`      | `%ProgramData%\Mannie\worker\data\` |

The data folder holds the database (`mannie-worker.sqlite`), its backups, the logs, the worker's private key for the link (`link-key`), the master's pinned key (`master-pin`), a pairing code while one is valid (`pairing-code`), the health marker of the running program (`health`), and the copy of the database while an update runs (`update-backup`). Only the service's account can open it: mode 0700 on Linux and FreeBSD, and on Windows an access list for SYSTEM, the Administrators and the service's account.

### The command line

The command line stays in English ([ADR-0019](../../../docs/adr/0019-keep-visible-text-in-message-catalogs.md) covers the desktop app's interface).

| Command                                                | Does                                                                                                                                                                                                                                                               |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `run [--config <file>] [--data-dir <folder>]`          | Runs the worker in the terminal until Ctrl+C or a termination signal. `--data-dir` replaces the configuration's data folder; without a configuration file, the worker runs with the default configuration and that folder, as service definitions from M0 start it |
| `pair [--config <file>]`                               | Creates a one-time code and prints it with the worker's address ([pairing](pairing-and-link.md#pairing)). Refuses when the worker is paired                                                                                                                        |
| `pair --reset [--config <file>]`                       | Forgets the paired master and restarts the service, which drops every lease and interrupts the tasks                                                                                                                                                               |
| `status [--config <file>]`                             | Says whether the configuration is valid, whether the service runs, whether the worker is paired and with which key, and where it listens                                                                                                                           |
| `facts`                                                | Prints the host facts that the worker reports to the master                                                                                                                                                                                                        |
| `self-check --json`                                    | Prints the program's version, system (`os`, `arch`) and the SHA-256 of its release key (`releaseKeySha256`), for `update` and the release workflow. Its output never changes shape                                                                                 |
| `update <archive> [--check]`                           | Installs a signed release ([updates](compatibility-and-updates.md#updating-a-worker)); `--check` only checks the signature                                                                                                                                         |
| `service install [--config <file>] [--user <account>]` | Copies the program into its folder, creates the configuration and the host instructions when they are missing, registers the service and starts it. `--user` is required on Linux and FreeBSD                                                                      |
| `service uninstall`                                    | Stops and removes the service; the program, the configuration and the data stay, and the command says where                                                                                                                                                        |

`pair`, `status` and `update` run as root or with administrator rights and never open the database, which would leave files that the service's account cannot open. A pairing code that root writes is handed to the owner of the data folder.

### The services

- Windows: the service `mannie-worker` under its virtual account `NT SERVICE\mannie-worker`, started at boot, restarted twice after a failure. It runs `service run --config <file>`.
- Linux: the systemd unit `mannie-worker.service` with `User=` the given account, `UMask=0077`, `LimitCORE=0`, `Restart=on-failure`, and a start command `run --config <file>`.
- FreeBSD: the rc.d script `mannie_worker`. daemon(8) runs as root, restarts the worker when it exits and starts it as the given account, which rc.conf names in `mannie_worker_account`; `mannie_worker_config` names the configuration. The account's variable is not `mannie_worker_user`, which rc.subr would use to run daemon(8) itself as the account, without the rights to start the worker.

`service install` sets the configuration's folder to root or Administrators with read access for the service's account, and the data folder to the service's account. The guide shows how the owner grants the service's account access to the allowed folders, which the worker's tools use with that account's rights.

When the service stops, the link takes no new calls and ends the streams that a master keeps open, its watch of the tasks and its subscriptions. It lets the calls that run finish for up to 10 seconds and then closes its connections; the calls that change something run to their end either way. Then the runtime ends its runs, and the master connects again once the worker is back.

### Hardening

- Linux: the worker turns off core dumps (`RLIMIT_CORE` 0) and marks itself not dumpable, so another process of its account cannot read its memory through `ptrace` or `/proc`.
- FreeBSD: the worker turns off core dumps and tracing by other processes (`PROC_TRACE_CTL`).
- Windows: other processes of the service's account can read the worker's memory. This is a residual risk ([security](../../../docs/architecture/security.md#9-if-a-worker-host-is-compromised)), which a leased credential's short life limits.

### Logs

The worker logs to `<data>/logs` as the desktop app does ([standards](../../../docs/engineering/standards.md#7-observability)). At `info` level it records connections, pairings, tasks, leases and operations with their IDs, states and codes, never instructions, reports, paths, commands or credentials.

### Platform differences

| Topic         | Windows                          | Linux                          | FreeBSD                                                                |
| ------------- | -------------------------------- | ------------------------------ | ---------------------------------------------------------------------- |
| Service       | Windows service, virtual account | systemd unit, a chosen account | rc.d with daemon(8), a chosen account                                  |
| Commands      | Windows PowerShell 5.1           | `/bin/sh`                      | `/bin/sh`                                                              |
| Process trees | Job Objects                      | A subreaper shell              | A reaper ([ADR-0014](../../../docs/adr/0014-contain-process-trees.md)) |
| Hardening     | None beyond the account          | Not dumpable, no core dumps    | No tracing, no core dumps                                              |
| Firewall      | The guide opens the port         | The guide opens the port       | The guide opens the port                                               |

## Acceptance

- `service install` and `service uninstall` work with the default folders on Windows, Linux and FreeBSD: the service runs under its account, the configuration belongs to root or Administrators, the data folder only to the service's account (smoke tests on each system).
- The worker refuses a configuration with an unknown key, a newer version, a folder that does not exist or a configuration that its own account can write, and says why (unit tests and the smoke tests).
- `pair`, `status` and `update` never create a file in the data folder that the service's account cannot open (smoke tests).
- An idle worker uses less than 50 MB of memory (NFR-PERF-05; measured in the [validation record](../../../docs/roadmap/milestones.md#4-validations)).
