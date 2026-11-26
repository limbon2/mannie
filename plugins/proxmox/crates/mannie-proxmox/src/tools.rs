//! Static tools contributed by the Proxmox plugin (ADR-0080, decision 7):
//! - Read-only (`ReadsOwnData`, offered to `AgentSelector::Every`):
//!   - `proxmox_list_vms`
//!   - `proxmox_get_vm_config`
//!   - `proxmox_list_isos`
//! - Side-effect (`SideEffects`, offered to `conversation` and `general`):
//!   - `proxmox_attach_iso` (`Config`)
//!   - `proxmox_eject_iso` (`Config`)
//!   - `proxmox_set_boot_order` (`Config`)
//!   - `proxmox_vm_power` (`Power`)
//!   - `proxmox_vnc_screenshot` (`Vnc`)
//!   - `proxmox_vnc_pointer` (`Vnc`)
//!   - `proxmox_vnc_keyboard` (`Vnc`)
//!   - `proxmox_vnc_close` (`Vnc`)
//!
//! Every tool enforces the owner's fail-closed per-server VM allowlist in Rust
//! before touching the network or asking for approval, and verifies TOCTOU
//! parity between `prepare()` and `execute()`.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures_util::FutureExt as _;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::ProxmoxVmScope;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, PluginTool, ProxmoxVmAccess, Tool, ToolAccess, ToolCall, ToolExecution,
    ToolImage, ToolRefusal, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::keysyms::{KeyStroke, parse_key_combo, text_to_keystrokes};
use crate::model::{AllowedVmRecord, ProxmoxError, ServerRecord, is_valid_node};
use crate::section::SIDE_EFFECT_AGENTS;
use crate::state::ProxmoxManager;
use crate::vnc::{MAX_SCREENSHOT_DELAY_MS, MAX_WAIT_FOR_CHANGE_MS};

/// Tool listing explicitly allowed QEMU VMs with live status.
pub const TOOL_LIST_VMS: &str = "proxmox_list_vms";
/// Tool reading configuration and status of an allowed QEMU VM.
pub const TOOL_GET_VM_CONFIG: &str = "proxmox_get_vm_config";
/// Tool listing ISO images available on an allowed VM's node.
pub const TOOL_LIST_ISOS: &str = "proxmox_list_isos";
/// Tool attaching an ISO image to an allowed VM's CD-ROM drive.
pub const TOOL_ATTACH_ISO: &str = "proxmox_attach_iso";
/// Tool ejecting an ISO image from an allowed VM's CD-ROM drive.
pub const TOOL_EJECT_ISO: &str = "proxmox_eject_iso";
/// Tool updating the boot device order of an allowed VM.
pub const TOOL_SET_BOOT_ORDER: &str = "proxmox_set_boot_order";
/// Tool performing a power action on an allowed VM.
pub const TOOL_VM_POWER: &str = "proxmox_vm_power";
/// Tool capturing a VNC console screenshot of an allowed VM.
pub const TOOL_VNC_SCREENSHOT: &str = "proxmox_vnc_screenshot";
/// Tool sending pointer input to an allowed VM's VNC console.
pub const TOOL_VNC_POINTER: &str = "proxmox_vnc_pointer";
/// Tool sending non-secret keyboard input to an allowed VM's VNC console.
pub const TOOL_VNC_KEYBOARD: &str = "proxmox_vnc_keyboard";
/// Tool closing this conversation's VNC session on an allowed VM.
pub const TOOL_VNC_CLOSE: &str = "proxmox_vnc_close";

/// Maximum characters of keyboard text sent in one tool call.
const MAX_KEYBOARD_TEXT_CHARS: usize = 512;

/// Maximum key entries sent in a `keys` list in one tool call.
const MAX_KEYBOARD_KEYS: usize = 64;

/// Builds all 11 static `PluginTool` registrations for the Proxmox plugin.
pub(crate) fn all_tools(manager: &ProxmoxManager) -> Vec<PluginTool> {
    let side_agents: Vec<String> = SIDE_EFFECT_AGENTS.iter().map(|s| (*s).to_owned()).collect();
    vec![
        // Read-only discovery tools (offered to Every agent, enforcing allowlist in Rust)
        PluginTool::new(
            Arc::new(ListVmsTool::new(manager.clone())),
            AgentSelector::Every,
        ),
        PluginTool::new(
            Arc::new(GetVmConfigTool::new(manager.clone())),
            AgentSelector::Every,
        ),
        PluginTool::new(
            Arc::new(ListIsosTool::new(manager.clone())),
            AgentSelector::Every,
        ),
        // Side-effect tools (conversation + general child agent only)
        PluginTool::new(
            Arc::new(AttachIsoTool::new(manager.clone())),
            AgentSelector::Agents(side_agents.clone()),
        )
        .with_side_effects(),
        PluginTool::new(
            Arc::new(EjectIsoTool::new(manager.clone())),
            AgentSelector::Agents(side_agents.clone()),
        )
        .with_side_effects(),
        PluginTool::new(
            Arc::new(SetBootOrderTool::new(manager.clone())),
            AgentSelector::Agents(side_agents.clone()),
        )
        .with_side_effects(),
        PluginTool::new(
            Arc::new(VmPowerTool::new(manager.clone())),
            AgentSelector::Agents(side_agents.clone()),
        )
        .with_side_effects(),
        PluginTool::new(
            Arc::new(VncScreenshotTool::new(manager.clone())),
            AgentSelector::Agents(side_agents.clone()),
        )
        .with_side_effects(),
        PluginTool::new(
            Arc::new(VncPointerTool::new(manager.clone())),
            AgentSelector::Agents(side_agents.clone()),
        )
        .with_side_effects(),
        PluginTool::new(
            Arc::new(VncKeyboardTool::new(manager.clone())),
            AgentSelector::Agents(side_agents.clone()),
        )
        .with_side_effects(),
        PluginTool::new(
            Arc::new(VncCloseTool::new(manager.clone())),
            AgentSelector::Agents(side_agents),
        )
        .with_side_effects(),
    ]
}

// ---------------------------------------------------------------------------
// Shared argument helpers & access builders
// ---------------------------------------------------------------------------

fn parse_args<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T, ToolRefusal> {
    serde_json::from_str(raw)
        .map_err(|error| ToolRefusal::new(format!("invalid arguments: {error}")))
}

fn error_refusal(error: &ProxmoxError) -> ToolRefusal {
    ToolRefusal::new(error.to_string()).with_code(error.tool_code())
}

fn error_result(error: &ProxmoxError) -> ToolResult {
    let text = match error {
        ProxmoxError::OwnerInControl => {
            "The owner currently has control of this VM's live VNC console. Wait for the owner to finish typing and click Resume before calling VNC tools again."
                .to_owned()
        }
        ProxmoxError::VmNotAllowed => {
            "This VM is not in the owner's allowed VMs list for Mannie. Ask the owner to allow it in Settings -> Proxmox."
                .to_owned()
        }
        ProxmoxError::CertificateUntrusted => {
            "The Proxmox server's TLS certificate is not trusted by this computer. Ask the owner to inspect and pin the certificate in Settings -> Proxmox."
                .to_owned()
        }
        ProxmoxError::CertificateMismatch => {
            "The Proxmox server presented a different TLS certificate than the pinned fingerprint. Ask the owner to re-inspect the certificate in Settings -> Proxmox."
                .to_owned()
        }
        other => format!("Proxmox operation failed: {other}"),
    };
    ToolResult::new(text).with_failure(error.tool_code())
}

