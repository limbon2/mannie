//! The plugin host (ADR-0030, decisions 6 and 7; ADR-0034, decisions 4 to 6)
//! with fake plugins, fake consumers and the credential store in memory, on
//! a temporary store: the default of a new installation and its keeping as
//! the owner's choice, starts at launch in the manifests' order, turning a
//! plugin on and off, a start that fails and is tried again, the host's
//! start codes, the services that each start gets, contributions that the
//! core refuses, a plugin's work, quitting, and the lists and parts that the
//! host refuses.

#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::{DefaultContext, HostInfo, HostProfile, StartContext};
use mannie_plugin_sdk::contracts::{Clock, Secret, UtcMillis, WorkItem};
use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint, IdRule, PointInfo};
use mannie_plugin_sdk::error::{
    CONTRIBUTIONS_REFUSED, GRANT_MISSING, SDK_INCOMPATIBLE, SERVICE_UNAVAILABLE, StartFailure,
};
use mannie_plugin_sdk::host::{
    BindRefusal, ContributionConsumer, ContributionRefusal, ID_TAKEN, PluginBinding,
    ServiceProvider,
};
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::manifest::ManifestError;
use mannie_plugin_sdk::plugin::{Instance, Plugin, PluginEntry, Started};
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::secrets::Secrets;
use mannie_plugin_sdk::services::store::{
    Migration, Statement, Store as StoreService, StoreModule,
};
use mannie_plugin_sdk::testing::FakeSecrets;
use mannie_plugins::{HostError, PluginHost, PluginHostDeps, PluginList};
use mannie_store::{Store, StoreOptions};
use mannie_types::clock::ManualClock;
use mannie_types::plugins::{
    PluginState, PluginUses, PluginWork, PluginsError, PluginsSettings, ServiceUse,
};
use tempfile::TempDir;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the plugins, their instances and the consumers did, in order.
type Events = Arc<Mutex<Vec<String>>>;

fn log_event(events: &Events, event: String) {
    lock(events).push(event);
}

/// A point that the fixture's consumer takes.
enum Things {}

impl ExtensionPoint for Things {
    const NAME: &'static str = "things";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Global;
    type Entry = &'static str;

    fn id(entry: &Self::Entry) -> &str {
        entry
    }
}

/// A point that a second consumer takes in some tests.
enum Others {}

impl ExtensionPoint for Others {
    const NAME: &'static str = "others";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Global;
    type Entry = &'static str;

    fn id(entry: &Self::Entry) -> &str {
        entry
    }
}

/// A point of a later SDK, which no consumer takes.
enum Unclaimed {}

impl ExtensionPoint for Unclaimed {
    const NAME: &'static str = "unclaimed";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Global;
    type Entry = &'static str;

    fn id(entry: &Self::Entry) -> &str {
        entry
    }
}

fn things() -> Contributions {
    Contributions::new().add::<Things>("thing")
}

/// A consumer of one point that records what it takes and drops.
#[derive(Debug)]
struct FakeConsumer {
    point: PointInfo,
    refuses_adds: bool,
    events: Events,
    /// How many entries each plugin has in the consumer.
    held: Mutex<BTreeMap<String, usize>>,
}

impl FakeConsumer {
    fn new(point: PointInfo, events: &Events) -> Arc<Self> {
        Arc::new(Self {
            point,
            refuses_adds: false,
            events: Arc::clone(events),
            held: Mutex::default(),
        })
    }

    /// A consumer whose add refuses, as when another plugin took an ID
    /// after the checks.
    fn refusing(point: PointInfo, events: &Events) -> Arc<Self> {
        Arc::new(Self {
            point,
            refuses_adds: true,
            events: Arc::clone(events),
            held: Mutex::default(),
        })
    }

    fn held(&self, plugin: &str) -> usize {
        lock(&self.held).get(plugin).copied().unwrap_or(0)
    }
}

impl ContributionConsumer for FakeConsumer {
    fn points(&self) -> Vec<PointInfo> {
        vec![self.point]
    }

    fn check(
        &self,
        _plugin: &PluginId,
        _contributions: &Contributions,
    ) -> Result<(), ContributionRefusal> {
        Ok(())
    }

    fn add(
        &self,
        plugin: &PluginId,
        contributions: Contributions,
    ) -> Result<(), ContributionRefusal> {
        if self.refuses_adds {
            return Err(ContributionRefusal::new(
                self.point.name,
                ID_TAKEN,
                "another plugin took the ID",
            ));
        }
        log_event(&self.events, format!("{} added {plugin}", self.point.name));
        lock(&self.held).insert(plugin.to_string(), contributions.ids().count());
        Ok(())
    }

