# Self-management

|              |                                                                                                                                                                                                                                                                        |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                               |
| Milestone    | Unscheduled                                                                                                                                                                                                                                                            |
| Last updated | 2026-11-26                                                                                                                                                                                                                                                             |
| Related      | [ADR-0081](../adr/0081-let-agents-inspect-and-configure-mannie.md), [Extension points](../architecture/extension-points.md), [Tools](../architecture/tools.md), [Security](../architecture/security.md), [Plugins](plugins.md), [System tools](system-tools/README.md) |

## Intent

The owner wants Mannie to know about itself (where and how it stores its database, backups, logs, skills and credentials, and what plugins and settings exist) so that when something breaks or the owner wants to configure Mannie, the agent can inspect and configure Mannie directly from chat. In the owner's words: "Basically let the user interact with the chat rather than searching for settings and such. Like for example if I ask it to add a new SSH host or something else and provide the required fields directly in chat, it will configure it itself without me opening settings and such."

## Behavior

### Self-knowledge and diagnostics

- Conversation agents receive a short instruction section describing Mannie's self-inspection, log reading and configuration tools, plus a one-line status line naming Mannie's version and every installed plugin with its live state (`on`, `off`, `starting`, `stopping` or `failed`).
- `mannie_inspect` runs without an approval prompt for conversation agents (including read-only agents such as `Plan`) and supports four topics:
  - `overview`: Mannie version, build profile, bundle identifier, OS, canonical paths (data folder, SQLite database, backups folder, log folder, skills folder, browser data folder), SQLite schema module versions and size, backup count, credential store backend, storage layout summary, installed plugins with live states and failure codes, and available core and plugin management targets.
  - `guide`: detailed self-knowledge and troubleshooting playbook for a core area (`core`, `storage`, `policy`, `plugins`, `troubleshooting`) or an active plugin, including its action list.
  - `config`: current secret-free configuration snapshot for any core domain or active plugin, with an optional `query` filter (such as an MCP server's stderr log or tool list, or a Network Client collection's requests).
  - `action_schema`: exact JSON Schema, security sensitivity flag and secret field descriptors for one action or all actions of a target.
- `mannie_read_logs` runs without an approval prompt for conversation agents. It lists available daily log files and crash files inside Mannie's log directory, reads bounded filtered excerpts (by severity level, tracing target prefix, substring and tail line count up to 500 lines), and refuses any path outside Mannie's canonical log folder or through symlinks.

### Chat-driven configuration

- `mannie_configure` lets non-read-only conversation agents invoke validated configuration actions on 15 core domains (`plugins`, `projects`, `providers`, `search_profiles`, `chatgpt`, `agents`, `skills`, `chat`, `tools`, `policy`, `child_agents`, `interface`, `window`, `notifications`, `updates`) and any active plugin or optional feature that contributes a `ManagementProvider`.
- Every call prepares a structured `Access::AppConfig` preview (target, action, title, sanitized fields, secret field descriptors and canonical sanitized JSON payload) without side effects, asks the owner in chat for `Allow once` or `Deny`, verifies the digest on re-prepare, commits `Executing` durably, and executes the change through the same Rust managers and validation that Settings uses.
- Conversation full access never skips approval for `Access::AppConfig`, and no reusable grant or command rule ever covers it.
- Turning an off plugin on via `mannie_configure` makes that plugin's `ManagementProvider` available immediately in the next step of the same reply so the agent can configure it without waiting for another turn. Operational tools of the newly enabled plugin (such as `run_ssh_command`) enter on the next user turn.
- When `mannie_configure` changes core entities whose UI selectors cache lists (providers, search profiles, custom agents, skills), the desktop UI refreshes those lists automatically. Plugins that stream changes over live channels update Settings views automatically.

### Hybrid secret handling

- For real or sensitive credentials, the agent omits secrets from `mannie_configure`. The chat approval card renders masked password inputs for every required or optional secret field declared by the action. When the owner types the value and chooses Allow, the desktop app sends it once via `decide_config_approval` directly to Rust and the OS credential store, never touching the model, transcript, SQLite or logs.
- For throwaway credentials that the owner intentionally pastes into chat, the agent may pass them in `mannie_configure`'s top-level `secrets` map (never inside `arguments`). The runtime scrubs those values to `"[redacted]"` before storing the tool call or emitting events, holds them only in memory cleared on drop for that single call, notes on the approval card that a value was provided from chat, and lets the owner override it in the masked input before allowing.

### What it does not do

- Child agents (`explore`, `general`, custom child agents) and workers cannot inspect or modify Mannie's configuration.
- `Plan` and read-only custom conversation agents can inspect Mannie and read logs, and cannot call `mannie_configure`.
- Agents cannot grant themselves new broad policy rules or grants (the `policy` domain only inspects and revokes existing grants and rules), cannot bypass native OS file pickers for local SQLite file registration or Notebook file uploads, and cannot restart Mannie automatically to install an update.

