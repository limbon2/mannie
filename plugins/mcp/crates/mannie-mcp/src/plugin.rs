//! The MCP plugin's definition (ADR-0043; plugins/mcp/docs/README.md): its
//! interface, its tables, its start and stop, its contributions and the
//! handlers of its commands and channel. Its manifest, `plugins/mcp/
//! plugin.toml`, gives the services that its start receives. It is off by
//! default, like every plugin (D28).
//!
//! A start reads the servers, contributes the tool source, the section of
//! the servers and the mention provider of their resources, and connects to
//! every server that is on. Turning the plugin off removes the
//! contributions, the host waits for the calls that run and drops the rest,
//! the stop closes every connection, and the core ends the local servers'
//! programs that are left (ADR-0043, decision 4).

use std::sync::Arc;

use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::StartContext;
use mannie_plugin_sdk::contracts::WorkItem;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{Handlers, Interface, PluginStream};
use mannie_plugin_sdk::plugin::{Instance, Plugin, Started};
use mannie_plugin_sdk::points::mentions::{MentionProvider, Mentions};
use mannie_plugin_sdk::points::sections::{Section, SectionBody, Sections};
use mannie_plugin_sdk::points::tool_sources::{PluginToolSource, ToolSources};
use mannie_plugin_sdk::services::local_servers::LocalServers;
use mannie_plugin_sdk::services::secrets::Secrets;
use mannie_plugin_sdk::services::store::{Store, StoreModule};
use tokio::sync::watch;

use crate::commands::{
    AcknowledgeToolsCommand, AddServerCommand, BeginSignIn, BeginSignInCommand, ConfigurationText,
    FollowServersChannel, ImportConfiguration, ImportConfigurationCommand, LocateProgram,
    LocateProgramCommand, McpError, ParseConfigurationCommand, RemoveServerCommand,
    RestartServerCommand, ServerInput, ServerListUpdate, ServerLogCommand, ServerRef,
    ServerToolsCommand, SetServerEnabled, SetServerEnabledCommand, SetToolEnabled,
    SetToolEnabledCommand, SignOutCommand, UpdateServer, UpdateServerCommand, interface,
};
use crate::imports;
use crate::mentions::ResourceMentions;
use crate::oauth;
use crate::section::{SECTION_ID, SECTION_POSITION, ServersSection};
use crate::servers::Servers;
use crate::source::{McpToolSource, SOURCE_ID};
use crate::store::MODULE;

/// The plugin's ID.
pub const MCP: &str = "mcp";

/// The code of a start that could not read the servers.
pub const STORAGE_FAILED: &str = "storage_failed";

/// The code of a start that could not build its HTTP client.
pub const HTTP_FAILED: &str = "http_client_failed";

/// The code of the work of connected servers, which the UI words.
pub const CONNECTED_SERVERS: &str = "connected_servers";

/// The code of the work of calls in progress, which the UI words.
pub const ACTIVE_CALLS: &str = "active_calls";

/// The MCP plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct McpPlugin;

impl McpPlugin {
    /// The plugin, off.
    pub fn new() -> Self {
        Self
    }
}

/// The servers as they are, then after each change, until the start stops.
fn server_list(
    servers: Servers,
    revisions: watch::Receiver<u64>,
) -> PluginStream<ServerListUpdate> {
    stream::unfold(
        (servers, revisions, true),
        |(servers, mut revisions, first)| async move {
            if !first && revisions.changed().await.is_err() {
                return None;
            }
            revisions.borrow_and_update();
            let update = servers.list_update();
            Some((update, (servers, revisions, false)))
        },
    )
    .boxed()
}