fn resolve_ready_allowed_vm(
    manager: &ProxmoxManager,
    server_ref: &str,
    vmid: u32,
) -> Result<(ServerRecord, AllowedVmRecord), ToolRefusal> {
    if vmid == 0 {
        return Err(ToolRefusal::new("vmid must be a positive integer"));
    }

    let (server, vm) = manager
        .resolve_allowed_vm(server_ref, vmid)
        .map_err(|error| error_refusal(&error))?;
    if !server.password_set {
        return Err(error_refusal(&ProxmoxError::PasswordMissing));
    }

    Ok((server, vm))
}

fn build_access(
    server: &ServerRecord,
    vm: &AllowedVmRecord,
    scope: ProxmoxVmScope,
    action: &str,
    summary: String,
) -> ProxmoxVmAccess {
    ProxmoxVmAccess::new(
        server.id.clone(),
        server.name.clone(),
        server.endpoint(),
        server.tls_identity(),
        vm.node.clone(),
        vm.vmid,
        vm.name.clone(),
        scope,
        action.to_owned(),
        summary,
    )
}

fn verify_authorized(execution: &ToolExecution, expected: &ProxmoxVmAccess) -> Option<ToolResult> {
    let expected_access = ToolAccess::ProxmoxVm(expected.clone());
    if execution.authorized.as_slice() != [expected_access] {
        return Some(
            ToolResult::new(
                "Not run: the Proxmox server, TLS certificate pin, or allowed VM changed after authorization.",
            )
            .with_failure("target_changed"),
        );
    }

    None
}

/// Validates a QEMU optical drive device name (`ide0..=3`, `sata0..=5`, `scsi0..=30`).
fn is_valid_cdrom_device(device: &str) -> bool {
    if let Some(num) = device.strip_prefix("ide") {
        return num.parse::<u8>().is_ok_and(|n| n <= 3);
    }

    if let Some(num) = device.strip_prefix("sata") {
        return num.parse::<u8>().is_ok_and(|n| n <= 5);
    }

    if let Some(num) = device.strip_prefix("scsi") {
        return num.parse::<u8>().is_ok_and(|n| n <= 30);
    }

    false
}

/// Validates a QEMU boot-order device identifier (`ideN`, `sataN`, `scsiN`, `virtioN`, `netN`).
fn is_valid_boot_device(device: &str) -> bool {
    if is_valid_cdrom_device(device) {
        return true;
    }

    if let Some(num) = device.strip_prefix("virtio") {
        return num.parse::<u8>().is_ok_and(|n| n <= 15);
    }

    if let Some(num) = device.strip_prefix("net") {
        return num.parse::<u8>().is_ok_and(|n| n <= 31);
    }

    false
}

/// Validates a Proxmox ISO volume ID (e.g. `local:iso/debian-12.iso`).
fn is_valid_iso_volid(volid: &str) -> bool {
    let trimmed = volid.trim();
    let len_ok = trimmed.len() >= 7 && trimmed.len() <= 256;
    if !len_ok {
        return false;
    }

    let safe_chars = !trimmed
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace() || matches!(ch, ',' | ';' | '\\'));
    if !safe_chars || trimmed.contains("..") {
        return false;
    }

    let Some((storage, rest)) = trimmed.split_once(":iso/") else {
        return false;
    };
    let storage_ok = !storage.is_empty()
        && storage
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    let filename_ok = !rest.is_empty() && !rest.starts_with('/') && !rest.ends_with('/');
    storage_ok && filename_ok
}

// ---------------------------------------------------------------------------
// 1. proxmox_list_vms (read-only)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ListVmsTool {
    manager: ProxmoxManager,
}

impl ListVmsTool {
    fn new(manager: ProxmoxManager) -> Self {
        Self { manager }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListVmsArgs {
    #[serde(default)]
    server: Option<String>,
}

impl Tool for ListVmsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            TOOL_LIST_VMS,
            "List QEMU virtual machines that the owner explicitly allowed for Mannie on configured Proxmox VE servers, with live power status.",
            json!({
                "type": "object",
                "properties": {
                    "server": {
                        "type": "string",
                        "description": "Optional Proxmox server name or ID to filter by."
                    }
                },
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let args = parse_args::<ListVmsArgs>(&call.arguments);
        let manager = self.manager.clone();
        async move {
            let args = args?;
            if let Some(ref server) = args.server
                && manager.find_server(server).is_none()
            {
                return Err(error_refusal(&ProxmoxError::ServerNotFound));
            }
            Ok(Vec::new())
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let manager = self.manager.clone();
        async move {
            let args: ListVmsArgs = match parse_args(&execution.arguments) {
                Ok(args) => args,
                Err(refusal) => return ToolResult::new(refusal.detail).with_failure("invalid_input"),
            };
            let listed = match manager.list_allowed_vms_live(args.server.as_deref()).await {
                Ok(listed) => listed,
                Err(error) => return error_result(&error),
            };

            if listed.is_empty() {
                return ToolResult::new(
                    "No Proxmox servers are registered yet. Ask the owner to add one in Settings -> Proxmox.",
                );
            }

            let mut lines = Vec::new();
            let mut total_vms = 0usize;
            for (server, vms) in &listed {
                lines.push(format!("Server {} ({}):", server.name, server.endpoint()));
                if vms.is_empty() {
                    lines.push(
                        "  (no allowed VMs configured yet; ask the owner to choose VMs in Settings -> Proxmox)"
                            .to_owned(),
                    );
                    continue;
                }
                for vm in vms {
                    total_vms += 1;
                    let cpu = vm
                        .maxcpu
                        .map_or_else(String::new, |c| format!(", {c} vCPU"));
                    let mem = vm.maxmem.map_or_else(String::new, |bytes| {
                        format!(", {} MiB RAM", bytes / (1024 * 1024))
                    });
                    lines.push(format!(
                        "  - VM {} ({}) on node {}: status={}{cpu}{mem}",
                        vm.vmid, vm.name, vm.node, vm.status
                    ));
                }
            }

            ToolResult::new(lines.join("\n")).with_data(json!({
                "action": "list_vms",
                "serverCount": listed.len(),
                "vmCount": total_vms,
            }))
        }
        .boxed()
    }
}

// ---------------------------------------------------------------------------
// 2. proxmox_get_vm_config (read-only)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct GetVmConfigTool {
    manager: ProxmoxManager,
}

impl GetVmConfigTool {
    fn new(manager: ProxmoxManager) -> Self {
        Self { manager }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetVmConfigArgs {
    server: String,
    vmid: u32,
}

impl Tool for GetVmConfigTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            TOOL_GET_VM_CONFIG,
            "Read the current configuration and power status of an allowed Proxmox QEMU VM (boot order, CD-ROM/ISO attachments, disks, CPU and memory).",
            json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string", "description": "Proxmox server name or ID." },
                    "vmid": { "type": "integer", "minimum": 1, "description": "Allowed QEMU VMID." }
                },
                "required": ["server", "vmid"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let args = parse_args::<GetVmConfigArgs>(&call.arguments);
        let manager = self.manager.clone();
        async move {
            let args = args?;
            drop(resolve_ready_allowed_vm(&manager, &args.server, args.vmid)?);
            Ok(Vec::new())
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let manager = self.manager.clone();
        async move {
            let args: GetVmConfigArgs = match parse_args(&execution.arguments) {
                Ok(args) => args,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail).with_failure("invalid_input");
                }
            };
            let (server, vm) = match resolve_ready_allowed_vm(&manager, &args.server, args.vmid) {
                Ok(pair) => pair,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail)
                        .with_failure(refusal.code.unwrap_or_else(|| "vm_not_allowed".to_owned()));
                }
            };

            let password = match manager.load_password(&server).await {
                Ok(password) => password,
                Err(error) => return error_result(&error),
            };
            let api = match manager.api_client(&server) {
                Ok(api) => api,
                Err(error) => return error_result(&error),
            };
            let config = match api.get_vm_config(&vm.node, vm.vmid, &password).await {
                Ok(cfg) => cfg,
                Err(error) => return error_result(&error),
            };
            let status = api.get_vm_status(&vm.node, vm.vmid, &password).await.ok();
            let power = status
                .as_ref()
                .and_then(|s| s.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");

            let formatted = format_vm_config(&server, &vm, power, &config);
            ToolResult::new(formatted).with_data(json!({
                "action": "get_vm_config",
                "server": server.name,
                "node": vm.node,
                "vmid": vm.vmid,
                "vmName": vm.name,
                "status": power,
            }))
        }
        .boxed()
    }
}

