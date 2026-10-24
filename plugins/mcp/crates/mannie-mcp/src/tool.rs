//! A server's tool as an agent's tool (plugins/mcp/docs/README.md, calling a
//! tool; ADR-0043, decision 4): one call of the tool on its server, as an
//! operation that the core's policy decides on and its journal records.
//!
//! Prepare reads only the plugin's stored servers and tools, never the
//! network, because the core prepares a call again after an approval and
//! compares the accesses: it parses the arguments into an object, where
//! empty or `null` reads as `{}`, names a renamed `activity` back, and
//! returns the call as an external-tool access with the arguments as JSON
//! with sorted keys. Execute acts only when the server still matches the
//! access that the core authorized, waits for the server's connection, and
//! calls the tool within the server's timeout.
//!
//! The core cancels a call by dropping its future (ADR-0034, decision 3),
//! which drops rmcp's request, and rmcp tells the server that the call is
//! cancelled. Whether the call took effect is then unknown.

use std::time::Duration;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::ToolServerEndpoint;
use mannie_plugin_sdk::points::tools::{
    ExternalToolAccess, Tool, ToolAccess, ToolCall, ToolExecution, ToolRefusal, ToolResult,
    ToolSpec,
};
use rmcp::model::{CallToolRequestParams, CallToolResponse};
use rmcp::service::ServiceError;
use serde_json::{Map, Value};

use crate::log::Redactions;
use crate::results::{Room, render};
use crate::schema::{model_schema, server_arguments};
use crate::servers::{Servers, Unavailable};
use crate::store::{ServerRecord, StoredEndpoint, ToolRecord};
use crate::texts;

/// The most characters of a server's error message that the model reads.
const MAX_ERROR_CHARS: usize = 2_000;

/// A tool of a server.
#[derive(Debug, Clone)]
pub(crate) struct ServerTool {
    servers: Servers,
    server_id: String,
    tool: String,
    spec: ToolSpec,
}

impl ServerTool {
    /// The tool `tool` of the server `server_id`, as the model is told of it.
    pub(crate) fn new(servers: Servers, server_id: String, tool: String, spec: ToolSpec) -> Self {
        Self {
            servers,
            server_id,
            tool,
            spec,
        }
    }
}

impl Tool for ServerTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let prepared = check(&self.servers, &self.server_id, &self.tool, &call.arguments)
            .map(|checked| vec![ToolAccess::ExternalTool(checked.access)]);
        Box::pin(std::future::ready(prepared))
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let servers = self.servers.clone();
        let server_id = self.server_id.clone();
        let tool = self.tool.clone();
        Box::pin(async move { run(&servers, &server_id, &tool, &execution).await })
    }
}

/// A call whose arguments passed the checks.
#[derive(Debug)]
struct Checked {
    record: ServerRecord,
    arguments: Map<String, Value>,
    access: ExternalToolAccess,
}

/// Reads and checks a call against the stored servers and tools.
fn check(
    servers: &Servers,
    server_id: &str,
    tool: &str,
    arguments: &str,
) -> Result<Checked, ToolRefusal> {
    let Some((record, tool)) = servers.tool_of(server_id, tool) else {
        return Err(ToolRefusal::new(texts::TOOL_GONE));
    };
    if !record.enabled {
        return Err(ToolRefusal::new(texts::server_off(&record.name)));
    }
    if !tool.enabled {
        return Err(ToolRefusal::new(texts::TOOL_OFF));
    }
    let parsed = parse_arguments(arguments)?;
    let renamed = model_schema(&tool.schema).is_ok_and(|model| model.renamed_activity);
    let arguments = server_arguments(parsed, renamed);
    let access = access(
        &record,
        &tool,
        canonical_json(&Value::Object(arguments.clone())),
    );
    Ok(Checked {
        record,
        arguments,
        access,
    })
}

