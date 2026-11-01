//! A plugin's optional features (ADR-0055) on a temporary store: a feature
//! starts on its plugin's running instance only while the owner keeps it on,
//! its choice outlives the plugin's switch, its contributions, calls and
//! services are its own, turning it off refuses its commands with
//! `feature_off` and ends its channels, and turning the plugin off stops its
//! features first.

#![cfg(test)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::{StreamExt, stream};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::{FeatureContext, HostInfo, HostProfile, StartContext};
use mannie_plugin_sdk::contracts::{Clock, SubscriptionEnd, UtcMillis, WorkItem};
use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint, IdRule, PointInfo};
use mannie_plugin_sdk::error::{FEATURE_UNKNOWN, SERVICE_UNAVAILABLE, StartFailure};
use mannie_plugin_sdk::host::{
    BindRefusal, ContributionConsumer, ContributionRefusal, PluginBinding, ServiceProvider,
};
use mannie_plugin_sdk::ids::Contributor;
use mannie_plugin_sdk::ipc::{
    ChannelMessage, ChannelSpec, CommandSpec, Handlers, HostRefusal, Interface, PluginError,
    PluginStream,
};
use mannie_plugin_sdk::plugin::{
    FeatureInstance, FeatureStarted, Instance, Plugin, PluginEntry, Started,
};
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::local_servers::LocalServers;
use mannie_plugins::{CallFailure, HostError, PluginHost, PluginHostDeps, PluginList};
use mannie_store::{Store, StoreOptions};
use mannie_types::clock::ManualClock;
use mannie_types::plugins::{PluginState, PluginsError, PluginsSettings};
use serde::Serialize;
use serde_json::{Value, json};
use tempfile::TempDir;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What happened, in order, across the plugin, its feature, the consumer
/// and the providers.
type Events = Arc<Mutex<Vec<String>>>;

fn log(events: &Events, event: impl Into<String>) {
    lock(events).push(event.into());
}

/// An `Owned` point whose entries are names.
enum Things {}

impl ExtensionPoint for Things {
    const NAME: &'static str = "things";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Owned;
    type Entry = &'static str;

    fn id(entry: &Self::Entry) -> &str {
        entry
    }
}

/// A consumer of `things` that logs each contributor that it adds and
/// removes.
#[derive(Debug)]
struct Consumer {
    events: Events,
}

impl ContributionConsumer for Consumer {
    fn points(&self) -> Vec<PointInfo> {
        vec![PointInfo::of::<Things>()]
    }

    fn check(
        &self,
        _contributor: &Contributor,
        _contributions: &Contributions,
    ) -> Result<(), ContributionRefusal> {
        Ok(())
    }

    fn add(
        &self,
        contributor: &Contributor,
        contributions: Contributions,
    ) -> Result<(), ContributionRefusal> {
        let ids: Vec<&str> = contributions.ids().map(|(_point, id)| id).collect();
        log(&self.events, format!("added {contributor} {ids:?}"));
        Ok(())
    }

    fn remove(&self, contributor: &Contributor) {
        log(&self.events, format!("removed {contributor}"));
    }
}

/// A provider of `local-servers` that binds nothing and logs each release.
#[derive(Debug)]
struct Releasing {
    events: Events,
}

impl ServiceProvider for Releasing {
    fn name(&self) -> &'static str {
        LocalServers::NAME
    }

    fn major(&self) -> u16 {
        LocalServers::MAJOR
    }

    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        Err(BindRefusal::new(
            LocalServers::NAME,
            format!("the test binds nothing to {}", binding.contributor()),
        ))
    }

    fn release(&self, contributor: &Contributor) -> BoxFuture<'static, ()> {
        log(&self.events, format!("released {contributor}"));
        Box::pin(std::future::ready(()))
    }
}

#[derive(Debug, Serialize, specta::Type, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
enum NotesError {
    #[expect(dead_code, reason = "the commands of these tests never fail")]
    #[error("the notes failed")]
    Failed,
}