fn format_vm_config(
    server: &ServerRecord,
    vm: &AllowedVmRecord,
    power: &str,
    config: &BTreeMap<String, Value>,
) -> String {
    let mut lines = vec![format!(
        "VM {} ({}) on node {} of {} — status: {power}",
        vm.vmid, vm.name, vm.node, server.name
    )];

    if let Some(boot) = config.get("boot").and_then(Value::as_str) {
        lines.push(format!("boot: {boot}"));
    }
    if let Some(bios) = config.get("bios").and_then(Value::as_str) {
        lines.push(format!("bios: {bios}"));
    }
    if let Some(machine) = config.get("machine").and_then(Value::as_str) {
        lines.push(format!("machine: {machine}"));
    }
    if let Some(cores) = config.get("cores") {
        lines.push(format!("cores: {cores}"));
    }
    if let Some(memory) = config.get("memory") {
        lines.push(format!("memory: {memory} MiB"));
    }

    for (key, value) in config {
        let interesting_device = key.starts_with("ide")
            || key.starts_with("sata")
            || key.starts_with("scsi")
            || key.starts_with("virtio")
            || key.starts_with("net")
            || key.starts_with("efidisk")
            || key.starts_with("tpmstate");
        if interesting_device {
            let rendered = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            lines.push(format!("{key}: {rendered}"));
        }
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// 3. proxmox_list_isos (read-only)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ListIsosTool {
    manager: ProxmoxManager,
}

impl ListIsosTool {
    fn new(manager: ProxmoxManager) -> Self {
        Self { manager }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListIsosArgs {
    server: String,
    #[serde(default)]
    vmid: Option<u32>,
    #[serde(default)]
    node: Option<String>,
    #[serde(default)]
    storage: Option<String>,
}

impl Tool for ListIsosTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            TOOL_LIST_ISOS,
            "List ISO images available on Proxmox storage for an allowed VM's node.",
            json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string", "description": "Proxmox server name or ID." },
                    "vmid": { "type": "integer", "minimum": 1, "description": "Allowed QEMU VMID (determines the node)." },
                    "node": { "type": "string", "description": "Proxmox node name (must host at least one allowed VM)." },
                    "storage": { "type": "string", "description": "Optional Proxmox storage ID to filter by." }
                },
                "required": ["server"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let args = parse_args::<ListIsosArgs>(&call.arguments);
        let manager = self.manager.clone();
        async move {
            let args = args?;
            drop(resolve_iso_target(&manager, &args)?);
            Ok(Vec::new())
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let manager = self.manager.clone();
        async move {
            let args: ListIsosArgs = match parse_args(&execution.arguments) {
                Ok(args) => args,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail).with_failure("invalid_input");
                }
            };
            let (server, node) = match resolve_iso_target(&manager, &args) {
                Ok(pair) => pair,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail)
                        .with_failure(refusal.code.unwrap_or_else(|| "vm_not_allowed".to_owned()));
                }
            };

            let listed = match manager
                .list_isos(&server, &node, args.storage.as_deref())
                .await
            {
                Ok(listed) => listed,
                Err(error) => return error_result(&error),
            };

            if listed.isos.is_empty() {
                let message = if listed.checked_storages.is_empty()
                    && listed.denied_storages.is_empty()
                {
                    format!(
                        "No ISO-capable storages on node {node} of {} are accessible to this Proxmox account (check Datastore.Audit permissions on the ISO storage, or pass an explicit storage name to verify).",
                        server.name
                    )
                } else if listed.checked_storages.is_empty() {
                    format!(
                        "No ISO storages on node {node} of {} could be read because this Proxmox account lacked permission on: {} (check Datastore.Audit permissions).",
                        server.name,
                        listed.denied_storages.join(", ")
                    )
                } else if listed.denied_storages.is_empty() {
                    format!(
                        "No ISO images were found on accessible ISO storage(s) of node {node} on {} (checked: {}).",
                        server.name,
                        listed.checked_storages.join(", ")
                    )
                } else {
                    format!(
                        "No ISO images were found on accessible ISO storage(s) of node {node} on {} (checked: {}; permission denied on: {}).",
                        server.name,
                        listed.checked_storages.join(", "),
                        listed.denied_storages.join(", ")
                    )
                };
                return ToolResult::new(message).with_data(json!({
                    "action": "list_isos",
                    "server": server.name,
                    "node": node,
                    "isoCount": 0,
                    "checkedStorages": listed.checked_storages,
                    "deniedStorages": listed.denied_storages,
                }));
            }

            let mut lines = vec![format!("ISOs on node {node} of {}:", server.name)];
            for iso in &listed.isos {
                let size_str = iso.size.map_or_else(String::new, |bytes| {
                    format!(" ({} MiB)", bytes / (1024 * 1024))
                });
                lines.push(format!("- {}{size_str}", iso.volid));
            }
            ToolResult::new(lines.join("\n")).with_data(json!({
                "action": "list_isos",
                "server": server.name,
                "node": node,
                "isoCount": listed.isos.len(),
                "checkedStorages": listed.checked_storages,
                "deniedStorages": listed.denied_storages,
            }))
        }
        .boxed()
    }
}

fn resolve_iso_target(
    manager: &ProxmoxManager,
    args: &ListIsosArgs,
) -> Result<(ServerRecord, String), ToolRefusal> {
    if let Some(ref storage) = args.storage {
        let valid_storage = !storage.is_empty()
            && storage.len() <= 64
            && storage
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
        if !valid_storage {
            return Err(ToolRefusal::new("invalid storage identifier"));
        }
    }

    if let Some(vmid) = args.vmid {
        let (server, vm) = resolve_ready_allowed_vm(manager, &args.server, vmid)?;
        return Ok((server, vm.node));
    }

    if let Some(ref node) = args.node {
        if !is_valid_node(node) {
            return Err(ToolRefusal::new("invalid Proxmox node name"));
        }
        let server = manager
            .ensure_node_has_allowed_vm(&args.server, node)
            .map_err(|error| error_refusal(&error))?;
        if !server.password_set {
            return Err(error_refusal(&ProxmoxError::PasswordMissing));
        }
        return Ok((server, node.clone()));
    }

    Err(ToolRefusal::new(
        "specify either vmid or node (hosting an allowed VM) to list ISOs",
    ))
}

// ---------------------------------------------------------------------------
// 4. proxmox_attach_iso (Config side effect)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct AttachIsoTool {
    manager: ProxmoxManager,
}

