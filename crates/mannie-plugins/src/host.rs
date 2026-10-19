//! The host of the plugins (ADR-0030, decisions 6 and 7; ADR-0034, decisions
//! 4 to 6): it starts the plugins that the owner turned on, in the order of
//! their manifests, turns plugins on and off while Mannie runs, keeps the
//! owner's choices, and publishes each plugin's state for the Plugins page.
//!
//! One transition runs at a time. Turning a plugin on stores the choice,
//! binds the services that the plugin's manifest declares and no other,
//! starts the plugin, hands its contributions to the core's consumers, all
//! or none, through the SDK's coordinator, and lets calls reach its
//! handlers. Turning it off stores the choice, takes its contributions out
//! of the consumers, refuses new calls, ends its channels, waits for its
//! calls in flight or aborts them, and then stops its instance (ADR-0034,
//! decision 8).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mannie_plugin_sdk::context::{DefaultContext, HostInfo, StartContext};
use mannie_plugin_sdk::contracts::{Clock, SubscriptionEnd};
use mannie_plugin_sdk::error::{CONTRIBUTIONS_REFUSED, SDK_INCOMPATIBLE, SERVICE_UNAVAILABLE};
use mannie_plugin_sdk::host::{
    ContributionConsumer, PluginBinding, ServiceProvider, add_contributions, remove_contributions,
};
use mannie_plugin_sdk::ids::{PluginId, prefixes_nest};
use mannie_plugin_sdk::ipc::Handlers;
use mannie_plugin_sdk::manifest::{Manifest, ManifestError};
use mannie_plugin_sdk::plugin::{Instance, Plugin, PluginEntry};
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::service_info;
use mannie_plugin_sdk::services::store::{
    ModuleQueries, ModuleStoreApi, Store as StoreService, StoreModule,
};
use mannie_store::{PluginModule, Store, StoreError};
use mannie_types::plugins::{
    PluginState, PluginUses, PluginView, PluginWork, PluginsError, PluginsSettings, ServiceUse,
};
use tokio::sync::watch;

use crate::dispatch::Calls;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only takes or replaces the value.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why the host could not take the composition root's list or the core's
/// parts. Each one is a fault of the build, which the desktop app's test of
/// the bundled plugins catches.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// The manifest of the list's entry at `index`, counting from 0, was
    /// refused, such as one with an unknown field or an ID that breaks the
    /// rules of plugin IDs.
    #[error("the manifest of the plugin at {index} in the list was refused")]
    Manifest {
        /// The entry's position in the list.
        index: usize,
        /// Why the manifest was refused.
        #[source]
        source: ManifestError,
    },
    /// Two plugins have one ID.
    #[error("two plugins have the ID {id}")]
    DuplicateId {
        /// The ID.
        id: String,
    },
    /// The tables of one plugin would be named like the tables of another,
    /// as `log_*` names `log_viewer_*`.
    #[error("the prefixes of the plugins {id} and {other} nest")]
    NestedPrefixes {
        /// The plugin listed later.
        id: String,
        /// The plugin listed earlier.
        other: String,
    },
    /// A plugin has a module of the store, and its manifest does not declare
    /// the service `store`.
    #[error("the plugin {id} has a module of the store and does not declare the service store")]
    ModuleWithoutStore {
        /// The plugin's ID.
        id: String,
    },
    /// The store was not opened with a plugin's module.
    #[error("the store was not opened with the module of the plugin {id}")]
    ModuleNotRegistered {
        /// The plugin's ID.
        id: String,
        /// The store's error, boxed to keep the result small.
        #[source]
        source: Box<StoreError>,
    },
    /// Two of the core's providers offer one service.
    #[error("two providers offer the service {service}")]
    DuplicateProvider {
        /// The service's name.
        service: &'static str,
    },
    /// Two of the core's consumers take one point.
    #[error("two consumers take the point {point}")]
    DuplicatePoint {
        /// The point's name.
        point: &'static str,
    },
}

/// A plugin of the list, as the host checked its manifest.
#[derive(Debug, Clone)]
struct Listed {
    id: PluginId,
    /// The manifest, or `None` for a plugin built for another major of the
    /// SDK, whose other fields this SDK does not read.
    manifest: Option<Manifest>,
    plugin: Arc<dyn Plugin>,
    /// The plugin's module of the store.
    module: Option<StoreModule>,
}

