//! The section `ssh-hosts` of the system prompt (plugins/ssh/docs/
//! README.md, what the model knows): the registered hosts, with their
//! endpoints and the owner's notes, for the runs that are offered
//! `run_ssh_command`. The section is rendered for each model call, so it
//! follows the hosts as they change.
//!
//! The owner wrote the names and notes, and the section introduces them as
//! data. Each text is cleaned as the core cleans labels: no control or
//! invisible characters, and white space collapsed.

use mannie_plugin_sdk::contracts::clean_label;
use mannie_plugin_sdk::points::sections::{SectionContext, SectionSource};

use crate::commands::HostStatus;
use crate::hosts::Hosts;
use crate::rules::MAX_NOTE_CHARS;
use crate::store::HostRecord;
use crate::texts;

/// The section's ID.
pub(crate) const SECTION_ID: &str = "ssh-hosts";

/// The section's place, after the core's environment (300) and the task
/// executors' sections.
pub(crate) const SECTION_POSITION: u32 = 320;

/// The agents whose runs are offered `run_ssh_command` and read the
/// section: the owner's conversations and the general child agent, whose
/// tools may change things (D35). The explore agent only reads.
pub(crate) const AGENTS: [&str; 2] = ["conversation", "general"];

/// The most characters of an endpoint in the section.
const MAX_ENDPOINT_CHARS: usize = 400;

/// The hosts, as the model reads them.
#[derive(Debug)]
pub(crate) struct HostsSection {
    hosts: Hosts,
}

impl HostsSection {
    pub(crate) fn new(hosts: Hosts) -> Self {
        Self { hosts }
    }
}

impl SectionSource for HostsSection {
    fn render(&self, context: &SectionContext) -> Option<String> {
        let is_offered = context.uses_tools && AGENTS.contains(&context.agent.as_str());
        if !is_offered {
            return None;
        }
        Some(render_hosts(&self.hosts.records()))
    }
}

/// The section for these hosts.
pub(crate) fn render_hosts(hosts: &[HostRecord]) -> String {
    if hosts.is_empty() {
        return texts::SECTION_EMPTY.to_owned();
    }
    let lines: Vec<String> = hosts.iter().map(host_line).collect();
    format!("{}\n{}", texts::SECTION_INTRODUCTION, lines.join("\n"))
}

/// One host: its name, its endpoint, whether commands can run on it, and
/// the owner's note.
fn host_line(host: &HostRecord) -> String {
    let endpoint = clean_label(&host.endpoint(), MAX_ENDPOINT_CHARS).unwrap_or_default();
    let status = match host.status() {
        HostStatus::Ready => "",
        HostStatus::KeyNotChecked => ", not usable yet: the owner has not trusted its host key",
        HostStatus::KeyChanged => ", not usable: it showed another host key than the trusted one",
        HostStatus::PasswordMissing => ", not usable: it has no password",
    };
    match clean_label(&host.note, MAX_NOTE_CHARS as usize) {
        Some(note) => format!("- {} ({endpoint}){status}: {note}", host.name),
        None => format!("- {} ({endpoint}){status}", host.name),
    }
}

#[cfg(test)]
mod tests {
    use super::render_hosts;
    use crate::store::{HostRecord, StoredKey};
    use crate::texts;

    fn host(name: &str, note: &str, key: bool) -> HostRecord {
        let key = key.then(|| StoredKey {
            algorithm: "ssh-ed25519".to_owned(),
            openssh: "ssh-ed25519 AAAA".to_owned(),
            fingerprint: "SHA256:abc".to_owned(),
            at: 1,
        });
        HostRecord {
            id: format!("id-{name}"),
            name: name.to_owned(),
            note: note.to_owned(),
            address: "10.0.0.5".to_owned(),
            port: 22,
            username: "deploy".to_owned(),
            password_set: true,
            key: key.clone(),
            seen: key,
        }
    }

    #[test]
    fn the_section_lists_each_host_with_its_endpoint_and_its_cleaned_note() {
        let text = render_hosts(&[
            host("build-box", "Builds the releases.\nUse /srv/build.", true),
            host("db01", "", false),
        ]);

        assert!(text.starts_with(texts::SECTION_INTRODUCTION));
        assert!(
            text.contains("- build-box (deploy@10.0.0.5:22): Builds the releases. Use /srv/build.")
        );
        assert!(text.contains(
            "- db01 (deploy@10.0.0.5:22), not usable yet: the owner has not trusted its host key"
        ));
    }

    #[test]
    fn the_section_says_when_no_host_is_registered() {
        assert_eq!(render_hosts(&[]), texts::SECTION_EMPTY);
    }
}
