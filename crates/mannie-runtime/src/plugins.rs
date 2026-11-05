//! The registry of what plugins contribute while they are on (ADR-0034,
//! decision 6; ADR-0030, decision 5): one registry keyed by extension point,
//! which holds each entry as the SDK defines it, with the plugin that made it
//! and the generation of the plugin's start.
//!
//! The runtime consumes every point of the SDK through one consumer,
//! [`RuntimeContributions`]. Each point's checks live in the module that owns
//! the point: `plugin_tools.rs` for tools and tool sources, `agents.rs`,
//! `instructions.rs`, `executors.rs`, `mentions.rs`, `images.rs` and
//! `attachments.rs`. The
//! consumer checks again and adds under one write lock, and swaps one copy of
//! the whole registry, so a [`Snapshot`] holds all of a plugin's
//! contributions or none.
//!
//! A plugin's tool source publishes its tools while the plugin runs
//! (ADR-0043, decision 2): a task drains the source's stream of sets, and
//! each set replaces the source's tools in a new copy of the registry, so a
//! run that starts later reads it and a run that started before keeps what
//! it read.
//!
//! A run reads one snapshot when it starts, so the tools and the prompt that
//! the model sees never change within a reply (ADR-0030, decision 8). A call
//! of a plugin's tool runs only while the plugin's start that offered it is
//! on: a plugin turned off, or turned off and on again, since the run
//! started gives the run's call `plugin_off`.
//!
//! A plugin's optional feature contributes as a contributor of its own
//! (ADR-0055, decision 6): its entries carry the feature, its group has a
//! generation of its own, and it is added and dropped apart from its
//! plugin's, so turning the feature off leaves the plugin's entries.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use futures_util::{FutureExt, StreamExt};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::WorkItem;
use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint, PointInfo};
use mannie_plugin_sdk::host::{ContributionConsumer, ContributionRefusal, PLUGIN_REGISTERED};
use mannie_plugin_sdk::ids::{Contributor, FeatureId, PluginId};
use mannie_plugin_sdk::points::agents::Agents;
use mannie_plugin_sdk::points::context::ContextProviders;
use mannie_plugin_sdk::points::executors::Executors;
use mannie_plugin_sdk::points::images::ImageSources;
use mannie_plugin_sdk::points::mentions::Mentions;
use mannie_plugin_sdk::points::sections::Sections;
use mannie_plugin_sdk::points::terminal_hosts::TerminalHosts;
use mannie_plugin_sdk::points::tool_sources::{PluginToolSource, ToolSet, ToolSources};
use mannie_plugin_sdk::points::tools::{PluginTool, Tools};
use mannie_plugin_sdk::services::background_commands::BACKGROUND_COMMANDS_WORK;

use crate::Runtime;
use crate::attachments::check_context_providers;
use crate::executors::check_executors;
use crate::images::check_image_sources;
use crate::instructions::check_sections;
use crate::plugin_executions::ExecutionGuard;
use crate::plugin_tools::{OfferedPluginTool, kept_published};

/// An entry of a point, with the plugin that contributed it, the feature
/// whose start contributed it, and the generation of that start.
pub(crate) struct Registered<P: ExtensionPoint> {
    /// The plugin.
    pub(crate) plugin: PluginId,
    /// The plugin's feature whose start contributed the entry, or `None` for
    /// the plugin's own (ADR-0055, decision 6).
    pub(crate) feature: Option<FeatureId>,
    /// The generation of the start, which a later start of the same plugin
    /// or feature does not share.
    pub(crate) generation: u64,
    /// The entry, as the SDK defines it.
    pub(crate) entry: P::Entry,
}

impl<P: ExtensionPoint> Clone for Registered<P> {
    fn clone(&self) -> Self {
        Self {
            plugin: self.plugin.clone(),
            feature: self.feature.clone(),
            generation: self.generation,
            entry: self.entry.clone(),
        }
    }
}