impl PluginError for NotesError {
    fn code(&self) -> &'static str {
        "notes.failed"
    }
}

enum CountNotes {}

impl CommandSpec for CountNotes {
    const NAME: &'static str = "count_notes";
    const DOCS: &'static str = "Counts the notes.";
    type Input = ();
    type Output = u32;
    type Error = NotesError;
}

enum CountHistory {}

impl CommandSpec for CountHistory {
    const NAME: &'static str = "count_history";
    const DOCS: &'static str = "Counts the history.";
    type Input = ();
    type Output = u32;
    type Error = NotesError;
}

enum HangHistory {}

impl CommandSpec for HangHistory {
    const NAME: &'static str = "hang_history";
    const DOCS: &'static str = "Never answers.";
    type Input = ();
    type Output = ();
    type Error = NotesError;
}

enum FollowHistory {}

impl ChannelSpec for FollowHistory {
    const NAME: &'static str = "follow_history";
    const DOCS: &'static str = "Follows the history.";
    type Input = ();
    type Item = u32;
    type Error = NotesError;
}

/// What the plugin and its feature saw and did.
#[derive(Debug, Default)]
struct Record {
    events: Events,
    fail_feature: AtomicBool,
    feature_work: Mutex<Vec<WorkItem>>,
}

/// The plugin `notes`, with the feature `history`.
#[derive(Debug)]
struct Notes {
    record: Arc<Record>,
}

impl Plugin for Notes {
    fn interface(&self) -> Interface {
        Interface::new()
            .command::<CountNotes>()
            .feature("history", |feature| {
                feature
                    .command::<CountHistory>()
                    .command::<HangHistory>()
                    .channel::<FollowHistory>()
            })
    }

    fn start(&self, _context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        let record = Arc::clone(&self.record);
        Box::pin(async move {
            log(&record.events, "notes started");
            let handlers = Handlers::new()
                .command::<CountNotes, _, _>(|()| async { Ok::<u32, NotesError>(3) });
            Ok(Started::new(NotesInstance {
                record: Arc::clone(&record),
            })
            .with_contributions(Contributions::new().add::<Things>("notes.things"))
            .with_handlers(handlers))
        })
    }
}

#[derive(Debug)]
struct NotesInstance {
    record: Arc<Record>,
}

impl Instance for NotesInstance {
    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        log(&self.record.events, "notes stopped");
        Box::pin(std::future::ready(()))
    }

    fn start_feature(
        &self,
        context: FeatureContext,
    ) -> BoxFuture<'static, Result<FeatureStarted, StartFailure>> {
        let record = Arc::clone(&self.record);
        Box::pin(async move {
            if context.feature().as_str() != "history" {
                return Err(StartFailure::new(FEATURE_UNKNOWN));
            }
            if record.fail_feature.load(Ordering::SeqCst) {
                return Err(StartFailure::new("history_failed"));
            }
            log(&record.events, format!("{} started", context.feature()));
            let handlers = Handlers::new()
                .command::<CountHistory, _, _>(|()| async { Ok::<u32, NotesError>(5) })
                .command::<HangHistory, _, _>(|()| async {
                    std::future::pending::<()>().await;
                    Ok::<(), NotesError>(())
                })
                .channel::<FollowHistory, _, _>(|()| async {
                    let items: PluginStream<u32> =
                        stream::iter([1]).chain(stream::pending()).boxed();
                    Ok::<PluginStream<u32>, NotesError>(items)
                });
            Ok(FeatureStarted::new(HistoryInstance {
                record: Arc::clone(&record),
            })
            .with_contributions(Contributions::new().add::<Things>("notes.history"))
            .with_handlers(handlers))
        })
    }
}

#[derive(Debug)]
struct HistoryInstance {
    record: Arc<Record>,
}