## Fit

- Adds `Access::AppConfig`, `ApprovalReason::AppConfig`, `ApprovalReason::AppConfigSecurity`, `Refusal::NoAppConfig`, `ToolsError::SecretRequired`, `ToolsError::SecretInvalid`, `ToolOutcome::MannieInspect`, `ToolOutcome::MannieLogs`, `ToolOutcome::MannieConfigure` and the master-only command `decide_config_approval` ([ADR-0081](../adr/0081-let-agents-inspect-and-configure-mannie.md)).
- Adds the Plugin SDK extension point `points::management` (`Management`, `IdRule::Owned`) so plugins expose self-descriptions, secret-free inspection and typed configuration actions without any core dependency on plugins ([ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md), [ADR-0034](../adr/0034-build-plugins-on-a-plugin-sdk.md)).
- Keeps `mannie.sqlite` protected under T-18, keeps `DecideApproval` unchanged for the Workers link, and keeps worker ceilings refusing `Access::AppConfig`.

## Open questions

None.

## Decisions

- 2026-11-26: The owner accepted [ADR-0081](../adr/0081-let-agents-inspect-and-configure-mannie.md) ([D75](../roadmap/milestones.md#5-decisions)): hybrid secret handling (masked approval-card inputs plus scrubbed throwaway chat secrets), always-ask authorization even in full access, unprompted read-only inspection and bounded redacted log reading, and full coverage across core domains and all 10 plugins.

## Design

### Contracts

- Runtime tools (offered when the desktop composition root binds `AppHost`):
  - `mannie_inspect(topic, target?, action?, query?)`: `topic` is `"overview" | "guide" | "config" | "action_schema"`.
  - `mannie_read_logs(file?, level?, target_prefix?, contains?, tail_lines?)`: bounded to `app_log_dir`, up to 500 lines.
  - `mannie_configure(target, action, arguments, secrets?)`: `arguments` is a JSON object of non-secret fields; `secrets` is an optional map of string keys to throwaway secret strings scrubbed prior to storage.
- IPC command: `decide_config_approval(DecideConfigApproval)` (`Deserialize` only, never `Serialize`, granted in `apps/desktop/src-tauri/build.rs` and `capabilities/main.json`).
- SDK extension point: `mannie_plugin_sdk::points::management::{Management, PluginManagement, ManagementProvider, ManagementGuide, ManagementActionSpec, SecretFieldSpec, ManagementSnapshot, PrepareActionRequest, PreparedAction, ExecuteActionRequest, ActionResult, ManagementError}`.

### Plugin coverage

- Full read and configure support: `ssh`, `mcp`, `proxmox`, `terminal`, `network`, `notebooks`, `browser`, `workers` and optional feature `workers.databases`.
- Read-only guide and diagnostics: `git`, `images`.

### Message keys

- Added under `tools_` in `apps/desktop/messages/en.json` for `Access::AppConfig` rendering, approval reasons, security notices, masked secret field labels and hints, error messages (`secret_required`, `secret_invalid`), refusal (`no_app_config`) and tool-call summaries (`mannie_inspect`, `mannie_read_logs`, `mannie_configure`).

### Threat notes

- Threats `T-128` through `T-134` in [security](../architecture/security.md#5-threats-and-mitigations) cover prompt-injected reconfiguration, throwaway secret scrubbing before persistence, approval-card secret isolation from the Workers link, log path confinement, prepare/re-prepare digest binding, child/worker isolation and active-work warnings before disabling plugins.

## Acceptance

- Policy unit tests confirm `Access::AppConfig` always asks (even in full access), never offers grants or rules, and refuses under every worker ceiling mode with `no_app_config`.
- SDK protocol and plugin harness conformance tests validate `points::management` types and every plugin's `ManagementProvider`.
- Runtime integration tests verify `mannie_inspect`, `mannie_read_logs` (including traversal and symlink refusal), `mannie_configure` (approval, denial, digest mismatch, crash recovery, mid-reply plugin enable followed by plugin configuration), planted-secret audits for both approval-card and scrubbed-chat paths, prompt-injection corpus blocking, and child/`Plan`/worker isolation.
- Plugin tests verify end-to-end management flows across all 10 plugins, including SSH `add_host` -> `scan_host_key` -> `trust_host_key` -> `test_host`, MCP dynamic secrets, and Proxmox certificate pinning and VM allowlist updates.
- Vitest UI tests verify `ToolApproval.svelte` and `ToolAccessList.svelte` rendering, masked secret inputs, required validation, keyboard operability and pseudo-locale coverage, and `task ci` passes cleanly.
