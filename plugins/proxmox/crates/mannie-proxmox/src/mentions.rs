//! Mention provider `proxmox` (ADR-0032, ADR-0080): `@` in the message box
//! lists registered Proxmox servers and their explicitly allowed QEMU VMs.
//! Unselected VMs are never listed or resolvable.

use std::collections::BTreeMap;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{MentionEntry, MentionSummary, clean_label};
use mannie_plugin_sdk::points::mentions::{
    MentionContent, MentionContext, MentionProvider, ProviderSuggestions, RenderedMention,
};

use crate::model::{
    AllowedVmRecord, MAX_NOTE_CHARS, PROXMOX, ServerRecord, ServerState, ServerStatus,
};
use crate::state::ProxmoxManager;

/// Mention variant for a Proxmox server.
pub(crate) const SERVER_VARIANT: &str = "server";

/// Mention variant for an explicitly allowed QEMU VM.
pub(crate) const VM_VARIANT: &str = "vm";

/// Summary codes formatted by the UI.
pub(crate) const SERVER_SUMMARY: &str = "server";
pub(crate) const VM_SUMMARY: &str = "vm";

/// Warning codes for incomplete server setup.
pub(crate) const NOT_READY_WARNING: &str = "not_ready";

/// Refusal codes when a mentioned target no longer exists or is disallowed.
pub(crate) const SERVER_NOT_FOUND: &str = "server_not_found";
pub(crate) const VM_NOT_ALLOWED: &str = "vm_not_allowed";

/// Key prefixes distinguishing server mentions from allowed VM mentions.
const SERVER_KEY_PREFIX: &str = "server:";
const VM_KEY_PREFIX: &str = "vm:";

/// Mention provider for registered Proxmox servers and allowed VMs.
#[derive(Debug)]
pub(crate) struct ProxmoxMentions {
    manager: ProxmoxManager,
}

impl ProxmoxMentions {
    pub(crate) fn new(manager: ProxmoxManager) -> Self {
        Self { manager }
    }
}

impl MentionProvider for ProxmoxMentions {
    fn id(&self) -> &str {
        PROXMOX
    }

    fn offered(&self, _context: &MentionContext) -> bool {
        true
    }

    fn suggest<'a>(
        &'a self,
        _context: &'a MentionContext,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Result<ProviderSuggestions, String>> {
        let states = self.manager.states();
        let out = build_suggestions(&states, query, limit);
        Box::pin(std::future::ready(Ok(out)))
    }

    fn resolve<'a>(
        &'a self,
        _context: &'a MentionContext,
        key: &'a str,
        _max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>> {
        let result = resolve_mention(&self.manager.states(), key);
        Box::pin(std::future::ready(result))
    }
}

fn build_suggestions(states: &[ServerState], query: &str, limit: usize) -> ProviderSuggestions {
    let needle = query.trim().to_ascii_lowercase();
    let mut candidates: Vec<(bool, String, MentionEntry)> = Vec::new();

    for state in states {
        let server_name = state.server.name.clone();
        let server_lower = server_name.to_ascii_lowercase();
        let server_starts = !needle.is_empty() && server_lower.starts_with(&needle);
        if needle.is_empty() || server_lower.contains(&needle) {
            let warning = (state.server.status(state.allowed_vms.len()) != ServerStatus::Ready)
                .then(|| NOT_READY_WARNING.to_owned());
            candidates.push((
                server_starts,
                server_lower.clone(),
                MentionEntry {
                    key: format!("{SERVER_KEY_PREFIX}{}", state.server.id),
                    label: server_name.clone(),
                    token: format!("@{server_name}"),
                    variant: SERVER_VARIANT.to_owned(),
                    warning,
                    expands: false,
                },
            ));
        }

        for vm in &state.allowed_vms {
            let label = format!("{} (VM {} on {})", vm.name, vm.vmid, state.server.name);
            let search = format!(
                "{} {} {} {}",
                vm.name.to_ascii_lowercase(),
                vm.vmid,
                vm.node.to_ascii_lowercase(),
                server_lower
            );
            let vm_starts = !needle.is_empty()
                && (vm.name.to_ascii_lowercase().starts_with(&needle)
                    || vm.vmid.to_string().starts_with(&needle));
            if needle.is_empty() || search.contains(&needle) {
                let warning = (!state.server.password_set).then(|| NOT_READY_WARNING.to_owned());
                candidates.push((
                    vm_starts,
                    format!("{}-{}", vm.name.to_ascii_lowercase(), vm.vmid),
                    MentionEntry {
                        key: format!("{VM_KEY_PREFIX}{}:{}", state.server.id, vm.vmid),
                        label,
                        token: format!("@{}:{}", state.server.name, vm.vmid),
                        variant: VM_VARIANT.to_owned(),
                        warning,
                        expands: false,
                    },
                ));
            }
        }
    }

    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    let incomplete = candidates.len() > limit;
    let entries = candidates
        .into_iter()
        .take(limit)
        .map(|(_, _, entry)| entry)
        .collect();
    ProviderSuggestions::new(entries, incomplete)
}