    fn remove(&self, plugin: &PluginId) {
        if lock(&self.held).remove(plugin.as_str()).is_some() {
            log_event(
                &self.events,
                format!("{} removed {plugin}", self.point.name),
            );
        }
    }
}

const COUNT_NOTES: &str = "SELECT count(*) FROM notes_items";

const NOTES_MIGRATIONS: &[Migration] = &[Migration::new(
    1,
    "create the notes",
    "CREATE TABLE notes_items (id TEXT PRIMARY KEY) STRICT;",
)];

/// The module of the plugin `notes`.
const NOTES_MODULE: StoreModule = StoreModule::new("notes", NOTES_MIGRATIONS, &[COUNT_NOTES]);

/// What a start of a test plugin saw.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    plugin: String,
    host: HostInfo,
    now: UtcMillis,
}

/// What a test plugin and its instances saw and did.
#[derive(Debug, Default)]
struct Record {
    events: Events,
    failing_starts: AtomicU32,
    modules_asked: AtomicU32,
    /// For each default: the rows that it read through the store, or `None`
    /// when it got no store.
    defaults: Mutex<Vec<Option<i64>>>,
    starts: AtomicU32,
    stops: AtomicU32,
    seen: Mutex<Vec<Seen>>,
    /// What the instances report as work.
    work: Mutex<Vec<WorkItem>>,
}

/// A plugin that asks for the services that `asks` names at each start and
/// contributes what `contributions` builds.
#[derive(Debug)]
struct TestPlugin {
    name: &'static str,
    default_on: bool,
    module: Option<StoreModule>,
    asks: &'static [&'static str],
    contributions: fn() -> Contributions,
    record: Arc<Record>,
}

impl TestPlugin {
    fn new(name: &'static str, events: &Events) -> Self {
        Self {
            name,
            default_on: false,
            module: None,
            asks: &[],
            contributions: things,
            record: Arc::new(Record {
                events: Arc::clone(events),
                ..Record::default()
            }),
        }
    }

    fn on_by_default(mut self) -> Self {
        self.default_on = true;
        self
    }

    fn with_module(mut self, module: StoreModule) -> Self {
        self.module = Some(module);
        self
    }

    fn asking(mut self, asks: &'static [&'static str]) -> Self {
        self.asks = asks;
        self
    }

    fn contributing(mut self, contributions: fn() -> Contributions) -> Self {
        self.contributions = contributions;
        self
    }

    fn fail_next_starts(&self, count: u32) {
        self.record.failing_starts.store(count, Ordering::SeqCst);
    }

    fn starts(&self) -> u32 {
        self.record.starts.load(Ordering::SeqCst)
    }

    fn stops(&self) -> u32 {
        self.record.stops.load(Ordering::SeqCst)
    }

    fn defaults(&self) -> Vec<Option<i64>> {
        lock(&self.record.defaults).clone()
    }
}

impl Plugin for TestPlugin {
    fn store_module(&self) -> Option<StoreModule> {
        self.record.modules_asked.fetch_add(1, Ordering::SeqCst);
        self.module
    }

    fn default_on(&self, context: DefaultContext) -> BoxFuture<'static, bool> {
        let record = Arc::clone(&self.record);
        let default_on = self.default_on;
        Box::pin(async move {
            let rows = match context.store() {
                Some(store) => {
                    let rows = store.query(Statement::new(COUNT_NOTES)).await.unwrap();
                    Some(rows[0].get::<i64>(0).unwrap())
                }
                None => None,
            };
            lock(&record.defaults).push(rows);
            default_on || rows.is_some_and(|rows| rows > 0)
        })
    }

    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        let record = Arc::clone(&self.record);
        let name = self.name;
        let asks = self.asks;
        let contributions = self.contributions;
        Box::pin(async move {
            record.starts.fetch_add(1, Ordering::SeqCst);
            let failing = record
                .failing_starts
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                    count.checked_sub(1)
                })
                .is_ok();
            if failing {
                return Err(StartFailure::new("storage_failed"));
            }
            for service in asks {
                match *service {
                    "secrets" => {
                        let secrets = context.service::<Secrets>()?;
                        let secret = Secret::new(format!("the token of {name}"));
                        secrets.save("token", secret).await.unwrap();
                    }
                    "store" => drop(context.service::<StoreService>()?),
                    other => panic!("no test asks for {other}"),
                }
            }
            lock(&record.seen).push(Seen {
                plugin: context.plugin().to_string(),
                host: context.host().clone(),
                now: context.clock().now(),
            });
            log_event(&record.events, format!("{name} started"));
            let instance = TestInstance {
                name,
                record: Arc::clone(&record),
            };
            Ok(Started::new(instance).with_contributions(contributions()))
        })
    }
}

