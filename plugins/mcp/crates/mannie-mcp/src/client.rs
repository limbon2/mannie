//! The connection to one server (plugins/mcp/docs/adr/0001-speak-mcp-
//! through-rmcp-in-both-protocol-eras.md): rmcp's client over the pipes of a
//! local server that the core started, or over Streamable HTTP through the
//! plugin's own HTTP client, in the protocol era that the server's last
//! connection found.
//!
//! A first connection probes with `server/discover` and falls back to
//! `initialize` (`ClientLifecycleMode::Auto`). A server that `npx` is still
//! downloading may miss the probe's ten seconds and then end the connection
//! with a late error, so a first connection that fails is tried once more,
//! and the era that a connection found is stored for the next one
//! (docs/roadmap/m12-validation.md, section 3). A connection in a stored era
//! that fails probes again.
//!
//! Mannie declares no capabilities: no roots, no sampling and no
//! elicitation, so a server never asks the owner anything through it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures_util::StreamExt as _;
use mannie_plugin_sdk::services::local_servers::{
    LocalServer, LocalServerError, LocalServerSpec, LocalServersApi, ServerEnding, ServerOutput,
};
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::ClientHandler;
use rmcp::model::{
    ClientCapabilities, ClientConfig, Implementation, PaginatedRequestParams, ProtocolVersion,
    Resource, ResourceTemplate, ServerNotification, SubscriptionFilter, Tool,
};
use rmcp::service::{
    ClientInitializeError, ClientLifecycleMode, NotificationContext, Peer, RoleClient,
    RunningService, ServiceError, serve_client_with_lifecycle,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::auth::AuthClient;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClient, StreamableHttpClientTransportConfig,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::{mpsc, oneshot};

use crate::store::Era;

/// The bytes that the pipes between rmcp and a local server hold in each
/// direction.
const PIPE_BYTES: usize = 256 * 1024;

/// The most bytes that one write to a local server carries.
const SEND_CHUNK_BYTES: usize = 64 * 1024;

/// How long a listing of one page may take.
const PAGE_TIMEOUT: Duration = Duration::from_secs(30);

/// The most pages of one listing, so a server cannot page forever.
const MAX_PAGES: usize = 32;

/// The most tools that a listing keeps.
pub(crate) const MAX_LISTED_TOOLS: usize = 512;

/// The most resources that a listing keeps.
pub(crate) const MAX_LISTED_RESOURCES: usize = 500;

/// The most resource templates that a listing keeps.
pub(crate) const MAX_LISTED_TEMPLATES: usize = 100;

/// A change that a server announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    /// Its tools changed.
    Tools,
    /// Its resources changed.
    Resources,
}

/// The client's side of a connection: what Mannie says of itself, and where
/// a server's announcements of changes go.
#[derive(Debug, Clone)]
pub(crate) struct Handler {
    changes: mpsc::Sender<Change>,
}

impl Handler {
    pub(crate) fn new(changes: mpsc::Sender<Change>) -> Self {
        Self { changes }
    }

    fn announce(&self, change: Change) {
        // A full queue already holds a change, which lists everything again.
        if self.changes.try_send(change).is_err() {
            tracing::trace!(?change, "a change of a server waits in the queue already");
        }
    }
}

impl ClientHandler for Handler {
    fn get_info(&self) -> ClientConfig {
        // The version of the `initialize` handshake. `server/discover`
        // offers its own versions.
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("Mannie", env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.announce(Change::Tools);
    }

    async fn on_resource_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.announce(Change::Resources);
    }
}

/// A sink for the lines of a local server's error output.
pub(crate) type ErrorLines = Arc<dyn Fn(String) + Send + Sync>;

/// Where a server runs.
#[derive(Clone)]
pub(crate) enum Target {
    /// A program that the core starts.
    Local {
        local: Arc<dyn LocalServersApi>,
        spec: LocalServerSpec,
        errors: ErrorLines,
    },
    /// A URL, with its headers, through the plugin's HTTP client.
    Remote {
        client: RemoteClient,
        url: String,
        headers: HashMap<HeaderName, HeaderValue>,
    },
}

