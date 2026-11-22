//! The four fixed static tools contributed by the Network Client plugin
//! (ADR-0075, decision 4):
//!
//! - `network_list_collections`: lists enabled collections (redacted).
//! - `network_list_requests`: lists saved requests in an enabled collection (redacted).
//! - `network_get_request`: returns one saved request's redacted definition.
//! - `network_send_request`: runs one saved request under `Access::NetworkRequest`
//!   and returns strictly redacted headers, cookies, body and archives.

use std::collections::BTreeMap;

use futures_util::FutureExt as _;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::points::tools::{
    MAX_TOOL_ARCHIVE_BYTES, NetworkRequestAccess, Tool, ToolAccess, ToolArchive, ToolCall,
    ToolExecution, ToolRefusal, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::{
    CollectionView, EnvironmentView, MAX_TIMEOUT_SECONDS, MIN_TIMEOUT_SECONDS, NetworkError,
    RequestView,
};
use crate::redact::{
    REDACTED, is_secret_header_name, is_secret_name, redact_auth, redact_body, redact_key_values,
    redact_variables,
};
use crate::state::NetworkManager;
use crate::template::{
    OriginPolicy, ResolveError, RuntimeOverrides, redact_url_template_for_agent,
    resolve_http_request,
};

/// Tool name for listing enabled collections.
pub const LIST_COLLECTIONS_TOOL: &str = "network_list_collections";

/// Tool name for listing saved requests in a collection.
pub const LIST_REQUESTS_TOOL: &str = "network_list_requests";

/// Tool name for inspecting a saved request's redacted schema.
pub const GET_REQUEST_TOOL: &str = "network_get_request";

/// Tool name for sending a saved request from an enabled collection.
pub const SEND_REQUEST_TOOL: &str = "network_send_request";

/// Agents allowed to send network requests: conversation agents and the
/// `general` child agent (never `explore` or read-only custom agents).
pub(crate) const SEND_AGENTS: [&str; 2] = ["conversation", "general"];

/// Lists enabled collections with redacted metadata.
#[derive(Debug, Clone)]
pub(crate) struct ListCollectionsTool {
    manager: NetworkManager,
}

impl ListCollectionsTool {
    pub(crate) fn new(manager: NetworkManager) -> Self {
        Self { manager }
    }
}

impl Tool for ListCollectionsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            LIST_COLLECTIONS_TOOL,
            "Lists the owner's enabled API collections in the Network Client plugin, with their protocol, active environment, redacted base URL and request counts. Prefer these collection tools over curl or shell HTTP commands when working with a covered API.",
            json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, _call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        async { Ok(Vec::new()) }.boxed()
    }

    fn execute(&self, _execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let manager = self.manager.clone();
        async move {
            let snap = manager.snapshot();
            let enabled: Vec<&CollectionView> =
                snap.collections.iter().filter(|col| col.enabled).collect();

            if enabled.is_empty() {
                return ToolResult::new(
                    "No enabled API collections are configured in the Network Client plugin.",
                )
                .with_data(json!({ "collections": 0 }));
            }

            let items: Vec<Value> = enabled
                .iter()
                .map(|col| {
                    let active_env = col.active_environment_id.as_deref().and_then(|id| {
                        col.environments
                            .iter()
                            .find(|e| e.id == id)
                            .map(|e| e.name.clone())
                    });
                    let env_names: Vec<&str> =
                        col.environments.iter().map(|e| e.name.as_str()).collect();
                    json!({
                        "id": col.id,
                        "name": col.name,
                        "description": col.description,
                        "protocol": col.protocol.as_str(),
                        "baseUrl": manager.redacted_base_url(col),
                        "activeEnvironment": active_env,
                        "environments": env_names,
                        "requestCount": col.requests.len(),
                    })
                })
                .collect();
            let output = serde_json::to_string_pretty(&json!({ "collections": items }))
                .unwrap_or_else(|_| "{}".to_owned());
            ToolResult::new(output).with_data(json!({ "collections": enabled.len() }))
        }
        .boxed()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListRequestsArgs {
    #[serde(default, alias = "collection_id")]
    collection_id: Option<String>,
    #[serde(default)]
    collection: Option<String>,
    #[serde(default)]
    query: Option<String>,
}

/// Lists saved requests in an enabled collection (or across all enabled
/// collections when none is named).
#[derive(Debug, Clone)]
pub(crate) struct ListRequestsTool {
    manager: NetworkManager,
}

impl ListRequestsTool {
    pub(crate) fn new(manager: NetworkManager) -> Self {
        Self { manager }
    }
}

impl Tool for ListRequestsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            LIST_REQUESTS_TOOL,
            "Lists saved API requests in the owner's enabled Network Client collections. Filter by collection_id or collection name and optional search query.",
            json!({
                "type": "object",
                "properties": {
                    "collection_id": {
                        "type": "string",
                        "description": "Optional collection ID or exact name."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional case-insensitive filter matching request name, path, method or description."
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        )
        .with_loose_schema()
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let parsed = parse_args::<ListRequestsArgs>(&call.arguments);
        async move {
            parsed?;
            Ok(Vec::new())
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let manager = self.manager.clone();
        async move {
            let args = match parse_args::<ListRequestsArgs>(&execution.arguments) {
                Ok(args) => args,
                Err(refusal) => return ToolResult::new(refusal.detail).with_failure("invalid_arguments"),
            };
            let snap = manager.snapshot();
            let selector = args
                .collection_id
                .as_deref()
                .or(args.collection.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let needle = args
                .query
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_ascii_lowercase);
            let collections: Vec<&CollectionView> = match selector {
                Some(sel) => {
                    let Some(found) = find_enabled_collection(&snap.collections, sel) else {
                        return ToolResult::new(format!(
                            "No enabled collection matches \"{sel}\". Call {LIST_COLLECTIONS_TOOL} to see enabled collections."
                        ))
                        .with_failure("collection_not_found");
                    };
                    vec![found]
                }
                None => snap.collections.iter().filter(|c| c.enabled).collect(),
            };

            let mut entries = Vec::new();

            for col in &collections {
                let folder_names: BTreeMap<&str, &str> = col
                    .folders
                    .iter()
                    .map(|f| (f.id.as_str(), f.name.as_str()))
                    .collect();

                for req in &col.requests {
                    let folder = req
                        .folder_id
                        .as_deref()
                        .and_then(|id| folder_names.get(id).copied());

                    let safe_url = redact_url_template_for_agent(&req.url_template);

                    if let Some(q) = &needle
                        && !request_matches_query(req, &safe_url, folder, q)
                    {
                        continue;
                    }

                    entries.push(json!({
                        "collectionId": col.id,
                        "collectionName": col.name,
                        "requestId": req.id,
                        "name": req.name,
                        "folder": folder,
                        "protocol": req.protocol.as_str(),
                        "method": req.method.as_str(),
                        "urlTemplate": safe_url,
                        "description": req.description,
                    }));
                }
            }

            let count = entries.len();
            let output = serde_json::to_string_pretty(&json!({ "requests": entries }))
                .unwrap_or_else(|_| "{}".to_owned());
            ToolResult::new(output).with_data(json!({ "requests": count }))
        }
        .boxed()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetRequestArgs {
    #[serde(default, alias = "collection_id")]
    collection_id: Option<String>,
    #[serde(default)]
    collection: Option<String>,
    #[serde(default, alias = "request_id")]
    request_id: Option<String>,
    #[serde(default)]
    request: Option<String>,
}

/// Returns the strictly redacted definition of one saved request so an agent
/// knows which path, query, header, variable or body inputs it takes.
#[derive(Debug, Clone)]
pub(crate) struct GetRequestTool {
    manager: NetworkManager,
}

impl GetRequestTool {
    pub(crate) fn new(manager: NetworkManager) -> Self {
        Self { manager }
    }
}

impl Tool for GetRequestTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            GET_REQUEST_TOOL,
            "Returns the full redacted definition of one saved request (method, URL template, path params, query params, headers, auth kind, body template and available variables). All secret values and cookies appear as [redacted].",
            json!({
                "type": "object",
                "properties": {
                    "collection_id": {
                        "type": "string",
                        "description": "Collection ID or name (optional when request_id is a unique ID)."
                    },
                    "request_id": {
                        "type": "string",
                        "description": "Saved request ID or name."
                    }
                },
                "required": ["request_id"],
                "additionalProperties": false
            }),
        )
        .with_loose_schema()
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let parsed = parse_args::<GetRequestArgs>(&call.arguments).and_then(|args| {
            let has_req = args
                .request_id
                .as_deref()
                .or(args.request.as_deref())
                .is_some_and(|s| !s.trim().is_empty());

            if !has_req {
                return Err(ToolRefusal::new("request_id is required"));
            }

            Ok(())
        });
        async move {
            parsed?;
            Ok(Vec::new())
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let manager = self.manager.clone();
        async move {
            let args = match parse_args::<GetRequestArgs>(&execution.arguments) {
                Ok(args) => args,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail).with_failure("invalid_arguments");
                }
            };
            let snap = manager.snapshot();
            let col_sel = args
                .collection_id
                .as_deref()
                .or(args.collection.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let req_sel = args
                .request_id
                .as_deref()
                .or(args.request.as_deref())
                .map_or("", str::trim);
            let (col, req) = match find_enabled_request(&snap.collections, col_sel, req_sel) {
                Ok(found) => found,
                Err(refusal) => {
                    return ToolResult::new(refusal.detail).with_failure(
                        refusal
                            .code
                            .unwrap_or_else(|| "request_not_found".to_owned()),
                    );
                }
            };

            let active_env = col
                .active_environment_id
                .as_deref()
                .and_then(|id| col.environments.iter().find(|e| e.id == id));
            let detail = json!({
                "collection": {
                    "id": col.id,
                    "name": col.name,
                    "protocol": col.protocol.as_str(),
                    "baseUrl": manager.redacted_base_url(col),
                    "activeEnvironment": active_env.map(|e| &e.name),
                    "environments": col.environments.iter().map(|e| json!({
                        "id": e.id,
                        "name": e.name,
                        "variables": redact_variables(&e.variables),
                    })).collect::<Vec<_>>(),
                    "variables": redact_variables(&col.variables),
                    "defaultHeaders": redact_key_values(&col.headers, true),
                    "defaultQueryParams": redact_key_values(&col.query_params, false),
                    "auth": redact_auth(&col.auth),
                },
                "request": {
                    "id": req.id,
                    "name": req.name,
                    "description": req.description,
                    "protocol": req.protocol.as_str(),
                    "method": req.method.as_str(),
                    "urlTemplate": redact_url_template_for_agent(&req.url_template),
                    "pathParams": redact_key_values(&req.path_params, false),
                    "queryParams": redact_key_values(&req.query_params, false),
                    "headers": redact_key_values(&req.headers, true),
                    "auth": redact_auth(&req.auth),
                    "body": redact_body(&req.body),
                    "timeoutSeconds": req.timeout_seconds,
                    "useCookies": req.use_cookies,
                },
                "globalVariables": redact_variables(&snap.global_variables),
                "cookieJarCount": snap.cookies.iter().filter(|c| c.enabled).count(),
            });
            let output = serde_json::to_string_pretty(&detail).unwrap_or_else(|_| "{}".to_owned());
            ToolResult::new(output).with_data(json!({
                "collectionName": col.name,
                "requestName": req.name,
                "method": req.method.as_str(),
            }))
        }
        .boxed()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendRequestArgs {
    #[serde(default, alias = "collection_id")]
    collection_id: Option<String>,
    #[serde(default)]
    collection: Option<String>,
    #[serde(default, alias = "request_id")]
    request_id: Option<String>,
    #[serde(default)]
    request: Option<String>,
    #[serde(default, alias = "environment_id")]
    environment_id: Option<String>,
    #[serde(default)]
    environment: Option<String>,
    #[serde(default, alias = "path_params")]
    path_params: BTreeMap<String, String>,
    #[serde(default, alias = "query_params")]
    query_params: BTreeMap<String, String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    variables: BTreeMap<String, String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default, alias = "timeout_seconds")]
    timeout_seconds: Option<u32>,
}

/// Sends one saved request from an enabled collection under
/// `Access::NetworkRequest`, returning strictly redacted results (ADR-0075).
#[derive(Debug, Clone)]
pub(crate) struct SendRequestTool {
    manager: NetworkManager,
}

impl SendRequestTool {
    pub(crate) fn new(manager: NetworkManager) -> Self {
        Self { manager }
    }
}

struct PreparedSend {
    collection: CollectionView,
    request: RequestView,
    environment: Option<EnvironmentView>,
    access: NetworkRequestAccess,
    resolved: crate::template::ResolvedHttpRequest,
}

fn request_matches_query(
    req: &RequestView,
    safe_url: &str,
    folder: Option<&str>,
    needle: &str,
) -> bool {
    req.name.to_ascii_lowercase().contains(needle)
        || safe_url.to_ascii_lowercase().contains(needle)
        || req.method.as_str().to_ascii_lowercase().contains(needle)
        || req.description.to_ascii_lowercase().contains(needle)
        || folder.is_some_and(|f| f.to_ascii_lowercase().contains(needle))
}

fn validate_agent_overrides(args: &SendRequestArgs) -> Result<(), ToolRefusal> {
    // Refuse secret overrides passed via agent arguments: secrets and cookies
    // belong in the owner's collection/environment/cookie jar.
    for header_name in args.headers.keys() {
        if is_secret_header_name(header_name) {
            return Err(ToolRefusal::new(format!(
                "Header \"{header_name}\" is a secret or cookie header and cannot be passed in tool arguments; configure credentials in the Network Client collection or Cookie Jar."
            ))
            .with_code("secret_in_arguments"));
        }
    }

    for var_name in args.variables.keys() {
        if is_secret_name(var_name) {
            return Err(ToolRefusal::new(format!(
                "Variable \"{var_name}\" looks like a secret and cannot be passed in tool arguments; configure it in the Network Client collection or environment."
            ))
            .with_code("secret_in_arguments"));
        }
    }

    for query_name in args.query_params.keys() {
        if is_secret_name(query_name) {
            return Err(ToolRefusal::new(format!(
                "Query parameter \"{query_name}\" looks like a secret and cannot be passed in tool arguments; configure it in the Network Client collection or environment."
            ))
            .with_code("secret_in_arguments"));
        }
    }

    if let Some(timeout) = args.timeout_seconds
        && !(MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&timeout)
    {
        return Err(ToolRefusal::new(format!(
            "timeout_seconds must be between {MIN_TIMEOUT_SECONDS} and {MAX_TIMEOUT_SECONDS}"
        )));
    }

    Ok(())
}

fn select_environment(
    col: &CollectionView,
    args: &SendRequestArgs,
) -> Result<Option<EnvironmentView>, ToolRefusal> {
    let env_sel = args
        .environment_id
        .as_deref()
        .or(args.environment.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    match env_sel {
        Some(sel) => col
            .environments
            .iter()
            .find(|e| e.id == sel || e.name.eq_ignore_ascii_case(sel))
            .cloned()
            .map(Some)
            .ok_or_else(|| {
                ToolRefusal::new(format!(
                    "Environment \"{sel}\" was not found in collection \"{}\".",
                    col.name
                ))
                .with_code("environment_not_found")
            }),
        None => Ok(col
            .active_environment_id
            .as_deref()
            .and_then(|id| col.environments.iter().find(|e| e.id == id).cloned())),
    }
}

impl SendRequestTool {
    fn prepare_send(&self, raw_arguments: &str) -> Result<PreparedSend, ToolRefusal> {
        let args = parse_args::<SendRequestArgs>(raw_arguments)?;
        let col_sel = args
            .collection_id
            .as_deref()
            .or(args.collection.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let req_sel = args
            .request_id
            .as_deref()
            .or(args.request.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolRefusal::new("request_id is required"))?;

        validate_agent_overrides(&args)?;

        let snap = self.manager.snapshot();
        let (col, req) = find_enabled_request(&snap.collections, col_sel, req_sel)?;

        let environment = select_environment(col, &args)?;
        let overrides = RuntimeOverrides {
            path_params: args.path_params,
            query_params: args.query_params,
            headers: args.headers,
            variables: args.variables,
            body: args.body,
            timeout_seconds: args.timeout_seconds,
        };
        let resolved = resolve_http_request(
            col,
            req,
            environment.as_ref(),
            &snap.global_variables,
            &overrides,
            OriginPolicy::AgentBoundToOwnerOrigin,
        )
        .map_err(map_resolve_refusal)?;

        let redactor = self
            .manager
            .redactor_for(col, req, environment.as_ref(), &resolved);
        let safe_url = redactor.scrub(&resolved.redacted_url);
        let safe_summary = redactor.scrub(&resolved.summary_json);
        let access = NetworkRequestAccess::new(
            col.id.clone(),
            col.name.clone(),
            req.id.clone(),
            req.name.clone(),
            req.protocol.as_str().to_owned(),
            req.method.as_str().to_owned(),
            resolved.origin.clone(),
            safe_url,
            safe_summary,
        );

        Ok(PreparedSend {
            collection: col.clone(),
            request: req.clone(),
            environment,
            access,
            resolved,
        })
    }
}

fn map_resolve_refusal(error: ResolveError) -> ToolRefusal {
    match error {
        ResolveError::InvalidUrl => ToolRefusal::new(
            "The saved request did not resolve to a valid http or https URL without userinfo. Check the collection's base URL, environment variables and path parameters.",
        )
        .with_code("invalid_url"),
        ResolveError::SecretInOrigin => ToolRefusal::new(
            "A secret variable cannot be used in the host or scheme of a request URL.",
        )
        .with_code("invalid_url"),
        ResolveError::InvalidPathParam => {
            ToolRefusal::new("Path parameters cannot contain control characters.")
                .with_code("invalid_url")
        }
        ResolveError::OriginOverrideRefused => ToolRefusal::new(
            "Tool argument overrides cannot change the target host, scheme or port configured by the owner. Select a configured environment or ask the owner to update the collection base URL.",
        )
        .with_code("origin_override_refused"),
    }
}

impl Tool for SendRequestTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            SEND_REQUEST_TOOL,
            "Sends a saved HTTP request defined in one of the owner's enabled Network Client collections. Prefer this over curl or shell HTTP tools for covered APIs. Secrets and cookies are attached automatically from the owner's store and shown as [redacted].",
            json!({
                "type": "object",
                "properties": {
                    "collection_id": {
                        "type": "string",
                        "description": "Collection ID or name (optional when request_id is a unique ID)."
                    },
                    "request_id": {
                        "type": "string",
                        "description": "Saved request ID or name from the collection."
                    },
                    "environment": {
                        "type": "string",
                        "description": "Optional environment ID or name to override the collection's active environment."
                    },
                    "path_params": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Optional {param} path parameter values."
                    },
                    "query_params": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Optional non-secret query parameter values."
                    },
                    "headers": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Optional non-secret request headers (never Authorization or Cookie)."
                    },
                    "variables": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Optional non-secret {{variable}} overrides."
                    },
                    "body": {
                        "type": "string",
                        "description": "Optional raw request body override."
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 300,
                        "description": "Optional timeout in seconds (1 to 300)."
                    }
                },
                "required": ["request_id"],
                "additionalProperties": false
            }),
        )
        .with_loose_schema()
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let prepared = self.prepare_send(&call.arguments);
        async move {
            let send = prepared?;
            Ok(vec![ToolAccess::NetworkRequest(send.access)])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let this = self.clone();
        async move {
            let prepared = match this.prepare_send(&execution.arguments) {
                Ok(send) => send,
                Err(refusal) => {
                    return ToolResult::new(format!(
                        "Not sent: {} The server received nothing.",
                        refusal.detail
                    ))
                    .with_failure(refusal.code.unwrap_or_else(|| "not_sent".to_owned()));
                }
            };

            // Verify TOCTOU parity: the live collection/request must still
            // produce the exact NetworkRequestAccess the core authorized.
            let expected = ToolAccess::NetworkRequest(prepared.access.clone());
            let authorized = execution.authorized.as_slice() == [expected];

            if !authorized {
                return ToolResult::new(
                    "Not sent: the saved request or its target origin changed after authorization. The server received nothing.",
                )
                .with_failure("request_changed");
            }

            let exchange_result = this
                .manager
                .execute_agent_http(
                    &prepared.collection,
                    &prepared.request,
                    prepared.environment.as_ref(),
                    &prepared.resolved,
                )
                .await;

            match exchange_result {
                Ok((exchange, redactor)) => format_agent_response(
                    &prepared,
                    exchange,
                    &redactor,
                    &execution,
                ),
                Err((NetworkError::InsecureCredentialTransport, _redactor)) => ToolResult::new(
                    "Not sent: this request carries credentials or cookies and requires https (or plain http only to localhost or a private network address). The server received nothing.",
                )
                .with_failure("insecure_transport"),
                Err((NetworkError::RequestTimedOut, _redactor)) => ToolResult::new(format!(
                    "The request timed out after {} seconds; whether the remote server processed it is unknown.",
                    prepared.resolved.timeout_seconds
                ))
                .with_failure("timed_out")
                .with_lost_answer(),
                Err((NetworkError::ResponseLost { detail }, redactor)) => {
                    let message = redactor.scrub(&detail);
                    ToolResult::new(format!(
                        "The connection closed while receiving the response ({message}); whether the remote server processed the request is unknown."
                    ))
                    .with_failure("response_lost")
                    .with_lost_answer()
                }
                Err((error, redactor)) => {
                    let message = redactor.scrub(&error.to_string());
                    ToolResult::new(format!("Request failed: {message}"))
                        .with_failure("request_failed")
                }
            }
        }
        .boxed()
    }
}