#[derive(Debug)]
struct TestInstance {
    name: &'static str,
    record: Arc<Record>,
}

impl Instance for TestInstance {
    fn work(&self) -> BoxFuture<'static, Vec<WorkItem>> {
        Box::pin(std::future::ready(lock(&self.record.work).clone()))
    }

    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        self.record.stops.fetch_add(1, Ordering::SeqCst);
        log_event(&self.record.events, format!("{} stopped", self.name));
        Box::pin(std::future::ready(()))
    }
}

/// A provider of `secrets` at another major, or one that refuses to bind.
#[derive(Debug)]
struct OddSecrets {
    major: u16,
    refuses: bool,
}

impl ServiceProvider for OddSecrets {
    fn name(&self) -> &'static str {
        Secrets::NAME
    }

    fn major(&self) -> u16 {
        self.major
    }

    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        if self.refuses {
            return Err(BindRefusal::new(
                Secrets::NAME,
                format!("the test refuses {}", binding.id),
            ));
        }
        FakeSecrets::new().bind(binding)
    }
}

/// A manifest of `id` at `order`, with `rest` after its `[rust]` table.
fn manifest(id: &str, order: u32, rest: &str) -> &'static str {
    let text = format!(
        "# The test's plugin {id}.\nid = \"{id}\"\nsdk = 1\norder = {order}\n\n[rust]\nentry = \"mannie-{id}\"\n{rest}"
    );
    Box::leak(text.into_boxed_str())
}

const DECLARES_SECRETS: &str = "\n[services]\nsecrets = 1\n\n[secrets]\nentries = [\"token\"]\n";
const DECLARES_STORE: &str = "\n[services]\nstore = 1\n";

fn entry(plugin: &Arc<TestPlugin>, order: u32, rest: &str) -> PluginEntry {
    let hosted: Arc<dyn Plugin> = Arc::<TestPlugin>::clone(plugin);
    PluginEntry::new(manifest(plugin.name, order, rest), hosted)
}

/// A temporary store, the credential store in memory, a clock that stands
/// still and a consumer of `things`.
struct Fixture {
    directory: TempDir,
    clock: Arc<dyn Clock>,
    secrets: FakeSecrets,
    events: Events,
    things: Arc<FakeConsumer>,
}

impl Fixture {
    fn new() -> Self {
        let events = Events::default();
        Self {
            directory: tempfile::tempdir().unwrap(),
            clock: Arc::new(ManualClock::new(UtcMillis::from_millis(1_234))),
            secrets: FakeSecrets::new(),
            things: FakeConsumer::new(PointInfo::of::<Things>(), &events),
            events,
        }
    }

    fn plugin(&self, name: &'static str) -> TestPlugin {
        TestPlugin::new(name, &self.events)
    }

    fn events(&self) -> Vec<String> {
        lock(&self.events).clone()
    }

    /// The store, opened with the list's modules.
    fn store(&self, list: &PluginList) -> Store {
        let options = StoreOptions::new(
            self.directory.path().join("backups"),
            Arc::clone(&self.clock),
        )
        .with_plugin_modules(list.store_modules());
        Store::open(&self.directory.path().join("mannie.sqlite"), &options).unwrap()
    }

    /// The providers of the desktop app, with the credential store in
    /// memory.
    fn providers(&self, store: &Store) -> Vec<Arc<dyn ServiceProvider>> {
        let mut providers: Vec<Arc<dyn ServiceProvider>> = vec![Arc::new(self.secrets.clone())];
        providers.extend(mannie_store::service_providers(store));
        providers
    }

    fn deps(&self, store: &Store, list: PluginList) -> PluginHostDeps {
        let things: Arc<dyn ContributionConsumer> = Arc::<FakeConsumer>::clone(&self.things);
        PluginHostDeps {
            store: store.clone(),
            plugins: list,
            providers: self.providers(store),
            consumers: vec![things],
            clock: Arc::clone(&self.clock),
            host: HostInfo::new("0.3.0", HostProfile::Validation),
        }
    }