impl std::fmt::Debug for Target {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local { spec, .. } => formatter
                .debug_struct("Local")
                .field("program", &spec.program)
                .finish_non_exhaustive(),
            Self::Remote { url, .. } => formatter
                .debug_struct("Remote")
                .field("url", url)
                .finish_non_exhaustive(),
        }
    }
}

/// The HTTP client of a remote server: the plugin's own, or the same with
/// the server's OAuth tokens.
#[derive(Debug, Clone)]
pub(crate) enum RemoteClient {
    Plain(reqwest::Client),
    Authorized(AuthClient<reqwest::Client>),
}

/// What a connection learned of its server.
#[derive(Debug, Clone, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each switch is a capability that the server declares on its own"
)]
pub(crate) struct ServerInfo {
    /// The protocol version that it speaks.
    pub(crate) version: String,
    /// Its instructions for the model.
    pub(crate) instructions: Option<String>,
    /// Whether it offers tools.
    pub(crate) tools: bool,
    /// Whether it offers resources.
    pub(crate) resources: bool,
    /// Whether it announces changes of its tools.
    pub(crate) tools_change: bool,
    /// Whether it announces changes of its resources.
    pub(crate) resources_change: bool,
}

/// A connected server.
#[derive(Debug)]
pub(crate) struct Connected {
    pub(crate) service: RunningService<RoleClient, Handler>,
    /// The program of a local server.
    pub(crate) process: Option<Process>,
    pub(crate) era: Era,
    pub(crate) info: ServerInfo,
}

/// Why a connection failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConnectError {
    /// The core did not start the program.
    Local(LocalServerError),
    /// The program ended before it answered.
    Exited(Option<i32>),
    /// The server asks for a sign-in.
    SignInNeeded,
    /// The server asks for more permissions than the sign-in has.
    AccessNeeded,
    /// The server did not answer in time.
    TimedOut,
    /// The URL could not be reached.
    Unreachable,
    /// The server answered with an HTTP status that ends the connection.
    HttpStatus(u16),
    /// The server does not speak MCP as the plugin does.
    Protocol,
}

/// Connects to a server, within `timeout`.
pub(crate) async fn connect(
    target: &Target,
    era: Option<Era>,
    handler: &Handler,
    timeout: Duration,
) -> Result<Connected, ConnectError> {
    let started = Instant::now();
    let first = match era {
        Some(Era::Initialize) => ClientLifecycleMode::Initialize,
        Some(Era::Discover) => ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::LATEST],
        },
        None => auto(),
    };
    let attempt = attempt(target, first, handler, timeout).await;
    let retry = match &attempt {
        Ok(_connected) => false,
        // A program that the core refused, a sign-in and an unreachable
        // URL fail the same way again.
        Err(
            ConnectError::Local(_)
            | ConnectError::SignInNeeded
            | ConnectError::AccessNeeded
            | ConnectError::Unreachable
            | ConnectError::TimedOut,
        ) => false,
        Err(_other) => true,
    };
    let left = timeout.saturating_sub(started.elapsed());
    if !retry || left.is_zero() {
        return attempt;
    }
    tracing::debug!("a connection to an MCP server failed; probing its era again");
    self::attempt(target, auto(), handler, left).await
}

/// The lifecycle of a first connection: `server/discover`, then
/// `initialize` for a server of an earlier era.
fn auto() -> ClientLifecycleMode {
    ClientLifecycleMode::Auto {
        preferred_versions: vec![ProtocolVersion::LATEST],
        legacy_version: Some(ProtocolVersion::LATEST_WITH_INITIALIZE),
    }
}

/// One connection attempt.
async fn attempt(
    target: &Target,
    lifecycle: ClientLifecycleMode,
    handler: &Handler,
    timeout: Duration,
) -> Result<Connected, ConnectError> {
    match target {
        Target::Local {
            local,
            spec,
            errors,
        } => attempt_local(local, spec, errors, lifecycle, handler, timeout).await,
        Target::Remote {
            client: RemoteClient::Plain(client),
            url,
            headers,
        } => attempt_remote(client.clone(), url, headers, lifecycle, handler, timeout).await,
        Target::Remote {
            client: RemoteClient::Authorized(client),
            url,
            headers,
        } => attempt_remote(client.clone(), url, headers, lifecycle, handler, timeout).await,
    }
}

