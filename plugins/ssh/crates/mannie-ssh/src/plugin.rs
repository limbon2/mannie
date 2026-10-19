//! The SSH plugin's definition (ADR-0036; plugins/ssh/docs/README.md): its
//! interface, its table, its start and stop, its contributions and the
//! handlers of its commands and channel. Its manifest, `plugins/ssh/
//! plugin.toml`, gives the services that its start receives. It is off by
//! default, like every plugin (D28).
//!
//! A start reads the hosts and contributes the tool `run_ssh_command`, the
//! section of the hosts and the mention provider. Turning the plugin off
//! removes them, the host waits for the tool's calls that run and drops the
//! rest (ADR-0036, decision 7), and the stop closes every session.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::StartContext;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{Handlers, Interface, PluginStream};
use mannie_plugin_sdk::plugin::{Instance, Plugin, Started};
use mannie_plugin_sdk::points::mentions::{MentionProvider, Mentions};
use mannie_plugin_sdk::points::sections::{Section, SectionBody, Sections};
use mannie_plugin_sdk::points::tools::{AgentSelector, PluginTool, Tools};
use mannie_plugin_sdk::services::secrets::Secrets;
use mannie_plugin_sdk::services::store::{Store, StoreModule};
use tokio::sync::{oneshot, watch};

use crate::commands::{
    AddHost, AddHostCommand, FollowHostsChannel, HostListUpdate, HostRef, RemoveHostCommand,
    ScanHostKeyCommand, SshError, TestHostCommand, TrustHostKey, TrustHostKeyCommand, UpdateHost,
    UpdateHostCommand, interface,
};
use crate::hosts::Hosts;
use crate::mentions::HostMentions;
use crate::section::{AGENTS, HostsSection, SECTION_ID, SECTION_POSITION};
use crate::store::{HostRecord, MODULE};
use crate::tool::RunSshCommand;

/// The plugin's ID.
pub const SSH: &str = "ssh";

/// The code of a start that could not read the hosts.
pub const STORAGE_FAILED: &str = "storage_failed";

/// How often the plugin closes the sessions that were idle too long.
const IDLE_CHECK: Duration = Duration::from_secs(60);

/// The SSH plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct SshPlugin;

impl SshPlugin {
    /// The plugin, off.
    pub fn new() -> Self {
        Self
    }
}

/// The hosts as they are, then after each change, until the start stops.
fn host_list(records: watch::Receiver<Vec<HostRecord>>) -> PluginStream<HostListUpdate> {
    stream::unfold((records, true), |(mut records, first)| async move {
        if !first && records.changed().await.is_err() {
            return None;
        }
        let hosts = records
            .borrow_and_update()
            .iter()
            .map(HostRecord::view)
            .collect();
        Some((HostListUpdate::Hosts { hosts }, (records, false)))
    })
    .boxed()
}

/// The handlers of a start, which reach its hosts.
fn handlers(hosts: &Hosts) -> Handlers {
    let adding = hosts.clone();
    let updating = hosts.clone();
    let removing = hosts.clone();
    let scanning = hosts.clone();
    let trusting = hosts.clone();
    let testing = hosts.clone();
    let following = hosts.clone();
    Handlers::new()
        .command::<AddHostCommand, _, _>(move |input: AddHost| {
            let hosts = adding.clone();
            async move { hosts.add(input).await }
        })
        .command::<UpdateHostCommand, _, _>(move |input: UpdateHost| {
            let hosts = updating.clone();
            async move { hosts.update(input).await }
        })
        .command::<RemoveHostCommand, _, _>(move |input: HostRef| {
            let hosts = removing.clone();
            async move { hosts.remove(&input.host_id).await }
        })
        .command::<ScanHostKeyCommand, _, _>(move |input: HostRef| {
            let hosts = scanning.clone();
            async move { hosts.scan(&input.host_id).await }
        })
        .command::<TrustHostKeyCommand, _, _>(move |input: TrustHostKey| {
            let hosts = trusting.clone();
            async move { hosts.trust(&input.host_id, &input.fingerprint).await }
        })
        .command::<TestHostCommand, _, _>(move |input: HostRef| {
            let hosts = testing.clone();
            async move { hosts.test(&input.host_id).await }
        })
        .channel::<FollowHostsChannel, _, _>(move |()| {
            std::future::ready(Ok::<_, SshError>(host_list(following.follow())))
        })
}

/// Closes the sessions that were idle too long, until the start stops,
/// which drops the sender of `stopped`.
async fn close_idle_sessions(hosts: Hosts, mut stopped: oneshot::Receiver<()>) {
    let mut interval = tokio::time::interval(IDLE_CHECK);
    loop {
        tokio::select! {
            _stopped = &mut stopped => return,
            _tick = interval.tick() => hosts.sessions().close_idle(Instant::now()),
        }
    }
}

impl SshPlugin {
    async fn start_now(context: StartContext) -> Result<Started, StartFailure> {
        let store = context.service::<Store>()?;
        let secrets = context.service::<Secrets>()?;
        let hosts = Hosts::load(store, secrets, context.clock())
            .await
            .map_err(|error| {
                tracing::error!(
                    error.code = error.code(),
                    "the SSH plugin could not read its hosts"
                );
                StartFailure::new(STORAGE_FAILED)
            })?;
        let agents = AGENTS.iter().map(|agent| (*agent).to_owned()).collect();
        let tool = PluginTool::new(
            Arc::new(RunSshCommand::new(hosts.clone())),
            AgentSelector::Agents(agents),
        )
        .with_side_effects();
        let section = Section::new(
            SECTION_ID,
            SECTION_POSITION,
            SectionBody::Source(Arc::new(HostsSection::new(hosts.clone()))),
        );
        let mentions: Arc<dyn MentionProvider> = Arc::new(HostMentions::new(hosts.clone()));
        let contributions = Contributions::new()
            .add::<Tools>(tool)
            .add::<Sections>(section)
            .add::<Mentions>(mentions);
        let handlers = handlers(&hosts);
        let (stop_closing, stopped) = oneshot::channel();
        tokio::spawn(close_idle_sessions(hosts.clone(), stopped));
        tracing::info!("the SSH plugin started");
        Ok(Started::new(SshInstance {
            hosts,
            stop_closing,
        })
        .with_contributions(contributions)
        .with_handlers(handlers))
    }
}

impl Plugin for SshPlugin {
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

/// The plugin while it is on: its hosts, and what stops the task that
/// closes idle sessions.
#[derive(Debug)]
struct SshInstance {
    hosts: Hosts,
    stop_closing: oneshot::Sender<()>,
}

impl Instance for SshInstance {
    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        // The host removed the contributions, ended the calls and the
        // channel, and waited for the tool's calls first.
        let Self {
            hosts,
            stop_closing,
        } = *self;
        drop(stop_closing);
        hosts.close_sessions();
        tracing::info!("the SSH plugin stopped");
        Box::pin(std::future::ready(()))
    }
}