impl AttachIsoTool {
    fn new(manager: ProxmoxManager) -> Self {
        Self { manager }
    }

    fn prepare_checked(
        &self,
        raw: &str,
    ) -> Result<
        (
            ServerRecord,
            AllowedVmRecord,
            String,
            String,
            ProxmoxVmAccess,
        ),
        ToolRefusal,
    > {
        let args: AttachIsoArgs = parse_args(raw)?;
        let device = args.device.unwrap_or_else(|| "ide2".to_owned());
        if !is_valid_cdrom_device(&device) {
            return Err(ToolRefusal::new(
                "device must be an optical bus slot such as ide2, ide0, sata0, or scsi1",
            ));
        }

        if !is_valid_iso_volid(&args.volid) {
            return Err(ToolRefusal::new(
                "volid must be a valid Proxmox ISO volume ID such as local:iso/debian.iso",
            ));
        }

        let (server, vm) = resolve_ready_allowed_vm(&self.manager, &args.server, args.vmid)?;
        let summary = format!("Attach ISO {} on {device}", args.volid);
        let access = build_access(&server, &vm, ProxmoxVmScope::Config, "attach_iso", summary);
        Ok((server, vm, device, args.volid, access))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttachIsoArgs {
    server: String,
    vmid: u32,
    volid: String,
    #[serde(default)]
    device: Option<String>,
}

impl Tool for AttachIsoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            TOOL_ATTACH_ISO,
            "Attach an ISO image from Proxmox storage to a CD-ROM drive on an allowed QEMU VM.",
            json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string", "description": "Proxmox server name or ID." },
                    "vmid": { "type": "integer", "minimum": 1, "description": "Allowed QEMU VMID." },
                    "volid": { "type": "string", "description": "ISO volume ID from proxmox_list_isos (e.g. local:iso/debian.iso)." },
                    "device": { "type": "string", "description": "CD-ROM device slot (default: ide2)." }
                },
                "required": ["server", "vmid", "volid"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked = self.prepare_checked(&call.arguments);
        async move {
            let (_, _, _, _, access) = checked?;
            Ok(vec![ToolAccess::ProxmoxVm(access)])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let this = self.clone();
        async move {
            let (server, vm, device, volid, access) = match this
                .prepare_checked(&execution.arguments)
            {
                Ok(checked) => checked,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail)
                        .with_failure(refusal.code.unwrap_or_else(|| "invalid_input".to_owned()));
                }
            };
            if let Some(result) = verify_authorized(&execution, &access) {
                return result;
            }

            let password = match this.manager.load_password(&server).await {
                Ok(password) => password,
                Err(error) => return error_result(&error),
            };
            let api = match this.manager.api_client(&server) {
                Ok(api) => api,
                Err(error) => return error_result(&error),
            };
            let value = format!("{volid},media=cdrom");
            if let Err(error) = api
                .update_vm_config(&vm.node, vm.vmid, &[(&device, value)], &password)
                .await
            {
                return error_result(&error);
            }

            ToolResult::new(format!(
                "Attached {volid} to {device} on VM {} ({}) on {}.",
                vm.vmid, vm.name, server.name
            ))
            .with_data(json!({
                "action": "attach_iso",
                "server": server.name,
                "node": vm.node,
                "vmid": vm.vmid,
                "vmName": vm.name,
                "device": device,
                "volid": volid,
            }))
        }
        .boxed()
    }
}

// ---------------------------------------------------------------------------
// 5. proxmox_eject_iso (Config side effect)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct EjectIsoTool {
    manager: ProxmoxManager,
}

impl EjectIsoTool {
    fn new(manager: ProxmoxManager) -> Self {
        Self { manager }
    }

    fn prepare_checked(
        &self,
        raw: &str,
    ) -> Result<(ServerRecord, AllowedVmRecord, String, ProxmoxVmAccess), ToolRefusal> {
        let args: EjectIsoArgs = parse_args(raw)?;
        let device = args.device.unwrap_or_else(|| "ide2".to_owned());
        if !is_valid_cdrom_device(&device) {
            return Err(ToolRefusal::new(
                "device must be an optical bus slot such as ide2, ide0, sata0, or scsi1",
            ));
        }

        let (server, vm) = resolve_ready_allowed_vm(&self.manager, &args.server, args.vmid)?;
        let summary = format!("Eject CD-ROM media from {device}");
        let access = build_access(&server, &vm, ProxmoxVmScope::Config, "eject_iso", summary);
        Ok((server, vm, device, access))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EjectIsoArgs {
    server: String,
    vmid: u32,
    #[serde(default)]
    device: Option<String>,
}

impl Tool for EjectIsoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            TOOL_EJECT_ISO,
            "Eject ISO media from a CD-ROM drive on an allowed QEMU VM.",
            json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string", "description": "Proxmox server name or ID." },
                    "vmid": { "type": "integer", "minimum": 1, "description": "Allowed QEMU VMID." },
                    "device": { "type": "string", "description": "CD-ROM device slot (default: ide2)." }
                },
                "required": ["server", "vmid"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked = self.prepare_checked(&call.arguments);
        async move {
            let (_, _, _, access) = checked?;
            Ok(vec![ToolAccess::ProxmoxVm(access)])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let this = self.clone();
        async move {
            let (server, vm, device, access) = match this.prepare_checked(&execution.arguments) {
                Ok(checked) => checked,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail)
                        .with_failure(refusal.code.unwrap_or_else(|| "invalid_input".to_owned()));
                }
            };
            if let Some(result) = verify_authorized(&execution, &access) {
                return result;
            }

            let password = match this.manager.load_password(&server).await {
                Ok(password) => password,
                Err(error) => return error_result(&error),
            };
            let api = match this.manager.api_client(&server) {
                Ok(api) => api,
                Err(error) => return error_result(&error),
            };
            if let Err(error) = api
                .update_vm_config(
                    &vm.node,
                    vm.vmid,
                    &[(&device, "none,media=cdrom".to_owned())],
                    &password,
                )
                .await
            {
                return error_result(&error);
            }

            ToolResult::new(format!(
                "Ejected ISO media from {device} on VM {} ({}) on {}.",
                vm.vmid, vm.name, server.name
            ))
            .with_data(json!({
                "action": "eject_iso",
                "server": server.name,
                "node": vm.node,
                "vmid": vm.vmid,
                "vmName": vm.name,
                "device": device,
            }))
        }
        .boxed()
    }
}

// ---------------------------------------------------------------------------
// 6. proxmox_set_boot_order (Config side effect)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct SetBootOrderTool {
    manager: ProxmoxManager,
}

impl SetBootOrderTool {
    fn new(manager: ProxmoxManager) -> Self {
        Self { manager }
    }