/// The entries of one point, behind a type that the registry copies, filters
/// and prints without knowing the point.
trait PointEntries: Any + Send + Sync {
    /// The entries without those of `contributor`.
    fn without(&self, contributor: &Contributor) -> Arc<dyn PointEntries>;

    /// The point's name, and the plugin and ID of each entry.
    fn describe(&self) -> (&'static str, Vec<(&str, &str)>);
}

/// The entries of the point `P`, in the order in which their plugins
/// started.
struct PointList<P: ExtensionPoint>(Vec<Registered<P>>);

impl<P: ExtensionPoint> PointEntries for PointList<P> {
    fn without(&self, contributor: &Contributor) -> Arc<dyn PointEntries> {
        let kept = self
            .0
            .iter()
            .filter(|registered| {
                !is_of(&registered.plugin, registered.feature.as_ref(), contributor)
            })
            .cloned()
            .collect();
        Arc::new(Self(kept))
    }

    fn describe(&self) -> (&'static str, Vec<(&str, &str)>) {
        let entries = self
            .0
            .iter()
            .map(|registered| (registered.plugin.as_str(), P::id(&registered.entry)))
            .collect();
        (P::NAME, entries)
    }
}

/// The registry's contents at one moment. A change copies them, which costs
/// one reference count for each point, and replaces them whole.
#[derive(Clone, Default)]
struct Contents {
    /// The entries of each point, by the point's marker type.
    points: HashMap<TypeId, Arc<dyn PointEntries>>,
    /// The plugins and features whose contributions are registered, with
    /// the generation of their start, in the order in which they started.
    plugins: Vec<(Contributor, u64)>,
    /// The tools that each source published last, in the order in which
    /// their plugins started and their sources were contributed.
    published: Vec<PublishedSet>,
}

/// The tools that one source of a plugin published last, as the core kept
/// them (ADR-0043, decision 2).
#[derive(Clone)]
pub(crate) struct PublishedSet {
    /// The plugin.
    pub(crate) plugin: PluginId,
    /// The plugin's feature whose start contributed the source, or `None`
    /// for the plugin's own.
    pub(crate) feature: Option<FeatureId>,
    /// The generation of the start.
    pub(crate) generation: u64,
    /// The source's ID.
    pub(crate) source: String,
    /// The tools, in the order in which the source published them.
    pub(crate) tools: Arc<[PluginTool]>,
}

impl Contents {
    /// The entries of the point `P`, in the order in which their plugins
    /// started. A point without entries gives an empty slice.
    fn items<P: ExtensionPoint>(&self) -> &[Registered<P>] {
        // Each value is stored under its point's marker by `push`, so the
        // downcast of a present point succeeds; one that did not would read
        // as no entries rather than panic.
        self.points
            .get(&TypeId::of::<P>())
            .and_then(|entries| {
                let entries: &dyn Any = entries.as_ref();
                entries.downcast_ref::<PointList<P>>()
            })
            .map_or(&[], |list| list.0.as_slice())
    }

    /// Adds a contributor's entries of the point `P` after those of the
    /// plugins and features that started before.
    fn push<P: ExtensionPoint>(
        &mut self,
        contributor: &Contributor,
        generation: u64,
        entries: Vec<P::Entry>,
    ) {
        if entries.is_empty() {
            return;
        }
        let mut list = self.items::<P>().to_vec();
        list.extend(entries.into_iter().map(|entry| Registered {
            plugin: contributor.plugin_id().clone(),
            feature: contributor.feature_id().cloned(),
            generation,
            entry,
        }));
        self.points
            .insert(TypeId::of::<P>(), Arc::new(PointList::<P>(list)));
    }

    /// The contents without a contributor's entries.
    fn without(&self, contributor: &Contributor) -> Self {
        Self {
            points: self
                .points
                .iter()
                .map(|(point, entries)| (*point, entries.without(contributor)))
                .collect(),
            plugins: self
                .plugins
                .iter()
                .filter(|(registered, _generation)| registered != contributor)
                .cloned()
                .collect(),
            published: self
                .published
                .iter()
                .filter(|set| !is_of(&set.plugin, set.feature.as_ref(), contributor))
                .cloned()
                .collect(),
        }
    }