impl Listed {
    /// Where the plugin starts. A plugin for another major of the SDK has no
    /// order that this SDK reads, so it comes after the others.
    fn order(&self) -> u32 {
        self.manifest
            .as_ref()
            .map_or(u32::MAX, |manifest| manifest.order)
    }

    /// What the manifest lets the plugin use, as the Plugins page lists it
    /// (ADR-0034, decision 13): its services by name, each marked sensitive
    /// as the SDK marks it, its entries in the credential store as the store
    /// names them, the commands and channels of its interface, which the
    /// build grants its UI (decision 8), its programs, the crates that only
    /// it may use and the kinds of direct native calls that it grants its
    /// library crates (ADR-0035, decision 6). A plugin for another major of
    /// the SDK uses nothing here, since its code is never called.
    fn uses(&self) -> PluginUses {
        let Some(manifest) = &self.manifest else {
            return PluginUses::default();
        };
        let services = manifest
            .services
            .iter()
            .map(|(name, &major)| ServiceUse {
                name: name.clone(),
                major,
                sensitive: service_info(name).is_some_and(|service| service.sensitive),
            })
            .collect();
        let mut secrets: Vec<String> = manifest
            .secrets
            .entries
            .iter()
            .map(|entry| format!("{}/{entry}", self.id))
            .collect();
        secrets.extend(manifest.secrets.legacy.iter().cloned());
        let interface = self.plugin.interface();
        let names = |names: Vec<&'static str>| names.into_iter().map(str::to_owned).collect();
        PluginUses {
            services,
            secrets,
            commands: names(interface.command_names()),
            channels: names(interface.channel_names()),
            programs: manifest
                .programs
                .iter()
                .map(|program| program.package.clone())
                .collect(),
            crates: manifest.crates.exclusive.clone(),
            native: manifest
                .native
                .grants()
                .into_iter()
                .map(|(category, _)| category.to_owned())
                .collect(),
        }
    }
}

/// The plugins with their manifests, checked, in the order in which they
/// start and the Plugins page lists them: by their manifests' order, then
/// by their IDs.
#[derive(Debug, Clone)]
pub struct PluginList {
    plugins: Vec<Listed>,
}

impl PluginList {
    /// Checks the manifest of each entry, then the list as a whole: two
    /// plugins never share an ID or a prefix, and a plugin with a module of
    /// the store declares the service `store`.
    pub fn new(entries: Vec<PluginEntry>) -> Result<Self, HostError> {
        let mut plugins: Vec<Listed> = Vec::with_capacity(entries.len());
        for (index, entry) in entries.into_iter().enumerate() {
            let listed = listed(index, entry)?;
            if plugins.iter().any(|other| other.id == listed.id) {
                return Err(HostError::DuplicateId {
                    id: listed.id.to_string(),
                });
            }
            if let Some(other) = plugins
                .iter()
                .find(|other| prefixes_nest(&other.id.prefix(), &listed.id.prefix()))
            {
                return Err(HostError::NestedPrefixes {
                    id: listed.id.to_string(),
                    other: other.id.to_string(),
                });
            }
            plugins.push(listed);
        }
        plugins.sort_by(|a, b| a.order().cmp(&b.order()).then_with(|| a.id.cmp(&b.id)));
        Ok(Self { plugins })
    }

    /// The plugins' modules of the store, with their prefixes, which the
    /// store opens with whether or not the plugins are on (ADR-0030,
    /// decision 5).
    pub fn store_modules(&self) -> Vec<PluginModule> {
        self.plugins
            .iter()
            .filter_map(|listed| {
                listed
                    .module
                    .map(|module| PluginModule::new(&listed.id, module))
            })
            .collect()
    }
}

