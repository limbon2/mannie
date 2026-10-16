//! The host of the internal plugins (ADR-0030, decisions 6 and 7), with a
//! fake plugin whose executor is the runtime's fake: the default of a new
//! installation and its keeping as the owner's choice, starts at launch,
//! turning a plugin on and off while Mannie runs, a start that fails and is
//! tried again, quitting, and the lists that the host refuses.

#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_plugins::{
    Grants, HostError, HostedPlugin, Plugin, PluginHost, PluginStartError, StartContext,
};
use mannie_runtime::PluginContributions;
use mannie_runtime::testing::{
    FAKE_EXECUTOR_KIND, FAKE_EXECUTOR_TOOL, FakeExecutor, Harness, HarnessOptions, reply,
};
use mannie_store::{ModuleStore, StoreModule};
use mannie_types::chat::StartConversation;
use mannie_types::clock::SystemClock;
use mannie_types::plugins::{PluginState, PluginsError, PluginsSettings};

/// A plugin whose executor is the runtime's fake, and which counts what the
/// host asks of it.
#[derive(Debug)]
struct FakePlugin {
    id: &'static str,
    default_on: bool,
    /// How many of the next starts fail.
    failing_starts: AtomicU32,
    defaults_asked: AtomicU32,
    starts: AtomicU32,
    stops: AtomicU32,
}

impl FakePlugin {
    fn new(default_on: bool) -> Arc<Self> {
        Self::with_id(FAKE_EXECUTOR_KIND, default_on)
    }

    fn with_id(id: &'static str, default_on: bool) -> Arc<Self> {
        Arc::new(Self {
            id,
            default_on,
            failing_starts: AtomicU32::new(0),
            defaults_asked: AtomicU32::new(0),
            starts: AtomicU32::new(0),
            stops: AtomicU32::new(0),
        })
    }

    fn fail_next_starts(&self, count: u32) {
        self.failing_starts.store(count, Ordering::SeqCst);
    }
}

impl Plugin for FakePlugin {
    fn id(&self) -> &'static str {
        self.id
    }

    fn store_modules(&self) -> Vec<StoreModule> {
        Vec::new()
    }

    fn default_on(&self, _stores: Vec<ModuleStore>) -> BoxFuture<'static, bool> {
        self.defaults_asked.fetch_add(1, Ordering::SeqCst);
        futures_util::future::ready(self.default_on).boxed()
    }

    fn start(
        &self,
        _context: StartContext,
    ) -> BoxFuture<'static, Result<PluginContributions, PluginStartError>> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let failing = self
            .failing_starts
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_sub(1)
            })
            .is_ok();
        let started = if failing {
            Err(PluginStartError::new("storage_failed"))
        } else {
            Ok(PluginContributions {
                executors: vec![FakeExecutor::new()],
                ..PluginContributions::default()
            })
        };
        futures_util::future::ready(started).boxed()
    }

    fn stop(&self) -> BoxFuture<'static, ()> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        futures_util::future::ready(()).boxed()
    }
}