    /// The generation of the start of the plugin `plugin`, or of its feature
    /// `feature`, while its contributions are registered.
    fn generation_of(&self, plugin: &str, feature: Option<&str>) -> Option<u64> {
        self.plugins
            .iter()
            .find(|(registered, _generation)| {
                registered.plugin_id().as_str() == plugin
                    && registered.feature_id().map(FeatureId::as_str) == feature
            })
            .map(|(_contributor, generation)| *generation)
    }
}

/// Whether an entry of `plugin` and `feature` is the contributor's.
fn is_of(plugin: &PluginId, feature: Option<&FeatureId>, contributor: &Contributor) -> bool {
    plugin == contributor.plugin_id() && feature == contributor.feature_id()
}

impl fmt::Debug for Contents {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The entries' own types need not implement Debug, so each shows as
        // its plugin and its ID, by point.
        let mut points: Vec<(&'static str, Vec<(&str, &str)>)> = self
            .points
            .values()
            .map(|entries| entries.describe())
            .filter(|(_name, entries)| !entries.is_empty())
            .collect();
        points.sort_by_key(|(name, _entries)| *name);
        let published: Vec<(&str, &str, Vec<&str>)> = self
            .published
            .iter()
            .map(|set| {
                let names = set
                    .tools
                    .iter()
                    .map(|tool| tool.spec.name.as_str())
                    .collect();
                (set.plugin.as_str(), set.source.as_str(), names)
            })
            .collect();
        let plugins: Vec<(String, u64)> = self
            .plugins
            .iter()
            .map(|(contributor, generation)| (contributor.key(), *generation))
            .collect();
        formatter
            .debug_struct("Contents")
            .field("plugins", &plugins)
            .field("points", &points)
            .field("published", &published)
            .finish()
    }
}

/// What plugins contribute while they are on. Copy on write: a snapshot is
/// one reference count, and a change replaces the contents whole.
#[derive(Debug, Default)]
pub(crate) struct Registry {
    current: RwLock<Arc<Contents>>,
    /// The last generation that a plugin's start got.
    generations: AtomicU64,
    /// The tasks that drain the tool sources of plugins and features, by
    /// contributor, which stopping the contributor aborts.
    sources: Mutex<HashMap<Contributor, Vec<tokio::task::AbortHandle>>>,
}

impl Registry {
    fn snapshot(&self) -> Snapshot {
        let current = self.current.read().unwrap_or_else(PoisonError::into_inner);
        Snapshot(Arc::clone(&current))
    }

    /// Runs `join` while the start of `generation` of the plugin `plugin`,
    /// or of its feature `feature`, is registered, under the read lock, so
    /// dropping its contributions, which takes the write lock, waits for it.
    /// `None` when that start is not registered any more.
    fn while_registered<T>(
        &self,
        plugin: &str,
        feature: Option<&str>,
        generation: u64,
        join: impl FnOnce() -> T,
    ) -> Option<T> {
        let current = self.current.read().unwrap_or_else(PoisonError::into_inner);
        (current.generation_of(plugin, feature) == Some(generation)).then(join)
    }
}

/// The contributions of the plugins that were on at one moment.
#[derive(Debug, Clone)]
pub(crate) struct Snapshot(Arc<Contents>);

impl Snapshot {
    /// The entries of the point `P`, in the order in which their plugins
    /// started.
    pub(crate) fn items<P: ExtensionPoint>(&self) -> &[Registered<P>] {
        self.0.items::<P>()
    }

    /// The entry of the point `P` with this ID.
    pub(crate) fn find<P: ExtensionPoint>(&self, id: &str) -> Option<&Registered<P>> {
        self.items::<P>()
            .iter()
            .find(|registered| P::id(&registered.entry) == id)
    }

    /// The generation of the start of a plugin, or of its feature, while
    /// its contributions are registered.
    pub(crate) fn generation_of(&self, plugin: &str, feature: Option<&str>) -> Option<u64> {
        self.0.generation_of(plugin, feature)
    }

