//! The host of the internal plugins (ADR-0030, decisions 6 and 7): it starts
//! the plugins that the owner turned on, in the order of the composition
//! root's list, turns plugins on and off while Mannie runs, keeps the
//! owner's choices, and publishes each plugin's state for the Plugins page.
//!
//! One transition runs at a time. Turning a plugin on stores the choice,
//! starts the plugin and hands its contributions to the runtime; turning it
//! off stores the choice, takes its contributions out of the runtime and then
//! stops it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mannie_runtime::Runtime;
use mannie_store::{ModuleStore, Store, StoreError};
use mannie_types::clock::Clock;
use mannie_types::plugins::{PluginState, PluginView, PluginWork, PluginsError, PluginsSettings};
use tokio::sync::watch;

use crate::{CONTRIBUTIONS_REFUSED, Grants, Plugin, StartContext, is_valid_plugin_id};

/// A plugin as the composition root lists it, with what it may use.
#[derive(Debug, Clone)]
pub struct HostedPlugin {
    /// The plugin.
    pub plugin: Arc<dyn Plugin>,
    /// The services that the composition root grants it.
    pub grants: Grants,
}

/// Why the host could not take the composition root's list.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// A plugin's ID is not lower-case words joined by hyphens.
    #[error("the plugin ID {id} is not lower-case words joined by hyphens")]
    InvalidId {
        /// The ID.
        id: String,
    },
    /// Two plugins have one ID.
    #[error("two plugins have the ID {id}")]
    DuplicateId {
        /// The ID.
        id: String,
    },
    /// The store was not opened with one of a plugin's modules.
    #[error("the store was not opened with the module of the plugin {id}")]
    ModuleNotRegistered {
        /// The plugin's ID.
        id: String,
        /// The store's error, boxed to keep the result small.
        #[source]
        source: Box<StoreError>,
    },
}

#[derive(Debug)]
struct Entry {
    plugin: Arc<dyn Plugin>,
    grants: Grants,
    stores: Vec<ModuleStore>,
}

#[derive(Debug)]
struct Inner {
    entries: Vec<Entry>,
    runtime: Runtime,
    store: Store,
    clock: Arc<dyn Clock>,
    version: String,
    /// The plugins as the Plugins page shows them, in the list's order.
    views: watch::Sender<Vec<PluginView>>,
    /// Lets one transition run at a time.
    transitions: tokio::sync::Mutex<()>,
    shutting_down: AtomicBool,
}

/// The host of the internal plugins. Cloning is cheap; clones share
/// everything.
#[derive(Debug, Clone)]
pub struct PluginHost {
    inner: Arc<Inner>,
}

impl PluginHost {
    /// A host of `plugins`, in the order in which they start, whose modules
    /// `store` was opened with. Every plugin is off until
    /// [`PluginHost::start_chosen`].
    pub fn new(
        runtime: Runtime,
        store: Store,
        plugins: Vec<HostedPlugin>,
        clock: Arc<dyn Clock>,
        version: String,
    ) -> Result<Self, HostError> {
        let mut entries: Vec<Entry> = Vec::with_capacity(plugins.len());
        for HostedPlugin { plugin, grants } in plugins {
            let id = plugin.id();
            if !is_valid_plugin_id(id) {
                return Err(HostError::InvalidId { id: id.to_owned() });
            }
            if entries.iter().any(|entry| entry.plugin.id() == id) {
                return Err(HostError::DuplicateId { id: id.to_owned() });
            }
            let stores = plugin
                .store_modules()
                .iter()
                .map(|module| store.module(module))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|source| HostError::ModuleNotRegistered {
                    id: id.to_owned(),
                    source: Box::new(source),
                })?;
            entries.push(Entry {
                plugin,
                grants,
                stores,
            });
        }
        let views = entries
            .iter()
            .map(|entry| PluginView {
                id: entry.plugin.id().to_owned(),
                state: PluginState::Off,
                enabled: false,
                failure: None,
            })
            .collect();
        Ok(Self {
            inner: Arc::new(Inner {
                entries,
                runtime,
                store,
                clock,
                version,
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
    /// update keeps a plugin on that the owner already uses.
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
            let id = entry.plugin.id();
            let enabled = if let Some(enabled) = settings.enabled.get(id) {
                *enabled
            } else {
                let enabled = entry.plugin.default_on(entry.stores.clone()).await;
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
            (false, PluginState::On) => self.stop_entry(entry).await,
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

    /// The work that a plugin has in progress: the tasks of its executor
    /// that have not ended, which turning it off lets go.
    pub async fn work(&self, id: &str) -> Result<PluginWork, PluginsError> {
        self.entry(id).ok_or(PluginsError::UnknownPlugin)?;
        let active = self
            .inner
            .runtime
            .executor_services()
            .active_executor_tasks(id)
            .await
            .map_err(|error| {
                tracing::error!(%error, error.code = error.code(), plugin = id, "could not read the tasks of a plugin");
                PluginsError::StorageFailed
            })?;
        Ok(PluginWork {
            active_tasks: u32::try_from(active.len()).unwrap_or(u32::MAX),
        })
    }

    /// Stops the plugins that are on, the last started first, because Mannie
    /// quits. The owner's choices stay (ADR-0030, decision 7).
    pub async fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        let _transition = self.inner.transitions.lock().await;
        for entry in self.inner.entries.iter().rev() {
            if self.state(entry.plugin.id()) == PluginState::On {
                self.stop_entry(entry).await;
            }
        }
    }

    fn entry(&self, id: &str) -> Option<&Entry> {
        self.inner
            .entries
            .iter()
            .find(|entry| entry.plugin.id() == id)
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

    /// Starts a plugin and hands its contributions to the runtime. A plugin
    /// whose start fails, or whose contributions the runtime refuses, ends
    /// failed with the code, and the rest of Mannie keeps working.
    async fn start_entry(&self, entry: &Entry) {
        let id = entry.plugin.id();
        self.change_view(id, |view| {
            view.state = PluginState::Starting;
            view.failure = None;
        });
        let context = StartContext {
            stores: entry.stores.clone(),
            grants: entry.grants.clone(),
            clock: Arc::clone(&self.inner.clock),
            version: self.inner.version.clone(),
        };
        let failure = match entry.plugin.start(context).await {
            Ok(contributions) => match self.inner.runtime.add_plugin(id, contributions) {
                Ok(()) => None,
                Err(error) => {
                    tracing::error!(%error, plugin = id, "the runtime refused a plugin's contributions");
                    entry.plugin.stop().await;
                    Some(CONTRIBUTIONS_REFUSED.to_owned())
                }
            },
            Err(error) => {
                tracing::warn!(plugin = id, code = %error.code, "a plugin could not start");
                Some(error.code)
            }
        };
        match failure {
            None => {
                tracing::info!(plugin = id, "a plugin started");
                self.change_view(id, |view| view.state = PluginState::On);
            }
            Some(code) => self.change_view(id, |view| {
                view.state = PluginState::Failed;
                view.failure = Some(code);
            }),
        }
    }

    /// Takes a plugin's contributions out of the runtime, then stops it, so
    /// no run that starts meanwhile gets them (ADR-0030, decision 6).
    async fn stop_entry(&self, entry: &Entry) {
        let id = entry.plugin.id();
        self.change_view(id, |view| view.state = PluginState::Stopping);
        self.inner.runtime.remove_plugin(id);
        entry.plugin.stop().await;
        tracing::info!(plugin = id, "a plugin stopped");
        self.change_view(id, |view| view.state = PluginState::Off);
    }
}
