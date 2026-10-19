//! One plugin through the real plugin host, on the core's test runtime: the
//! runtime with its store, its fake model and a profile, and the host with
//! the runtime's and the store's providers of services beside the test's own,
//! as the desktop app's composition root sets them up. The plugin's module of
//! the store is registered when the store opens, as the composition root
//! registers it, and the plugin starts off, as in a new installation.

use std::sync::Arc;

use mannie_plugin_sdk::context::{HostInfo, HostProfile};
use mannie_plugin_sdk::host::ServiceProvider;
use mannie_plugin_sdk::ipc::{ChannelMessage, PluginStream};
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::plugin::PluginEntry;
use mannie_plugins::{PluginHostDeps, PluginList};
use mannie_runtime::testing::{Harness, HarnessOptions};
use mannie_types::clock::SystemClock;
use mannie_types::plugins::PluginView;
use serde_json::Value;

// What a test reads of the host: its answers to the plugin's calls, and the
// host itself.
pub use mannie_plugins::{CallFailure, PluginHost};

/// How the host of a hosted plugin is set up.
#[derive(Debug, Clone)]
pub struct HostOptions {
    core: HarnessOptions,
    profile: HostProfile,
    providers: Vec<Arc<dyn ServiceProvider>>,
}

impl Default for HostOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl HostOptions {
    /// A host of a release build, on a test runtime with its defaults.
    pub fn new() -> Self {
        Self {
            core: HarnessOptions::default(),
            profile: HostProfile::Release,
            providers: Vec::new(),
        }
    }

    /// The same host on a test runtime set up as `core`. The plugin's
    /// module of the store joins the modules that `core` lists.
    #[must_use]
    pub fn with_core(mut self, core: HarnessOptions) -> Self {
        self.core = core;
        self
    }

    /// The same host as a host of `profile`, such as a validation build.
    #[must_use]
    pub fn with_profile(mut self, profile: HostProfile) -> Self {
        self.profile = profile;
        self
    }

    /// The same host with `provider` beside the runtime's and the store's,
    /// such as a fake of a service that only the desktop app provides.
    #[must_use]
    pub fn with_provider(mut self, provider: Arc<dyn ServiceProvider>) -> Self {
        self.providers.push(provider);
        self
    }
}

/// One plugin, hosted as the desktop app hosts it.
#[derive(Debug)]
pub struct HostedPlugin {
    /// The core: its runtime, its store, the fake model behind its profile
    /// and its credential vault.
    pub core: Harness,
    /// The host, which lists the plugin alone.
    pub host: PluginHost,
    id: String,
}

impl HostedPlugin {
    /// Hosts the plugin of `entry` on a new test runtime. The plugin is off
    /// until the test turns it on.
    ///
    /// # Panics
    ///
    /// When the plugin's manifest or its module of the store is refused,
    /// which fails the test with the reason.
    pub async fn new(entry: PluginEntry, options: HostOptions) -> Self {
        let manifest = Manifest::parse(entry.manifest).expect("the plugin's manifest is valid");
        let id = manifest.id.as_str().to_owned();
        let plugins = PluginList::new(vec![entry]).expect("the host lists the plugin");
        let mut core = options.core;
        core.store_modules.extend(plugins.store_modules());
        let core = Harness::with_options(core).await;
        let mut providers = core.runtime.service_providers();
        providers.extend(mannie_store::service_providers(&core.store));
        providers.extend(options.providers);
        let host = PluginHost::new(PluginHostDeps {
            store: core.store.clone(),
            plugins,
            providers,
            consumers: core.runtime.contribution_consumers(),
            clock: Arc::new(SystemClock),
            host: HostInfo::new(env!("CARGO_PKG_VERSION"), options.profile),
        })
        .expect("the host takes the plugin");
        Self { core, host, id }
    }

    /// The plugin's ID.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The core and the host, for a test that calls them itself.
    pub fn into_parts(self) -> (Harness, PluginHost) {
        (self.core, self.host)
    }

    /// The plugin as the Plugins page shows it.
    ///
    /// # Panics
    ///
    /// Never: the host lists the plugin.
    pub fn view(&self) -> PluginView {
        self.host
            .views()
            .into_iter()
            .find(|view| view.id == self.id)
            .expect("the host lists the plugin")
    }

    /// Turns the plugin on, as its switch does, and returns its view once
    /// the start ended: `on`, or `failed` with the code of the failure.
    ///
    /// # Panics
    ///
    /// When the host cannot keep the owner's choice, which fails the test.
    pub async fn turn_on(&self) -> PluginView {
        self.host
            .set_enabled(&self.id, true)
            .await
            .expect("the host keeps the choice")
    }

    /// Turns the plugin off, as its switch does, and returns its view once
    /// it stopped.
    ///
    /// # Panics
    ///
    /// When the host cannot keep the owner's choice, which fails the test.
    pub async fn turn_off(&self) -> PluginView {
        self.host
            .set_enabled(&self.id, false)
            .await
            .expect("the host keeps the choice")
    }

    /// Calls the plugin's command `command` with `input` through the host's
    /// dispatch, as the plugin's UI does.
    pub async fn invoke(&self, command: &str, input: Value) -> Result<Value, CallFailure> {
        self.host.invoke(&self.id, command, input).await
    }

    /// Opens the plugin's channel `channel` with `input` through the host's
    /// dispatch, as the plugin's UI does.
    pub async fn open_channel(
        &self,
        channel: &str,
        input: Value,
    ) -> Result<PluginStream<ChannelMessage<Value>>, CallFailure> {
        self.host.open_channel(&self.id, channel, input).await
    }
}