    /// The tools that the plugins' sources published last, in the order in
    /// which their plugins started and their sources were contributed.
    pub(crate) fn published(&self) -> &[PublishedSet] {
        &self.0.published
    }
}

/// The runtime's consumer of the points of SDK 1.
#[derive(Debug)]
struct RuntimeContributions {
    runtime: Runtime,
}

/// The points that the runtime consumes.
const RUNTIME_POINTS: [PointInfo; 9] = [
    PointInfo::of::<Tools>(),
    PointInfo::of::<Agents>(),
    PointInfo::of::<Sections>(),
    PointInfo::of::<Executors>(),
    PointInfo::of::<Mentions>(),
    PointInfo::of::<ImageSources>(),
    PointInfo::of::<ToolSources>(),
    PointInfo::of::<ContextProviders>(),
    PointInfo::of::<TerminalHosts>(),
];

impl ContributionConsumer for RuntimeContributions {
    fn points(&self) -> Vec<PointInfo> {
        RUNTIME_POINTS.to_vec()
    }

    fn check(
        &self,
        contributor: &Contributor,
        contributions: &Contributions,
    ) -> Result<(), ContributionRefusal> {
        self.runtime
            .check_contributions(&self.runtime.contributions(), contributor, contributions)
    }

    fn add(
        &self,
        contributor: &Contributor,
        mut contributions: Contributions,
    ) -> Result<(), ContributionRefusal> {
        // A contributor without entries of the runtime's points has nothing
        // to run, so the registry does not hold it.
        if contributions.is_empty() {
            return Ok(());
        }
        let plugin = contributor.plugin_id();
        let registry = &self.runtime.inner.contributions;
        let mut current = registry
            .current
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        // Checked again under the lock, so two plugins that start at once
        // never both take one ID, and the contents that the check read are
        // the ones that change.
        let before = Snapshot(Arc::clone(&current));
        self.runtime
            .check_contributions(&before, contributor, &contributions)?;
        let generation = registry.generations.fetch_add(1, Ordering::Relaxed) + 1;
        let tools = contributions.take::<Tools>();
        let agents = contributions.take::<Agents>();
        let sections = contributions.take::<Sections>();
        let executors = contributions.take::<Executors>();
        let mentions = contributions.take::<Mentions>();
        let images = contributions.take::<ImageSources>();
        let sources = contributions.take::<ToolSources>();
        let context = contributions.take::<ContextProviders>();
        let terminal_hosts = contributions.take::<TerminalHosts>();
        tracing::info!(
            plugin = %contributor,
            generation,
            tools = tools.len(),
            agents = agents.len(),
            sections = sections.len(),
            executors = executors.len(),
            mentions = mentions.len(),
            images = images.len(),
            tool_sources = sources.len(),
            context_providers = context.len(),
            terminal_hosts = terminal_hosts.len(),
            "a plugin's contributions were registered"
        );
        self.runtime.inner.agent_plugins.record(plugin, &agents);
        let mut next = Contents::clone(&current);
        next.push::<Tools>(contributor, generation, tools);
        next.push::<Agents>(contributor, generation, agents);
        next.push::<Sections>(contributor, generation, sections);
        next.push::<Executors>(contributor, generation, executors);
        next.push::<Mentions>(contributor, generation, mentions);
        next.push::<ImageSources>(contributor, generation, images);
        next.push::<ContextProviders>(contributor, generation, context);
        // Each source starts empty, so its place in the order is fixed
        // before its first set arrives.
        next.published
            .extend(sources.iter().map(|source| PublishedSet {
                plugin: plugin.clone(),
                feature: contributor.feature_id().cloned(),
                generation,
                source: source.id.clone(),
                tools: Arc::from(Vec::new()),
            }));
        next.push::<ToolSources>(contributor, generation, sources.clone());
        next.plugins.push((contributor.clone(), generation));
        *current = Arc::new(next);
        drop(current);
        self.runtime.drain_sources(contributor, generation, sources);
        self.runtime
            .add_terminal_sources(contributor, terminal_hosts);
        Ok(())
    }

