# Goals and scope

|              |                                                                                                                                                                                           |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                          |
| Last updated | 2026-10-20                                                                                                                                                                                |
| Related      | [Vision](vision.md), [Feature protocol](../features/README.md), [Milestones](../roadmap/milestones.md), [Releasing](../engineering/releasing.md), [Security](../architecture/security.md) |

## 1. What Mannie is for

Mannie lets one owner put AI agents to work on their own machines: chat on the desktop, act on the workstation, and carry out tasks on other hosts through workers. The first features are chat, system tools, child agents and workers ([decision D4](../roadmap/milestones.md#5-decisions)). Since M5, Mannie is a core with chat, system tools and child agents, and plugins that each owner turns on and off, with workers as the first plugin ([decision D26](../roadmap/milestones.md#5-decisions)). Everything after them arrives through the [feature protocol](../features/README.md), so the scope of a release is decided as the project goes. The quality bar in this document applies to every release regardless of its scope.

## 2. Goals

| ID  | Goal                                                                                                                                                                       | How we know                                                                                                                            |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| G1  | Work survives failures. Replies, runs and tasks survive window reloads, crashes, power loss and dropped links.                                                             | Reload, kill and power-loss tests lose no committed data (NFR-REL-01 to NFR-REL-03).                                                   |
| G2  | Models act on the owner's machines only within the owner's rules.                                                                                                          | A prompt-injection corpus and crash tests show no unapproved and no repeated side effect.                                              |
| G3  | Work reaches the right host. With the Workers plugin on, the master delegates tasks to workers on Windows, Linux and FreeBSD, and each worker handles its host on its own. | Delegated tasks pass the system tools suite on each worker operating system.                                                           |
| G4  | The owner chooses the models. OpenAI and OpenAI-compatible servers work through provider profiles, and quirk profiles handle server-specific behavior.                     | Provider conformance tests (V8) pass for every supported server.                                                                       |
| G5  | Features are added without rewrites.                                                                                                                                       | Each shipped feature changed only its own slice, its document, one roadmap line and the changelog, plus ADRs for new extension points. |
| G6  | Every milestone ships a release that upgrades the data of the releases before it.                                                                                          | The release pipeline (V10) and upgrade tests from fixture databases of every earlier release pass.                                     |

## 3. Non-goals

| Non-goal                                                          | Status                                                                                                                                                                                                                          |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| macOS as a desktop or worker platform                             | Not planned (D6)                                                                                                                                                                                                                |
| More than one user: accounts, teams, a hosted service             | Not planned                                                                                                                                                                                                                     |
| Remote access to the master from a browser or a phone             | Not planned                                                                                                                                                                                                                     |
| The master running tools on a worker's host                       | Never ([ADR-0015](../adr/0015-run-an-agent-runtime-on-every-machine.md)). A computer without Mannie is reached through a plugin's remote commands instead ([ADR-0036](../adr/0036-let-plugins-run-commands-on-remote-hosts.md)) |
| Workers delegating tasks to other workers                         | Not planned. It can return as a feature request.                                                                                                                                                                                |
| Interface languages other than English                            | Not yet. Adding one takes a new message catalog and no code changes (D10, [ADR-0019](../adr/0019-keep-visible-text-in-message-catalogs.md))                                                                                     |
| Telemetry, usage reporting or any other phone-home                | Never                                                                                                                                                                                                                           |
| Releases below the Definition of Done, such as prototypes or MVPs | Never (D2)                                                                                                                                                                                                                      |
| An LLM framework or agent library in the core                     | Never ([ADR-0011](../adr/0011-own-the-provider-adapters.md))                                                                                                                                                                    |
| Third-party scripts or plugins running inside the webview         | Never ([ADR-0004](../adr/0004-keep-system-access-in-the-rust-core.md))                                                                                                                                                          |
| Third-party plugins, or an API or SDK for them                    | Not yet. Plugins are internal, and opening them to third parties needs an ADR of its own ([ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md))                                                                      |

## 4. Non-functional requirements

Every feature meets these requirements. The targets in §4.1 are checked by benchmarks from M1 on, on a reference machine: a Windows 11 x86_64 desktop with 8 cores, 16 GB of memory and an SSD.

### 4.1 Performance

| ID          | Requirement                                                                       |
| ----------- | --------------------------------------------------------------------------------- |
| NFR-PERF-01 | A streamed delta appears in the UI within 100 ms of reaching the Rust core (p95). |
| NFR-PERF-02 | With 8 runs streaming at once, the UI responds to input within 50 ms (p95).       |
| NFR-PERF-03 | A cold start shows a usable window within 2 s.                                    |
| NFR-PERF-04 | A conversation with 10,000 parts opens within 1 s.                                |
| NFR-PERF-05 | An idle worker uses less than 50 MB of memory.                                    |

### 4.2 Reliability

| ID         | Requirement                                                                                                                                     |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| NFR-REL-01 | Committed data survives a crash or a power loss. Streamed text loses at most its last batch of 250 ms.                                          |
| NFR-REL-02 | An operation never executes twice. An operation that may have executed is marked uncertain and is not repeated automatically.                   |
| NFR-REL-03 | Runs and tasks continue when a window reloads or a link drops. A subscriber that reconnects replays every event after its cursor, without gaps. |
| NFR-REL-04 | Each release upgrades the data of every earlier release. A database with a newer schema is refused and left untouched.                          |
| NFR-REL-05 | A failing run, task or worker does not stop the others.                                                                                         |

### 4.3 Security

| ID         | Requirement                                                                                                                                              |
| ---------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| NFR-SEC-01 | Provider credentials are stored only in the master's operating system credential store. Leased copies on workers exist only in memory.                   |
| NFR-SEC-02 | No side effect happens without an allow decision from policy or from the owner.                                                                          |
| NFR-SEC-03 | Model output, tool output and file contents shown in the UI cannot run script or load remote resources.                                                  |
| NFR-SEC-04 | The master and each worker authenticate each other with pinned keys, and all link traffic is encrypted.                                                  |
| NFR-SEC-05 | Secrets never appear in logs, databases, crash files, URLs, command arguments or child process environments. Tests check this with planted test secrets. |

### 4.4 Operability

| ID         | Requirement                                                                                                         |
| ---------- | ------------------------------------------------------------------------------------------------------------------- |
| NFR-OPS-01 | Logs are structured, stay on the machine, rotate, and contain no secrets.                                           |
| NFR-OPS-02 | Following the documentation, the owner installs a worker as a service in under 10 minutes on each supported system. |
| NFR-OPS-03 | The master works with workers that are one minor version behind.                                                    |
| NFR-OPS-04 | Mannie needs no network access other than to the providers, workers and remote hosts that the owner configured.     |

### 4.5 Usability

| ID        | Requirement                                                                                                            |
| --------- | ---------------------------------------------------------------------------------------------------------------------- |
| NFR-UX-01 | The desktop app meets WCAG 2.1 AA, and every action works from the keyboard.                                           |
| NFR-UX-02 | Every view has loading, empty and error states. An error says what failed and what the owner can do about it.          |
| NFR-UX-03 | The app opens into the last conversation. There are no landing, welcome or onboarding screens.                         |
| NFR-UX-04 | All visible text comes from message catalogs. In the pseudo-locale, no text bypasses the catalog and no layout breaks. |

## 5. Compatibility

| Area          | Supported                                                                                                                                                         |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Desktop       | Windows 11 x86_64 with the WebView2 Evergreen runtime. Linux x86_64 with WebKitGTK 4.1: Ubuntu 22.04 LTS and newer, Debian 12 and newer, current Fedora releases. |
| Worker        | Windows 11, Windows Server 2022 and Windows Server 2025 on x86_64. Linux x86_64 on any distribution, as a static binary. FreeBSD 14 and 15 on amd64.              |
| Providers     | The OpenAI Responses API. OpenAI-compatible Chat Completions on vLLM, SGLang, LM Studio, Ollama and OpenRouter.                                                   |
| Architectures | x86_64 only. Other architectures can be added as features.                                                                                                        |

Windows 10 reached its end of support on 2025-10-14 and is not supported. [ADR-0008](../adr/0008-one-workspace-two-binaries.md) records the baselines, and M0 validates them.

## 6. Releases

Every milestone from M1 on ends with a 0.x release (D15). The first one, v0.1.0, ships at the end of M1. Version 1.0 is declared by a later decision and has its own gate: desktop auto-update, a security review, user documentation, and another look at code signing (D16). [Releasing](../engineering/releasing.md) describes the process.

## 7. Assumptions and constraints

- There is one owner and one master, and each worker is paired with one master.
- The master can open a network connection to each worker's port. Workers do not connect to the master.
- A worker can reach the provider endpoints that its tasks use.
- The owner administers the worker hosts and decides their policy ceilings.
- The owner is responsible for disk encryption, operating system updates and machine backups. Mannie keeps its own data consistent.
- The bundle identifier is io.github.limbon2.mannie (D9).
