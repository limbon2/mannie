//! MCP servers for the MCP plugin's tests (plugins/mcp/docs/README.md):
//! [`TestServer`], an rmcp server of the current protocol era that the tests
//! reach over Streamable HTTP in their own process ([`serve_http`]) or over
//! stdio as the example program `mcp-test-server`, and a scripted server of
//! the 2025-06-18 era, which the example runs with `--legacy`
//! ([`legacy::serve`]). Nothing here ships.
//!
//! The server's tools cover what the plugin maps: text, structured content,
//! an image, audio, a resource link and an embedded resource, an error, a
//! slow call, a crash, a tool with an argument named `activity`, a variable
//! of the server's environment and a tool that the server adds later. Over
//! HTTP, the server may take one fixed `Authorization` header, or sign in
//! with OAuth through a fake authorization server of its own ([`oauth`]).

use std::borrow::Cow;
use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use http_body_util::combinators::BoxBody;
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    Implementation, InitializeResult, JsonObject, ListResourceTemplatesResult, ListResourcesResult,
    ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, ResourceTemplate, ServerCapabilities,
    ServerConfig, SubscriptionFilter, Tool, ToolAnnotations,
};
use rmcp::service::{
    MaybeSendFuture, RequestContext, RoleServer, SubscriptionContext, SubscriptionSink,
};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ServerHandler, ServiceExt as _};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::oauth::{Authorization, OAuthOptions};

pub mod legacy;
pub mod oauth;

/// The variable whose value the program writes to its error output and
/// `read_variable` returns, so a test can plant a secret there. While it is
/// set, the server also repeats it where a careless server would: in its
/// instructions, in the description of `read_variable`, and in the error
/// of `fail` when the call names the variable as `leak`.
pub const SECRET_VARIABLE: &str = "MCP_TEST_TOKEN";

/// The server's name, as it introduces itself.
pub const SERVER_NAME: &str = "mannie-mcp-test-server";

/// What the server tells the client about using it.
pub const INSTRUCTIONS: &str = "Use echo to repeat text. Read test://readme first.";

/// The text resource.
pub const README_URI: &str = "test://readme";

/// The text of the text resource.
pub const README_TEXT: &str = "Read me first: this server only tests.";

/// The binary resource, a PNG image.
pub const LOGO_URI: &str = "test://logo";

/// The template of the server's notes.
pub const NOTE_TEMPLATE: &str = "test://notes/{id}";

/// A PNG image of one transparent pixel, as base64.
pub const PIXEL_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/// The tool that the server adds when `add_tool` runs.
pub const LATE_TOOL: &str = "late_tool";

/// The server of the current era. Clones share its state, so the tools that
/// it adds stay added across the requests of Streamable HTTP, each of which
/// may get a clone.
#[derive(Debug, Clone, Default)]
pub struct TestServer {
    state: Arc<State>,
}

#[derive(Debug, Default)]
struct State {
    /// Whether `add_tool` ran.
    late_tool: AtomicBool,
    /// The subscriptions of clients of the current era, which hear of a
    /// changed list.
    sinks: Mutex<Vec<SubscriptionSink>>,
}

impl TestServer {
    /// A server with its first tools.
    pub fn new() -> Self {
        Self::default()
    }