    fn remove(&self, contributor: &Contributor) {
        self.runtime.drop_contributions(contributor);
    }

    /// Waits for the executions of the contributor's tools, and aborts those
    /// that do not end within `grace` (ADR-0034, decision 8). A run records
    /// an aborted call with side effects as uncertain. Then the commands that
    /// its tools started in the background end (ADR-0041, decision 11):
    /// after the calls, so no ticket of the contributor is left, and before
    /// it stops, so it ends them while their sessions are open.
    fn drain(&self, contributor: &Contributor, grace: Duration) -> BoxFuture<'static, ()> {
        let executions = Arc::clone(&self.runtime.inner.plugin_executions);
        let runtime = self.runtime.clone();
        let contributor = contributor.clone();
        Box::pin(async move {
            executions.drain(&contributor.key(), grace).await;
            runtime.end_plugin_commands(&contributor).await;
        })
    }

    /// The commands that the contributor's tools started and that still
    /// run.
    fn work(&self, contributor: &Contributor) -> Vec<WorkItem> {
        let count = self.runtime.plugin_command_count(contributor);
        vec![WorkItem::new(BACKGROUND_COMMANDS_WORK, count)]
    }
}

impl Runtime {
    /// The consumers of the extension points that the runtime owns, which
    /// the plugin host hands each plugin's contributions to (ADR-0034,
    /// decision 6).
    pub fn contribution_consumers(&self) -> Vec<Arc<dyn ContributionConsumer>> {
        vec![Arc::new(RuntimeContributions {
            runtime: self.clone(),
        })]
    }

    /// The contributions of the plugins that are on now.
    pub(crate) fn contributions(&self) -> Snapshot {
        self.inner.contributions.snapshot()
    }

    /// Lets a call of a plugin's tool execute while the plugin's start that
    /// offered the tool is on, so turning the plugin off waits for it, or
    /// `None` when that start is off (ADR-0034, decision 8).
    pub(crate) fn begin_plugin_execution(
        &self,
        offered: &OfferedPluginTool,
    ) -> Option<ExecutionGuard> {
        let executions = &self.inner.plugin_executions;
        self.inner.contributions.while_registered(
            &offered.plugin,
            offered.feature.as_deref(),
            offered.generation,
            || executions.join(&offered.contributor_key()),
        )
    }

    /// Refuses a plugin's contributions with an ID that the core or another
    /// plugin has, or that a point's own checks do not accept, before any of
    /// them is registered. The coordinator of the SDK checked the rules that
    /// need only the plugin's own entries already.
    fn check_contributions(
        &self,
        snapshot: &Snapshot,
        contributor: &Contributor,
        contributions: &Contributions,
    ) -> Result<(), ContributionRefusal> {
        let Some((point, _id)) = contributions.ids().next() else {
            return Ok(());
        };
        let plugin = contributor.plugin_id();
        let feature = contributor.feature_id().map(FeatureId::as_str);
        if snapshot.generation_of(plugin.as_str(), feature).is_some() {
            return Err(ContributionRefusal::new(
                point.name,
                PLUGIN_REGISTERED,
                format!("the contributions of {contributor} are registered already"),
            ));
        }
        self.check_tools(snapshot, plugin, contributions)?;
        self.check_agents(snapshot, contributions)?;
        check_sections(&self.inner.instructions, snapshot, contributions)?;
        check_executors(snapshot, contributions)?;
        self.check_mention_providers(snapshot, contributions)?;
        check_image_sources(snapshot, contributions)?;
        check_context_providers(snapshot, contributions)
    }