/// One entry of the list, as its manifest says. The code of a plugin for
/// another major of the SDK is never called, so its module is not asked.
fn listed(index: usize, entry: PluginEntry) -> Result<Listed, HostError> {
    let plugin = entry.plugin;
    match Manifest::parse(entry.manifest) {
        Ok(manifest) => {
            let module = plugin.store_module();
            if module.is_some() && !manifest.declares(StoreService::NAME) {
                return Err(HostError::ModuleWithoutStore {
                    id: manifest.id.to_string(),
                });
            }
            Ok(Listed {
                id: manifest.id.clone(),
                manifest: Some(manifest),
                plugin,
                module,
            })
        }
        Err(ManifestError::SdkIncompatible { id: Some(raw), sdk }) => {
            match PluginId::parse(&raw) {
                Ok(id) => Ok(Listed {
                    id,
                    manifest: None,
                    plugin,
                    module: None,
                }),
                // Without a valid ID the plugin cannot be listed even as one
                // that cannot start.
                Err(_invalid) => Err(HostError::Manifest {
                    index,
                    source: ManifestError::SdkIncompatible { id: Some(raw), sdk },
                }),
            }
        }
        Err(source) => Err(HostError::Manifest { index, source }),
    }
}

/// What the host needs of the core, which the composition root hands it.
#[derive(Debug)]
pub struct PluginHostDeps {
    /// The store, opened with the list's modules, which keeps the owner's
    /// choices.
    pub store: Store,
    /// The plugins.
    pub plugins: PluginList,
    /// The core's providers of the services that manifests declare, such as
    /// the runtime's and the store's (ADR-0034, decision 5).
    pub providers: Vec<Arc<dyn ServiceProvider>>,
    /// The core's consumers of the extension points (decision 6).
    pub consumers: Vec<Arc<dyn ContributionConsumer>>,
    /// The clock of every timestamp.
    pub clock: Arc<dyn Clock>,
    /// The host that runs the plugins.
    pub host: HostInfo,
}

#[derive(Debug)]
struct Entry {
    listed: Listed,
    /// The plugin's instance while it is on.
    running: Mutex<Option<Box<dyn Instance>>>,
    /// The handlers of the plugin's commands and channels while it is on,
    /// and its calls in flight.
    calls: Calls,
}

#[derive(Debug)]
struct Inner {
    entries: Vec<Entry>,
    store: Store,
    providers: Vec<Arc<dyn ServiceProvider>>,
    consumers: Vec<Arc<dyn ContributionConsumer>>,
    clock: Arc<dyn Clock>,
    host: HostInfo,
    /// The plugins as the Plugins page shows them, in the list's order.
    views: watch::Sender<Vec<PluginView>>,
    /// Lets one transition run at a time.
    transitions: tokio::sync::Mutex<()>,
    shutting_down: AtomicBool,
}

/// The host of the plugins. Cloning is cheap; clones share everything.
#[derive(Debug, Clone)]
pub struct PluginHost {
    inner: Arc<Inner>,
}

/// The queries of a plugin's module, which its default reads before the
/// plugin starts and can write.
fn queries_of(store: Arc<dyn ModuleStoreApi>) -> Arc<dyn ModuleQueries> {
    store
}

impl PluginHost {
    /// A host of the list's plugins, every one off until
    /// [`PluginHost::start_chosen`]. It refuses two providers of one service,
    /// two consumers of one point, and a store that was not opened with a
    /// plugin's module.
    pub fn new(deps: PluginHostDeps) -> Result<Self, HostError> {
        let PluginHostDeps {
            store,
            plugins,
            providers,
            consumers,
            clock,
            host,
        } = deps;
        let mut services = BTreeSet::new();
        for provider in &providers {
            if !services.insert(provider.name()) {
                return Err(HostError::DuplicateProvider {
                    service: provider.name(),
                });
            }
        }
        let mut points = BTreeSet::new();
        for consumer in &consumers {
            for point in consumer.points() {
                if !points.insert((point.name, point.major)) {
                    return Err(HostError::DuplicatePoint { point: point.name });
                }
            }
        }
        for listed in &plugins.plugins {
            if listed.module.is_none() {
                continue;
            }
            match store.plugin_store(&listed.id) {
                // A module whose statements did not prepare stays listed: the
                // store's provider refuses to bind it, so only that plugin
                // fails to start, with `service_unavailable` (ADR-0034,
                // decision 7).
                Ok(_) | Err(StoreError::StatementsRefused { .. }) => {}
                Err(source) => {
                    return Err(HostError::ModuleNotRegistered {
                        id: listed.id.to_string(),
                        source: Box::new(source),
                    });
                }
            }
        }
        let views = plugins
            .plugins
            .iter()
            .map(|listed| PluginView {
                id: listed.id.to_string(),
                state: PluginState::Off,
                enabled: false,
                failure: None,
                uses: listed.uses(),
            })
            .collect();
        let entries = plugins
            .plugins
            .into_iter()
            .map(|listed| Entry {
                listed,
                running: Mutex::new(None),
                calls: Calls::default(),
            })
            .collect();
        Ok(Self {
            inner: Arc::new(Inner {
                entries,
                store,
                providers,
                consumers,
                clock,
                host,
                views: watch::Sender::new(views),
                transitions: tokio::sync::Mutex::new(()),
                shutting_down: AtomicBool::new(false),
            }),
        })
    }