    fn tools(&self) -> Vec<Tool> {
        let mut tools = vec![
            tool(
                "echo",
                "Repeats the text.",
                json!({ "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] }),
            )
            .with_hints(ToolAnnotations::new().read_only(true)),
            tool(
                "add",
                "Adds two numbers.",
                json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": { "a": { "$ref": "#/$defs/number" }, "b": { "$ref": "#/$defs/number" } },
                    "required": ["a", "b"],
                    "$defs": { "number": { "type": "number" } }
                }),
            )
            .with_hints(ToolAnnotations::new().read_only(true).idempotent(true)),
            tool(
                "show_image",
                "Returns a picture of one pixel.",
                json!({ "type": "object" }),
            ),
            tool(
                "read_variable",
                with_planted("Returns the value of a variable of the server's environment."),
                json!({ "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] }),
            ),
            tool(
                "sleep",
                "Waits for some seconds, then answers.",
                json!({ "type": "object", "properties": { "seconds": { "type": "number" } }, "required": ["seconds"] }),
            ),
            tool(
                "fail",
                "Reports an error.",
                json!({ "type": "object", "properties": { "leak": { "type": "string" } } }),
            )
            .with_hints(ToolAnnotations::new().destructive(true).open_world(true)),
            tool(
                "crash",
                "Ends the server's process.",
                json!({ "type": "object" }),
            ),
            tool(
                "track-activity",
                "Records an activity.",
                json!({ "type": "object", "properties": { "activity": { "type": "string" } }, "required": ["activity"] }),
            ),
            tool(
                "add_tool",
                "Adds the tool late_tool and tells the client.",
                json!({ "type": "object" }),
            ),
            tool(
                "mixed_content",
                "Returns audio, a resource link and an embedded resource.",
                json!({ "type": "object" }),
            ),
        ];
        if self.state.late_tool.load(Ordering::SeqCst) {
            tools.push(tool(
                LATE_TOOL,
                "Arrived after the first listing.",
                json!({ "type": "object" }),
            ));
        }
        tools
    }

    /// Adds the late tool and tells the clients: through their
    /// subscriptions in the current era, or through the session before it.
    async fn add_late_tool(&self, context: &RequestContext<RoleServer>) {
        self.state.late_tool.store(true, Ordering::SeqCst);
        let sinks = self
            .state
            .sinks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if sinks.is_empty() {
            if let Err(error) = context.peer.notify_tool_list_changed().await {
                tracing_note(&format!("the change of the tools was not sent: {error}"));
            }
            return;
        }
        for sink in sinks {
            if let Err(error) = sink.notify_tool_list_changed().await {
                tracing_note(&format!("a subscription missed the change: {error}"));
            }
        }
    }
}

/// A tool with a schema, which must be an object.
fn tool(name: &'static str, description: impl Into<Cow<'static, str>>, schema: Value) -> Tool {
    let Value::Object(schema) = schema else {
        return Tool::new(name, description, Arc::new(JsonObject::new()));
    };
    Tool::new(name, description, Arc::new(schema))
}

/// `text` with the planted secret after it, while a test set one.
fn with_planted(text: &'static str) -> Cow<'static, str> {
    match std::env::var(SECRET_VARIABLE) {
        Ok(token) if !token.is_empty() => Cow::Owned(format!("{text} Your token is {token}.")),
        _unset => Cow::Borrowed(text),
    }
}

/// The hints of a tool, set after it is made.
trait WithHints {
    fn with_hints(self, hints: ToolAnnotations) -> Self;
}

impl WithHints for Tool {
    fn with_hints(mut self, hints: ToolAnnotations) -> Self {
        self.annotations = Some(hints);
        self
    }
}

/// A line on the program's error output, which the plugin keeps in its log.
#[expect(
    clippy::print_stderr,
    reason = "a server's error output is what the plugin's log shows"
)]
fn tracing_note(line: &str) {
    eprintln!("{line}");
}

/// A text answer.
fn text(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text)])
}