    /// Takes each of a contributor's tool sources and drains its stream of
    /// sets in a task of its own, until the stream ends or the contributor
    /// stops (ADR-0043, decision 2). A stream that ends empties its source.
    fn drain_sources(
        &self,
        contributor: &Contributor,
        generation: u64,
        sources: Vec<PluginToolSource>,
    ) {
        if sources.is_empty() {
            return;
        }
        let tasks = sources
            .into_iter()
            .filter_map(|source| {
                let mut sets = source.source.sets();
                // A set that is ready when the plugin starts, such as the
                // tools that a plugin kept from before, reaches the runs that
                // start right after the start.
                match sets.next().now_or_never() {
                    Some(Some(first)) => {
                        self.publish_tools(contributor, generation, &source.id, first);
                    }
                    // The stream ended at once, and the source stays empty.
                    Some(None) => return None,
                    None => {}
                }
                let runtime = self.clone();
                let contributor = contributor.clone();
                let drain = tokio::spawn(async move {
                    while let Some(set) = sets.next().await {
                        runtime.publish_tools(&contributor, generation, &source.id, set);
                    }
                    runtime.publish_tools(&contributor, generation, &source.id, ToolSet::default());
                });
                Some(drain.abort_handle())
            })
            .collect();
        let mut drains = self
            .inner
            .contributions
            .sources
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // A start leaves its tasks behind only when its contributions were
        // never dropped, which the host's order of a stop rules out.
        if let Some(stale) = drains.insert(contributor.clone(), tasks) {
            stale.iter().for_each(tokio::task::AbortHandle::abort);
        }
    }

    /// Replaces the tools that a contributor's source published, while its
    /// start of `generation` is registered (ADR-0043, decision 2). The core
    /// keeps each tool that passes its checks alone, and leaves the rest out
    /// with a log line.
    fn publish_tools(
        &self,
        contributor: &Contributor,
        generation: u64,
        source: &str,
        set: ToolSet,
    ) {
        let plugin = contributor.plugin_id();
        let feature = contributor.feature_id().map(FeatureId::as_str);
        let registry = &self.inner.contributions;
        let mut current = registry
            .current
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if current.generation_of(plugin.as_str(), feature) != Some(generation) {
            return;
        }
        let Some(index) = current.published.iter().position(|published| {
            is_of(&published.plugin, published.feature.as_ref(), contributor)
                && published.source == source
        }) else {
            return;
        };
        let before = Snapshot(Arc::clone(&current));
        let kept = kept_published(
            &self.inner.tools.registry,
            &before,
            plugin,
            source,
            set.tools,
        );
        tracing::info!(
            plugin = %contributor,
            source,
            tools = kept.len(),
            "a plugin's tool source published its tools"
        );
        let mut next = Contents::clone(&current);
        next.published[index].tools = Arc::from(kept);
        *current = Arc::new(next);
    }

    /// Drops a contributor's contributions, as the plugin or the feature
    /// stops (ADR-0030, decision 6). Runs that started while they were
    /// registered keep them, and their calls into it are refused; each run
    /// that follows a task of the plugin's executor lets the task go.
    fn drop_contributions(&self, contributor: &Contributor) {
        // The contributor's hosts leave the terminals' registry.
        self.remove_terminal_sources(contributor);
        // The contributor's sources publish nothing more.
        let drains = self
            .inner
            .contributions
            .sources
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(contributor);
        drains
            .iter()
            .flatten()
            .for_each(tokio::task::AbortHandle::abort);
        let dropped = {
            let mut current = self
                .inner
                .contributions
                .current
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            let feature = contributor.feature_id().map(FeatureId::as_str);
            let registered = current
                .generation_of(contributor.plugin_id().as_str(), feature)
                .is_some();
            if registered {
                *current = Arc::new(current.without(contributor));
            }
            registered
        };
        if !dropped {
            return;
        }
        // After the swap, so a run that starts following a task from now on
        // finds the executor gone and lets the task go itself. An executor's
        // kind is its plugin's ID (the rule PluginKind), and a feature has
        // none.
        let let_go = match contributor.feature_id() {
            None => self
                .inner
                .executors
                .let_go_of(contributor.plugin_id().as_str()),
            Some(_feature) => 0,
        };
        tracing::info!(
            plugin = %contributor,
            tasks_let_go = let_go,
            "a plugin's contributions were dropped"
        );
    }
}