/// The arguments of a call as an object: empty text and `null` read as an
/// empty object.
pub(crate) fn parse_arguments(arguments: &str) -> Result<Map<String, Value>, ToolRefusal> {
    if arguments.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(arguments) {
        Ok(Value::Object(object)) => Ok(object),
        Ok(Value::Null) => Ok(Map::new()),
        Ok(_other) => Err(ToolRefusal::new(texts::invalid_arguments(
            "they are not an object",
        ))),
        Err(error) => Err(ToolRefusal::new(texts::invalid_arguments(
            &error.to_string(),
        ))),
    }
}

/// A value as JSON with the keys of every object sorted, so the same
/// arguments always read the same, whichever order `serde_json` keeps.
pub(crate) fn canonical_json(value: &Value) -> String {
    let mut text = String::new();
    write_canonical(value, &mut text);
    text
}

fn write_canonical(value: &Value, text: &mut String) {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<(&String, &Value)> = object.iter().collect();
            entries.sort_by_key(|(key, _value)| *key);
            text.push('{');
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    text.push(',');
                }
                text.push_str(&Value::String(key.clone()).to_string());
                text.push(':');
                write_canonical(value, text);
            }
            text.push('}');
        }
        Value::Array(items) => {
            text.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    text.push(',');
                }
                write_canonical(item, text);
            }
            text.push(']');
        }
        other => text.push_str(&other.to_string()),
    }
}

/// Where a server runs, as the core's access names it.
pub(crate) fn endpoint(record: &ServerRecord) -> ToolServerEndpoint {
    match &record.endpoint {
        StoredEndpoint::Program {
            program, arguments, ..
        } => ToolServerEndpoint::Program {
            program: program.clone(),
            arguments: arguments.clone(),
        },
        StoredEndpoint::Url { url } => ToolServerEndpoint::Url { url: url.clone() },
    }
}

/// The call as the core's policy decides on it: the server's ID as the
/// target, its name as the label, its endpoint, the tool, the arguments
/// and what the server says of the tool.
fn access(record: &ServerRecord, tool: &ToolRecord, arguments: String) -> ExternalToolAccess {
    let target = record.id.clone();
    let label = record.name.clone();
    let tool_name = tool.name.clone();
    let access = ExternalToolAccess::new(target, label, endpoint(record), tool_name, arguments)
        .with_hints(tool.hints.clone());
    match &tool.title {
        Some(title) => access.with_title(title.clone()),
        None => access,
    }
}

/// A result that says the call did not reach the server.
pub(crate) fn not_called(reason: &str) -> ToolResult {
    ToolResult::new(texts::not_called(reason)).with_failure(texts::NOT_CALLED)
}

/// What a missing connection means for the model.
pub(crate) fn unavailable(unavailable: &Unavailable) -> &'static str {
    match unavailable {
        Unavailable::Removed => texts::SERVER_REMOVED,
        Unavailable::Off => texts::SERVER_IS_OFF,
        Unavailable::Failed(_failure) => texts::SERVER_FAILED,
        Unavailable::SignInNeeded => texts::SIGN_IN_NEEDED,
        Unavailable::AccessNeeded => texts::ACCESS_NEEDED,
        Unavailable::TimedOut => texts::NOT_CONNECTED,
    }
}

/// What a failed request means for the model. The server's message loses
/// the server's secrets before it is cut, so no start of one stays.
pub(crate) fn service_failure(error: &ServiceError, redactions: &Redactions) -> ToolResult {
    match error {
        ServiceError::McpError(error) => {
            let message: String = redactions
                .apply(&error.message)
                .chars()
                .take(MAX_ERROR_CHARS)
                .collect();
            ToolResult::new(texts::server_error(&message)).with_failure(texts::SERVER_ERROR)
        }
        ServiceError::Timeout { timeout } => {
            ToolResult::new(texts::timed_out(*timeout)).with_failure(texts::TIMED_OUT)
        }
        _other => ToolResult::new(texts::SERVER_STOPPED_TEXT).with_failure(texts::SERVER_STOPPED),
    }
}