impl ServerHandler for TestServer {
    fn get_info(&self) -> ServerConfig {
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_tool_list_changed()
            .enable_resources()
            .enable_resources_list_changed()
            .build();
        InitializeResult::new(capabilities)
            .with_server_info(Implementation::new(SERVER_NAME, "1.0.0"))
            .with_instructions(with_planted(INSTRUCTIONS))
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.clone())
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        self.state
            .sinks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(context.sink().clone());
        context.cancelled().await;
        Ok(())
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(self.tools())))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments = request.arguments.unwrap_or_default();
        let string = |name: &str| {
            arguments
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let number = |name: &str| arguments.get(name).and_then(Value::as_f64).unwrap_or(0.0);
        let result = match request.name.as_ref() {
            "echo" => text(format!("Echo: {}", string("text"))),
            "add" => {
                let sum = number("a") + number("b");
                let mut result = text(sum.to_string());
                result.structured_content = Some(json!({ "sum": sum }));
                result
            }
            "show_image" => CallToolResult::success(vec![
                ContentBlock::text("One pixel:"),
                ContentBlock::image(PIXEL_PNG, "image/png"),
            ]),
            "read_variable" => text(std::env::var(string("name")).unwrap_or_default()),
            "sleep" => {
                tokio::time::sleep(Duration::from_secs_f64(number("seconds").max(0.0))).await;
                text("Slept.")
            }
            "fail" if arguments.contains_key("leak") => {
                let value = std::env::var(string("leak")).unwrap_or_default();
                return Err(ErrorData::internal_error(
                    Cow::Owned(format!("the call failed with the token {value}")),
                    None,
                ));
            }
            "fail" => {
                CallToolResult::error(vec![ContentBlock::text("The tool failed on purpose.")])
            }
            "crash" => end_the_process(),
            "track-activity" => text(format!("Activity: {}", string("activity"))),
            "add_tool" => {
                self.add_late_tool(&context).await;
                text("Added late_tool.")
            }
            "mixed_content" => CallToolResult::success(vec![
                ContentBlock::audio("UklGRgAAAABXQVZF", "audio/wav"),
                ContentBlock::ResourceLink(
                    Resource::new(README_URI, "readme").with_mime_type("text/plain"),
                ),
                ContentBlock::resource(ResourceContents::text("Embedded text.", "test://embedded")),
            ]),
            LATE_TOOL if self.state.late_tool.load(Ordering::SeqCst) => text("Late."),
            other => {
                return Err(ErrorData::invalid_params(
                    Cow::Owned(format!("there is no tool {other}")),
                    None,
                ));
            }
        };
        Ok(result.into())
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> + MaybeSendFuture + '_ {
        std::future::ready(Ok(ListResourcesResult::with_all_items(vec![
            Resource::new(README_URI, "readme")
                .with_title("Read me")
                .with_mime_type("text/plain"),
            Resource::new(LOGO_URI, "logo").with_mime_type("image/png"),
        ])))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, ErrorData>> + MaybeSendFuture + '_
    {
        std::future::ready(Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new(NOTE_TEMPLATE, "note"),
        ])))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, ErrorData>> + MaybeSendFuture + '_ {
        let contents = match request.uri.as_str() {
            README_URI => Ok(ResourceContents::text(README_TEXT, README_URI)),
            LOGO_URI => Ok(ResourceContents::blob(PIXEL_PNG, LOGO_URI).with_mime_type("image/png")),
            other => Err(ErrorData::invalid_params(
                Cow::Owned(format!("there is no resource {other}")),
                None,
            )),
        };
        std::future::ready(contents.map(|contents| ReadResourceResult::new(vec![contents]).into()))
    }
}

/// Ends the program, as a server that crashes during a call does.
#[expect(
    clippy::exit,
    reason = "the tool crash ends the server's process on purpose"
)]
fn end_the_process() -> CallToolResult {
    std::process::exit(3)
}

/// Serves [`TestServer`] over stdin and stdout until the client closes them,
/// with a line on the error output that names the planted secret.
pub async fn serve_stdio() -> std::io::Result<()> {
    if let Ok(value) = std::env::var(SECRET_VARIABLE) {
        tracing_note(&format!("starting with the token {value}"));
    }
    let running = TestServer::new()
        .serve(rmcp::transport::stdio())
        .await
        .map_err(std::io::Error::other)?;
    running.waiting().await.map_err(std::io::Error::other)?;
    Ok(())
}

