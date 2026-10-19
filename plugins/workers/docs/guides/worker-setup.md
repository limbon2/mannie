# Setting up a worker

|              |                                                                                                                                                                                                       |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                      |
| Last updated | 2026-10-16                                                                                                                                                                                            |
| Related      | [Workers](../README.md), [The worker service](../service.md), [Host and ceiling](../host-and-ceiling.md), [Workers as a plugin](../plugin.md), [Releasing](../../../../docs/engineering/releasing.md) |

A worker lets Mannie on your desktop give tasks to an agent on another computer: a Windows, Linux or FreeBSD host. This guide installs a worker as a service, pairs it with your Mannie, and shows how to update and remove it. It takes about ten minutes.

## Before you start

- Download the worker for the host's system from the release page: `mannie-worker-<version>-windows-x86_64.zip`, `mannie-worker-<version>-linux-x86_64.tar.gz` or `mannie-worker-<version>-freebsd-x86_64.tar.gz`, and its signature, the same name with `.sig`.
- Decide which account the worker runs as. Its tools act on the host with that account's rights, so give it only the access the tasks need. On Linux and FreeBSD, create a system account without a login, such as `mannie-worker`. On Windows, the worker runs as its own virtual account, `NT SERVICE\mannie-worker`.
- The host must reach the model provider that your Mannie's provider profile uses, and your desktop must reach the host on port 7445.
- In Mannie, Workers is a plugin. If the Workers settings are not in the settings sidebar, turn Workers on in the Plugins settings first.

## Install

Unpack the archive, then run the install command from an administrator's terminal (Windows) or as root (Linux and FreeBSD).

Linux:

```sh
useradd --system --no-create-home --shell /usr/sbin/nologin mannie-worker
tar -xzf mannie-worker-<version>-linux-x86_64.tar.gz
./mannie-worker service install --user mannie-worker
```

FreeBSD:

```sh
pw useradd mannie-worker -d /nonexistent -s /usr/sbin/nologin -c "Mannie worker"
pkg install ca_root_nss
tar -xzf mannie-worker-<version>-freebsd-x86_64.tar.gz
./mannie-worker service install --user mannie-worker
```

Windows (PowerShell as administrator):

```powershell
Expand-Archive mannie-worker-<version>-windows-x86_64.zip -DestinationPath .\mannie-worker
.\mannie-worker\mannie-worker.exe service install
```

`service install` copies the program into its folder, writes a configuration and an empty host instructions file when there are none, and starts the service:

| System  | Configuration and host instructions | Data                                |
| ------- | ----------------------------------- | ----------------------------------- |
| Linux   | `/etc/mannie-worker/`               | `/var/lib/mannie-worker/`           |
| FreeBSD | `/usr/local/etc/mannie-worker/`     | `/var/db/mannie-worker/`            |
| Windows | `%ProgramData%\Mannie\worker\`      | `%ProgramData%\Mannie\worker\data\` |

## Configure

Edit `worker.toml` in the configuration folder as root or administrator. The worker's own account can only read it, and the worker refuses to start if it could change it.

```toml
version = 1
name = "build-01"
purpose = "Builds and tests the backend services"

[ceiling]
mode = "ask"
allowed_folders = ["/srv/app"]
rules = [{ command = "git status" }]
```

- `name` and `purpose` tell Mannie's assistant what the host is for.
- `mode` decides what the worker may do: `read_only` only reads and searches; `ask` asks you, through Mannie, before every change and command; `autonomous` changes files inside the allowed folders and runs the commands that `rules` allow without asking, and asks for everything else; `full_access` changes files and runs commands anywhere on the host without asking anybody.
- `allow_full_access_from_master` under `[ceiling]`, `true` unless you set it, lets a conversation that is in full access in Mannie answer this host's requests without asking you. Set it to `false` on a host where you want to decide every request yourself.
- `allowed_folders` are the folders where the worker reads without asking, and in autonomous mode also writes. Grant the worker's account access to them: on Windows, for example, `icacls D:\work /grant "NT SERVICE\mannie-worker:(OI)(CI)M"`.
- `host-instructions.md` beside the configuration is read for every task: write there what the host is for, its conventions and what to leave alone.

Restart the service after changing the configuration: `systemctl restart mannie-worker` (Linux), `service mannie_worker restart` (FreeBSD), or `Restart-Service mannie-worker` (Windows).

## Open the port

Allow incoming connections to port 7445 from your desktop:

- Linux with ufw: `ufw allow from <desktop address> to any port 7445 proto tcp`.
- FreeBSD with pf: add a `pass in proto tcp from <desktop address> to port 7445` rule.
- Windows: `New-NetFirewallRule -DisplayName "Mannie worker" -Direction Inbound -Protocol TCP -LocalPort 7445 -RemoteAddress <desktop address> -Action Allow`.

## Pair

On the host, as root or administrator:

```sh
mannie-worker pair
```

It prints the worker's address and a code that is valid for ten minutes. In Mannie, choose "Pair a worker…" in the Workers settings, enter the address and the code, and, if you like, a name that Mannie uses for the worker in place of `name` in its configuration, then choose "Pair". "Rename…" in the worker's menu changes that name later. From then on, the worker accepts only your Mannie, and your Mannie only this worker's key.

To pair the worker with another Mannie, run `mannie-worker pair --reset` on the host first.

## Update

Download the new release's archive for the host and its `.sig` file into the same folder, then run, as root or administrator:

```sh
mannie-worker update mannie-worker-<version>-linux-x86_64.tar.gz
```

It checks the signature, installs the new program, restarts the service, and puts the previous program back if the new one does not start. Mannie works with workers of its own minor version and the one before, and its Workers settings say when a worker needs an update.

## Remove

Remove the worker in Mannie's Workers settings, then on the host:

```sh
mannie-worker service uninstall
```

The program, the configuration and the data stay, and the command says where; delete their folders to remove them.

## When something does not work

- `mannie-worker status` says whether the configuration is valid, whether the service runs, whether the worker is paired and where it listens.
- The worker's logs are in the `logs` folder of its data folder.
- "Key changed" in Mannie means the worker was reset or reinstalled: remove it in Mannie and pair it again.
- A worker that cannot reach the provider fails its tasks with the provider's error: check the host's network and, on FreeBSD, that `ca_root_nss` is installed.
