//! The mention provider `ssh` (ADR-0032; plugins/ssh/docs/README.md,
//! mentions): "@" in the message box lists the registered hosts by name,
//! and a mention tells the model which host the owner means. It is offered
//! in every conversation, since every conversation can use every host
//! (D35).
//!
//! A mention reads nothing from the host: it names the host, its endpoint
//! and the owner's note, and the model runs commands there only through
//! `run_ssh_command` and the core's policy.

use std::collections::BTreeMap;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{MentionEntry, MentionSummary, clean_label};
use mannie_plugin_sdk::points::mentions::{
    MentionContent, MentionContext, MentionProvider, ProviderSuggestions, RenderedMention,
};

use crate::commands::HostStatus;
use crate::hosts::Hosts;
use crate::rules::MAX_NOTE_CHARS;
use crate::store::HostRecord;
use crate::texts;

/// The provider's ID, which is the plugin's.
pub(crate) const PROVIDER_ID: &str = "ssh";

/// The variant of a host's entries, which chooses its icon.
pub(crate) const HOST_VARIANT: &str = "host";

/// The summary code of a mentioned host, which the UI words.
pub(crate) const HOST_SUMMARY: &str = "host";

/// The warning of a host that cannot run commands yet, which the UI words.
pub(crate) const NOT_READY_WARNING: &str = "not_ready";

/// The code of a key that names no registered host.
pub(crate) const HOST_NOT_FOUND: &str = "host_not_found";

/// The most characters of an endpoint in what the model reads.
const MAX_ENDPOINT_CHARS: usize = 400;

/// The hosts, as "@" lists them.
#[derive(Debug)]
pub(crate) struct HostMentions {
    hosts: Hosts,
}

impl HostMentions {
    pub(crate) fn new(hosts: Hosts) -> Self {
        Self { hosts }
    }
}

impl MentionProvider for HostMentions {
    fn id(&self) -> &str {
        PROVIDER_ID
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
        let suggestions = suggestions(&self.hosts.records(), query, limit);
        Box::pin(std::future::ready(Ok(suggestions)))
    }

    fn resolve<'a>(
        &'a self,
        _context: &'a MentionContext,
        key: &'a str,
        _max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>> {
        let resolved = match self.hosts.by_id(key) {
            Some(host) => {
                let content: Box<dyn MentionContent> = Box::new(MentionedHost::of(&host));
                Ok(content)
            }
            None => Err(HOST_NOT_FOUND.to_owned()),
        };
        Box::pin(std::future::ready(resolved))
    }
}

/// The hosts whose names hold the query, those that start with it first,
/// then by name.
fn suggestions(hosts: &[HostRecord], query: &str, limit: usize) -> ProviderSuggestions {
    let query = query.to_ascii_lowercase();
    let mut matching: Vec<(bool, &HostRecord)> = hosts
        .iter()
        .filter_map(|host| {
            let name = host.name.to_ascii_lowercase();
            let starts = name.starts_with(&query);
            (starts || name.contains(&query)).then_some((starts, host))
        })
        .collect();
    matching.sort_by(|(left_starts, left), (right_starts, right)| {
        right_starts.cmp(left_starts).then_with(|| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        })
    });
    let incomplete = matching.len() > limit;
    let entries = matching
        .into_iter()
        .take(limit)
        .map(|(_starts, host)| MentionEntry {
            key: host.id.clone(),
            label: host.name.clone(),
            token: format!("@{}", host.name),
            variant: HOST_VARIANT.to_owned(),
            warning: (host.status() != HostStatus::Ready).then(|| NOT_READY_WARNING.to_owned()),
            expands: false,
        })
        .collect();
    ProviderSuggestions::new(entries, incomplete)
}

/// A host, as a sent mention keeps it.
#[derive(Debug)]
struct MentionedHost {
    name: String,
    text: String,
}

impl MentionedHost {
    fn of(host: &HostRecord) -> Self {
        let endpoint = clean_label(&host.endpoint(), MAX_ENDPOINT_CHARS).unwrap_or_default();
        let note = clean_label(&host.note, MAX_NOTE_CHARS as usize);
        Self {
            name: host.name.clone(),
            text: texts::mentioned(&host.name, &endpoint, note.as_deref()),
        }
    }
}

impl MentionContent for MentionedHost {
    fn render(&self, limit: usize) -> RenderedMention {
        RenderedMention::new(
            cut(&self.text, limit),
            self.name.clone(),
            HOST_VARIANT.to_owned(),
            MentionSummary {
                code: HOST_SUMMARY.to_owned(),
                values: BTreeMap::new(),
            },
        )
    }
}

/// At most `limit` bytes of `text`, cut at a character.
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
    use mannie_plugin_sdk::points::mentions::MentionContent;

    use super::{MentionedHost, NOT_READY_WARNING, cut, suggestions};
    use crate::store::{HostRecord, StoredKey};

    fn host(name: &str, ready: bool) -> HostRecord {
        let key = ready.then(|| StoredKey {
            algorithm: "ssh-ed25519".to_owned(),
            openssh: "ssh-ed25519 AAAA".to_owned(),
            fingerprint: "SHA256:abc".to_owned(),
            at: 1,
        });
        HostRecord {
            id: format!("id-{name}"),
            name: name.to_owned(),
            note: "The \u{202E}build machine.".to_owned(),
            address: "build-box.lan".to_owned(),
            port: 22,
            username: "deploy".to_owned(),
            password_set: true,
            key: key.clone(),
            seen: key,
        }
    }

    #[test]
    fn hosts_that_start_with_the_query_come_first_and_a_host_that_is_not_ready_is_marked() {
        let hosts = [
            host("web-build", true),
            host("build-box", true),
            host("db01", false),
        ];

        let found = suggestions(&hosts, "BUILD", 10);
        let all = suggestions(&hosts, "", 2);

        let labels: Vec<&str> = found
            .entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect();
        assert_eq!(labels, ["build-box", "web-build"]);
        assert_eq!(found.entries[0].token, "@build-box");
        assert_eq!(found.entries[0].key, "id-build-box");
        assert!(!found.incomplete);
        assert_eq!(all.entries.len(), 2);
        assert!(all.incomplete);
        let db = suggestions(&hosts, "db", 10);
        assert_eq!(db.entries[0].warning.as_deref(), Some(NOT_READY_WARNING));
    }

    #[test]
    fn a_mention_names_the_host_its_endpoint_and_its_cleaned_note_within_the_limit() {
        let mentioned = MentionedHost::of(&host("build-box", true));

        let rendered = mentioned.render(10_000);
        let short = mentioned.render(20);

        assert!(
            rendered
                .text
                .contains("build-box (deploy@build-box.lan:22)"),
            "{}",
            rendered.text
        );
        assert!(
            rendered.text.contains("The build machine."),
            "{}",
            rendered.text
        );
        assert!(!rendered.text.contains('\u{202E}'));
        assert_eq!(rendered.label, "build-box");
        assert_eq!(rendered.summary.code, "host");
        assert!(short.text.len() <= 20);
        assert_eq!(cut("héllo", 2), "h");
    }
}