/// How [`serve_http`] answers.
#[derive(Debug, Clone, Default)]
pub struct HttpOptions {
    /// The value that the `Authorization` header must have, such as
    /// `Bearer planted`, or `None` to take every request.
    pub authorization: Option<String>,
    /// A fake authorization server on the same origin, whose access tokens
    /// the endpoint takes instead, or `None`.
    pub oauth: Option<OAuthOptions>,
}

/// A remote server in the test's process, on a loopback port.
#[derive(Debug)]
pub struct HttpServer {
    address: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    authorization: Option<Arc<Authorization>>,
}

impl HttpServer {
    /// The URL of the server's endpoint.
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.address)
    }

    /// The access tokens that its authorization server issued.
    pub fn access_tokens(&self) -> Vec<String> {
        self.authorization
            .as_ref()
            .map(|authorization| authorization.access_tokens())
            .unwrap_or_default()
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        let stopped = self.stop.take().is_none_or(|stop| stop.send(()).is_ok());
        if !stopped {
            tracing_note("the HTTP server had stopped already");
        }
    }
}

/// Serves [`TestServer`] over Streamable HTTP on a loopback port, until the
/// returned server is dropped.
pub async fn serve_http(options: HttpOptions) -> std::io::Result<HttpServer> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    let authorization = options
        .oauth
        .clone()
        .map(|oauth| Arc::new(Authorization::new(format!("http://{address}"), oauth)));
    let serving = authorization.clone();
    let server = TestServer::new();
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let (stop, mut stopped) = oneshot::channel();
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _stopped = &mut stopped => return,
                accepted = listener.accept() => accepted,
            };
            let Ok((stream, _peer)) = accepted else {
                continue;
            };
            let inner = TowerToHyperService::new(service.clone());
            let expected = options.authorization.clone();
            let signs_in = serving.clone();
            tokio::spawn(async move {
                let handle = hyper::service::service_fn(move |request: Request<Incoming>| {
                    let inner = inner.clone();
                    let expected = expected.clone();
                    let signs_in = signs_in.clone();
                    async move {
                        if let Some(authorization) = signs_in {
                            if Authorization::handles(request.uri().path()) {
                                return Ok::<_, Infallible>(authorization.answer(request).await);
                            }
                            if !authorization.takes(&request) {
                                return Ok(authorization.challenge());
                            }
                        } else if !is_authorized(&request, expected.as_deref()) {
                            return Ok(unauthorized());
                        }
                        hyper::service::Service::call(&inner, request).await
                    }
                });
                let connection = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), handle)
                    .await;
                if let Err(error) = connection {
                    tracing_note(&format!("a connection ended early: {error}"));
                }
            });
        }
    });
    Ok(HttpServer {
        address,
        stop: Some(stop),
        authorization,
    })
}

/// Whether a request carries the `Authorization` header that the server
/// expects.
fn is_authorized(request: &Request<Incoming>, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    request
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        == Some(expected)
}

/// The answer to a request without the expected `Authorization` header.
fn unauthorized() -> Response<BoxBody<Bytes, Infallible>> {
    oauth::text(StatusCode::UNAUTHORIZED, "unauthorized")
}

/// The example program `mcp-test-server`, which cargo builds beside the
/// tests of the plugin's workspace into the shared target folder.
///
/// # Panics
///
/// When the program is not there, as after `cargo test -p mannie-mcp`
/// alone, which builds no example of this crate; the message says what to
/// run instead.
pub fn server_program() -> PathBuf {
    let test = std::env::current_exe().unwrap_or_default();
    // A test runs as target/<profile>/deps/<test>, beside the examples in
    // target/<profile>/examples.
    let profile = test
        .parent()
        .and_then(Path::parent)
        .unwrap_or(Path::new(""));
    let name = if cfg!(windows) {
        "mcp-test-server.exe"
    } else {
        "mcp-test-server"
    };
    let program = profile.join("examples").join(name);
    assert!(
        program.is_file(),
        "{} is missing: run the tests of the plugin's whole workspace, as `task test` does, which builds the example",
        program.display()
    );
    program
}
