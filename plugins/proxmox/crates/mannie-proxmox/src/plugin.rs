//! Proxmox plugin entry point and lifecycle (`Plugin` + `Instance`, ADR-0080).

use std::sync::Arc;
use std::time::Duration;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::{HostProfile, StartContext};
use mannie_plugin_sdk::contracts::{ConversationId, WorkItem};
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::Interface;
use mannie_plugin_sdk::plugin::{Instance, Plugin, Started};
use mannie_plugin_sdk::points::management::{Management, PluginManagement};
use mannie_plugin_sdk::points::mentions::{MentionProvider, Mentions};
use mannie_plugin_sdk::points::sections::{Section, SectionBody, Sections};
use mannie_plugin_sdk::points::tools::Tools;
use mannie_plugin_sdk::services::secrets::Secrets;
use mannie_plugin_sdk::services::store::{Store, StoreModule};
use tokio::sync::oneshot;

use crate::commands::{handlers, interface};
use crate::management::ProxmoxManagement;
use crate::mentions::ProxmoxMentions;
use crate::model::PROXMOX;
use crate::section::{ProxmoxSection, SECTION_ID, SECTION_POSITION};
use crate::state::ProxmoxManager;
use crate::store::MODULE;
use crate::tools::all_tools;
use crate::vnc::DEFAULT_IDLE_TIMEOUT;

/// Start failure code when SQLite state cannot be read.
pub const STORAGE_FAILED: &str = "storage_failed";

/// Work item code reported by `Instance::work()` for active VNC sessions.
pub const WORK_VNC_SESSIONS: &str = "vnc_sessions";

/// How often the idle session reaper runs.
const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Idle timeout in validation builds (short enough for timeout tests without flaking E2E).
const VALIDATION_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// The Proxmox VMs and VNC plugin (off by default on new installations per D28).
#[derive(Debug, Clone, Copy, Default)]
pub struct ProxmoxPlugin {
    idle_timeout_override: Option<Duration>,
}

impl ProxmoxPlugin {
    /// Creates the plugin with standard production idle timeouts.
    pub fn new() -> Self {
        Self {
            idle_timeout_override: None,
        }
    }

    /// Creates the plugin with a custom VNC idle timeout (used in integration tests).
    pub fn with_idle_timeout(idle_timeout: Duration) -> Self {
        Self {
            idle_timeout_override: Some(idle_timeout),
        }
    }

    async fn start_now(
        context: StartContext,
        idle_timeout_override: Option<Duration>,
    ) -> Result<Started, StartFailure> {
        let store = context.service::<Store>()?;
        let secrets = context.service::<Secrets>()?;
        let idle_timeout = idle_timeout_override.unwrap_or_else(|| {
            if context.host().profile == HostProfile::Validation {
                VALIDATION_IDLE_TIMEOUT
            } else {
                DEFAULT_IDLE_TIMEOUT
            }
        });
        let manager = ProxmoxManager::load(store, secrets, context.clock(), idle_timeout)
            .await
            .map_err(|error| {
                tracing::error!(
                    error.code = error.code(),
                    "the Proxmox plugin could not read its servers"
                );
                StartFailure::new(STORAGE_FAILED)
            })?;

        let mut contributions = Contributions::new();
        for tool in all_tools(&manager) {
            contributions = contributions.add::<Tools>(tool);
        }

        let section = Section::new(
            SECTION_ID,
            SECTION_POSITION,
            SectionBody::Source(Arc::new(ProxmoxSection::new(manager.clone()))),
        );
        let mentions: Arc<dyn MentionProvider> = Arc::new(ProxmoxMentions::new(manager.clone()));
        let management =
            PluginManagement::new(PROXMOX, Arc::new(ProxmoxManagement::new(manager.clone())));
        contributions = contributions
            .add::<Sections>(section)
            .add::<Mentions>(mentions)
            .add::<Management>(management);

        let ipc_handlers = handlers(&manager);
        let (stop_reaper, reaper_rx) = oneshot::channel();
        let check_interval = idle_timeout
            .min(IDLE_CHECK_INTERVAL)
            .max(Duration::from_millis(50));
        tokio::spawn(reap_idle_sessions(
            manager.clone(),
            check_interval,
            reaper_rx,
        ));

        tracing::info!("the Proxmox plugin started");
        Ok(Started::new(ProxmoxInstance {
            manager,
            stop_reaper,
        })
        .with_contributions(contributions)
        .with_handlers(ipc_handlers))
    }
}

impl Plugin for ProxmoxPlugin {
    fn interface(&self) -> Interface {
        interface()
    }

    fn store_module(&self) -> Option<StoreModule> {
        Some(MODULE)
    }

    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        Box::pin(Self::start_now(context, self.idle_timeout_override))
    }
}

#[derive(Debug)]
struct ProxmoxInstance {
    manager: ProxmoxManager,
    stop_reaper: oneshot::Sender<()>,
}

impl Instance for ProxmoxInstance {
    fn work(&self) -> BoxFuture<'static, Vec<WorkItem>> {
        let count = self.manager.vnc().active_count();
        let items = if count > 0 {
            vec![WorkItem::new(WORK_VNC_SESSIONS, count)]
        } else {
            Vec::new()
        };
        Box::pin(std::future::ready(items))
    }

    fn conversations_deleted(&self, conversations: Vec<ConversationId>) -> BoxFuture<'static, ()> {
        self.manager.vnc().close_conversations(&conversations);
        Box::pin(std::future::ready(()))
    }

    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        let Self {
            manager,
            stop_reaper,
        } = *self;
        drop(stop_reaper);
        Box::pin(async move {
            manager.shutdown().await;
            tracing::info!("the Proxmox plugin stopped");
        })
    }
}

async fn reap_idle_sessions(
    manager: ProxmoxManager,
    interval: Duration,
    mut stopped: oneshot::Receiver<()>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = &mut stopped => return,
            _ = ticker.tick() => manager.vnc().close_idle(),
        }
    }
}