async fn harness() -> Harness {
    Harness::with_options(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await
}

fn host(harness: &Harness, plugin: &Arc<FakePlugin>) -> PluginHost {
    let plugin: Arc<dyn Plugin> = Arc::<FakePlugin>::clone(plugin);
    PluginHost::new(
        harness.runtime.clone(),
        harness.store.clone(),
        vec![HostedPlugin {
            plugin,
            grants: Grants::default(),
        }],
        Arc::new(SystemClock),
        "0.3.0".to_owned(),
    )
    .unwrap()
}

/// Whether a run that starts now is offered the fake plugin's tool.
async fn offers_the_tool(harness: &Harness, marker: &str) -> bool {
    harness.model.push_reply(reply("Hello."));
    let started = harness
        .runtime
        .start_conversation(StartConversation {
            text: format!("{marker}: hello"),
            profile_id: None,
            project_id: None,
        })
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    harness.model.requests_for(marker)[0]
        .tools
        .iter()
        .any(|tool| tool.name == FAKE_EXECUTOR_TOOL)
}

#[tokio::test]
async fn a_new_installation_starts_its_plugins_off_and_keeps_that_as_the_choice() {
    let harness = harness().await;
    let plugin = FakePlugin::new(false);
    let host = host(&harness, &plugin);

    host.start_chosen().await;
    let settings: PluginsSettings = harness.store.load_settings().await.unwrap();

    assert_eq!(host.views()[0].state, PluginState::Off);
    assert!(!host.views()[0].enabled);
    assert_eq!(plugin.starts.load(Ordering::SeqCst), 0);
    assert_eq!(settings.enabled.get(FAKE_EXECUTOR_KIND), Some(&false));
    assert!(!offers_the_tool(&harness, "OFF").await);
}

#[tokio::test]
async fn a_plugin_whose_default_is_on_starts_and_the_next_launch_follows_the_choice() {
    let harness = harness().await;
    let plugin = FakePlugin::new(true);
    let first = host(&harness, &plugin);
    first.start_chosen().await;
    let on = offers_the_tool(&harness, "FIRST").await;
    first.shutdown().await;

    // The next launch reads the kept choice and does not ask the default.
    let second = host(&harness, &plugin);
    second.start_chosen().await;

    assert!(on);
    assert_eq!(second.views()[0].state, PluginState::On);
    assert_eq!(plugin.defaults_asked.load(Ordering::SeqCst), 1);
    assert_eq!(plugin.starts.load(Ordering::SeqCst), 2);
    assert!(offers_the_tool(&harness, "SECOND").await);
}

#[tokio::test]
async fn turning_a_plugin_on_and_off_takes_effect_at_once_and_lasts() {
    let harness = harness().await;
    let plugin = FakePlugin::new(false);
    let host = host(&harness, &plugin);
    host.start_chosen().await;
    let mut states = host.subscribe();
    states.borrow_and_update();

    let on = host.set_enabled(FAKE_EXECUTOR_KIND, true).await.unwrap();
    let offered = offers_the_tool(&harness, "ON").await;
    let off = host.set_enabled(FAKE_EXECUTOR_KIND, false).await.unwrap();
    let settings: PluginsSettings = harness.store.load_settings().await.unwrap();

    assert_eq!(on.state, PluginState::On);
    assert!(on.enabled);
    assert!(offered);
    assert!(
        states.has_changed().unwrap(),
        "the Plugins page hears of it"
    );
    assert_eq!(off.state, PluginState::Off);
    assert!(!off.enabled);
    assert_eq!(plugin.stops.load(Ordering::SeqCst), 1);
    assert_eq!(settings.enabled.get(FAKE_EXECUTOR_KIND), Some(&false));
    assert!(!offers_the_tool(&harness, "OFF").await);
    // Turning it off again changes nothing.
    let again = host.set_enabled(FAKE_EXECUTOR_KIND, false).await.unwrap();
    assert_eq!(again.state, PluginState::Off);
    assert_eq!(plugin.stops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_start_that_fails_leaves_the_plugin_failed_and_trying_again_starts_it() {
    let harness = harness().await;
    let plugin = FakePlugin::new(true);
    plugin.fail_next_starts(1);
    let host = host(&harness, &plugin);

    host.start_chosen().await;
    let failed = host.views()[0].clone();
    let retried = host.set_enabled(FAKE_EXECUTOR_KIND, true).await.unwrap();

    assert_eq!(failed.state, PluginState::Failed);
    assert_eq!(failed.failure.as_deref(), Some("storage_failed"));
    assert!(failed.enabled, "the choice stays");
    assert_eq!(retried.state, PluginState::On);
    assert_eq!(retried.failure, None);
    assert!(offers_the_tool(&harness, "RETRIED").await);
}

#[tokio::test]
async fn turning_off_a_failed_plugin_clears_the_failure() {
    let harness = harness().await;
    let plugin = FakePlugin::new(true);
    plugin.fail_next_starts(1);
    let host = host(&harness, &plugin);
    host.start_chosen().await;

    let off = host.set_enabled(FAKE_EXECUTOR_KIND, false).await.unwrap();

    assert_eq!(off.state, PluginState::Off);
    assert_eq!(off.failure, None);
    assert_eq!(
        plugin.stops.load(Ordering::SeqCst),
        0,
        "nothing ran to stop"
    );
}

#[tokio::test]
async fn quitting_stops_the_plugins_that_are_on_and_keeps_the_choices() {
    let harness = harness().await;
    let plugin = FakePlugin::new(true);
    let host = host(&harness, &plugin);
    host.start_chosen().await;

    host.shutdown().await;
    let settings: PluginsSettings = harness.store.load_settings().await.unwrap();
    let refused = host.set_enabled(FAKE_EXECUTOR_KIND, false).await;

    assert_eq!(plugin.stops.load(Ordering::SeqCst), 1);
    assert_eq!(host.views()[0].state, PluginState::Off);
    assert_eq!(settings.enabled.get(FAKE_EXECUTOR_KIND), Some(&true));
    assert_eq!(refused, Err(PluginsError::ShuttingDown));
}

#[tokio::test]
async fn the_work_of_a_plugin_counts_its_tasks_that_have_not_ended() {
    let harness = harness().await;
    let plugin = FakePlugin::new(true);
    let host = host(&harness, &plugin);
    host.start_chosen().await;

    let work = host.work(FAKE_EXECUTOR_KIND).await.unwrap();
    let unknown = host.work("nothing").await;

    assert_eq!(work.active_tasks, 0);
    assert_eq!(unknown, Err(PluginsError::UnknownPlugin));
}

#[tokio::test]
async fn the_host_refuses_unknown_plugins_and_lists_it_cannot_run() {
    let harness = harness().await;
    let plugin = FakePlugin::new(false);
    let host = host(&harness, &plugin);
    let unknown = host.set_enabled("nothing", true).await;
    let invalid: Arc<dyn Plugin> = FakePlugin::with_id("Not_Valid", false);
    let first: Arc<dyn Plugin> = FakePlugin::new(false);
    let second: Arc<dyn Plugin> = FakePlugin::new(false);
    let unregistered: Arc<dyn Plugin> = Arc::new(WithModule);

    let refused = |plugins: Vec<Arc<dyn Plugin>>| {
        PluginHost::new(
            harness.runtime.clone(),
            harness.store.clone(),
            plugins
                .into_iter()
                .map(|plugin| HostedPlugin {
                    plugin,
                    grants: Grants::default(),
                })
                .collect(),
            Arc::new(SystemClock),
            "0.3.0".to_owned(),
        )
        .unwrap_err()
    };

    assert_eq!(unknown, Err(PluginsError::UnknownPlugin));
    assert!(matches!(
        refused(vec![invalid]),
        HostError::InvalidId { .. }
    ));
    assert!(matches!(
        refused(vec![first, second]),
        HostError::DuplicateId { .. }
    ));
    assert!(matches!(
        refused(vec![unregistered]),
        HostError::ModuleNotRegistered { .. }
    ));
}

/// A plugin with a module that the test store was not opened with.
#[derive(Debug)]
struct WithModule;

impl Plugin for WithModule {
    fn id(&self) -> &'static str {
        "with-module"
    }

    fn store_modules(&self) -> Vec<StoreModule> {
        vec![StoreModule {
            name: "with_module",
            migrations: &[],
        }]
    }

    fn default_on(&self, _stores: Vec<ModuleStore>) -> BoxFuture<'static, bool> {
        futures_util::future::ready(false).boxed()
    }

    fn start(
        &self,
        _context: StartContext,
    ) -> BoxFuture<'static, Result<PluginContributions, PluginStartError>> {
        futures_util::future::ready(Ok(PluginContributions::default())).boxed()
    }

    fn stop(&self) -> BoxFuture<'static, ()> {
        futures_util::future::ready(()).boxed()
    }
}