/// The handlers of a start, which reach its servers.
fn handlers(servers: &Servers) -> Handlers {
    let adding = servers.clone();
    let updating = servers.clone();
    let removing = servers.clone();
    let enabling = servers.clone();
    let restarting = servers.clone();
    let listing = servers.clone();
    let switching = servers.clone();
    let acknowledging = servers.clone();
    let logging = servers.clone();
    let parsing = servers.clone();
    let importing = servers.clone();
    let locating = servers.clone();
    let signing_in = servers.clone();
    let signing_out = servers.clone();
    let following = servers.clone();
    Handlers::new()
        .command::<AddServerCommand, _, _>(move |input: ServerInput| {
            let servers = adding.clone();
            async move { servers.add(input).await }
        })
        .command::<UpdateServerCommand, _, _>(move |input: UpdateServer| {
            let servers = updating.clone();
            async move { servers.update(input).await }
        })
        .command::<RemoveServerCommand, _, _>(move |input: ServerRef| {
            let servers = removing.clone();
            async move { servers.remove(&input.server_id).await }
        })
        .command::<SetServerEnabledCommand, _, _>(move |input: SetServerEnabled| {
            let servers = enabling.clone();
            async move { servers.set_enabled(&input.server_id, input.enabled).await }
        })
        .command::<RestartServerCommand, _, _>(move |input: ServerRef| {
            let servers = restarting.clone();
            async move { servers.restart(&input.server_id).await }
        })
        .command::<ServerToolsCommand, _, _>(move |input: ServerRef| {
            std::future::ready(listing.tools(&input.server_id))
        })
        .command::<SetToolEnabledCommand, _, _>(move |input: SetToolEnabled| {
            let servers = switching.clone();
            async move {
                servers
                    .set_tool_enabled(&input.server_id, &input.tool, input.enabled)
                    .await
            }
        })
        .command::<AcknowledgeToolsCommand, _, _>(move |input: ServerRef| {
            let servers = acknowledging.clone();
            async move { servers.acknowledge_tools(&input.server_id).await }
        })
        .command::<ServerLogCommand, _, _>(move |input: ServerRef| {
            std::future::ready(logging.log(&input.server_id))
        })
        .command::<ParseConfigurationCommand, _, _>(move |input: ConfigurationText| {
            std::future::ready(imports::preview(&parsing, &input.text))
        })
        .command::<ImportConfigurationCommand, _, _>(move |input: ImportConfiguration| {
            let servers = importing.clone();
            async move { imports::import(&servers, &input.text, &input.keys).await }
        })
        .command::<LocateProgramCommand, _, _>(move |input: LocateProgram| {
            let servers = locating.clone();
            async move { servers.locate(&input.program).await }
        })
        .command::<BeginSignInCommand, _, _>(move |input: BeginSignIn| {
            let servers = signing_in.clone();
            async move { oauth::begin(&servers, &input.server_id, input.page).await }
        })
        .command::<SignOutCommand, _, _>(move |input: ServerRef| {
            let servers = signing_out.clone();
            async move { servers.sign_out(&input.server_id).await }
        })
        .channel::<FollowServersChannel, _, _>(move |()| {
            let revisions = following.follow();
            std::future::ready(Ok::<_, McpError>(server_list(following.clone(), revisions)))
        })
}

/// The plugin's HTTP client of the remote servers: rustls with ring and the
/// operating system's trust store, the system's proxy settings, and no
/// redirects, since a redirect could carry a secret header to another host.
fn http_client() -> Result<reqwest::Client, reqwest::Error> {
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_err()
    {
        tracing::trace!("the crypto provider was already installed");
    }
    reqwest::Client::builder()
        .user_agent(concat!("Mannie/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
}

impl McpPlugin {
    async fn start_now(context: StartContext) -> Result<Started, StartFailure> {
        let store = context.service::<Store>()?;
        let secrets = context.service::<Secrets>()?;
        let local = context.service::<LocalServers>()?;
        let http = http_client().map_err(|error| {
            tracing::error!(%error, "the MCP plugin could not build its HTTP client");
            StartFailure::new(HTTP_FAILED)
        })?;
        let servers = Servers::load(
            context.plugin().clone(),
            store,
            secrets,
            local,
            http,
            context.clock(),
        )
        .await
        .map_err(|error| {
            tracing::error!(
                error.code = error.code(),
                "the MCP plugin could not read its servers"
            );
            StartFailure::new(STORAGE_FAILED)
        })?;
        let source =
            PluginToolSource::new(SOURCE_ID, Arc::new(McpToolSource::new(servers.clone())));
        let section = Section::new(
            SECTION_ID,
            SECTION_POSITION,
            SectionBody::Source(Arc::new(ServersSection::new(servers.clone()))),
        );
        let mentions: Arc<dyn MentionProvider> = Arc::new(ResourceMentions::new(servers.clone()));
        let contributions = Contributions::new()
            .add::<ToolSources>(source)
            .add::<Sections>(section)
            .add::<Mentions>(mentions);
        let handlers = handlers(&servers);
        servers.start_all();
        tracing::info!("the MCP plugin started");
        Ok(Started::new(McpInstance { servers })
            .with_contributions(contributions)
            .with_handlers(handlers))
    }
}

impl Plugin for McpPlugin {
    fn interface(&self) -> Interface {
        interface()
    }

    fn store_module(&self) -> Option<StoreModule> {
        Some(MODULE)
    }

    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        Box::pin(Self::start_now(context))
    }
}

/// The plugin while it is on.
#[derive(Debug)]
struct McpInstance {
    servers: Servers,
}

impl Instance for McpInstance {
    fn work(&self) -> BoxFuture<'static, Vec<WorkItem>> {
        let (connected, calls) = self.servers.work();
        let mut work = Vec::new();
        if calls > 0 {
            work.push(WorkItem::new(ACTIVE_CALLS, calls));
        }
        if connected > 0 {
            work.push(WorkItem::new(CONNECTED_SERVERS, connected));
        }
        Box::pin(std::future::ready(work))
    }

    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        // The host removed the contributions, ended the calls and the
        // channel, and waited for the tools' calls first.
        let servers = self.servers;
        Box::pin(async move {
            servers.stop_all().await;
            tracing::info!("the MCP plugin stopped");
        })
    }
}