    /// The store and a host of `entries`.
    fn start(&self, entries: Vec<PluginEntry>) -> (Store, PluginHost) {
        let list = PluginList::new(entries).unwrap();
        let store = self.store(&list);
        let host = PluginHost::new(self.deps(&store, list)).unwrap();
        (store, host)
    }
}

#[tokio::test]
async fn a_new_installation_starts_its_plugins_off_and_keeps_that_as_the_choice() {
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes"));
    let (store, host) = fixture.start(vec![entry(&plugin, 10, "")]);

    host.start_chosen().await;
    let settings: PluginsSettings = store.load_settings().await.unwrap();

    assert_eq!(host.views()[0].state, PluginState::Off);
    assert!(!host.views()[0].enabled);
    assert_eq!(plugin.starts(), 0);
    assert_eq!(settings.enabled.get("notes"), Some(&false));
    assert_eq!(fixture.things.held("notes"), 0);
}

#[tokio::test]
async fn a_plugin_whose_default_is_on_starts_and_the_next_launch_follows_the_choice() {
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes").on_by_default());
    let list = PluginList::new(vec![entry(&plugin, 10, "")]).unwrap();
    let store = fixture.store(&list);
    let first = PluginHost::new(fixture.deps(&store, list.clone())).unwrap();
    first.start_chosen().await;
    let held = fixture.things.held("notes");
    first.shutdown().await;

    // The next launch reads the kept choice and does not ask the default.
    let second = PluginHost::new(fixture.deps(&store, list)).unwrap();
    second.start_chosen().await;

    assert_eq!(held, 1);
    assert_eq!(second.views()[0].state, PluginState::On);
    assert_eq!(plugin.defaults().len(), 1);
    assert_eq!(plugin.starts(), 2);
    assert_eq!(fixture.things.held("notes"), 1);
}

#[tokio::test]
async fn turning_a_plugin_on_and_off_takes_effect_at_once_and_lasts() {
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes"));
    let (store, host) = fixture.start(vec![entry(&plugin, 10, "")]);
    host.start_chosen().await;
    let mut states = host.subscribe();
    states.borrow_and_update();

    let on = host.set_enabled("notes", true).await.unwrap();
    let held = fixture.things.held("notes");
    let off = host.set_enabled("notes", false).await.unwrap();
    let settings: PluginsSettings = store.load_settings().await.unwrap();

    assert_eq!(on.state, PluginState::On);
    assert!(on.enabled);
    assert_eq!(held, 1);
    assert!(
        states.has_changed().unwrap(),
        "the Plugins page hears of it"
    );
    assert_eq!(off.state, PluginState::Off);
    assert!(!off.enabled);
    assert_eq!(plugin.stops(), 1);
    assert_eq!(settings.enabled.get("notes"), Some(&false));
    assert_eq!(fixture.things.held("notes"), 0);
    // Turning it off again changes nothing.
    let again = host.set_enabled("notes", false).await.unwrap();
    assert_eq!(again.state, PluginState::Off);
    assert_eq!(plugin.stops(), 1);
}

#[tokio::test]
async fn a_start_that_fails_leaves_the_plugin_failed_and_trying_again_starts_it() {
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes").on_by_default());
    plugin.fail_next_starts(1);
    let (_store, host) = fixture.start(vec![entry(&plugin, 10, "")]);

    host.start_chosen().await;
    let failed = host.views()[0].clone();
    let retried = host.set_enabled("notes", true).await.unwrap();

    assert_eq!(failed.state, PluginState::Failed);
    assert_eq!(failed.failure.as_deref(), Some("storage_failed"));
    assert!(failed.enabled, "the choice stays");
    assert_eq!(retried.state, PluginState::On);
    assert_eq!(retried.failure, None);
    assert_eq!(fixture.things.held("notes"), 1);
}

#[tokio::test]
async fn turning_off_a_failed_plugin_clears_the_failure() {
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes").on_by_default());
    plugin.fail_next_starts(1);
    let (_store, host) = fixture.start(vec![entry(&plugin, 10, "")]);
    host.start_chosen().await;

    let off = host.set_enabled("notes", false).await.unwrap();

    assert_eq!(off.state, PluginState::Off);
    assert_eq!(off.failure, None);
    assert_eq!(plugin.stops(), 0, "nothing ran to stop");
}