impl FeatureInstance for HistoryInstance {
    fn work(&self) -> BoxFuture<'static, Vec<WorkItem>> {
        Box::pin(std::future::ready(lock(&self.record.feature_work).clone()))
    }

    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        log(&self.record.events, "history stopped");
        Box::pin(std::future::ready(()))
    }
}

const MANIFEST: &str = "# The test's plugin.\nid = \"notes\"\nsdk = 1\norder = 10\n\n[rust]\nentry = \"mannie-notes\"\n\n[features.history]\n";

/// The manifest with the feature declaring a service that no provider binds.
const MANIFEST_WITH_SERVICE: &str = "# The test's plugin.\nid = \"notes\"\nsdk = 1\norder = 10\n\n[rust]\nentry = \"mannie-notes\"\n\n[features.history.services]\nlocal-servers = 1\n";

/// A host of `notes` on a temporary store, whose manifest is `manifest`.
struct Fixture {
    _directory: TempDir,
    store: Store,
    record: Arc<Record>,
    host: PluginHost,
}

impl Fixture {
    async fn new(manifest: &'static str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let record = Arc::new(Record::default());
        let plugin: Arc<dyn Plugin> = Arc::new(Notes {
            record: Arc::clone(&record),
        });
        let list = PluginList::new(vec![PluginEntry::new(manifest, plugin)]).unwrap();
        let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(UtcMillis::from_millis(1_234)));
        let options = StoreOptions::new(directory.path().join("backups"), Arc::clone(&clock))
            .with_plugin_modules(list.store_modules());
        let store = Store::open(&directory.path().join("mannie.sqlite"), &options).unwrap();
        let mut providers = mannie_store::service_providers(&store);
        providers.push(Arc::new(Releasing {
            events: Arc::clone(&record.events),
        }));
        let consumer: Arc<dyn ContributionConsumer> = Arc::new(Consumer {
            events: Arc::clone(&record.events),
        });
        let host = PluginHost::new(PluginHostDeps {
            store: store.clone(),
            plugins: list,
            providers,
            consumers: vec![consumer],
            clock,
            host: HostInfo::new("0.3.0", HostProfile::Validation),
        })
        .unwrap();
        host.start_chosen().await;
        Self {
            _directory: directory,
            store,
            record,
            host,
        }
    }

    /// A host whose plugin is on and whose feature is off.
    async fn on(manifest: &'static str) -> Self {
        let fixture = Self::new(manifest).await;
        fixture.host.set_enabled("notes", true).await.unwrap();
        fixture
    }

    /// A second host of `notes` on the same store, as the next launch of
    /// Mannie makes, with a plugin that records into the same log.
    async fn relaunch(&self, manifest: &'static str) -> PluginHost {
        let plugin: Arc<dyn Plugin> = Arc::new(Notes {
            record: Arc::clone(&self.record),
        });
        let list = PluginList::new(vec![PluginEntry::new(manifest, plugin)]).unwrap();
        let consumer: Arc<dyn ContributionConsumer> = Arc::new(Consumer {
            events: Arc::clone(&self.record.events),
        });
        let host = PluginHost::new(PluginHostDeps {
            store: self.store.clone(),
            plugins: list,
            providers: mannie_store::service_providers(&self.store),
            consumers: vec![consumer],
            clock: Arc::new(ManualClock::new(UtcMillis::from_millis(5_678))),
            host: HostInfo::new("0.3.0", HostProfile::Validation),
        })
        .unwrap();
        host.start_chosen().await;
        host
    }

    fn events(&self) -> Vec<String> {
        lock(&self.record.events).clone()
    }

    fn clear(&self) {
        lock(&self.record.events).clear();
    }

    fn feature_state(&self) -> PluginState {
        self.host.views()[0].features[0].state
    }

    async fn turn_feature(&self, enabled: bool) {
        self.host
            .set_feature_enabled("notes", "history", enabled)
            .await
            .unwrap();
    }

    async fn invoke(&self, command: &str) -> Result<Value, CallFailure> {
        self.host.invoke("notes", command, Value::Null).await
    }
}

