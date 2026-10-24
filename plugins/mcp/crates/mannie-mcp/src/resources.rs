//! The tool `mcp_read_resource` (plugins/mcp/docs/README.md, resources):
//! one read of a resource of a connected server, which the core's policy
//! decides on like a call of the server's tools, as the tool `read_resource`
//! of that server with the URI as its argument (ADR-0043, decision 4).
//!
//! The plugin publishes it while some connected server lists resources or
//! templates. Mannie never fetches a resource itself: the server reads it.

use std::time::Duration;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::points::tools::{
    ExternalToolAccess, Tool, ToolAccess, ToolCall, ToolExecution, ToolRefusal, ToolResult,
    ToolSpec,
};
use rmcp::model::{ReadResourceRequestParams, ReadResourceResponse};
use serde::Deserialize;
use serde_json::json;

use crate::naming::READ_RESOURCE;
use crate::results::{Room, render_resource};
use crate::servers::Servers;
use crate::store::ServerRecord;
use crate::texts;
use crate::tool::{endpoint, not_called, service_failure, unavailable};

/// The tool of a server that a read is, as the core's access names it.
pub(crate) const READ_TOOL: &str = "read_resource";

/// The most bytes of a URI.
pub(crate) const MAX_URI_BYTES: usize = 4096;

/// How long a read may take.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// The arguments of `mcp_read_resource`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    server: String,
    uri: String,
}

/// Whether a text can be a resource's URI: not empty, within its limit and
/// without control characters.
pub(crate) fn is_uri(uri: &str) -> bool {
    !uri.is_empty() && uri.len() <= MAX_URI_BYTES && !uri.chars().any(char::is_control)
}

/// The tool.
#[derive(Debug, Clone)]
pub(crate) struct ReadResource {
    servers: Servers,
}

impl ReadResource {
    pub(crate) fn new(servers: Servers) -> Self {
        Self { servers }
    }
}

/// What the model is told of the tool.
pub(crate) fn spec() -> ToolSpec {
    ToolSpec::new(
        READ_RESOURCE,
        texts::READ_RESOURCE_DESCRIPTION,
        json!({
            "type": "object",
            "properties": {
                "server": { "type": "string", "description": texts::READ_RESOURCE_SERVER },
                "uri": { "type": "string", "description": texts::READ_RESOURCE_URI }
            },
            "required": ["server", "uri"],
            "additionalProperties": false
        }),
    )
}

impl Tool for ReadResource {
    fn spec(&self) -> ToolSpec {
        spec()
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let prepared = check(&self.servers, &call.arguments)
            .map(|(record, uri)| vec![ToolAccess::ExternalTool(access(&record, &uri))]);
        Box::pin(std::future::ready(prepared))
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let servers = self.servers.clone();
        Box::pin(async move { run(&servers, &execution).await })
    }
}

/// Reads and checks a read's arguments against the stored servers.
fn check(servers: &Servers, arguments: &str) -> Result<(ServerRecord, String), ToolRefusal> {
    let arguments: Arguments = serde_json::from_str(arguments)
        .map_err(|error| ToolRefusal::new(texts::invalid_arguments(&error.to_string())))?;
    if !is_uri(&arguments.uri) {
        return Err(ToolRefusal::new(texts::INVALID_URI));
    }
    let record = servers
        .by_prefix(&arguments.server)
        .filter(|record| record.enabled)
        .ok_or_else(|| {
            let prefixes: Vec<String> = servers
                .connected()
                .into_iter()
                .map(|server| server.prefix)
                .collect();
            ToolRefusal::new(texts::unknown_server(&arguments.server, &prefixes))
        })?;
    Ok((record, arguments.uri))
}

/// The read as the core's policy decides on it.
fn access(record: &ServerRecord, uri: &str) -> ExternalToolAccess {
    let target = record.id.clone();
    let label = record.name.clone();
    ExternalToolAccess::new(
        target,
        label,
        endpoint(record),
        READ_TOOL.to_owned(),
        uri.to_owned(),
    )
}

/// Runs an authorized read.
async fn run(servers: &Servers, execution: &ToolExecution) -> ToolResult {
    let _call = servers.call_started();
    let (record, uri) = match check(servers, &execution.arguments) {
        Ok(checked) => checked,
        Err(refusal) => return not_called(&refusal.detail),
    };
    let expected = ToolAccess::ExternalTool(access(&record, &uri));
    if !execution.authorized.contains(&expected) {
        return not_called(texts::SERVER_CHANGED);
    }
    let connection = match servers.connection(&record.id).await {
        Ok(connection) => connection,
        Err(reason) => return not_called(unavailable(&reason)),
    };
    if endpoint(&connection.record) != endpoint(&record) {
        return not_called(texts::SERVER_CHANGED);
    }
    tracing::info!(server = %record.id, "an MCP resource is read");
    let read = tokio::time::timeout(
        READ_TIMEOUT,
        connection
            .peer
            .read_resource_once(ReadResourceRequestParams::new(uri)),
    )
    .await;
    let room = Room {
        result_limit: usize::try_from(execution.result_limit).unwrap_or(usize::MAX),
        archive_ids: &execution.archive_ids,
    };
    match read {
        Err(_elapsed) => {
            ToolResult::new(texts::timed_out(READ_TIMEOUT)).with_failure(texts::TIMED_OUT)
        }
        Ok(Err(error)) => service_failure(&error, &connection.redactions),
        Ok(Ok(ReadResourceResponse::Complete(read))) => {
            render_resource(read, record.images, &connection.redactions, room)
        }
        Ok(Ok(ReadResourceResponse::InputRequired(_input))) => {
            ToolResult::new(texts::INPUT_REQUIRED_TEXT).with_failure(texts::INPUT_REQUIRED)
        }
        Ok(Ok(_other)) => ToolResult::new(texts::UNKNOWN_ANSWER).with_failure(texts::SERVER_ERROR),
    }
}

#[cfg(test)]
mod tests {
    use super::{is_uri, spec};

    #[test]
    fn the_tool_is_strict_and_a_uri_is_a_short_line() {
        let spec = spec();

        assert!(spec.strict);
        assert_eq!(
            spec.parameters["required"],
            serde_json::json!(["server", "uri"])
        );
        assert!(is_uri("test://readme"));
        assert!(!is_uri(""));
        assert!(!is_uri("test://read\nme"));
        assert!(!is_uri(&"a".repeat(4097)));
    }
}