fn format_agent_response(
    prepared: &PreparedSend,
    exchange: crate::http::HttpExchange,
    redactor: &crate::redact::Redactor,
    execution: &ToolExecution,
) -> ToolResult {
    let mut header_lines = Vec::with_capacity(exchange.headers.len());

    for (name, value) in &exchange.headers {
        let shown = if is_secret_header_name(name) {
            REDACTED.to_owned()
        } else {
            redactor.scrub(value)
        };
        header_lines.push(format!("{name}: {shown}"));
    }

    let body_text = match String::from_utf8(exchange.body_bytes) {
        Ok(text) => redactor.scrub(&text),
        Err(err) => {
            let content_type = exchange
                .content_type
                .as_deref()
                .unwrap_or("application/octet-stream");
            format!(
                "[Binary response body ({content_type}, {} bytes) omitted.]",
                err.into_bytes().len()
            )
        }
    };

    let full_text = format!(
        "HTTP {} {}\nURL: {}\nDuration: {} ms\nBytes: {}{}\n\nHeaders:\n{}\n\nBody:\n{}",
        exchange.status,
        exchange.status_text,
        prepared.access.target_url,
        exchange.duration_ms,
        exchange.total_bytes,
        if exchange.truncated {
            " (truncated at 2 MiB limit)"
        } else {
            ""
        },
        if header_lines.is_empty() {
            "(none)".to_owned()
        } else {
            header_lines.join("\n")
        },
        body_text,
    );

    let limit = usize::try_from(execution.result_limit).unwrap_or(usize::MAX);
    let mut archives = Vec::new();
    let output = if full_text.len() > limit {
        let note = if let Some(archive_id) = execution.archive_ids.first() {
            let archived_content = cap_archive_text(&full_text, MAX_TOOL_ARCHIVE_BYTES);
            archives.push(ToolArchive::new(
                archive_id.clone(),
                "response",
                archived_content,
            ));
            format!(
                "\n[Response truncated to fit result limit; redacted response archived as {archive_id} for read_tool_output.]"
            )
        } else {
            "\n[Response truncated to fit result limit.]".to_owned()
        };

        let end = full_text.floor_char_boundary(limit.saturating_sub(note.len()));
        format!("{}{note}", &full_text[..end])
    } else {
        full_text
    };

    let card_data = json!({
        "collectionName": prepared.collection.name,
        "requestName": prepared.request.name,
        "protocol": prepared.request.protocol.as_str(),
        "method": prepared.request.method.as_str(),
        "url": prepared.access.target_url,
        "status": exchange.status,
        "statusText": exchange.status_text,
        "durationMs": exchange.duration_ms,
        "bytes": exchange.total_bytes,
        "truncated": exchange.truncated,
    });
    let mut result = ToolResult::new(output).with_data(card_data);

    if !archives.is_empty() {
        result = result.with_archives(archives);
    }

    result
}