    fn prepare_checked(
        &self,
        raw: &str,
    ) -> Result<(ServerRecord, AllowedVmRecord, Vec<String>, ProxmoxVmAccess), ToolRefusal> {
        let args: SetBootOrderArgs = parse_args(raw)?;
        if args.order.is_empty() || args.order.len() > 16 {
            return Err(ToolRefusal::new(
                "order must contain between 1 and 16 QEMU device names (e.g. [\"ide2\", \"scsi0\"])",
            ));
        }

        for device in &args.order {
            if !is_valid_boot_device(device) {
                return Err(ToolRefusal::new(format!(
                    "invalid boot device {device:?}; expected names like ide2, scsi0, sata0, virtio0, or net0"
                )));
            }
        }

        let (server, vm) = resolve_ready_allowed_vm(&self.manager, &args.server, args.vmid)?;
        let joined = args.order.join(";");
        let summary = format!("Set boot order to {joined}");
        let access = build_access(
            &server,
            &vm,
            ProxmoxVmScope::Config,
            "set_boot_order",
            summary,
        );
        Ok((server, vm, args.order, access))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetBootOrderArgs {
    server: String,
    vmid: u32,
    order: Vec<String>,
}

impl Tool for SetBootOrderTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            TOOL_SET_BOOT_ORDER,
            "Set the QEMU boot device order on an allowed Proxmox VM (for example [\"ide2\", \"scsi0\"] to boot an attached ISO first).",
            json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string", "description": "Proxmox server name or ID." },
                    "vmid": { "type": "integer", "minimum": 1, "description": "Allowed QEMU VMID." },
                    "order": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "maxItems": 16,
                        "description": "Ordered list of QEMU boot devices (e.g. [\"ide2\", \"scsi0\"])."
                    }
                },
                "required": ["server", "vmid", "order"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked = self.prepare_checked(&call.arguments);
        async move {
            let (_, _, _, access) = checked?;
            Ok(vec![ToolAccess::ProxmoxVm(access)])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let this = self.clone();
        async move {
            let (server, vm, order, access) = match this.prepare_checked(&execution.arguments) {
                Ok(checked) => checked,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail)
                        .with_failure(refusal.code.unwrap_or_else(|| "invalid_input".to_owned()));
                }
            };
            if let Some(result) = verify_authorized(&execution, &access) {
                return result;
            }

            let password = match this.manager.load_password(&server).await {
                Ok(password) => password,
                Err(error) => return error_result(&error),
            };
            let api = match this.manager.api_client(&server) {
                Ok(api) => api,
                Err(error) => return error_result(&error),
            };
            let joined = order.join(";");
            let boot_param = format!("order={joined}");
            if let Err(error) = api
                .update_vm_config(&vm.node, vm.vmid, &[("boot", boot_param)], &password)
                .await
            {
                return error_result(&error);
            }

            ToolResult::new(format!(
                "Updated boot order on VM {} ({}) on {} to order={joined}.",
                vm.vmid, vm.name, server.name
            ))
            .with_data(json!({
                "action": "set_boot_order",
                "server": server.name,
                "node": vm.node,
                "vmid": vm.vmid,
                "vmName": vm.name,
                "order": order,
            }))
        }
        .boxed()
    }
}

// ---------------------------------------------------------------------------
// 7. proxmox_vm_power (Power side effect)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct VmPowerTool {
    manager: ProxmoxManager,
}

impl VmPowerTool {
    fn new(manager: ProxmoxManager) -> Self {
        Self { manager }
    }

    fn prepare_checked(
        &self,
        raw: &str,
    ) -> Result<(ServerRecord, AllowedVmRecord, String, bool, ProxmoxVmAccess), ToolRefusal> {
        let args: VmPowerArgs = parse_args(raw)?;
        let action = args.action.trim().to_ascii_lowercase();
        let valid_action = matches!(
            action.as_str(),
            "start" | "shutdown" | "reboot" | "stop" | "reset"
        );
        if !valid_action {
            return Err(ToolRefusal::new(
                "action must be one of: start, shutdown, reboot, stop, reset",
            ));
        }

        let (server, vm) = resolve_ready_allowed_vm(&self.manager, &args.server, args.vmid)?;
        let summary = format!("Power action: {action}");
        let access_action = format!("power_{action}");
        let access = build_access(&server, &vm, ProxmoxVmScope::Power, &access_action, summary);
        let wait = args.wait.unwrap_or(true);
        Ok((server, vm, action, wait, access))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VmPowerArgs {
    server: String,
    vmid: u32,
    action: String,
    #[serde(default)]
    wait: Option<bool>,
}

impl Tool for VmPowerTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            TOOL_VM_POWER,
            "Change the power state of an allowed Proxmox QEMU VM (start, shutdown, reboot, stop, or reset).",
            json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string", "description": "Proxmox server name or ID." },
                    "vmid": { "type": "integer", "minimum": 1, "description": "Allowed QEMU VMID." },
                    "action": {
                        "type": "string",
                        "enum": ["start", "shutdown", "reboot", "stop", "reset"],
                        "description": "Power action to run."
                    },
                    "wait": {
                        "type": "boolean",
                        "description": "Wait for the Proxmox task to finish (default: true)."
                    }
                },
                "required": ["server", "vmid", "action"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked = self.prepare_checked(&call.arguments);
        async move {
            let (_, _, _, _, access) = checked?;
            Ok(vec![ToolAccess::ProxmoxVm(access)])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let this = self.clone();
        async move {
            let (server, vm, action, wait, access) = match this
                .prepare_checked(&execution.arguments)
            {
                Ok(checked) => checked,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail)
                        .with_failure(refusal.code.unwrap_or_else(|| "invalid_input".to_owned()));
                }
            };
            if let Some(result) = verify_authorized(&execution, &access) {
                return result;
            }

            let password = match this.manager.load_password(&server).await {
                Ok(password) => password,
                Err(error) => return error_result(&error),
            };
            let api = match this.manager.api_client(&server) {
                Ok(api) => api,
                Err(error) => return error_result(&error),
            };
            let upid = match api.vm_power(&vm.node, vm.vmid, &action, &password).await {
                Ok(upid) => upid,
                Err(error) => return error_result(&error),
            };

            let task_exit = if wait && let Some(ref upid_str) = upid {
                match api.wait_task(&vm.node, upid_str, &password).await {
                    Ok(exit) => Some(exit),
                    Err(error) => return error_result(&error),
                }
            } else {
                None
            };

            let detail = match &task_exit {
                Some(exit) => format!(
                    "Power action '{action}' on VM {} ({}) on {} finished with status: {exit}.",
                    vm.vmid, vm.name, server.name
                ),
                None => format!(
                    "Started power action '{action}' on VM {} ({}) on {}.",
                    vm.vmid, vm.name, server.name
                ),
            };
            ToolResult::new(detail).with_data(json!({
                "action": format!("power_{action}"),
                "server": server.name,
                "node": vm.node,
                "vmid": vm.vmid,
                "vmName": vm.name,
                "upid": upid,
                "exitStatus": task_exit,
            }))
        }
        .boxed()
    }
}

// ---------------------------------------------------------------------------
// 8. proxmox_vnc_screenshot (Vnc side effect)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct VncScreenshotTool {
    manager: ProxmoxManager,
}

struct PreparedScreenshot {
    server: ServerRecord,
    vm: AllowedVmRecord,
    delay_ms: Option<u64>,
    wait_for_change_ms: Option<u64>,
    access: ProxmoxVmAccess,
}

impl VncScreenshotTool {
    fn new(manager: ProxmoxManager) -> Self {
        Self { manager }
    }