#[tokio::test]
async fn quitting_stops_the_plugins_that_are_on_the_last_started_first_and_keeps_the_choices() {
    let fixture = Fixture::new();
    let notes = Arc::new(fixture.plugin("notes").on_by_default());
    let journal = Arc::new(fixture.plugin("journal").on_by_default());
    let (store, host) = fixture.start(vec![entry(&notes, 10, ""), entry(&journal, 20, "")]);
    host.start_chosen().await;

    host.shutdown().await;
    let settings: PluginsSettings = store.load_settings().await.unwrap();
    let refused = host.set_enabled("notes", false).await;
    let stopped: Vec<String> = fixture
        .events()
        .into_iter()
        .filter(|event| event.ends_with("stopped"))
        .collect();

    assert_eq!(stopped, ["journal stopped", "notes stopped"]);
    assert!(
        host.views()
            .iter()
            .all(|view| view.state == PluginState::Off)
    );
    assert_eq!(settings.enabled.get("notes"), Some(&true));
    assert_eq!(settings.enabled.get("journal"), Some(&true));
    assert_eq!(refused, Err(PluginsError::ShuttingDown));
}

#[tokio::test]
async fn the_plugins_are_listed_and_started_in_the_manifests_order() {
    let fixture = Fixture::new();
    let journal = Arc::new(fixture.plugin("journal").on_by_default());
    let notes = Arc::new(fixture.plugin("notes").on_by_default());
    let alpha = Arc::new(fixture.plugin("alpha").on_by_default());
    let (_store, host) = fixture.start(vec![
        entry(&journal, 20, ""),
        entry(&notes, 10, ""),
        entry(&alpha, 10, ""),
    ]);

    host.start_chosen().await;
    let listed: Vec<String> = host.views().into_iter().map(|view| view.id).collect();
    let started: Vec<String> = fixture
        .events()
        .into_iter()
        .filter(|event| event.ends_with("started"))
        .collect();

    assert_eq!(listed, ["alpha", "notes", "journal"]);
    assert_eq!(
        started,
        ["alpha started", "notes started", "journal started"]
    );
}

#[tokio::test]
async fn the_work_of_a_plugin_is_what_its_instance_reports() {
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes"));
    *lock(&plugin.record.work) = vec![WorkItem::new("active_tasks", 2), WorkItem::new("idle", 0)];
    let (_store, host) = fixture.start(vec![entry(&plugin, 10, "")]);
    host.start_chosen().await;

    let off = host.work("notes").await.unwrap();
    host.set_enabled("notes", true).await.unwrap();
    let on = host.work("notes").await.unwrap();
    host.set_enabled("notes", false).await.unwrap();
    let stopped = host.work("notes").await.unwrap();
    let unknown = host.work("nothing").await;

    assert_eq!(off, PluginWork::default());
    assert_eq!(on.items, vec![WorkItem::new("active_tasks", 2)]);
    assert_eq!(stopped, PluginWork::default());
    assert_eq!(unknown, Err(PluginsError::UnknownPlugin));
}

#[tokio::test]
async fn the_host_refuses_a_plugin_that_it_does_not_list() {
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes"));
    let (_store, host) = fixture.start(vec![entry(&plugin, 10, "")]);

    let refused = host.set_enabled("nothing", true).await;

    assert_eq!(refused, Err(PluginsError::UnknownPlugin));
}