fn cap_archive_text(text: &str, max_bytes: usize) -> String {
    const SUFFIX: &str = "\n[Archived response truncated at archive size limit.]";

    if text.len() <= max_bytes {
        return text.to_owned();
    }

    let budget = max_bytes.saturating_sub(SUFFIX.len());
    let end = text.floor_char_boundary(budget);
    format!("{}{SUFFIX}", &text[..end])
}

fn parse_args<T: serde::de::DeserializeOwned + Default>(raw: &str) -> Result<T, ToolRefusal> {
    let trimmed = raw.trim();

    if trimmed.is_empty() || trimmed == "null" {
        return Ok(T::default());
    }

    serde_json::from_str::<T>(trimmed).map_err(|error| ToolRefusal::new(error.to_string()))
}

fn find_enabled_collection<'a>(
    collections: &'a [CollectionView],
    selector: &str,
) -> Option<&'a CollectionView> {
    collections
        .iter()
        .find(|col| col.enabled && (col.id == selector || col.name.eq_ignore_ascii_case(selector)))
}

fn find_enabled_request<'a>(
    collections: &'a [CollectionView],
    col_selector: Option<&str>,
    req_selector: &str,
) -> Result<(&'a CollectionView, &'a RequestView), ToolRefusal> {
    if let Some(col_sel) = col_selector {
        let Some(col) = find_enabled_collection(collections, col_sel) else {
            return Err(ToolRefusal::new(format!(
                "No enabled collection matches \"{col_sel}\". Call {LIST_COLLECTIONS_TOOL} to inspect available collections."
            ))
            .with_code("collection_not_found"));
        };

        if let Some(req) = col.requests.iter().find(|r| r.id == req_selector) {
            return Ok((col, req));
        }

        let matches: Vec<&RequestView> = col
            .requests
            .iter()
            .filter(|r| r.name.eq_ignore_ascii_case(req_selector))
            .collect();
        return match matches.as_slice() {
            [single] => Ok((col, *single)),
            [] => Err(ToolRefusal::new(format!(
                "No saved request matches \"{req_selector}\" in collection \"{}\". Call {LIST_REQUESTS_TOOL} to inspect available endpoints.",
                col.name
            ))
            .with_code("request_not_found")),
            many => {
                let choices: Vec<String> = many
                    .iter()
                    .map(|r| format!("{} ({} {}, request_id: {})", r.name, r.method.as_str(), redact_url_template_for_agent(&r.url_template), r.id))
                    .collect();
                Err(ToolRefusal::new(format!(
                    "Multiple saved requests in collection \"{}\" match \"{req_selector}\": {}. Pass the exact request_id.",
                    col.name,
                    choices.join("; ")
                ))
                .with_code("ambiguous_request"))
            }
        };
    }

    // Match by exact request ID first across all enabled collections.
    for col in collections.iter().filter(|c| c.enabled) {
        if let Some(req) = col.requests.iter().find(|r| r.id == req_selector) {
            return Ok((col, req));
        }
    }

    // Otherwise require a unique case-insensitive name match across enabled
    // collections rather than picking an arbitrary match.
    let matches: Vec<(&CollectionView, &RequestView)> = collections
        .iter()
        .filter(|c| c.enabled)
        .flat_map(|col| {
            col.requests
                .iter()
                .filter(|r| r.name.eq_ignore_ascii_case(req_selector))
                .map(move |req| (col, req))
        })
        .collect();

    match matches.as_slice() {
        [(col, req)] => Ok((*col, *req)),
        [] => Err(ToolRefusal::new(format!(
            "No saved request matches \"{req_selector}\" in an enabled collection. Call {LIST_COLLECTIONS_TOOL} or {LIST_REQUESTS_TOOL} to inspect available endpoints."
        ))
        .with_code("request_not_found")),
        many => {
            let choices: Vec<String> = many
                .iter()
                .map(|(col, r)| {
                    format!(
                        "{} / {} (request_id: {})",
                        col.name, r.name, r.id
                    )
                })
                .collect();
            Err(ToolRefusal::new(format!(
                "Multiple saved requests match \"{req_selector}\": {}. Specify collection_id or pass the exact request_id.",
                choices.join("; ")
            ))
            .with_code("ambiguous_request"))
        }
    }
}