    fn prepare_checked(&self, raw: &str) -> Result<PreparedScreenshot, ToolRefusal> {
        let args: VncScreenshotArgs = parse_args(raw)?;
        validate_vnc_waits(args.delay_ms, args.wait_for_change_ms)?;
        let (server, vm) = resolve_ready_allowed_vm(&self.manager, &args.server, args.vmid)?;
        let summary = match (
            args.delay_ms.filter(|&ms| ms > 0),
            args.wait_for_change_ms.filter(|&ms| ms > 0),
        ) {
            (Some(delay), Some(wait)) => {
                format!(
                    "Capture VNC screenshot (delay {delay} ms, wait up to {wait} ms for change)"
                )
            }
            (Some(delay), None) => format!("Capture VNC screenshot (delay {delay} ms)"),
            (None, Some(wait)) => {
                format!("Capture VNC screenshot (wait up to {wait} ms for change)")
            }
            (None, None) => "Capture VNC console screenshot".to_owned(),
        };
        let access = build_access(&server, &vm, ProxmoxVmScope::Vnc, "vnc_screenshot", summary);
        Ok(PreparedScreenshot {
            server,
            vm,
            delay_ms: args.delay_ms,
            wait_for_change_ms: args.wait_for_change_ms,
            access,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VncScreenshotArgs {
    server: String,
    vmid: u32,
    #[serde(default)]
    delay_ms: Option<u64>,
    #[serde(default)]
    wait_for_change_ms: Option<u64>,
}

fn validate_vnc_waits(
    delay_ms: Option<u64>,
    wait_for_change_ms: Option<u64>,
) -> Result<(), ToolRefusal> {
    if let Some(delay) = delay_ms
        && delay > MAX_SCREENSHOT_DELAY_MS
    {
        return Err(ToolRefusal::new(format!(
            "delay_ms must be at most {MAX_SCREENSHOT_DELAY_MS} milliseconds (60 seconds)"
        )));
    }
    if let Some(wait) = wait_for_change_ms
        && wait > MAX_WAIT_FOR_CHANGE_MS
    {
        return Err(ToolRefusal::new(format!(
            "wait_for_change_ms must be at most {MAX_WAIT_FOR_CHANGE_MS} milliseconds; use delay_ms (up to {MAX_SCREENSHOT_DELAY_MS}) to pause during slow transitions, boots, or installs with spinners"
        )));
    }
    Ok(())
}

impl Tool for VncScreenshotTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            TOOL_VNC_SCREENSHOT,
            "Capture the current VNC console screen of an allowed Proxmox QEMU VM as an image, opening the conversation's VNC session if needed. Use delay_ms (up to 60000) to pause unconditionally during slow boots, package installs, or screens with a blinking cursor or spinner; use wait_for_change_ms (up to 10000) to return early as soon as any pixel changes.",
            json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string", "description": "Proxmox server name or ID." },
                    "vmid": { "type": "integer", "minimum": 1, "description": "Allowed QEMU VMID." },
                    "delay_ms": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 60000,
                        "description": "Optional unconditional pause in milliseconds before capturing (up to 60000). Prefer this over wait_for_change_ms when a blinking cursor or spinner would otherwise trigger early return."
                    },
                    "wait_for_change_ms": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 10000,
                        "description": "Optional milliseconds to wait for any pixel change before capturing (returns early on first change; max 10000)."
                    }
                },
                "required": ["server", "vmid"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked = self.prepare_checked(&call.arguments);
        async move {
            let prepared = checked?;
            Ok(vec![ToolAccess::ProxmoxVm(prepared.access)])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let this = self.clone();
        async move {
            let PreparedScreenshot {
                server,
                vm,
                delay_ms,
                wait_for_change_ms: wait_ms,
                access,
            } = match this.prepare_checked(&execution.arguments) {
                Ok(checked) => checked,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail)
                        .with_failure(refusal.code.unwrap_or_else(|| "invalid_input".to_owned()));
                }
            };
            if let Some(result) = verify_authorized(&execution, &access) {
                return result;
            }

            let shot = match this
                .manager
                .agent_vnc_screenshot(execution.conversation_id, &server, &vm, delay_ms, wait_ms)
                .await
            {
                Ok(shot) => shot,
                Err(error) => return error_result(&error),
            };

            let text = format!(
                "Captured VNC screenshot of VM {} ({}) on {}: {}x{} pixels (frame #{}, changed={}).",
                vm.vmid, vm.name, server.name, shot.width, shot.height, shot.seq, shot.changed
            );
            ToolResult::new(text)
                .with_images(vec![ToolImage::new(shot.png_bytes)])
                .with_data(json!({
                    "action": "vnc_screenshot",
                    "server": server.name,
                    "node": vm.node,
                    "vmid": vm.vmid,
                    "vmName": vm.name,
                    "width": shot.width,
                    "height": shot.height,
                    "changed": shot.changed,
                    "seq": shot.seq,
                }))
        }
        .boxed()
    }
}

// ---------------------------------------------------------------------------
// 9. proxmox_vnc_pointer (Vnc side effect)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct VncPointerTool {
    manager: ProxmoxManager,
}

impl VncPointerTool {
    fn new(manager: ProxmoxManager) -> Self {
        Self { manager }
    }

    fn prepare_checked(
        &self,
        raw: &str,
    ) -> Result<
        (
            ServerRecord,
            AllowedVmRecord,
            VncPointerArgs,
            ProxmoxVmAccess,
        ),
        ToolRefusal,
    > {
        let args: VncPointerArgs = parse_args(raw)?;
        validate_vnc_waits(args.delay_ms, args.wait_for_change_ms)?;
        let valid_action = matches!(
            args.action.as_str(),
            "move"
                | "click"
                | "double_click"
                | "right_click"
                | "middle_click"
                | "down"
                | "up"
                | "scroll"
        );
        if !valid_action {
            return Err(ToolRefusal::new(
                "action must be one of: move, click, double_click, right_click, middle_click, down, up, scroll",
            ));
        }

        let (server, vm) = resolve_ready_allowed_vm(&self.manager, &args.server, args.vmid)?;
        let summary = format!("VNC pointer {} at ({}, {})", args.action, args.x, args.y);
        let access = build_access(&server, &vm, ProxmoxVmScope::Vnc, "vnc_pointer", summary);
        Ok((server, vm, args, access))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VncPointerArgs {
    server: String,
    vmid: u32,
    action: String,
    x: u16,
    y: u16,
    #[serde(default)]
    button: Option<String>,
    #[serde(default)]
    scroll_delta: Option<i32>,
    #[serde(default)]
    delay_ms: Option<u64>,
    #[serde(default)]
    wait_for_change_ms: Option<u64>,
    #[serde(default)]
    screenshot: Option<bool>,
}

impl Tool for VncPointerTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            TOOL_VNC_POINTER,
            "Send mouse pointer movement, clicks, or scrolling to an allowed Proxmox QEMU VM's VNC console using framebuffer pixel coordinates. Use delay_ms (up to 60000) to pause unconditionally after clicking before capturing the screenshot when a page transition or spinner would otherwise return early.",
            json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string", "description": "Proxmox server name or ID." },
                    "vmid": { "type": "integer", "minimum": 1, "description": "Allowed QEMU VMID." },
                    "action": {
                        "type": "string",
                        "enum": ["move", "click", "double_click", "right_click", "middle_click", "down", "up", "scroll"],
                        "description": "Pointer gesture to perform."
                    },
                    "x": { "type": "integer", "minimum": 0, "maximum": 4095, "description": "X coordinate in VNC pixels." },
                    "y": { "type": "integer", "minimum": 0, "maximum": 4095, "description": "Y coordinate in VNC pixels." },
                    "button": { "type": "string", "enum": ["left", "middle", "right"], "description": "Mouse button for click/down (default: left)." },
                    "scroll_delta": { "type": "integer", "description": "Positive scrolls up, negative scrolls down." },
                    "delay_ms": { "type": "integer", "minimum": 0, "maximum": 60000, "description": "Optional unconditional pause in milliseconds after the gesture and before capturing the screenshot (up to 60000)." },
                    "wait_for_change_ms": { "type": "integer", "minimum": 0, "maximum": 10000, "description": "Optional milliseconds to wait for screen change." },
                    "screenshot": { "type": "boolean", "description": "Return a screenshot after the gesture (default: true)." }
                },
                "required": ["server", "vmid", "action", "x", "y"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked = self.prepare_checked(&call.arguments);
        async move {
            let (_, _, _, access) = checked?;
            Ok(vec![ToolAccess::ProxmoxVm(access)])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let this = self.clone();
        async move {
            let (server, vm, args, access) = match this.prepare_checked(&execution.arguments) {
                Ok(checked) => checked,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail)
                        .with_failure(refusal.code.unwrap_or_else(|| "invalid_input".to_owned()));
                }
            };
            if let Some(result) = verify_authorized(&execution, &access) {
                return result;
            }

            let take_screenshot = args.screenshot.unwrap_or(true);
            let shot = match this
                .manager
                .agent_vnc_pointer(
                    execution.conversation_id,
                    &server,
                    &vm,
                    &args.action,
                    args.x,
                    args.y,
                    args.button.as_deref(),
                    args.scroll_delta,
                    args.delay_ms,
                    args.wait_for_change_ms,
                    take_screenshot,
                )
                .await
            {
                Ok(shot) => shot,
                Err(error) => return error_result(&error),
            };

            let mut result = match &shot {
                Some(s) => ToolResult::new(format!(
                    "Sent VNC pointer '{}' at ({}, {}) on VM {} ({}); captured {}x{} frame #{} (changed={}).",
                    args.action, args.x, args.y, vm.vmid, vm.name, s.width, s.height, s.seq, s.changed
                ))
                .with_images(vec![ToolImage::new(s.png_bytes.clone())]),
                None => ToolResult::new(format!(
                    "Sent VNC pointer '{}' at ({}, {}) on VM {} ({}).",
                    args.action, args.x, args.y, vm.vmid, vm.name
                )),
            };
            result = result.with_data(json!({
                "action": "vnc_pointer",
                "server": server.name,
                "node": vm.node,
                "vmid": vm.vmid,
                "vmName": vm.name,
                "pointerAction": args.action,
                "x": args.x,
                "y": args.y,
                "changed": shot.as_ref().map(|s| s.changed),
            }));
            result
        }
        .boxed()
    }
}

