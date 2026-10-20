# CI runners

|              |                                                                                                                                |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Draft for review                                                                                                               |
| Last updated | 2026-10-20                                                                                                                     |
| Related      | [D27](../roadmap/milestones.md#5-decisions), [Repository](repository.md), [Releasing](releasing.md), [Standards](standards.md) |

The workflows in `.github/workflows/` run most of their jobs on the owner's self-hosted runners: virtual machines that run one job and are then reset. This document says which jobs run where, what each runner machine needs, the rules that keep the runners safe, and how to set the runners up on Hyper-V.

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

The programs declared by plugins determine the number of build and smoke-test jobs. A machine runs one job at a time, so more machines for a label run more of its jobs at once. A job that finds no runner with its labels waits, and fails after 24 hours (verified on 2026-10-16). The nightly live tests and the weekly evaluations wait for the machines like every other job.

## 3. Runner machines

Each runner is a virtual machine, such as a Hyper-V machine, that is reset after every job:

1. Set up a machine for each system label with the software below and the runner application, without registering the runner, and take a checkpoint.
2. For each job, restore the checkpoint, start the machine and start the runner with a just-in-time configuration from GitHub's REST API, or register it as an ephemeral runner. Either way GitHub sends it one job and then removes it from the repository (verified on 2026-10-16). A request for a just-in-time configuration lists all three labels, `self-hosted` included, as GitHub's example does; a registration adds the other two, such as `--labels disposable,mannie-ubuntu-24.04`. On Hyper-V, the scripts of [section 6](#6-setup-on-hyper-v) do this when a job waits.
3. When the runner exits, shut the machine down and go back to step 2.

The checkpoint holds no registration and no token: they reach the machine after the restore. Update the runner application in the checkpoint when GitHub releases a new version. Otherwise every restored runner downloads the update itself, or, with automatic updates turned off, gets no jobs once it is 30 days behind (verified on 2026-10-16).

### Windows 11

- 4 virtual processors, 8 GB of memory and 64 GB of disk. The owner chose 8 GB on 2026-10-20 to leave more memory for work on the host. Validate memory use in the first complete Windows job before increasing the allocation.
- An administrator account that signs in automatically, and the runner started in that session with administrator rights, for example by a scheduled task at sign-in that runs with the highest privileges. The end-to-end tests open the app's window on that session's desktop, and the worker service job installs a Windows service. A runner installed as a Windows service runs without a desktop.
- Git for Windows, whose bash runs the steps with `shell: bash` (verified on 2026-10-20). The image setup checks that Git Bash is installed, and the runner adds Git's `bin` folder to the command search path that its jobs inherit.
- PowerShell 7 from the MSI or ZIP package, with its folder on `PATH`, which runs the steps without a `shell` and those with `shell: pwsh`. For WinGet, use `winget install --id Microsoft.PowerShell --source winget --installer-type wix`: WinGet defaults to MSIX starting with PowerShell 7.6, and its app execution alias failed the Actions tool installer's executable check on this Windows image. The image preflight rejects that alias ([installation methods](https://learn.microsoft.com/en-us/powershell/scripting/install/install-powershell-on-windows), verified on 2026-10-20).
- Visual Studio's C++ build tools, with the Desktop development with C++ workload, as for development ([CONTRIBUTING](../../CONTRIBUTING.md)).
- rustup without a toolchain. Each job installs the toolchain that `rust-toolchain.toml` pins.
- The WebView2 runtime, which Windows 11 includes. `task e2e` downloads the matching Microsoft Edge WebDriver.

### Ubuntu 24.04 and 22.04

- 4 virtual processors, 8 GB of memory and 64 GB of disk. GitHub's Linux runners for private repositories, which ran these jobs before, have 2 processors and the same memory (verified on 2026-10-17). With 4 processors, Cargo builds more crates at once, so a machine whose build steps die with `Killed`, the kernel's sign that memory ran out, needs more.
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

## 6. Setup on Hyper-V

<!-- cspell:words Autologon DPAPI fcopy jitconfig journalctl poweroff UEFI vhdx VMTPM -->

The scripts in `scripts/ci/runners/` run the runner machines of [section 3](#3-runner-machines) on a Windows 11 host with Hyper-V, such as the owner's workstation:

| Script                                                                                   | Runs on          | Does                                                                                             |
| ---------------------------------------------------------------------------------------- | ---------------- | ------------------------------------------------------------------------------------------------ |
| `host/install-controller.ps1`                                                            | Host             | Checks the prepared images and token, installs the host scripts and registers their sign-in task |
| `host/run-runners.ps1`                                                                   | Host             | Watches the queued jobs and runs a cycle on a machine for each job that the machine can take     |
| `host/run-runner-cycle.ps1`                                                              | Host             | Runs one cycle on one machine, from the just-in-time configuration to the cleanup                |
| `host/isolate-runner-machine.ps1`                                                        | Host             | Gives a machine the internet and nothing else on the owner's network                             |
| `host/save-github-token.ps1`                                                             | Host             | Stores the GitHub token, encrypted for the Windows user                                          |
| `windows/setup-runner-image.ps1`, `windows/start-runner.ps1`                             | Windows 11 guest | Install the runner application, the startup task and the script that the task runs               |
| `ubuntu/setup-runner-image.sh`, `ubuntu/start-runner.sh`, `ubuntu/mannie-runner.service` | Ubuntu guests    | Install the packages, the runner application, the unit and the script that the unit runs         |

The machines start on demand and one at a time, as the owner decided on 2026-10-16 and 2026-10-17: the owner's workstation has 31 GB of memory, which leaves room for one machine beside the owner's own work. A run of `ci.yml` then runs its jobs one after another, and no machine runs while no job waits.

### How a job reaches a machine

1. `run-runners.ps1` reads the repository's queued jobs every 30 seconds. For a job that asks for `self-hosted` and whose labels a configured machine has, it starts `run-runner-cycle.ps1` on that machine in the background, unless `MaxRunning` machines run already. A machine in a cycle whose runner has not taken a job yet counts for one of the queued jobs.
2. The cycle asks GitHub for a just-in-time configuration that lists the machine's three labels, restores the machine's `clean` checkpoint, checks the restored machine and starts it. The check expects automatic checkpoints turned off, Guest services turned on, and every network adapter on an internal switch with the port ACLs, extended port ACLs and guards of the [network](#network). A checkpoint keeps the machine's settings, so restoring one that was taken before them would take them away again. In that case the cycle names what is missing and starts nothing.
3. The cycle writes the configuration to the file `jitconfig` in the machine's folder under `C:\ProgramData\Mannie\ci-runners\handover`, copies it into the guest's inbox with `Copy-VMFile`, which goes through Guest services and needs no network, and deletes the file. Only the owner, SYSTEM, the administrators and the machine's own account may open that folder: the machine's worker process, which runs as that account, reads the file, and it cannot reach into the owner's profile (observed on 2026-10-17). Then it copies a marker the same way. The image's setup script creates the inbox, and the cycle never creates it, so a guest whose setup did not finish refuses the copy. Until the guest has booted, a copy fails too, and the cycle tries again every 5 seconds for 10 minutes.
4. In the guest, the startup task or unit waits for the marker, reads the configuration, deletes it and starts the runner with the configuration in the variable `ACTIONS_RUNNER_INPUT_JITCONFIG`. The runner masks the value in its logs and removes the variable from the environment of the job's steps (verified on 2026-10-16); on the command line, every process in the guest could read it. When the runner exits, the guest powers off.
5. Every 20 seconds the cycle reads the machine's state and the runner's status. It turns the machine off when the guest has powered off, when the runner is not online 10 minutes after the hand-over, takes no job within 10 minutes of coming online or stays offline for 10 minutes, when the guest is still on 5 minutes after GitHub removed the runner after its job, and after 7 hours at the latest. Then it removes the runner's registration, unless GitHub already did.
6. After a cycle without a job, the machine waits a minute before its next cycle, and twice as long after each further cycle without a job, up to 30 minutes.

### Host

- The Hyper-V cmdlets need an administrator or a member of the Hyper-V Administrators group. To run the runner scripts without elevation, add your account to the group once and sign in again:

  ```powershell
  Add-LocalGroupMember -SID S-1-5-32-578 -Member "$env:USERDOMAIN\$env:USERNAME"
  ```

- Each machine needs its memory free while it runs, 8 GB for Ubuntu or Windows 11, and room for its files: put them on a drive with at least 80 GB free for each machine.
- Install from the checked-out version of the scripts with `task ci:runners:install`. The controller and each cycle run the installed copies in `%ProgramData%\Mannie\ci-runners\host`; stop the controller after its current job finishes and install again when those scripts change.

### Network

The machines get an internal switch with its own subnet behind the host's NAT, so they share no network segment with the owner's network, and port ACLs on their network adapters, which the virtual switch enforces whatever runs in the guest. In an elevated PowerShell on the host:

1. Create the switch and give the host an address on it. The examples use 10.77.1.0/24; pick a subnet that none of your networks and VPNs use.

   ```powershell
   New-VMSwitch -Name 'Mannie CI' -SwitchType Internal
   New-NetIPAddress -InterfaceAlias 'vEthernet (Mannie CI)' -IPAddress 10.77.1.1 -PrefixLength 24
   ```

2. Translate the subnet's addresses to the host's. Windows allows one NAT per host (verified on 2026-10-16), so look at `Get-NetNat` first. Without a NAT, create one:

   ```powershell
   New-NetNat -Name 'Mannie CI' -InternalIPInterfaceAddressPrefix 10.77.1.0/24
   ```

   With one, as on the owner's workstation, where a NAT covers 10.77.0.0/24, create it again with a prefix that covers its subnet and the runners' subnet. Its machines lose the internet for the moment in between.

   ```powershell
   $nat = Get-NetNat
   Remove-NetNat -Name $nat.Name -Confirm:$false
   New-NetNat -Name $nat.Name -InternalIPInterfaceAddressPrefix 10.77.0.0/23
   ```

3. Let the host's firewall refuse every connection from the switch as well. The extended port ACLs of step 4 keep the guests away from the host even while its firewall is off, as it was on the owner's workstation on 2026-10-19, and this rule adds a second layer where the firewall is on:

   ```powershell
   New-NetFirewallRule -DisplayName 'Mannie CI: block the runner machines' -Direction Inbound -InterfaceAlias 'vEthernet (Mannie CI)' -Action Block
   ```

4. For each machine, once it is connected to the switch, add the port ACLs and guards:

   ```powershell
   .\scripts\ci\runners\host\isolate-runner-machine.ps1 -Machine mannie-ci-ubuntu-24.04
   ```

   The script denies traffic with the private, shared and link-local IPv4 ranges, multicast and the reserved range, and with all of IPv6, which the NAT does not carry. It allows the rest of IPv4, which is the internet, and the host's address on the switch, which is the guests' gateway: the deny rules also drop ARP for the addresses they cover, so a guest whose gateway is denied reaches nothing (observed on 2026-10-17). For each packet, the rule with the longest matching prefix applies (verified on 2026-10-16). Behind that address the host's services would answer, so the script adds extended port ACLs, which filter IP traffic and leave ARP alone: they refuse IP traffic with the host's address in both directions, and a stateful rule lets through only the SSH connections that the host opens, so that you can look into a running guest and the guest opens no connection to the host (observed on 2026-10-19). It also turns on DHCP guard and router guard, so that a guest cannot hand out addresses or routes to the other machines. The image validation accepts only the IPv4 default allow `0.0.0.0/0` and the gateway's host address, guarded by the extended ACLs. It rejects every other allow, including single private addresses, narrower or broader IPv4 prefixes, and all IPv6 prefixes. A server exception on the owner's network ([section 4](#4-security)) needs a separately reviewed address and port policy and its configuration before these scripts accept it.

5. Give each guest a fixed address, since the switch has no DHCP server: 10.77.1.11 for Windows 11, 10.77.1.12 for Ubuntu 24.04 and 10.77.1.13 for Ubuntu 22.04, with the prefix length 24, the gateway 10.77.1.1, and public DNS servers such as 1.1.1.1 and 9.9.9.9. The port ACLs block the DNS servers of the host and of your router.
6. Check from the guest that the internet answers and that the host and your router do not. Test the host on port 445, where Windows listens for file sharing: a port without a listener refuses the connection, which looks like a block but means that the guest reaches the host. On Ubuntu, with your router's address in the last line:

   ```bash
   curl -sS -o /dev/null -w '%{http_code}\n' https://api.github.com
   nc -zv -w 5 10.77.1.1 445
   nc -zv -w 5 192.168.0.1 80
   ```

   The first prints 200, and the other two time out. On Windows, `Test-NetConnection api.github.com -Port 443` succeeds, and `Test-NetConnection 10.77.1.1 -Port 445` and the same test of your router fail.

### Machines

Create each machine in an elevated PowerShell on the host, here the one for Ubuntu 24.04, with its files on the drive `E:` and the installation image of its system:

```powershell
$name = 'mannie-ci-ubuntu-24.04'
$folder = 'E:\Hyper-V'
New-VM -Name $name -Generation 2 -MemoryStartupBytes 8GB -Path $folder -NewVHDPath "$folder\$name\$name.vhdx" -NewVHDSizeBytes 64GB -SwitchName 'Mannie CI'
Set-VM -Name $name -ProcessorCount 4 -StaticMemory -CheckpointType ProductionOnly -AutomaticCheckpointsEnabled $false -AutomaticStartAction Nothing -AutomaticStopAction TurnOff
Enable-VMIntegrationService -VMName $name -Name 'Guest Service Interface'
Set-VMFirmware -VMName $name -SecureBootTemplate MicrosoftUEFICertificateAuthority
Add-VMDvdDrive -VMName $name -Path 'E:\ISO\ubuntu-24.04-live-server-amd64.iso'
Set-VMFirmware -VMName $name -FirstBootDevice (Get-VMDvdDrive -VMName $name)
.\scripts\ci\runners\host\isolate-runner-machine.ps1 -Machine $name
```

For Windows 11, give the machine `12GB` of memory, leave out the line with the Secure Boot template, which is for Linux, and add the TPM that Windows 11 requires:

```powershell
Set-VMKeyProtector -VMName $name -NewLocalKeyProtector
Enable-VMTPM -VMName $name
```

Automatic checkpoints would add a checkpoint at every start, and a production checkpoint holds no memory, so every restore boots the machine. When the host shuts down, the machines are turned off; the next cycle restores them anyway.

When the installer of the system asks for its medium to be removed, eject the image, so that the machine boots from its disk:

```powershell
Set-VMDvdDrive -VMName $name -Path $null
Set-VMFirmware -VMName $name -FirstBootDevice (Get-VMHardDiskDrive -VMName $name)
```

### Windows 11 image

1. Install Windows 11 with a local administrator account, without a Microsoft account. Set the address of the [network](#network), step 5, and check the network as in step 6:

   ```powershell
   New-NetIPAddress -InterfaceAlias Ethernet -IPAddress 10.77.1.11 -PrefixLength 24 -DefaultGateway 10.77.1.1
   Set-DnsClientServerAddress -InterfaceAlias Ethernet -ServerAddresses 1.1.1.1, 9.9.9.9
   ```

2. Install the software of [section 3](#windows-11), in an elevated PowerShell, signed in as that administrator:

   ```powershell
   winget install --exact --id Git.Git
   winget install --exact --id Microsoft.PowerShell
   winget install --exact --id Microsoft.VisualStudio.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
   Invoke-WebRequest -Uri https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe -OutFile "$env:TEMP\rustup-init.exe"
   & "$env:TEMP\rustup-init.exe" -y --default-toolchain none
   ```

3. On the host, in the repository's folder, copy the guest scripts in:

   ```powershell
   foreach ($file in 'setup-runner-image.ps1', 'start-runner.ps1') {
       Copy-VMFile -Name mannie-ci-windows-11 -SourcePath "$PWD\scripts\ci\runners\windows\$file" -DestinationPath "C:\mannie-runner-setup\$file" -FileSource Host -CreateFullPath
   }
   ```

4. In the guest, run `powershell -ExecutionPolicy Bypass -File C:\mannie-runner-setup\setup-runner-image.ps1` in an elevated PowerShell. The script checks the software, installs the newest runner in `C:\actions-runner` after checking its SHA-256 digest, and creates `C:\ProgramData\mannie-runner`, which only SYSTEM and the administrators may open, for the host's files. It registers the task `\Mannie\Mannie runner`, which runs `start-runner.ps1` at the administrator's sign-in with the highest privileges, at normal priority and without a time limit. It also turns off sleep, hibernation and automatic updates: an update could restart the machine during a job, and the next restore throws the update away. Name lookups to 1.1.1.1 and 9.9.9.9 go over HTTPS, without falling back to UDP, for the reason of the [Ubuntu images](#ubuntu-images), step 4.
5. Turn on automatic sign-in for the administrator with [Sysinternals Autologon](https://learn.microsoft.com/sysinternals/downloads/autologon), which keeps the password encrypted. Restart, and check that `C:\ProgramData\mannie-runner\start-runner.log` says that the task waits for the configuration.
6. Delete `C:\mannie-runner-setup` and shut down. On the host, take the checkpoint: `Checkpoint-VM -Name mannie-ci-windows-11 -SnapshotName clean`.

Watch a Windows machine in Virtual Machine Connection with a basic session. An enhanced session takes over the administrator's session, whose desktop the end-to-end tests need.

### Ubuntu images

For Ubuntu 24.04, and for 22.04 with its own name and address:

1. Install Ubuntu Server. Set the IPv4 address of the [network](#network), step 5, by hand: the subnet 10.77.1.0/24, the address 10.77.1.12, the gateway 10.77.1.1 and the name servers 1.1.1.1 and 9.9.9.9. Use the entire disk. With "Set up this disk as an LVM group", which the installer turns on, the root file system gets only half of it, and the setup script of step 4 extends it over the rest (observed on 2026-10-19). Create your own user and skip the snaps. The OpenSSH server is optional: with your public key in that user's `~/.ssh/authorized_keys`, you can look into a running guest from the host, and no other machine reaches it, since the port ACLs refuse the private networks and the NAT lets no connection in. Never forward your SSH agent into a guest, where a job could use it. The setup script adds the user `runner`, as whom the jobs run. Keep the kernel that the installer chooses, from `linux-generic`: its file copy daemon receives `Copy-VMFile`, and kernels from 6.10 on use a newer daemon, which mishandled the host's copies until a fix of 2025 (verified on 2026-10-16).
2. Sign in on the console and install the file copy daemon, so that the host can copy files in, then restart: the package's udev rules, which the daemon's unit needs, apply from the next boot. The Clipboard menu of Virtual Machine Connection can type text from the host.

   ```bash
   sudo apt-get update && sudo apt-get install -y linux-cloud-tools-generic && sudo reboot
   ```

3. On the host, in the repository's folder, copy the guest scripts in. On a Linux guest, the destination of `Copy-VMFile` is a folder, to which the guest's copy daemon adds the source file's name, and the trailing slash lets the daemon create that folder (observed on 2026-10-17).

   ```powershell
   foreach ($file in 'setup-runner-image.sh', 'start-runner.sh', 'mannie-runner.service') {
       Copy-VMFile -Name mannie-ci-ubuntu-24.04 -SourcePath "$PWD\scripts\ci\runners\ubuntu\$file" -DestinationPath '/tmp/mannie-runner-setup/' -FileSource Host -CreateFullPath
   }
   ```

4. In the guest, run `sudo sh /tmp/mannie-runner-setup/setup-runner-image.sh`. The script extends an LVM root volume over the free space of its volume group and sends name lookups over TLS to 1.1.1.1 and 9.9.9.9, for containers too, through a name server on Docker's bridge address: plain lookups use UDP, which the host's route to the internet may lose, as the owner's lost up to 60 percent of them while every lookup over TCP passed (observed on 2026-10-19). It installs the packages of [section 3](#ubuntu-2404-and-2204) and Docker, creates the user `runner` without a password, adds it to the `docker` group, lets it use `sudo` without a password, and installs rustup without a toolchain and the newest runner in `/home/runner/actions-runner`, after checking its SHA-256 digest. It creates `/var/lib/mannie-runner`, which only `runner` may open, for the host's files, and enables `mannie-runner.service`, which runs `start-runner.sh` as `runner` at every boot. It also turns off the daily package timers: each boot starts from the checkpoint, so they would start at once and hold the package lock that the jobs need. Its last line is `setup-runner-image: done`. When it stops earlier, it names what failed, and the image is not ready.
5. Check that `systemctl is-enabled mannie-runner` prints `enabled`, check the network as in the [network](#network), step 6, and power off with `sudo poweroff`. On the host, take the checkpoint: `Checkpoint-VM -Name mannie-ci-ubuntu-24.04 -SnapshotName clean`.

### GitHub token

1. On GitHub, under Settings, Developer settings, Personal access tokens, Fine-grained tokens, generate a token for the resource owner `limbon2`, with access to the repository `limbon2/mannie` only and the repository permissions Administration: Read and write, which creating and removing runners needs, and Actions: Read, which reading the queue needs (verified on 2026-10-16). Administration also lets the token change the repository's settings, which is why it covers no other repository. The expiry is yours to choose; a new token needs the next step again.
2. On the host, as the Windows user that runs the scripts, run `.\scripts\ci\runners\host\save-github-token.ps1` and paste the token at its prompt. The script checks that the token can read the runners and the workflow runs, and saves it in `%LOCALAPPDATA%\Mannie\ci-runners\github-token`, encrypted with DPAPI so that only that user on this host can read it. The scripts put the token only in the headers of their requests, and log neither the token nor a runner's configuration.

### Running the runners

1. After preparing the images and saving the token, run this in an elevated PowerShell as the Windows user who saved the token:

   ```powershell
   task ci:runners:install
   ```

   The installer checks the machines, checkpoints, isolation and token before installing the host scripts under `%ProgramData%\Mannie\ci-runners\host`, where only SYSTEM and administrators can change them. It writes `%LOCALAPPDATA%\Mannie\ci-runners\runners.psd1` and registers and starts the task `Mannie CI runners`. The task runs with the user's full administrator rights, at normal priority, without a window or a time limit, and starts again at sign-in. Only one controller can serve the repository at once, including a controller started manually.

2. To bring up the Ubuntu machines before the Windows image is ready, list only the prepared machines:

   ```powershell
   task ci:runners:install MACHINES=mannie-ci-ubuntu-24.04,mannie-ci-ubuntu-22.04
   ```

   Jobs for other labels wait. After preparing another machine, let any current job finish, stop the task, and install again with all the names. An installation refuses to replace the scripts of a running controller.

3. Read `%LOCALAPPDATA%\Mannie\ci-runners\runners.log` for each cycle and its failures. The controller reconciles registrations at startup and leaves a configured machine with a busy runner to finish its job. It watches that runner's status: it stops the machine after 10 minutes offline or unreadable, after 5 minutes without shutting down after a job finishes or its registration disappears, or after 7 hours since the machine booted. Deadlines are checked before requests, so an API outage cannot extend them, and stopping the VM frees its slot even when registration cleanup fails. If GitHub is unavailable at startup, the controller watches already-running configured machines under those bounds and discovers their runners when it recovers. It turns off other configured machines whose registrations are readable and idle. `MaxRunning` in the configuration defaults to one, so the machines share the workstation's available memory.

Each GitHub request has a 30-second connection limit. Windows PowerShell 5.1 uses `TimeoutSec`; PowerShell 7.4 and later also set `OperationTimeoutSeconds` to 30 seconds for stalled reads, because `TimeoutSec` became a connection-only alias ([Microsoft documentation](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/invoke-restmethod?view=powershell-7.5), verified on 2026-10-20). A normal cycle keeps its 7-hour overall deadline while the API is unavailable; inherited cycles use the shorter unreadable limit above. A limit may be exceeded by one polling interval and a request already in progress.

To run the controller in a console instead, stop its scheduled task and start `scripts\ci\runners\host\run-runners.ps1` as the same user with administrator rights or membership of Hyper-V Administrators. Ctrl+C stops it together with the cycle that is running, whose machine is turned off; a job that was running there fails and can be run again.

To try one machine without the queue, run one cycle on it and give it a job, for example by starting the Evaluations workflow, whose jobs ask for `mannie-ubuntu-24.04`, from the repository's Actions tab:

```powershell
.\scripts\ci\runners\host\run-runner-cycle.ps1 -Machine mannie-ci-ubuntu-24.04
```

### Updating an image

Update an image for updates of its system, for a new version of the runner application, and for new software that the jobs need:

1. Stop `run-runners.ps1`.
2. On the host, restore the checkpoint and start the machine with `Restore-VMSnapshot -VMName <machine> -Name clean -Confirm:$false` and `Start-VM -Name <machine>`. Its startup task or unit waits for a configuration that does not come; on Ubuntu, `sudo systemctl stop mannie-runner` ends the wait without powering off.
3. Install the updates: on Windows from Windows Update in the Settings, and on Ubuntu with `sudo apt-get update && sudo apt-get full-upgrade`, followed by a restart when a new kernel came with them.
4. Copy the guest scripts in and run the setup script again, as when you built the image. It installs the newest runner.
5. Shut the machine down, and replace its checkpoint on the host:

   ```powershell
   Remove-VMSnapshot -VMName <machine> -Name clean
   Checkpoint-VM -Name <machine> -SnapshotName clean
   ```

6. Start `run-runners.ps1` again.

### When a cycle fails

`runners.log` says what each cycle did and why it ended.

- "is not ready for jobs" names what the restored machine lacks. Fix the machine and take its checkpoint again.
- "could not copy jitconfig" means that the guest did not boot within 10 minutes, that its file copy service did not run, or that the image lacks the inbox because its setup script did not finish, which Hyper-V reports as "Unspecified error (0x80004005)" (observed on 2026-10-19). On Ubuntu, `systemctl status hv-fcopy-daemon` shows the service, and `systemctl is-enabled mannie-runner` shows whether the setup finished.
- "the runner did not come online" means that the guest received the configuration but its runner did not connect. The guest's logs are in `C:\ProgramData\mannie-runner\start-runner.log` and `C:\actions-runner\_diag` on Windows, and in `journalctl -u mannie-runner` on Ubuntu. To read them, stop `run-runners.ps1`, run one cycle with `run-runner-cycle.ps1`, and watch the machine in Virtual Machine Connection, or sign in with SSH, until the cycle turns it off.