fn resolve_mention(states: &[ServerState], key: &str) -> Result<Box<dyn MentionContent>, String> {
    if let Some(server_id) = key.strip_prefix(SERVER_KEY_PREFIX) {
        let state = states
            .iter()
            .find(|s| s.server.id == server_id)
            .ok_or_else(|| SERVER_NOT_FOUND.to_owned())?;
        return Ok(Box::new(MentionedServer::of(
            &state.server,
            &state.allowed_vms,
        )));
    }

    if let Some(rest) = key.strip_prefix(VM_KEY_PREFIX)
        && let Some((server_id, vmid_str)) = rest.rsplit_once(':')
        && let Ok(vmid) = vmid_str.parse::<u32>()
    {
        let state = states
            .iter()
            .find(|s| s.server.id == server_id)
            .ok_or_else(|| SERVER_NOT_FOUND.to_owned())?;
        let vm = state
            .allowed_vms
            .iter()
            .find(|allowed| allowed.vmid == vmid)
            .ok_or_else(|| VM_NOT_ALLOWED.to_owned())?;
        return Ok(Box::new(MentionedVm::of(&state.server, vm)));
    }

    Err(SERVER_NOT_FOUND.to_owned())
}

#[derive(Debug)]
struct MentionedServer {
    label: String,
    text: String,
}

impl MentionedServer {
    fn of(server: &ServerRecord, allowed_vms: &[AllowedVmRecord]) -> Self {
        let name = clean_label(&server.name, 128).unwrap_or_else(|| server.name.clone());
        let endpoint = server.endpoint();
        let note = clean_label(&server.note, MAX_NOTE_CHARS);
        let vm_list = if allowed_vms.is_empty() {
            "none allowed yet".to_owned()
        } else {
            allowed_vms
                .iter()
                .map(|vm| format!("VM {} ({} on {})", vm.vmid, vm.name, vm.node))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let text = match note {
            Some(text) => {
                format!("Proxmox server {name} ({endpoint}; allowed VMs: {vm_list}). Note: {text}")
            }
            None => format!("Proxmox server {name} ({endpoint}; allowed VMs: {vm_list})."),
        };
        Self { label: name, text }
    }
}

impl MentionContent for MentionedServer {
    fn render(&self, limit: usize) -> RenderedMention {
        RenderedMention::new(
            cut(&self.text, limit),
            self.label.clone(),
            SERVER_VARIANT.to_owned(),
            MentionSummary {
                code: SERVER_SUMMARY.to_owned(),
                values: BTreeMap::new(),
            },
        )
    }
}

#[derive(Debug)]
struct MentionedVm {
    label: String,
    text: String,
    vmid: u32,
}

impl MentionedVm {
    fn of(server: &ServerRecord, vm: &AllowedVmRecord) -> Self {
        let server_name = clean_label(&server.name, 128).unwrap_or_else(|| server.name.clone());
        let vm_name = clean_label(&vm.name, 128).unwrap_or_else(|| vm.name.clone());
        let endpoint = server.endpoint();
        let text = format!(
            "Proxmox QEMU VM {} ({vm_name}) on node {} of server {server_name} ({endpoint}).",
            vm.vmid, vm.node
        );
        Self {
            label: format!("{vm_name} (VM {})", vm.vmid),
            text,
            vmid: vm.vmid,
        }
    }
}

impl MentionContent for MentionedVm {
    fn render(&self, limit: usize) -> RenderedMention {
        let mut values = BTreeMap::new();
        values.insert("vmid".to_owned(), self.vmid);
        RenderedMention::new(
            cut(&self.text, limit),
            self.label.clone(),
            VM_VARIANT.to_owned(),
            MentionSummary {
                code: VM_SUMMARY.to_owned(),
                values,
            },
        )
    }
}

fn cut(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }

    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::{VM_NOT_ALLOWED, build_suggestions, resolve_mention};
    use crate::model::{AllowedVmRecord, ServerRecord, ServerState};

    fn sample_states() -> Vec<ServerState> {
        vec![ServerState {
            server: ServerRecord {
                id: "s1".to_owned(),
                name: "Home Lab".to_owned(),
                host: "pve.lan".to_owned(),
                port: 8006,
                username: "ops@pve".to_owned(),
                password_set: true,
                pinned_cert_sha256: None,
                pinned_cert_subject: None,
                pinned_cert_at: None,
                note: String::new(),
            },
            allowed_vms: vec![AllowedVmRecord {
                server_id: "s1".to_owned(),
                vmid: 101,
                node: "pve1".to_owned(),
                name: "debian-test".to_owned(),
                added_at: 1,
            }],
        }]
    }

    #[test]
    fn mentions_suggest_and_resolve_allowed_vms_and_refuse_disallowed_ones() {
        let states = sample_states();
        let suggested = build_suggestions(&states, "debian", 10);
        assert_eq!(suggested.entries.len(), 1);
        assert_eq!(suggested.entries[0].key, "vm:s1:101");

        let ok = resolve_mention(&states, "vm:s1:101").unwrap();
        let rendered = ok.render(1000);
        assert!(rendered.text.contains("VM 101 (debian-test)"));

        let disallowed = resolve_mention(&states, "vm:s1:999");
        assert_eq!(disallowed.err().as_deref(), Some(VM_NOT_ALLOWED));
    }
}