// ---------------------------------------------------------------------------
// 10. proxmox_vnc_keyboard (Vnc side effect)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct VncKeyboardTool {
    manager: ProxmoxManager,
}

struct PreparedKeyboard {
    server: ServerRecord,
    vm: AllowedVmRecord,
    strokes: Vec<KeyStroke>,
    delay_ms: Option<u64>,
    wait_for_change_ms: Option<u64>,
    take_screenshot: bool,
    access: ProxmoxVmAccess,
}

impl VncKeyboardTool {
    fn new(manager: ProxmoxManager) -> Self {
        Self { manager }
    }

    fn prepare_checked(&self, raw: &str) -> Result<PreparedKeyboard, ToolRefusal> {
        let args: VncKeyboardArgs = parse_args(raw)?;
        validate_vnc_waits(args.delay_ms, args.wait_for_change_ms)?;
        let (strokes, summary) = parse_keyboard_input(&args)?;
        let (server, vm) = resolve_ready_allowed_vm(&self.manager, &args.server, args.vmid)?;
        let access = build_access(&server, &vm, ProxmoxVmScope::Vnc, "vnc_keyboard", summary);
        let take_screenshot = args.screenshot.unwrap_or(true);
        Ok(PreparedKeyboard {
            server,
            vm,
            strokes,
            delay_ms: args.delay_ms,
            wait_for_change_ms: args.wait_for_change_ms,
            take_screenshot,
            access,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VncKeyboardArgs {
    server: String,
    vmid: u32,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    combo: Option<String>,
    #[serde(default)]
    keys: Option<Vec<String>>,
    #[serde(default)]
    delay_ms: Option<u64>,
    #[serde(default)]
    wait_for_change_ms: Option<u64>,
    #[serde(default)]
    screenshot: Option<bool>,
}

fn parse_keyboard_input(args: &VncKeyboardArgs) -> Result<(Vec<KeyStroke>, String), ToolRefusal> {
    if args.text.is_some() && args.combo.is_some() {
        return Err(ToolRefusal::new(
            "do not combine text and combo in one call; send combo (such as Ctrl+c or Shift+Tab) in its own call before or after text, and use text with key or keys only for trailing keys such as Enter or Tab",
        ));
    }

    let mut strokes = Vec::new();
    let mut summary_parts = Vec::new();

    if let Some(ref text) = args.text {
        let len = text.chars().count();
        if len == 0 || len > MAX_KEYBOARD_TEXT_CHARS {
            return Err(ToolRefusal::new(format!(
                "text length must be between 1 and {MAX_KEYBOARD_TEXT_CHARS} characters"
            )));
        }
        let parsed = text_to_keystrokes(text).map_err(|error| error_refusal(&error))?;
        strokes.extend(parsed);
        summary_parts.push(format!("type {len} character(s)"));
    }

    if let Some(ref key) = args.key {
        let parsed = parse_key_combo(key).map_err(|error| error_refusal(&error))?;
        strokes.push(parsed);
        summary_parts.push(format!("press {key}"));
    }

    if let Some(ref combo) = args.combo {
        let parsed = parse_key_combo(combo).map_err(|error| error_refusal(&error))?;
        strokes.push(parsed);
        summary_parts.push(format!("press {combo}"));
    }

    if let Some(ref keys) = args.keys {
        if keys.is_empty() || keys.len() > MAX_KEYBOARD_KEYS {
            return Err(ToolRefusal::new(format!(
                "keys must contain between 1 and {MAX_KEYBOARD_KEYS} entries"
            )));
        }
        for key in keys {
            let parsed = parse_key_combo(key).map_err(|error| error_refusal(&error))?;
            strokes.push(parsed);
        }
        summary_parts.push(format!("send {} key(s)", keys.len()));
    }

    if strokes.is_empty() {
        return Err(ToolRefusal::new(
            "provide at least one of text, key, combo, or keys",
        ));
    }

    let summary = format!("VNC keyboard: {}", summary_parts.join(", "));
    Ok((strokes, summary))
}

impl Tool for VncKeyboardTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            TOOL_VNC_KEYBOARD,
            "Type non-secret text or send key presses / chords (such as Enter, Escape, Tab, F2, Alt+F2, or Ctrl+Alt+Delete) to an allowed Proxmox QEMU VM's VNC console. Do not combine text with combo in one call (send Ctrl+c or Shift+Tab in its own call first); when combining text with key or keys, text is typed first followed by key/keys (for example text with key='Enter'). Use delay_ms (up to 60000) to pause unconditionally after sending input before capturing the screenshot when a page transition, boot, or spinner would otherwise return early. Never type passwords or passphrases; ask the owner to take over the live VNC panel instead.",
            json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string", "description": "Proxmox server name or ID." },
                    "vmid": { "type": "integer", "minimum": 1, "description": "Allowed QEMU VMID." },
                    "text": { "type": "string", "description": "Non-secret text to type." },
                    "key": { "type": "string", "description": "Single named key or character (e.g. Enter, Tab, Escape, Down, F2)." },
                    "combo": { "type": "string", "description": "Key combination (e.g. Ctrl+Alt+Delete, Alt+F2, Ctrl+c)." },
                    "keys": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Ordered sequence of keys or chords."
                    },
                    "delay_ms": { "type": "integer", "minimum": 0, "maximum": 60000, "description": "Optional unconditional pause in milliseconds after sending input and before capturing the screenshot (up to 60000)." },
                    "wait_for_change_ms": { "type": "integer", "minimum": 0, "maximum": 10000, "description": "Optional milliseconds to wait for screen change." },
                    "screenshot": { "type": "boolean", "description": "Return a screenshot after typing (default: true)." }
                },
                "required": ["server", "vmid"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked = self.prepare_checked(&call.arguments);
        async move {
            let prepared = checked?;
            Ok(vec![ToolAccess::ProxmoxVm(prepared.access)])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let this = self.clone();
        async move {
            let PreparedKeyboard {
                server,
                vm,
                strokes,
                delay_ms,
                wait_for_change_ms: wait_ms,
                take_screenshot,
                access,
            } = match this.prepare_checked(&execution.arguments) {
                Ok(checked) => checked,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail).with_failure(
                        refusal.code.unwrap_or_else(|| "invalid_input".to_owned()),
                    );
                }
            };
            if let Some(result) = verify_authorized(&execution, &access) {
                return result;
            }

            let stroke_count = strokes.len();
            let shot = match this
                .manager
                .agent_vnc_keyboard(
                    execution.conversation_id,
                    &server,
                    &vm,
                    &strokes,
                    delay_ms,
                    wait_ms,
                    take_screenshot,
                )
                .await
            {
                Ok(shot) => shot,
                Err(error) => return error_result(&error),
            };

            let mut result = match &shot {
                Some(s) => ToolResult::new(format!(
                    "Sent {stroke_count} keystroke(s) to VM {} ({}); captured {}x{} frame #{} (changed={}).",
                    vm.vmid, vm.name, s.width, s.height, s.seq, s.changed
                ))
                .with_images(vec![ToolImage::new(s.png_bytes.clone())]),
                None => ToolResult::new(format!(
                    "Sent {stroke_count} keystroke(s) to VM {} ({}).",
                    vm.vmid, vm.name
                )),
            };
            result = result.with_data(json!({
                "action": "vnc_keyboard",
                "server": server.name,
                "node": vm.node,
                "vmid": vm.vmid,
                "vmName": vm.name,
                "keystrokes": stroke_count,
                "changed": shot.as_ref().map(|s| s.changed),
            }));
            result
        }
        .boxed()
    }
}