#[test]
fn the_list_refuses_manifests_and_plugins_that_it_cannot_run() {
    let events = Events::default();
    let plugin = |name| -> Arc<dyn Plugin> { Arc::new(TestPlugin::new(name, &events)) };
    let refused = |entries: Vec<PluginEntry>| PluginList::new(entries).unwrap_err();
    let with_module: Arc<dyn Plugin> =
        Arc::new(TestPlugin::new("notes", &events).with_module(NOTES_MODULE));

    // Manifests whose shape or ID the parse refuses, with what the message
    // names.
    for (text, named) in [
        (manifest("Not_Valid", 10, ""), "is not a plugin ID"),
        (manifest("plugins", 10, ""), "is reserved"),
        (manifest("tauri-plugin-fs", 10, ""), "tauri-plugin-"),
        ("id = \"notes\"\nsdk = ", ""),
        (
            manifest("notes", 10, "colour = \"red\"\n"),
            "unknown field `colour`",
        ),
    ] {
        let error = refused(vec![PluginEntry::new(text, plugin("notes"))]);
        let HostError::Manifest {
            index: 0,
            source: ManifestError::Toml { message },
        } = &error
        else {
            panic!("{text}: {error:?}");
        };
        assert!(message.contains(named), "{text}: {message}");
    }
    assert!(matches!(
        refused(vec![PluginEntry::new(
            manifest("notes", 10, "\n[services]\nteleport = 1\n"),
            plugin("notes")
        )]),
        HostError::Manifest {
            index: 0,
            source: ManifestError::UnknownService { .. }
        }
    ));
    assert!(matches!(
        refused(vec![
            PluginEntry::new(manifest("notes", 10, ""), plugin("notes")),
            PluginEntry::new(manifest("notes", 20, ""), plugin("notes")),
        ]),
        HostError::DuplicateId { id } if id == "notes"
    ));
    assert!(matches!(
        refused(vec![
            PluginEntry::new(manifest("log", 10, ""), plugin("log")),
            PluginEntry::new(manifest("log-viewer", 20, ""), plugin("log-viewer")),
        ]),
        HostError::NestedPrefixes { id, other } if id == "log-viewer" && other == "log"
    ));
    assert!(matches!(
        refused(vec![PluginEntry::new(manifest("notes", 10, ""), with_module)]),
        HostError::ModuleWithoutStore { id } if id == "notes"
    ));
}

#[tokio::test]
async fn the_host_refuses_an_unregistered_module_and_two_providers_or_consumers_of_one_thing() {
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes").with_module(NOTES_MODULE));
    let list = PluginList::new(vec![entry(&plugin, 10, DECLARES_STORE)]).unwrap();
    // A store opened without the plugin's module.
    let without = fixture.store(&PluginList::new(Vec::new()).unwrap());
    let unregistered = PluginHost::new(fixture.deps(&without, list.clone())).unwrap_err();
    drop(without);
    let store = fixture.store(&list);
    let mut two_providers = fixture.deps(&store, list.clone());
    two_providers.providers.push(Arc::new(FakeSecrets::new()));
    let mut two_consumers = fixture.deps(&store, list);
    two_consumers.consumers.push(FakeConsumer::new(
        PointInfo::of::<Things>(),
        &fixture.events,
    ));

    assert!(
        matches!(&unregistered, HostError::ModuleNotRegistered { id, .. } if id == "notes"),
        "{unregistered:?}"
    );
    assert!(matches!(
        PluginHost::new(two_providers).unwrap_err(),
        HostError::DuplicateProvider { service: "secrets" }
    ));
    assert!(matches!(
        PluginHost::new(two_consumers).unwrap_err(),
        HostError::DuplicatePoint { point: "things" }
    ));
}

#[tokio::test]
async fn a_service_that_the_manifest_does_not_declare_is_refused_with_grant_missing() {
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes").asking(&["secrets"]));
    let (_store, host) = fixture.start(vec![entry(&plugin, 10, "")]);

    let view = host.set_enabled("notes", true).await.unwrap();

    assert_eq!(view.state, PluginState::Failed);
    assert_eq!(view.failure.as_deref(), Some(GRANT_MISSING));
    assert_eq!(fixture.things.held("notes"), 0);
}

#[tokio::test]
async fn a_declared_service_that_no_provider_offers_fails_with_service_unavailable_before_the_plugin_runs()
 {
    let missing: Vec<Arc<dyn ServiceProvider>> = Vec::new();
    let other_major: Vec<Arc<dyn ServiceProvider>> = vec![Arc::new(OddSecrets {
        major: 2,
        refuses: false,
    })];
    let refusing: Vec<Arc<dyn ServiceProvider>> = vec![Arc::new(OddSecrets {
        major: 1,
        refuses: true,
    })];

    for providers in [missing, other_major, refusing] {
        let fixture = Fixture::new();
        let plugin = Arc::new(fixture.plugin("notes").asking(&["secrets"]));
        let list = PluginList::new(vec![entry(&plugin, 10, DECLARES_SECRETS)]).unwrap();
        let store = fixture.store(&list);
        let mut deps = fixture.deps(&store, list);
        deps.providers = providers;
        let host = PluginHost::new(deps).unwrap();

        let view = host.set_enabled("notes", true).await.unwrap();

        assert_eq!(view.state, PluginState::Failed);
        assert_eq!(view.failure.as_deref(), Some(SERVICE_UNAVAILABLE));
        assert_eq!(plugin.starts(), 0, "the plugin's code did not run");
    }
}