async fn attempt_local(
    local: &Arc<dyn LocalServersApi>,
    spec: &LocalServerSpec,
    errors: &ErrorLines,
    lifecycle: ClientLifecycleMode,
    handler: &Handler,
    timeout: Duration,
) -> Result<Connected, ConnectError> {
    let server = local
        .start(spec.clone())
        .await
        .map_err(ConnectError::Local)?;
    let (pipe, process) = Process::pipe(server, Arc::clone(errors));
    let started = tokio::time::timeout(
        timeout,
        serve_client_with_lifecycle(handler.clone(), tokio::io::split(pipe), lifecycle),
    )
    .await;
    match started {
        Ok(Ok(service)) => {
            let (era, info) = learned(&service);
            Ok(Connected {
                service,
                process: Some(process),
                era,
                info,
            })
        }
        Ok(Err(error)) => {
            let ending = process.stop(Duration::ZERO).await;
            tracing::debug!(error = %error, "a local MCP server did not connect");
            Err(match ending {
                Some(ServerEnding::Exited { code }) => ConnectError::Exited(Some(code)),
                Some(_ended) => ConnectError::Exited(None),
                None => initialize_failure(&error),
            })
        }
        Err(_elapsed) => {
            process.stop(Duration::ZERO).await;
            Err(ConnectError::TimedOut)
        }
    }
}

async fn attempt_remote<C>(
    client: C,
    url: &str,
    headers: &HashMap<HeaderName, HeaderValue>,
    lifecycle: ClientLifecycleMode,
    handler: &Handler,
    timeout: Duration,
) -> Result<Connected, ConnectError>
where
    C: StreamableHttpClient + Send + Sync + 'static,
{
    let config = StreamableHttpClientTransportConfig::with_uri(url)
        .custom_headers(headers.clone())
        .reinit_on_expired_session(true);
    let transport = StreamableHttpClientTransport::with_client(client, config);
    let started = tokio::time::timeout(
        timeout,
        serve_client_with_lifecycle(handler.clone(), transport, lifecycle),
    )
    .await;
    match started {
        Ok(Ok(service)) => {
            let (era, info) = learned(&service);
            Ok(Connected {
                service,
                process: None,
                era,
                info,
            })
        }
        Ok(Err(error)) => Err(initialize_failure(&error)),
        Err(_elapsed) => Err(ConnectError::TimedOut),
    }
}

/// The era and what the server said of itself.
fn learned(service: &RunningService<RoleClient, Handler>) -> (Era, ServerInfo) {
    let Some(peer) = service.peer_info() else {
        return (Era::Initialize, ServerInfo::default());
    };
    let era = if peer.protocol_version.has_initialize() {
        Era::Initialize
    } else {
        Era::Discover
    };
    let capabilities = &peer.capabilities;
    let info = ServerInfo {
        version: peer.protocol_version.as_str().to_owned(),
        instructions: peer.instructions.clone(),
        tools: capabilities.tools.is_some(),
        resources: capabilities.resources.is_some(),
        tools_change: capabilities
            .tools
            .as_ref()
            .and_then(|tools| tools.list_changed)
            == Some(true),
        resources_change: capabilities
            .resources
            .as_ref()
            .and_then(|resources| resources.list_changed)
            == Some(true),
    };
    (era, info)
}

/// What a failed handshake means.
fn initialize_failure(error: &ClientInitializeError) -> ConnectError {
    if let Some(challenge) = error.auth_challenge() {
        let wants_scope = challenge
            .to_ascii_lowercase()
            .contains("insufficient_scope");
        return if wants_scope {
            ConnectError::AccessNeeded
        } else {
            ConnectError::SignInNeeded
        };
    }
    if error.is_authorization_required() {
        return ConnectError::SignInNeeded;
    }
    transport_failure(error).unwrap_or(ConnectError::Protocol)
}