// ---------------------------------------------------------------------------
// 11. proxmox_vnc_close (Vnc side effect)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct VncCloseTool {
    manager: ProxmoxManager,
}

impl VncCloseTool {
    fn new(manager: ProxmoxManager) -> Self {
        Self { manager }
    }

    fn prepare_checked(
        &self,
        raw: &str,
    ) -> Result<(ServerRecord, AllowedVmRecord, ProxmoxVmAccess), ToolRefusal> {
        let args: VncCloseArgs = parse_args(raw)?;
        let (server, vm) = resolve_ready_allowed_vm(&self.manager, &args.server, args.vmid)?;
        let access = build_access(
            &server,
            &vm,
            ProxmoxVmScope::Vnc,
            "vnc_close",
            "Close VNC console session".to_owned(),
        );
        Ok((server, vm, access))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VncCloseArgs {
    server: String,
    vmid: u32,
}

impl Tool for VncCloseTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            TOOL_VNC_CLOSE,
            "Close this conversation's open VNC console session for an allowed Proxmox QEMU VM.",
            json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string", "description": "Proxmox server name or ID." },
                    "vmid": { "type": "integer", "minimum": 1, "description": "Allowed QEMU VMID." }
                },
                "required": ["server", "vmid"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked = self.prepare_checked(&call.arguments);
        async move {
            let (_, _, access) = checked?;
            Ok(vec![ToolAccess::ProxmoxVm(access)])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let this = self.clone();
        async move {
            let (server, vm, access) = match this.prepare_checked(&execution.arguments) {
                Ok(checked) => checked,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail)
                        .with_failure(refusal.code.unwrap_or_else(|| "invalid_input".to_owned()));
                }
            };
            if let Some(result) = verify_authorized(&execution, &access) {
                return result;
            }

            if let Err(error) =
                this.manager
                    .agent_vnc_close(execution.conversation_id, &server.id, vm.vmid)
            {
                if matches!(error, ProxmoxError::SessionNotFound) {
                    return ToolResult::new(format!(
                        "No live VNC session was open for VM {} ({}).",
                        vm.vmid, vm.name
                    ));
                }
                return error_result(&error);
            }

            ToolResult::new(format!(
                "Closed VNC console session for VM {} ({}) on {}.",
                vm.vmid, vm.name, server.name
            ))
            .with_data(json!({
                "action": "vnc_close",
                "server": server.name,
                "node": vm.node,
                "vmid": vm.vmid,
                "vmName": vm.name,
            }))
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        VncKeyboardArgs, is_valid_boot_device, is_valid_cdrom_device, is_valid_iso_volid,
        parse_keyboard_input, validate_vnc_waits,
    };

    #[test]
    fn validates_cdrom_boot_devices_and_iso_volids() {
        assert!(is_valid_cdrom_device("ide2"));
        assert!(is_valid_cdrom_device("sata0"));
        assert!(is_valid_cdrom_device("scsi1"));
        assert!(!is_valid_cdrom_device("virtio0"));
        assert!(!is_valid_cdrom_device("ide9"));

        assert!(is_valid_boot_device("ide2"));
        assert!(is_valid_boot_device("scsi0"));
        assert!(is_valid_boot_device("virtio0"));
        assert!(is_valid_boot_device("net0"));
        assert!(!is_valid_boot_device("ide2;evil"));

        assert!(is_valid_iso_volid(
            "local:iso/debian-12.7.0-amd64-netinst.iso"
        ));
        assert!(!is_valid_iso_volid("local:iso/../secret.iso"));
        assert!(!is_valid_iso_volid("local:iso/debian.iso,media=cdrom"));
        assert!(!is_valid_iso_volid("local:vztmpl/alpine.tar.gz"));
    }

    #[test]
    fn validates_screenshot_and_action_waits() {
        validate_vnc_waits(Some(60_000), Some(10_000)).unwrap();
        let delay_err = validate_vnc_waits(Some(60_001), None).unwrap_err();
        assert!(delay_err.detail.contains("60000"));

        let wait_err = validate_vnc_waits(None, Some(15_000)).unwrap_err();
        assert!(
            wait_err.detail.contains("delay_ms"),
            "refusal should point callers to delay_ms: {}",
            wait_err.detail
        );
    }

    #[test]
    fn rejects_combining_text_with_combo_but_allows_trailing_keys() {
        let mixed = VncKeyboardArgs {
            server: "Lab".to_owned(),
            vmid: 100,
            text: Some("lab".to_owned()),
            key: None,
            combo: Some("Shift+Tab".to_owned()),
            keys: None,
            delay_ms: None,
            wait_for_change_ms: None,
            screenshot: None,
        };
        let err = parse_keyboard_input(&mixed).unwrap_err();
        assert!(err.detail.contains("do not combine text and combo"));

        let submit = VncKeyboardArgs {
            server: "Lab".to_owned(),
            vmid: 100,
            text: Some("ls -la".to_owned()),
            key: Some("Enter".to_owned()),
            combo: None,
            keys: None,
            delay_ms: Some(1500),
            wait_for_change_ms: None,
            screenshot: None,
        };
        let (strokes, _) = parse_keyboard_input(&submit).unwrap();
        assert_eq!(strokes.len(), 7);
    }
}