#[tokio::test]
async fn a_plugin_for_another_sdk_major_fails_with_sdk_incompatible_and_is_never_asked_its_default()
{
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes").on_by_default());
    let hosted: Arc<dyn Plugin> = Arc::<TestPlugin>::clone(&plugin);
    let later = "id = \"notes\"\nsdk = 2\norder = 10\nwidgets = [\"a widget\"]\n";
    let (store, host) = fixture.start(vec![PluginEntry::new(later, hosted)]);

    host.start_chosen().await;
    let listed = host.views()[0].clone();
    let settings: PluginsSettings = store.load_settings().await.unwrap();
    let turned_on = host.set_enabled("notes", true).await.unwrap();

    assert_eq!(listed.id, "notes");
    assert_eq!(listed.state, PluginState::Off);
    assert_eq!(settings.enabled.get("notes"), None, "no choice is kept");
    assert_eq!(turned_on.state, PluginState::Failed);
    assert_eq!(turned_on.failure.as_deref(), Some(SDK_INCOMPATIBLE));
    assert_eq!(plugin.defaults().len(), 0);
    assert_eq!(plugin.record.modules_asked.load(Ordering::SeqCst), 0);
    assert_eq!(plugin.starts(), 0);
}

#[tokio::test]
async fn the_start_context_carries_the_plugin_the_host_and_the_clock() {
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes"));
    let (_store, host) = fixture.start(vec![entry(&plugin, 10, "")]);

    host.set_enabled("notes", true).await.unwrap();
    let seen = lock(&plugin.record.seen).clone();

    assert_eq!(
        seen,
        [Seen {
            plugin: "notes".to_owned(),
            host: HostInfo::new("0.3.0", HostProfile::Validation),
            now: UtcMillis::from_millis(1_234),
        }]
    );
}

#[tokio::test]
async fn each_start_binds_the_declared_services_to_the_plugin() {
    let fixture = Fixture::new();
    let notes = Arc::new(fixture.plugin("notes").asking(&["secrets"]));
    let journal = Arc::new(fixture.plugin("journal").asking(&["secrets"]));
    let (_store, host) = fixture.start(vec![
        entry(&notes, 10, DECLARES_SECRETS),
        entry(&journal, 20, DECLARES_SECRETS),
    ]);

    let notes_view = host.set_enabled("notes", true).await.unwrap();
    let journal_view = host.set_enabled("journal", true).await.unwrap();

    assert_eq!(notes_view.state, PluginState::On);
    assert_eq!(journal_view.state, PluginState::On);
    assert_eq!(
        fixture.secrets.stored("notes/token").unwrap().expose(),
        "the token of notes"
    );
    assert_eq!(
        fixture.secrets.stored("journal/token").unwrap().expose(),
        "the token of journal"
    );
}

#[tokio::test]
async fn a_contribution_to_a_point_without_a_consumer_refuses_the_start_and_stops_the_instance() {
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes").contributing(|| {
        Contributions::new()
            .add::<Things>("thing")
            .add::<Unclaimed>("later")
    }));
    let (_store, host) = fixture.start(vec![entry(&plugin, 10, "")]);

    let view = host.set_enabled("notes", true).await.unwrap();

    assert_eq!(view.state, PluginState::Failed);
    assert_eq!(view.failure.as_deref(), Some(CONTRIBUTIONS_REFUSED));
    assert!(view.enabled, "the choice stays");
    assert_eq!(plugin.stops(), 1);
    assert_eq!(fixture.things.held("notes"), 0);
}

#[tokio::test]
async fn a_refusal_of_one_consumer_leaves_no_entry_of_the_plugin_in_any_consumer() {
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes").contributing(|| {
        Contributions::new()
            .add::<Things>("thing")
            .add::<Others>("other")
    }));
    let list = PluginList::new(vec![entry(&plugin, 10, "")]).unwrap();
    let store = fixture.store(&list);
    let mut deps = fixture.deps(&store, list);
    deps.consumers.push(FakeConsumer::refusing(
        PointInfo::of::<Others>(),
        &fixture.events,
    ));
    let host = PluginHost::new(deps).unwrap();

    let view = host.set_enabled("notes", true).await.unwrap();
    let consumed: Vec<String> = fixture
        .events()
        .into_iter()
        .filter(|event| event.starts_with("things"))
        .collect();

    assert_eq!(view.failure.as_deref(), Some(CONTRIBUTIONS_REFUSED));
    assert_eq!(consumed, ["things added notes", "things removed notes"]);
    assert_eq!(fixture.things.held("notes"), 0);
    assert_eq!(plugin.stops(), 1);
}