    /// The plugins as they are now.
    pub fn views(&self) -> Vec<PluginView> {
        self.inner.views.borrow().clone()
    }

    /// The plugins, as they are now and as they change.
    pub fn subscribe(&self) -> watch::Receiver<Vec<PluginView>> {
        self.inner.views.subscribe()
    }

    /// Starts the plugins that the owner turned on, in the list's order, as
    /// Mannie starts after the core's recovery (ADR-0030, decision 7). A
    /// plugin without the owner's choice follows its default for the data
    /// that the store holds, and the outcome is kept as the choice, so an
    /// update keeps a plugin on that the owner already uses. A plugin for
    /// another major of the SDK is never asked its default, and no choice is
    /// kept for it.
    pub async fn start_chosen(&self) {
        let _transition = self.inner.transitions.lock().await;
        let mut settings = match self.inner.store.load_settings::<PluginsSettings>().await {
            Ok(settings) => settings,
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read the owner's choices of plugins; every plugin stays off");
                return;
            }
        };
        let mut decided = false;
        for entry in &self.inner.entries {
            if self.inner.shutting_down.load(Ordering::SeqCst) {
                return;
            }
            let id = entry.listed.id.as_str();
            let enabled = if let Some(enabled) = settings.enabled.get(id) {
                *enabled
            } else if entry.listed.manifest.is_none() {
                tracing::warn!(
                    plugin = id,
                    "a plugin built for another major of the plugin SDK stays off"
                );
                continue;
            } else {
                let enabled = entry
                    .listed
                    .plugin
                    .default_on(self.default_context(&entry.listed))
                    .await;
                tracing::info!(
                    plugin = id,
                    enabled,
                    "a plugin without the owner's choice follows its default"
                );
                settings.enabled.insert(id.to_owned(), enabled);
                decided = true;
                enabled
            };
            self.change_view(id, |view| view.enabled = enabled);
            if enabled {
                self.start_entry(entry).await;
            }
        }
        if decided && let Err(error) = self.inner.store.save_settings(&settings).await {
            tracing::warn!(%error, error.code = error.code(), "could not keep the defaults of plugins as the owner's choices");
        }
    }

    /// Turns a plugin on or off, as the owner's lasting choice (ADR-0030,
    /// decision 6). The choice is stored first, so it lasts whatever the
    /// plugin's start does: a start that fails leaves the plugin failed, and
    /// turning it on again tries again.
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<PluginView, PluginsError> {
        let entry = self.entry(id).ok_or(PluginsError::UnknownPlugin)?;
        let _transition = self.inner.transitions.lock().await;
        if self.inner.shutting_down.load(Ordering::SeqCst) {
            return Err(PluginsError::ShuttingDown);
        }
        let mut settings = self
            .inner
            .store
            .load_settings::<PluginsSettings>()
            .await
            .map_err(|error| {
                tracing::error!(%error, error.code = error.code(), "could not read the owner's choices of plugins");
                PluginsError::StorageFailed
            })?;
        settings.enabled.insert(id.to_owned(), enabled);
        self.inner
            .store
            .save_settings(&settings)
            .await
            .map_err(|error| {
                tracing::error!(%error, error.code = error.code(), "could not keep the owner's choice of a plugin");
                PluginsError::StorageFailed
            })?;
        self.change_view(id, |view| view.enabled = enabled);
        let state = self.state(id);
        match (enabled, state) {
            (true, PluginState::Off | PluginState::Failed) => self.start_entry(entry).await,
            (false, PluginState::On) => {
                self.stop_entry(entry, SubscriptionEnd::PluginOff).await;
            }
            (false, PluginState::Failed) => self.change_view(id, |view| {
                view.state = PluginState::Off;
                view.failure = None;
            }),
            // A transition ends before the next one starts, so the plugin is
            // already where the owner wants it.
            _settled => {}
        }
        self.view(id).ok_or(PluginsError::UnknownPlugin)
    }

    /// The work that a plugin has in progress, which turning it off lets go,
    /// as its instance reports it (ADR-0034, decision 4): none while the
    /// plugin is not on, and never an item whose count is 0.
    pub async fn work(&self, id: &str) -> Result<PluginWork, PluginsError> {
        let entry = self.entry(id).ok_or(PluginsError::UnknownPlugin)?;
        let pending = lock(&entry.running)
            .as_ref()
            .map(|instance| instance.work());
        let Some(pending) = pending else {
            return Ok(PluginWork::default());
        };
        let items = pending
            .await
            .into_iter()
            .filter(|item| item.count > 0)
            .collect();
        Ok(PluginWork { items })
    }

    /// Stops the plugins that are on, the last started first, because Mannie
    /// quits. The owner's choices stay (ADR-0030, decision 7).
    pub async fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        let _transition = self.inner.transitions.lock().await;
        for entry in self.inner.entries.iter().rev() {
            if self.state(entry.listed.id.as_str()) == PluginState::On {
                self.stop_entry(entry, SubscriptionEnd::Shutdown).await;
            }
        }
    }

    /// The calls of the plugin `id`, for the dispatch.
    pub(crate) fn calls(&self, id: &str) -> Option<&Calls> {
        self.entry(id).map(|entry| &entry.calls)
    }

    /// Whether Mannie quits, so the dispatch refuses every call.
    pub(crate) fn shutting_down(&self) -> bool {
        self.inner.shutting_down.load(Ordering::SeqCst)
    }

    fn entry(&self, id: &str) -> Option<&Entry> {
        self.inner
            .entries
            .iter()
            .find(|entry| entry.listed.id.as_str() == id)
    }

    fn view(&self, id: &str) -> Option<PluginView> {
        self.inner
            .views
            .borrow()
            .iter()
            .find(|view| view.id == id)
            .cloned()
    }

    fn state(&self, id: &str) -> PluginState {
        self.view(id).map_or(PluginState::Off, |view| view.state)
    }

    fn change_view(&self, id: &str, change: impl FnOnce(&mut PluginView)) {
        self.inner.views.send_modify(|views| {
            if let Some(view) = views.iter_mut().find(|view| view.id == id) {
                change(view);
            }
        });
    }

    /// Binds one service that a plugin's manifest declares, with the
    /// provider's name for it, or logs why it could not: no provider offers
    /// it, the provider offers another major, or the provider refused.
    fn bind_service(
        &self,
        listed: &Listed,
        manifest: &Manifest,
        name: &str,
        major: u16,
    ) -> Option<(&'static str, BoundService)> {
        let plugin = listed.id.as_str();
        let Some(provider) = self
            .inner
            .providers
            .iter()
            .find(|provider| provider.name() == name)
        else {
            tracing::error!(
                plugin,
                service = name,
                "no part of the core provides a service that the plugin's manifest declares"
            );
            return None;
        };
        if provider.major() != major {
            tracing::error!(
                plugin,
                service = name,
                major,
                provided = provider.major(),
                "the core provides another major of a service that the plugin's manifest declares"
            );
            return None;
        }
        let binding = PluginBinding::new(&listed.id, manifest, self.inner.host.profile);
        match provider.bind(&binding) {
            Ok(bound) => Some((provider.name(), bound)),
            Err(refusal) => {
                tracing::error!(
                    plugin,
                    service = refusal.service,
                    detail = %refusal.detail,
                    "a provider did not bind its service to the plugin"
                );
                None
            }
        }
    }

    /// The context of a plugin's default: the host, and the queries of the
    /// plugin's module through the service `store`, bound as at a start.
    fn default_context(&self, listed: &Listed) -> DefaultContext {
        let store = listed.manifest.as_ref().and_then(|manifest| {
            let major = *manifest.services.get(StoreService::NAME)?;
            let (_name, bound) = self.bind_service(listed, manifest, StoreService::NAME, major)?;
            bound.get::<StoreService>().map(queries_of)
        });
        DefaultContext::new(self.inner.host.clone(), store)
    }

    /// Starts a plugin and keeps its instance. A plugin whose start fails
    /// ends failed with the code, and the rest of Mannie keeps working.
    async fn start_entry(&self, entry: &Entry) {
        let id = entry.listed.id.as_str();
        self.change_view(id, |view| {
            view.state = PluginState::Starting;
            view.failure = None;
        });
        match self.start_instance(&entry.listed).await {
            Ok((instance, handlers)) => {
                *lock(&entry.running) = Some(instance);
                entry.calls.open(handlers);
                tracing::info!(plugin = id, "a plugin started");
                self.change_view(id, |view| view.state = PluginState::On);
            }
            Err(code) => self.change_view(id, |view| {
                view.state = PluginState::Failed;
                view.failure = Some(code);
            }),
        }
    }

    /// Starts a plugin, in this order: a plugin for another major of the SDK
    /// fails with `sdk_incompatible` before any of its code runs; each
    /// service of the manifest is bound, or the start fails with
    /// `service_unavailable`; the plugin starts; and its contributions go to
    /// the core's consumers, all or none, or its instance stops and the start
    /// fails with `contributions_refused`. Returns the instance and the
    /// handlers of its commands and channels, or the code of the failure.
    async fn start_instance(
        &self,
        listed: &Listed,
    ) -> Result<(Box<dyn Instance>, Handlers), String> {
        let id = listed.id.as_str();
        let Some(manifest) = &listed.manifest else {
            tracing::warn!(
                plugin = id,
                "a plugin built for another major of the plugin SDK cannot start"
            );
            return Err(SDK_INCOMPATIBLE.to_owned());
        };
        let mut bound = BTreeMap::new();
        for (name, &major) in &manifest.services {
            let (name, service) = self
                .bind_service(listed, manifest, name, major)
                .ok_or_else(|| SERVICE_UNAVAILABLE.to_owned())?;
            bound.insert(name, service);
        }
        let declared = manifest.services.keys().cloned().collect();
        let context = StartContext::new(
            listed.id.clone(),
            self.inner.host.clone(),
            Arc::clone(&self.inner.clock),
            declared,
            bound,
        );
        let started = listed.plugin.start(context).await.map_err(|failure| {
            tracing::warn!(
                plugin = id,
                code = failure.code(),
                "a plugin could not start"
            );
            failure.code().to_owned()
        })?;
        let instance = started.instance;
        let handlers = started.handlers;
        if let Err(refusal) =
            add_contributions(&self.inner.consumers, &listed.id, started.contributions)
        {
            tracing::error!(
                plugin = id,
                point = refusal.point,
                code = refusal.code,
                detail = %refusal.detail,
                "the core refused a plugin's contributions"
            );
            instance.stop().await;
            return Err(CONTRIBUTIONS_REFUSED.to_owned());
        }
        Ok((instance, handlers))
    }

    /// Takes a plugin's contributions out of the core's consumers, so no run
    /// that starts meanwhile gets them (ADR-0030, decision 6), refuses new
    /// calls, ends its channels with `end`, waits for its calls in flight or
    /// aborts them, and then stops its instance (ADR-0034, decision 8).
    async fn stop_entry(&self, entry: &Entry, end: SubscriptionEnd) {
        let id = entry.listed.id.as_str();
        let instance = lock(&entry.running).take();
        self.change_view(id, |view| view.state = PluginState::Stopping);
        remove_contributions(&self.inner.consumers, &entry.listed.id);
        entry.calls.close(id, end).await;
        if let Some(instance) = instance {
            instance.stop().await;
        }
        tracing::info!(plugin = id, "a plugin stopped");
        self.change_view(id, |view| view.state = PluginState::Off);
    }
}
