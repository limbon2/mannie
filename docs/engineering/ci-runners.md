# CI runners

|              |                                                                                                                                |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Draft for review                                                                                                               |
| Last updated | 2026-10-16                                                                                                                     |
| Related      | [D27](../roadmap/milestones.md#5-decisions), [Repository](repository.md), [Releasing](releasing.md), [Standards](standards.md) |

The workflows in `.github/workflows/` run most of their jobs on the owner's self-hosted runners: virtual machines that run one job and are then reset. This document says which jobs run where, what each runner machine needs, and the rules that keep the runners safe.

## 1. Where the jobs run

| Workflow                     | Jobs                                                                                    | Runners                     |
| ---------------------------- | --------------------------------------------------------------------------------------- | --------------------------- |
| `ci.yml`                     | Every job                                                                               | Self-hosted                 |
| `provider-live.yml`          | Every job                                                                               | Self-hosted                 |
| `evals.yml`                  | Every job                                                                               | Self-hosted                 |
| `nightly.yml`, `freebsd.yml` | The worker's tests in a FreeBSD virtual machine                                         | GitHub-hosted, Ubuntu 24.04 |
| `release.yml`                | The checks of `ci.yml`                                                                  | Self-hosted                 |
| `release.yml`                | The FreeBSD tests, the builds, the install checks, the signatures and the draft release | GitHub-hosted               |

GitHub bills the minutes of its hosted runners in a private repository, and most of those minutes went to the Windows jobs of `ci.yml`. It does not bill jobs on self-hosted runners (verified on 2026-10-16). The FreeBSD virtual machine needs KVM, which GitHub's Linux runners have. The release's own jobs build, install and sign the artifacts on clean machines that the self-hosted runners never touch, so the signing key never reaches the owner's machines.

## 2. Labels

A self-hosted job asks for three labels, and only a runner that has all three takes it:

| Labels                                             | Machine              | Jobs                                                                                                             |
| -------------------------------------------------- | -------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `self-hosted`, `disposable`, `mannie-windows-11`   | Windows 11 x64       | `task ci`, the end-to-end tests on WebView2, the worker service                                                  |
| `self-hosted`, `disposable`, `mannie-ubuntu-24.04` | Ubuntu 24.04 LTS x64 | `task ci`, the credential store, the worker service, the worker builds, the live provider tests, the evaluations |
| `self-hosted`, `disposable`, `mannie-ubuntu-22.04` | Ubuntu 22.04 LTS x64 | The end-to-end tests on WebKitGTK, with the WebKitGTK 4.1 that V1 names                                          |

Every self-hosted runner has the label `self-hosted`. The owner gives a runner `disposable` and its system's label when registering it, and GitHub does not check them (verified on 2026-10-16).

`disposable` promises that the machine is thrown away after one job. The jobs expect a fresh machine, as GitHub's runners give them: they install and remove the `mannie-worker` service and the `mannie-ci` account, write to the system's folders, create a keyring and a wallet with a test password, and install packages. Give the label only to runners that keep the promise.

A run of `ci.yml` has ten jobs: three for Windows 11, six for Ubuntu 24.04 and one for Ubuntu 22.04. A machine runs one job at a time, so more machines for a label run more of its jobs at once. A job that finds no runner with its labels waits, and fails after 24 hours (verified on 2026-10-16). The nightly live tests and the weekly evaluations wait for the machines like every other job.

## 3. Runner machines

Each runner is a virtual machine, such as a Hyper-V machine, that is reset after every job:

1. Set up a machine for each system label with the software below and the runner application, without registering the runner, and take a checkpoint.
2. For each job, restore the checkpoint, start the machine and start the runner with a just-in-time configuration from GitHub's REST API, or register it as an ephemeral runner. Either way GitHub sends it one job and then removes it from the repository (verified on 2026-10-16). A request for a just-in-time configuration lists all three labels, `self-hosted` included, as GitHub's example does; a registration adds the other two, such as `--labels disposable,mannie-ubuntu-24.04`.
3. When the runner exits, shut the machine down and go back to step 2.

The checkpoint holds no registration and no token: they reach the machine after the restore. Update the runner application in the checkpoint when GitHub releases a new version. Otherwise every restored runner downloads the update itself, or, with automatic updates turned off, gets no jobs once it is 30 days behind (verified on 2026-10-16).

### Windows 11

- At least 4 virtual processors, 16 GB of memory and 64 GB of disk.
- An administrator account that signs in automatically, and the runner started in that session with administrator rights, for example by a scheduled task at sign-in that runs with the highest privileges. The end-to-end tests open the app's window on that session's desktop, and the worker service job installs a Windows service. A runner installed as a Windows service runs without a desktop.
- Git for Windows, whose bash runs the steps with `shell: bash` (verified on 2026-10-16).
- PowerShell 7, which runs the steps without a `shell` and those with `shell: pwsh` (verified on 2026-10-16).
- Visual Studio's C++ build tools, with the Desktop development with C++ workload, as for development ([CONTRIBUTING](../../CONTRIBUTING.md)).
- rustup without a toolchain. Each job installs the toolchain that `rust-toolchain.toml` pins.
- The WebView2 runtime, which Windows 11 includes. `task e2e` downloads the matching Microsoft Edge WebDriver.

### Ubuntu 24.04 and 22.04

- At least 4 virtual processors, 16 GB of memory and 64 GB of disk.
- A user for the runner that may use `sudo` without a password. The jobs install packages, and the worker service job installs a systemd unit.
- systemd as the init system, which a virtual machine has and a container usually lacks.
- The packages `build-essential`, `pkg-config`, `curl`, `git`, `python3` and `dbus`. The jobs install the rest, among them the desktop app's libraries, Xvfb and WebKitWebDriver.
- Docker, with the runner's user in the `docker` group. `cross` builds the FreeBSD worker in a container, and the Ollama live test runs Ollama as a service container, which needs a Linux runner with Docker (verified on 2026-10-16).
- rustup for the runner's user, without a toolchain.

## 4. Security

- Self-hosted runners serve this repository only while it is private. In a public repository, anyone's pull request could run code on them, and GitHub recommends self-hosted runners only for private repositories (verified on 2026-10-16). Before the repository goes public, move the jobs back to GitHub's runners ([section 5](#5-moving-a-job-back-to-githubs-runners)).
- Keep "Run workflows from fork pull requests" turned off in the repository's Actions settings, as it was on 2026-10-16. Only people with access to the private repository can fork it, and a fork's pull request would run its code on these machines.
- Give the machines the internet and nothing else on the owner's network. The jobs run the code of the repository's dependencies, which on GitHub's runners could reach none of the owner's machines. When a live provider test or an evaluation should use a server on the owner's network, allow that server's address and port alone.
- No self-hosted runner gets a label of one of GitHub's images, such as `ubuntu-24.04`. GitHub gives a job to a runner that has all of the job's labels, so such a runner could take a job meant for GitHub's runners, such as the release's signing job. The system labels therefore start with `mannie-`.
- Register the runners on this repository, on machines that hold nothing else: no personal files, no credentials and no worker in use. The worker service jobs install, change and remove the `mannie-worker` service and its folders, which would replace those of a real worker.
- A job gets a `GITHUB_TOKEN` that can only read the repository's contents, and the live tests and evaluations get API keys from the repository's secrets. The reset after the job discards both.
- The release's signing key reaches only GitHub-hosted runners.

## 5. Moving a job back to GitHub's runners

Replace the job's `runs-on` with a GitHub-hosted image: `windows-2025` in place of `mannie-windows-11`, and `ubuntu-24.04` or `ubuntu-22.04` in place of `mannie-ubuntu-24.04` or `mannie-ubuntu-22.04`. GitHub then bills the job's minutes again while the repository is private.