/// The cause of a failure that the transport reported, from the chain of an
/// error's sources.
pub(crate) fn transport_failure(error: &(dyn std::error::Error + 'static)) -> Option<ConnectError> {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if let Some(http) = current.downcast_ref::<reqwest::Error>() {
            if http.is_timeout() {
                return Some(ConnectError::TimedOut);
            }
            if http.is_connect() {
                return Some(ConnectError::Unreachable);
            }
            if let Some(status) = http.status() {
                return Some(ConnectError::HttpStatus(status.as_u16()));
            }
        }
        // rmcp reports an unexpected status as `HTTP <status>: <body>`.
        let text = current.to_string();
        let status = text
            .find("HTTP ")
            .and_then(|start| text.get(start + 5..start + 8))
            .and_then(|digits| digits.parse::<u16>().ok());
        if let Some(status) = status.filter(|status| (400..600).contains(status)) {
            return Some(ConnectError::HttpStatus(status));
        }
        source = current.source();
    }
    None
}

/// The program of a local server, behind the pipes that rmcp speaks over.
#[derive(Debug)]
pub(crate) struct Process {
    server: Arc<Mutex<Option<Box<dyn LocalServer>>>>,
    /// How the program ended, which the pump of its output learns.
    ending: Option<oneshot::Receiver<ServerEnding>>,
}

