//! The mention provider `mcp` (ADR-0032; plugins/mcp/docs/README.md,
//! resources): "@" in the message box lists the resources of the connected
//! servers from what their connections listed, without touching the
//! network, and a mention attaches the resource's text to the owner's
//! message.
//!
//! A key is `<server ID>:<URI>`. A key comes from the webview, so resolving
//! one reads only a URI that the server listed, with `resources/read` when
//! the owner sends the message. The owner chose the resource, so the read
//! asks nothing, as a mention of a file reads it.

use std::collections::BTreeMap;
use std::time::Duration;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{MentionEntry, MentionSummary};
use mannie_plugin_sdk::points::mentions::{
    MentionContent, MentionContext, MentionProvider, ProviderSuggestions, RenderedMention,
};
use rmcp::model::{ReadResourceRequestParams, ReadResourceResponse, Resource, ResourceContents};

use crate::servers::{ConnectedServer, Servers, cut};
use crate::texts;

/// The provider's ID, which is the plugin's.
pub(crate) const PROVIDER_ID: &str = "mcp";

/// The variant of a resource's entries, which chooses its icon.
pub(crate) const RESOURCE_VARIANT: &str = "resource";

/// The summary code of a mentioned resource, which the UI words.
pub(crate) const RESOURCE_SUMMARY: &str = "resource";

/// The code of a key that names no listed resource of a connected server.
pub(crate) const RESOURCE_NOT_FOUND: &str = "resource_not_found";

/// The code of a resource that the server did not give.
pub(crate) const READ_FAILED: &str = "read_failed";

/// How long a read of a mentioned resource may take.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The resources of the connected servers, as "@" lists them.
#[derive(Debug)]
pub(crate) struct ResourceMentions {
    servers: Servers,
}

impl ResourceMentions {
    pub(crate) fn new(servers: Servers) -> Self {
        Self { servers }
    }
}

impl MentionProvider for ResourceMentions {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn offered(&self, _context: &MentionContext) -> bool {
        self.servers
            .connected()
            .iter()
            .any(|server| !server.resources.is_empty())
    }

    fn suggest<'a>(
        &'a self,
        _context: &'a MentionContext,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Result<ProviderSuggestions, String>> {
        let suggestions = suggestions(&self.servers.connected(), query, limit);
        Box::pin(std::future::ready(Ok(suggestions)))
    }

    fn resolve<'a>(
        &'a self,
        _context: &'a MentionContext,
        key: &'a str,
        max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>> {
        Box::pin(async move {
            let (server_id, uri) = key.split_once(':').ok_or(RESOURCE_NOT_FOUND)?;
            let connected = self.servers.connected();
            let server = connected
                .iter()
                .find(|server| server.id == server_id)
                .ok_or(RESOURCE_NOT_FOUND)?;
            let resource = server
                .resources
                .iter()
                .find(|resource| resource.uri == uri)
                .ok_or(RESOURCE_NOT_FOUND)?;
            let text = read(&self.servers, server, resource).await?;
            let content: Box<dyn MentionContent> = Box::new(MentionedResource {
                label: label(resource),
                text: cut(&text, max_bytes),
            });
            Ok(content)
        })
    }
}

/// What the model reads of a mentioned resource.
async fn read(
    servers: &Servers,
    server: &ConnectedServer,
    resource: &Resource,
) -> Result<String, String> {
    let connection = servers
        .connection(&server.id)
        .await
        .map_err(|_unavailable| READ_FAILED.to_owned())?;
    let read = tokio::time::timeout(
        READ_TIMEOUT,
        connection
            .peer
            .read_resource_once(ReadResourceRequestParams::new(resource.uri.clone())),
    )
    .await;
    let Ok(Ok(ReadResourceResponse::Complete(read))) = read else {
        return Err(READ_FAILED.to_owned());
    };
    let mut texts_read = Vec::new();
    for contents in read.contents {
        match contents {
            ResourceContents::TextResourceContents { text, .. } => texts_read.push(text),
            ResourceContents::BlobResourceContents { mime_type, .. } => {
                let mime_type = mime_type.unwrap_or_else(|| "application/octet-stream".to_owned());
                return Ok(connection.redactions.apply(&texts::mentioned_binary(
                    &server.name,
                    &resource.uri,
                    &mime_type,
                )));
            }
            _other => {}
        }
    }
    let text = texts::mentioned(&server.name, &resource.uri, &texts_read.join("\n"));
    Ok(connection.redactions.apply(&text))
}

/// What the owner reads of a resource: its title, else its name.
fn label(resource: &Resource) -> String {
    resource
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| resource.name.clone())
}

/// The resources whose labels or URIs hold the query, those whose labels
/// start with it first, then by label.
fn suggestions(servers: &[ConnectedServer], query: &str, limit: usize) -> ProviderSuggestions {
    let query = query.to_lowercase();
    let mut matching: Vec<(bool, String, &ConnectedServer, &Resource)> = servers
        .iter()
        .flat_map(|server| {
            server
                .resources
                .iter()
                .map(move |resource| (server, resource))
        })
        .filter_map(|(server, resource)| {
            let label = label(resource);
            let lower = label.to_lowercase();
            let starts = lower.starts_with(&query);
            let holds =
                starts || lower.contains(&query) || resource.uri.to_lowercase().contains(&query);
            holds.then_some((starts, label, server, resource))
        })
        .collect();
    matching.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.to_lowercase().cmp(&right.1.to_lowercase()))
    });
    let incomplete = matching.len() > limit;
    let entries = matching
        .into_iter()
        .take(limit)
        .map(|(_starts, label, server, resource)| MentionEntry {
            key: format!("{}:{}", server.id, resource.uri),
            token: format!("@{label}"),
            label,
            variant: RESOURCE_VARIANT.to_owned(),
            warning: None,
            expands: false,
        })
        .collect();
    ProviderSuggestions::new(entries, incomplete)
}

/// A resource, as a sent mention keeps it.
#[derive(Debug)]
struct MentionedResource {
    label: String,
    text: String,
}

impl MentionContent for MentionedResource {
    fn render(&self, limit: usize) -> RenderedMention {
        RenderedMention::new(
            cut(&self.text, limit),
            self.label.clone(),
            RESOURCE_VARIANT.to_owned(),
            MentionSummary {
                code: RESOURCE_SUMMARY.to_owned(),
                values: BTreeMap::new(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use rmcp::model::Resource;

    use super::suggestions;
    use crate::servers::ConnectedServer;

    fn server(id: &str, resources: Vec<Resource>) -> ConnectedServer {
        ConnectedServer {
            id: id.to_owned(),
            name: format!("Server {id}"),
            prefix: id.to_owned(),
            instructions: None,
            resources,
            templates: Vec::new(),
        }
    }

    #[test]
    fn resources_that_start_with_the_query_come_first_under_their_server_and_uri() {
        let servers = [
            server(
                "a",
                vec![
                    Resource::new("test://notes", "release notes"),
                    Resource::new("test://readme", "readme").with_title("Read me"),
                ],
            ),
            server("b", vec![Resource::new("file:///read.txt", "notes")]),
        ];

        let found = suggestions(&servers, "READ", 10);
        let few = suggestions(&servers, "", 2);

        let labels: Vec<&str> = found
            .entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect();
        assert_eq!(labels, ["Read me", "notes"]);
        assert_eq!(found.entries[0].key, "a:test://readme");
        assert_eq!(found.entries[0].token, "@Read me");
        assert!(few.incomplete);
        assert_eq!(few.entries.len(), 2);
    }
}