#[tokio::test]
async fn turning_off_removes_the_contributions_before_the_instance_stops() {
    let fixture = Fixture::new();
    let plugin = Arc::new(fixture.plugin("notes"));
    let (_store, host) = fixture.start(vec![entry(&plugin, 10, "")]);

    host.set_enabled("notes", true).await.unwrap();
    host.set_enabled("notes", false).await.unwrap();

    assert_eq!(
        fixture.events(),
        [
            "notes started",
            "things added notes",
            "things removed notes",
            "notes stopped"
        ]
    );
}

#[tokio::test]
async fn default_on_reads_the_plugins_module_through_the_store_service() {
    let fixture = Fixture::new();
    let notes = Arc::new(
        fixture
            .plugin("notes")
            .with_module(NOTES_MODULE)
            .asking(&["store"]),
    );
    let journal = Arc::new(fixture.plugin("journal"));
    let list = PluginList::new(vec![
        entry(&notes, 10, DECLARES_STORE),
        entry(&journal, 20, ""),
    ])
    .unwrap();
    let store = fixture.store(&list);
    store
        .execute_unlisted("INSERT INTO notes_items (id) VALUES ('n1'), ('n2')")
        .await
        .unwrap();
    let host = PluginHost::new(fixture.deps(&store, list)).unwrap();

    host.start_chosen().await;

    assert_eq!(notes.defaults(), [Some(2)]);
    assert_eq!(
        journal.defaults(),
        [None],
        "a plugin without a module gets no store"
    );
    assert_eq!(host.views()[0].state, PluginState::On);
    assert_eq!(host.views()[1].state, PluginState::Off);
}

#[tokio::test]
async fn a_plugin_whose_statements_were_refused_fails_with_service_unavailable_and_another_starts()
{
    const BROKEN: StoreModule = StoreModule::new(
        "notes",
        NOTES_MIGRATIONS,
        &["SELECT title FROM notes_items"],
    );
    let fixture = Fixture::new();
    let notes = Arc::new(
        fixture
            .plugin("notes")
            .on_by_default()
            .with_module(BROKEN)
            .asking(&["store"]),
    );
    let journal = Arc::new(fixture.plugin("journal").on_by_default());
    let (_store, host) = fixture.start(vec![
        entry(&notes, 10, DECLARES_STORE),
        entry(&journal, 20, ""),
    ]);

    host.start_chosen().await;
    let views = host.views();

    assert_eq!(notes.defaults(), [None], "its default got no store");
    assert_eq!(views[0].state, PluginState::Failed);
    assert_eq!(views[0].failure.as_deref(), Some(SERVICE_UNAVAILABLE));
    assert_eq!(notes.starts(), 0);
    assert_eq!(views[1].state, PluginState::On);
}

#[tokio::test]
async fn the_views_list_what_each_manifest_lets_its_plugin_use() {
    let fixture = Fixture::new();
    let notes = Arc::new(fixture.plugin("notes").with_module(NOTES_MODULE));
    let journal = Arc::new(fixture.plugin("journal"));
    let both = "\n[services]\nsecrets = 1\nstore = 1\n\n[secrets]\nentries = [\"token\"]\nkeyed = [\"device\"]\n\n[crates]\nexclusive = [\"tonic\"]\n\n[native]\nsockets = \"Syncs the notes.\"\n\n[[programs]]\npackage = \"mannie-notes-sync\"\n";
    let (_store, host) = fixture.start(vec![entry(&notes, 10, both), entry(&journal, 20, "")]);

    let views = host.views();

    assert_eq!(
        views[0].uses,
        PluginUses {
            services: vec![
                ServiceUse {
                    name: "secrets".to_owned(),
                    major: 1,
                    sensitive: true,
                },
                ServiceUse {
                    name: "store".to_owned(),
                    major: 1,
                    sensitive: false,
                },
            ],
            secrets: vec!["notes/token".to_owned(), "notes/device/<key>".to_owned()],
            commands: Vec::new(),
            channels: Vec::new(),
            programs: vec!["mannie-notes-sync".to_owned()],
            crates: vec!["tonic".to_owned()],
            native: vec!["sockets".to_owned()],
        }
    );
    assert_eq!(views[1].uses, PluginUses::default());
}