/// Runs an authorized call.
async fn run(
    servers: &Servers,
    server_id: &str,
    tool: &str,
    execution: &ToolExecution,
) -> ToolResult {
    let _call = servers.call_started();
    let checked = match check(servers, server_id, tool, &execution.arguments) {
        Ok(checked) => checked,
        Err(refusal) => return not_called(&refusal.detail),
    };
    // The core authorized these exact accesses. A server whose endpoint or
    // tool changed since is not the one that the owner allowed.
    let expected = ToolAccess::ExternalTool(checked.access.clone());
    if !execution.authorized.contains(&expected) {
        return not_called(texts::SERVER_CHANGED);
    }
    let connection = match servers.connection(server_id).await {
        Ok(connection) => connection,
        Err(reason) => return not_called(unavailable(&reason)),
    };
    if endpoint(&connection.record) != checked.access.endpoint {
        return not_called(texts::SERVER_CHANGED);
    }
    tracing::info!(server = %server_id, tool, "an MCP tool is called");
    let params = CallToolRequestParams::new(tool.to_owned()).with_arguments(checked.arguments);
    let timeout = Duration::from_secs(u64::from(checked.record.timeout_seconds));
    let answered = tokio::time::timeout(timeout, connection.peer.call_tool_once(params)).await;
    let room = Room {
        result_limit: usize::try_from(execution.result_limit).unwrap_or(usize::MAX),
        archive_ids: &execution.archive_ids,
    };
    let result = match answered {
        Err(_elapsed) => ToolResult::new(texts::timed_out(timeout)).with_failure(texts::TIMED_OUT),
        Ok(Err(error)) => service_failure(&error, &connection.redactions),
        Ok(Ok(CallToolResponse::Complete(result))) => {
            render(result, checked.record.images, &connection.redactions, room)
        }
        Ok(Ok(CallToolResponse::InputRequired(_input))) => {
            ToolResult::new(texts::INPUT_REQUIRED_TEXT).with_failure(texts::INPUT_REQUIRED)
        }
        Ok(Ok(CallToolResponse::Task(_task))) => {
            ToolResult::new(texts::TASK_TEXT).with_failure(texts::TASK_STARTED)
        }
        Ok(Ok(_other)) => ToolResult::new(texts::UNKNOWN_ANSWER).with_failure(texts::SERVER_ERROR),
    };
    tracing::info!(server = %server_id, tool, failure = ?result.failure, "an MCP tool's call ended");
    result
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::Secret;
    use rmcp::model::ErrorData;
    use rmcp::service::ServiceError;
    use serde_json::json;

    use super::{MAX_ERROR_CHARS, canonical_json, parse_arguments, service_failure};
    use crate::log::Redactions;

    #[test]
    fn a_servers_error_loses_its_secret_before_it_is_cut() {
        let secret = "planted-secret-value";
        let redactions = Redactions::new([Secret::new(secret.to_owned())]);
        // The secret starts just before the cut, so cutting first would
        // leave its start.
        let message = format!("{}{secret}", "x".repeat(MAX_ERROR_CHARS - 5));
        let error = ServiceError::McpError(ErrorData::internal_error(message, None));

        let result = service_failure(&error, &redactions);

        assert!(!result.output.contains("plant"), "{}", result.output);
        assert!(result.output.contains("[RED"), "{}", result.output);
    }

    #[test]
    fn arguments_are_an_object_and_empty_or_null_reads_as_one() {
        assert!(parse_arguments("").unwrap().is_empty());
        assert!(parse_arguments("null").unwrap().is_empty());
        assert_eq!(parse_arguments("{\"a\":1}").unwrap()["a"], json!(1));
        parse_arguments("[1]").unwrap_err();
        parse_arguments("{").unwrap_err();
    }

    #[test]
    fn the_same_arguments_always_read_the_same() {
        let value = json!({ "b": [{ "z": 1, "a": "é\n" }], "a": null });

        assert_eq!(
            canonical_json(&value),
            r#"{"a":null,"b":[{"a":"é\n","z":1}]}"#
        );
    }
}
