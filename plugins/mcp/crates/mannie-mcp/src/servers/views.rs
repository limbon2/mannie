//! What the servers' records become for the settings and the store: the view
//! of a server, and the record of a tool that a listing gives, with what the
//! server says of it.

use mannie_plugin_sdk::contracts::ToolHint;
use mannie_plugin_sdk::ids::PluginId;
use rmcp::model::{Tool, ToolAnnotations};

use super::{Entry, cut};
use crate::commands::{NamedValue, ServerStatus, ServerTransport, ServerView, ServerWarning};
use crate::log::Redactions;
use crate::offered::offered_tools;
use crate::rules::looks_like_token;
use crate::store::{StoredEndpoint, ToolRecord};

/// The most bytes of a tool's description that the plugin keeps.
const MAX_DESCRIPTION_BYTES: usize = 8 * 1024;

/// The most bytes of a tool's schema, as JSON, that the plugin keeps. A
/// larger one is kept as `null`, and the tool cannot be offered.
const MAX_SCHEMA_BYTES: usize = 256 * 1024;

/// A tool as the store keeps it, from a listing, without the server's
/// secrets in its title, its description or its schema (ADR-0043, decision
/// 7): a careless server may repeat them there.
pub(super) fn tool_record(
    tool: &Tool,
    position: u32,
    enabled: bool,
    new: bool,
    redactions: &Redactions,
) -> ToolRecord {
    let mut schema = serde_json::Value::Object((*tool.input_schema).clone());
    redactions.apply_to_json(&mut schema);
    let schema_bytes = serde_json::to_string(&schema).map_or(usize::MAX, |text| text.len());
    let schema = if schema_bytes > MAX_SCHEMA_BYTES {
        serde_json::Value::Null
    } else {
        schema
    };
    let title = tool.title.clone().or_else(|| {
        tool.annotations
            .as_ref()
            .and_then(|hints| hints.title.clone())
    });
    let description = redactions.apply(tool.description.as_deref().unwrap_or(""));
    ToolRecord {
        name: tool.name.to_string(),
        position,
        title: title.map(|title| redactions.apply(&title)),
        description: cut(&description, MAX_DESCRIPTION_BYTES),
        schema,
        hints: hints(tool.annotations.as_ref()),
        enabled,
        new,
    }
}

/// What the server says of a tool, as hints: only what it states, without
/// the defaults of the specification, in order.
fn hints(annotations: Option<&ToolAnnotations>) -> Vec<ToolHint> {
    let Some(annotations) = annotations else {
        return Vec::new();
    };
    let mut hints = Vec::new();
    if annotations.read_only_hint == Some(true) {
        hints.push(ToolHint::ReadOnly);
    }
    if annotations.destructive_hint == Some(true) {
        hints.push(ToolHint::Destructive);
    }
    if annotations.idempotent_hint == Some(true) {
        hints.push(ToolHint::Idempotent);
    }
    if annotations.open_world_hint == Some(true) {
        hints.push(ToolHint::OpenWorld);
    }
    hints.sort();
    hints
}

/// A server as the settings show it.
pub(super) fn view(plugin: &PluginId, entry: &Entry) -> ServerView {
    let record = &entry.record;
    let (transport, program, arguments, folder, url) = match &record.endpoint {
        StoredEndpoint::Program {
            program,
            arguments,
            folder,
        } => (
            ServerTransport::Stdio,
            Some(program.clone()),
            arguments.clone(),
            folder.clone(),
            None,
        ),
        StoredEndpoint::Url { url } => (
            ServerTransport::Http,
            None,
            Vec::new(),
            None,
            Some(url.clone()),
        ),
    };
    let mut warnings: Vec<ServerWarning> = arguments
        .iter()
        .zip(0_u32..)
        .filter(|(argument, _index)| looks_like_token(argument))
        .map(|(_argument, index)| ServerWarning::TokenInArgument { index })
        .collect();
    let new_tools = entry.tools.iter().filter(|tool| tool.new).count();
    if new_tools > 0 {
        warnings.push(ServerWarning::NewTools {
            count: u32::try_from(new_tools).unwrap_or(u32::MAX),
        });
    }
    let unusable = offered_tools(plugin, record, &entry.tools)
        .into_iter()
        .filter(|(_tool, offered)| offered.is_err())
        .count();
    if unusable > 0 {
        warnings.push(ServerWarning::UnusableTools {
            count: u32::try_from(unusable).unwrap_or(u32::MAX),
        });
    }
    let status = if record.enabled {
        entry.live.status
    } else {
        ServerStatus::Off
    };
    let count = |count: usize| u32::try_from(count).unwrap_or(u32::MAX);
    ServerView {
        id: record.id.clone(),
        name: record.name.clone(),
        prefix: record.prefix.clone(),
        transport,
        program,
        arguments,
        folder,
        url,
        plain: record
            .plain
            .iter()
            .map(|(name, value)| NamedValue {
                name: name.clone(),
                value: value.clone(),
            })
            .collect(),
        secret_names: record.secret_names.clone(),
        missing_secrets: record.missing_secrets.clone(),
        oauth: record.oauth,
        client_id: record.client_id.clone(),
        client_secret_set: record.client_secret_set,
        signed_in: record.signed_in,
        enabled: record.enabled,
        images: record.images,
        instructions: record.instructions,
        timeout_seconds: record.timeout_seconds,
        status,
        failure: (status == ServerStatus::Failed)
            .then(|| entry.live.failure.clone())
            .flatten(),
        protocol_version: (status == ServerStatus::Connected)
            .then(|| entry.live.info.as_ref().map(|info| info.version.clone()))
            .flatten(),
        tool_count: count(entry.tools.len()),
        enabled_tool_count: count(entry.tools.iter().filter(|tool| tool.enabled).count()),
        resource_count: count(entry.live.resources.len()),
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_plugin_sdk::contracts::{Secret, ToolHint};
    use rmcp::model::{Tool, ToolAnnotations};
    use serde_json::json;

    use super::{hints, tool_record};
    use crate::log::Redactions;

    #[test]
    fn a_tool_is_kept_without_the_servers_secret() {
        let schema = json!({
            "type": "object",
            "properties": { "token": { "type": "string", "default": "planted-secret" } },
        });
        let serde_json::Value::Object(schema) = schema else {
            unreachable!("the schema is an object");
        };
        let mut tool = Tool::new(
            "echo",
            "Repeats text with planted-secret.",
            Arc::new(schema),
        );
        tool.title = Some("Echo planted-secret".to_owned());
        let redactions = Redactions::new([Secret::new("planted-secret".to_owned())]);

        let record = tool_record(&tool, 0, true, false, &redactions);

        assert_eq!(record.description, "Repeats text with [REDACTED].");
        assert_eq!(record.title.as_deref(), Some("Echo [REDACTED]"));
        assert_eq!(
            record.schema["properties"]["token"]["default"],
            json!("[REDACTED]")
        );
    }

    #[test]
    fn hints_are_what_the_server_states() {
        let annotations = ToolAnnotations::new().read_only(true).open_world(false);

        assert_eq!(hints(Some(&annotations)), [ToolHint::ReadOnly]);
        assert_eq!(hints(None), []);
    }
}