impl Process {
    /// Connects a started server to one end of an in-memory pipe and
    /// returns the other end, which rmcp reads and writes, with the server.
    fn pipe(
        mut server: Box<dyn LocalServer>,
        errors: ErrorLines,
    ) -> (tokio::io::DuplexStream, Self) {
        let mut output = server.output();
        let mut error_lines = server.errors();
        let server = Arc::new(Mutex::new(Some(server)));
        let (rmcp_side, plugin_side) = tokio::io::duplex(PIPE_BYTES);
        let (mut from_rmcp, mut to_rmcp) = tokio::io::split(plugin_side);
        let sending = Arc::clone(&server);
        tokio::spawn(async move {
            let mut buffer = vec![0_u8; SEND_CHUNK_BYTES];
            loop {
                let read = match from_rmcp.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                let send = sending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_ref()
                    .map(|server| server.send(buffer[..read].to_vec()));
                let Some(send) = send else {
                    break;
                };
                if send.await.is_err() {
                    break;
                }
            }
        });
        let (ended, ending) = oneshot::channel();
        tokio::spawn(async move {
            while let Some(item) = output.next().await {
                match item {
                    ServerOutput::Bytes { bytes } => {
                        if to_rmcp.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                    ServerOutput::Ended { ending } => {
                        if ended.send(ending).is_err() {
                            tracing::trace!("a local MCP server ended after its connection");
                        }
                        break;
                    }
                    _other => {}
                }
            }
            // rmcp reads the end of the output as the end of the connection.
            // Dropping one half of the split pipe leaves it open, since the
            // other half still reads, so the pump shuts its side down.
            if let Err(error) = to_rmcp.shutdown().await {
                tracing::trace!(%error, "the pipe of a local MCP server closed already");
            }
        });
        tokio::spawn(async move {
            while let Some(line) = error_lines.next().await {
                errors(line);
            }
        });
        (
            rmcp_side,
            Self {
                server,
                ending: Some(ending),
            },
        )
    }

    /// How the program ended, once it did, or `None` while it runs.
    pub(crate) fn ended(&mut self) -> Option<ServerEnding> {
        let ending = self.ending.as_mut()?;
        match ending.try_recv() {
            Ok(ending) => Some(ending),
            Err(oneshot::error::TryRecvError::Empty) => None,
            Err(oneshot::error::TryRecvError::Closed) => Some(ServerEnding::Ended),
        }
    }

    /// Closes the program's input, waits at most `grace` for it to end, then
    /// ends its tree. Returns how it ended, or `None` for a program that was
    /// stopped already.
    pub(crate) async fn stop(mut self, grace: Duration) -> Option<ServerEnding> {
        let server = self
            .server
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let ending = match server {
            Some(server) => server.stop(grace).await,
            None => return None,
        };
        Some(self.ended().unwrap_or(ending))
    }
}

/// Lists a server's tools, page by page, within the limits.
pub(crate) async fn list_tools(peer: &Peer<RoleClient>) -> Result<Vec<Tool>, ServiceError> {
    let mut tools = Vec::new();
    let mut cursor = None;
    for _page in 0..MAX_PAGES {
        let params = PaginatedRequestParams::default().with_cursor(cursor);
        let page = timed(peer.list_tools(Some(params))).await?;
        tools.extend(page.tools);
        cursor = page.next_cursor;
        if cursor.is_none() || tools.len() >= MAX_LISTED_TOOLS {
            break;
        }
    }
    tools.truncate(MAX_LISTED_TOOLS);
    Ok(tools)
}

/// Lists a server's resources and templates, page by page, within the
/// limits.
pub(crate) async fn list_resources(
    peer: &Peer<RoleClient>,
) -> Result<(Vec<Resource>, Vec<ResourceTemplate>), ServiceError> {
    let mut resources = Vec::new();
    let mut cursor = None;
    for _page in 0..MAX_PAGES {
        let params = PaginatedRequestParams::default().with_cursor(cursor);
        let page = timed(peer.list_resources(Some(params))).await?;
        resources.extend(page.resources);
        cursor = page.next_cursor;
        if cursor.is_none() || resources.len() >= MAX_LISTED_RESOURCES {
            break;
        }
    }
    resources.truncate(MAX_LISTED_RESOURCES);
    let mut templates = Vec::new();
    let mut cursor = None;
    for _page in 0..MAX_PAGES {
        let params = PaginatedRequestParams::default().with_cursor(cursor);
        let page = match timed(peer.list_resource_templates(Some(params))).await {
            Ok(page) => page,
            // A server may list resources and no templates.
            Err(ServiceError::McpError(_)) => break,
            Err(error) => return Err(error),
        };
        templates.extend(page.resource_templates);
        cursor = page.next_cursor;
        if cursor.is_none() || templates.len() >= MAX_LISTED_TEMPLATES {
            break;
        }
    }
    templates.truncate(MAX_LISTED_TEMPLATES);
    Ok((resources, templates))
}

/// A request of a listing, within [`PAGE_TIMEOUT`].
async fn timed<T>(
    request: impl Future<Output = Result<T, ServiceError>>,
) -> Result<T, ServiceError> {
    match tokio::time::timeout(PAGE_TIMEOUT, request).await {
        Ok(result) => result,
        Err(_elapsed) => Err(ServiceError::Timeout {
            timeout: PAGE_TIMEOUT,
        }),
    }
}

/// Follows the announcements of a server of the current era, which reach a
/// client only through a subscription, until the connection ends. A server
/// of an earlier era announces through the session, which the handler
/// hears.
pub(crate) fn follow_changes(
    peer: Peer<RoleClient>,
    era: Era,
    info: &ServerInfo,
    changes: mpsc::Sender<Change>,
) {
    let announces = info.tools_change || info.resources_change;
    if era != Era::Discover || !announces {
        return;
    }
    let mut filter = SubscriptionFilter::builder();
    if info.tools_change {
        filter = filter.tools_list_changed();
    }
    if info.resources_change {
        filter = filter.resources_list_changed();
    }
    let filter = filter.build();
    tokio::spawn(async move {
        let mut subscription = match peer.listen(filter).await {
            Ok(subscription) => subscription,
            Err(error) => {
                tracing::debug!(%error, "an MCP server took no subscription");
                return;
            }
        };
        while let Ok(Some(notification)) = subscription.next().await {
            let change = match notification {
                ServerNotification::ToolListChangedNotification(_) => Change::Tools,
                ServerNotification::ResourceListChangedNotification(_) => Change::Resources,
                _other => continue,
            };
            if changes.try_send(change).is_err() && changes.is_closed() {
                return;
            }
        }
    });
}