fn feature_off() -> CallFailure {
    CallFailure::Host(HostRefusal::feature_off("notes", "history"))
}

fn plugin_off() -> CallFailure {
    CallFailure::Host(HostRefusal::plugin_off("notes"))
}

#[tokio::test]
async fn a_feature_starts_off_and_its_view_lists_what_it_uses() {
    let fixture = Fixture::on(MANIFEST).await;

    let view = &fixture.host.views()[0];

    assert_eq!(view.state, PluginState::On);
    assert_eq!(view.uses.commands, ["count_notes"]);
    assert_eq!(view.uses.channels, Vec::<String>::new());
    let feature = &view.features[0];
    assert_eq!(feature.id, "history");
    assert_eq!(feature.state, PluginState::Off);
    assert!(!feature.enabled);
    assert_eq!(feature.uses.commands, ["count_history", "hang_history"]);
    assert_eq!(feature.uses.channels, ["follow_history"]);
    assert_eq!(
        fixture.events(),
        ["notes started", r#"added notes ["notes.things"]"#]
    );
}

#[tokio::test]
async fn turning_a_feature_on_starts_it_on_the_running_plugin_with_contributions_of_its_own() {
    let fixture = Fixture::on(MANIFEST).await;
    fixture.clear();

    fixture.turn_feature(true).await;
    let settings: PluginsSettings = fixture.store.load_settings().await.unwrap();

    assert_eq!(fixture.feature_state(), PluginState::On);
    assert!(fixture.host.views()[0].features[0].enabled);
    assert!(settings.feature_enabled("notes", "history"));
    assert_eq!(
        fixture.events(),
        [
            "history started",
            r#"added notes/history ["notes.history"]"#
        ]
    );
    assert_eq!(fixture.invoke("count_history").await, Ok(json!(5)));
    assert_eq!(fixture.invoke("count_notes").await, Ok(json!(3)));
}

#[tokio::test]
async fn turning_a_feature_off_drops_its_part_alone_and_refuses_its_commands() {
    let fixture = Fixture::on(MANIFEST).await;
    fixture.turn_feature(true).await;
    fixture.clear();

    fixture.turn_feature(false).await;

    assert_eq!(fixture.feature_state(), PluginState::Off);
    assert_eq!(fixture.host.views()[0].state, PluginState::On);
    assert_eq!(
        fixture.events(),
        [
            "removed notes/history",
            "history stopped",
            "released notes/history"
        ]
    );
    assert_eq!(fixture.invoke("count_history").await, Err(feature_off()));
    assert_eq!(fixture.invoke("count_notes").await, Ok(json!(3)));
    assert_eq!(
        fixture.invoke("unknown").await,
        Err(CallFailure::Host(HostRefusal::UnknownCommand))
    );
}

#[tokio::test]
async fn turning_a_feature_off_ends_its_channels_with_feature_off() {
    let fixture = Fixture::on(MANIFEST).await;
    fixture.turn_feature(true).await;
    let mut channel = fixture
        .host
        .open_channel("notes", "follow_history", Value::Null)
        .await
        .unwrap();

    let first = channel.next().await;
    fixture.turn_feature(false).await;
    let last = channel.next().await;

    assert_eq!(first, Some(ChannelMessage::Item { item: json!(1) }));
    assert_eq!(
        last,
        Some(ChannelMessage::Ended {
            reason: SubscriptionEnd::FeatureOff
        })
    );
    assert!(channel.next().await.is_none());
}

#[tokio::test]
async fn turning_the_plugin_off_stops_its_features_first_and_keeps_their_choices() {
    let fixture = Fixture::on(MANIFEST).await;
    fixture.turn_feature(true).await;
    fixture.clear();

    fixture.host.set_enabled("notes", false).await.unwrap();
    let settings: PluginsSettings = fixture.store.load_settings().await.unwrap();

    assert_eq!(
        fixture.events(),
        [
            "removed notes/history",
            "history stopped",
            "released notes/history",
            "removed notes",
            "notes stopped",
            "released notes",
        ]
    );
    let feature = &fixture.host.views()[0].features[0];
    assert_eq!(feature.state, PluginState::Off);
    assert!(feature.enabled);
    assert!(settings.feature_enabled("notes", "history"));
    assert_eq!(fixture.invoke("count_history").await, Err(plugin_off()));
}

#[tokio::test]
async fn a_feature_turned_on_while_its_plugin_is_off_keeps_the_choice_and_starts_with_the_plugin() {
    let fixture = Fixture::new(MANIFEST).await;

    fixture.turn_feature(true).await;
    let while_off = fixture.feature_state();
    fixture.host.set_enabled("notes", true).await.unwrap();

    assert_eq!(while_off, PluginState::Off);
    assert_eq!(fixture.feature_state(), PluginState::On);
    assert_eq!(
        fixture.events(),
        [
            "notes started",
            r#"added notes ["notes.things"]"#,
            "history started",
            r#"added notes/history ["notes.history"]"#,
        ]
    );
}

#[tokio::test]
async fn the_next_launch_starts_the_features_that_the_owner_left_on() {
    let fixture = Fixture::on(MANIFEST).await;
    fixture.turn_feature(true).await;
    fixture.host.shutdown().await;
    fixture.clear();

    let next = fixture.relaunch(MANIFEST).await;

    let view = &next.views()[0];
    assert_eq!(view.state, PluginState::On);
    assert_eq!(view.features[0].state, PluginState::On);
    assert!(view.features[0].enabled);
    assert_eq!(
        fixture.events(),
        [
            "notes started",
            r#"added notes ["notes.things"]"#,
            "history started",
            r#"added notes/history ["notes.history"]"#,
        ]
    );
}

#[tokio::test]
async fn the_next_launch_leaves_off_a_feature_that_the_owner_turned_off() {
    let fixture = Fixture::on(MANIFEST).await;
    fixture.turn_feature(true).await;
    fixture.turn_feature(false).await;
    fixture.host.shutdown().await;

    let next = fixture.relaunch(MANIFEST).await;
    let settings: PluginsSettings = fixture.store.load_settings().await.unwrap();

    assert_eq!(next.views()[0].features[0].state, PluginState::Off);
    assert!(!next.views()[0].features[0].enabled);
    assert!(!settings.feature_enabled("notes", "history"));
}

#[tokio::test]
async fn a_feature_that_fails_to_start_leaves_its_plugin_on_and_trying_again_starts_it() {
    let fixture = Fixture::on(MANIFEST).await;
    fixture.record.fail_feature.store(true, Ordering::SeqCst);

    fixture.turn_feature(true).await;
    let failed = fixture.host.views()[0].features[0].clone();
    fixture.record.fail_feature.store(false, Ordering::SeqCst);
    fixture.turn_feature(true).await;

    assert_eq!(failed.state, PluginState::Failed);
    assert_eq!(failed.failure.as_deref(), Some("history_failed"));
    assert_eq!(fixture.host.views()[0].state, PluginState::On);
    assert_eq!(fixture.feature_state(), PluginState::On);
    assert_eq!(fixture.host.views()[0].features[0].failure, None);
    assert!(
        fixture
            .events()
            .contains(&"released notes/history".to_owned())
    );
}

#[tokio::test]
async fn turning_off_a_failed_feature_clears_its_failure() {
    let fixture = Fixture::on(MANIFEST).await;
    fixture.record.fail_feature.store(true, Ordering::SeqCst);
    fixture.turn_feature(true).await;

    fixture.turn_feature(false).await;

    let feature = &fixture.host.views()[0].features[0];
    assert_eq!(feature.state, PluginState::Off);
    assert_eq!(feature.failure, None);
    assert!(!feature.enabled);
}

#[tokio::test]
async fn a_features_service_that_no_provider_binds_fails_its_start_and_its_plugin_stays_on() {
    let fixture = Fixture::on(MANIFEST_WITH_SERVICE).await;

    fixture.turn_feature(true).await;

    let view = &fixture.host.views()[0];
    assert_eq!(view.state, PluginState::On);
    assert_eq!(view.features[0].state, PluginState::Failed);
    assert_eq!(
        view.features[0].failure.as_deref(),
        Some(SERVICE_UNAVAILABLE)
    );
    assert_eq!(view.features[0].uses.services[0].name, "local-servers");
    assert_eq!(view.uses.services, Vec::new());
    assert!(!fixture.events().contains(&"history started".to_owned()));
}

#[tokio::test]
async fn the_work_of_a_plugin_includes_its_features_and_a_feature_reports_its_own() {
    let fixture = Fixture::on(MANIFEST).await;
    lock(&fixture.record.feature_work).push(WorkItem::new("open_databases", 2));

    let before = fixture.host.feature_work("notes", "history").await.unwrap();
    fixture.turn_feature(true).await;
    let feature = fixture.host.feature_work("notes", "history").await.unwrap();
    let plugin = fixture.host.work("notes").await.unwrap();

    assert_eq!(before.items, Vec::new());
    assert_eq!(feature.items, [WorkItem::new("open_databases", 2)]);
    assert_eq!(plugin.items, [WorkItem::new("open_databases", 2)]);
}

#[tokio::test]
async fn an_unknown_feature_is_refused() {
    let fixture = Fixture::on(MANIFEST).await;

    let unknown = fixture
        .host
        .set_feature_enabled("notes", "other", true)
        .await;
    let work = fixture.host.feature_work("notes", "other").await;

    assert_eq!(unknown, Err(PluginsError::UnknownFeature));
    assert_eq!(work, Err(PluginsError::UnknownFeature));
}

#[test]
fn an_interface_that_names_a_feature_outside_the_manifest_refuses_the_list() {
    let plugin: Arc<dyn Plugin> = Arc::new(Notes {
        record: Arc::new(Record::default()),
    });
    let without_feature = "# The test's plugin.\nid = \"notes\"\nsdk = 1\norder = 10\n\n[rust]\nentry = \"mannie-notes\"\n";

    let refused = PluginList::new(vec![PluginEntry::new(without_feature, plugin)]).unwrap_err();

    assert!(
        matches!(
            &refused,
            HostError::FeatureNotInManifest { id, feature } if id == "notes" && feature == "history"
        ),
        "{refused:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_feature_call_that_turning_the_feature_off_aborted_answers_feature_off() {
    let fixture = Fixture::on(MANIFEST).await;
    fixture.turn_feature(true).await;
    let host = fixture.host.clone();
    let call = tokio::spawn(async move { host.invoke("notes", "hang_history", Value::Null).await });
    tokio::task::yield_now().await;

    fixture.turn_feature(false).await;

    assert_eq!(call.await.unwrap(), Err(feature_off()));
    assert_eq!(fixture.feature_state(), PluginState::Off);
}

#[tokio::test(start_paused = true)]
async fn a_feature_call_that_turning_the_plugin_off_aborted_answers_plugin_off() {
    let fixture = Fixture::on(MANIFEST).await;
    fixture.turn_feature(true).await;
    let host = fixture.host.clone();
    let call = tokio::spawn(async move { host.invoke("notes", "hang_history", Value::Null).await });
    tokio::task::yield_now().await;

    fixture.host.set_enabled("notes", false).await.unwrap();

    assert_eq!(call.await.unwrap(), Err(plugin_off()));
}
