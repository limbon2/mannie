//! The check that a plugin keeps the SDK's rules (ADR-0034, decision 1),
//! through the real host on the core's test runtime: its manifest, its
//! module of the store, whose migrations and listed statements the store
//! prepares under its authorizer when it opens, the codes of its interface,
//! none of which may equal a host code, the host's refusals while it is off,
//! its start with contributions whose IDs the core's consumers take, a
//! handler for every command and channel of its interface and none beside
//! them, inputs that refuse an unknown field, and its stop and second start.
//!
//! Each optional feature of the manifest is checked the same way while the
//! plugin is on (ADR-0055, decision 11): its commands and channels answer
//! `feature_off` while it is off, its start hands over exactly the handlers
//! that the interface names for it, its inputs refuse an unknown field, it
//! stops and starts again, and turning the plugin off stops it first.
//!
//! An input that refuses an unknown field may also refuse the probe because
//! it lacks a field that it needs; the plugin's own tests check its inputs
//! with valid values.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::bindings::check_interface;
use mannie_plugin_sdk::context::{DefaultContext, FeatureContext, StartContext};
use mannie_plugin_sdk::contracts::WorkItem;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{Endpoint, HostRefusal, Interface};
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::plugin::{FeatureStarted, Instance, Plugin, PluginEntry, Started};
use mannie_plugin_sdk::services::store::StoreModule;
use mannie_plugins::CallFailure;
use mannie_types::plugins::PluginState;
use serde_json::{Value, json};

use crate::host::{HostOptions, HostedPlugin};

/// The field of the probe that every input must refuse.
const PROBE_FIELD: &str = "conformanceProbe";

/// The names of the commands and of the channels of an interface or a start,
/// each sorted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Names {
    commands: Vec<String>,
    channels: Vec<String>,
}

impl Names {
    fn new(commands: Vec<&'static str>, channels: Vec<&'static str>) -> Self {
        Self {
            commands: sorted(commands),
            channels: sorted(channels),
        }
    }

    /// The names of the endpoints of the feature `feature`, or of the plugin
    /// itself for `None`.
    fn of(interface: &Interface, feature: Option<&str>) -> Self {
        let names = |endpoints: &[Endpoint]| {
            endpoints
                .iter()
                .filter(|endpoint| endpoint.feature == feature)
                .map(|endpoint| endpoint.name)
                .collect()
        };
        Self::new(names(interface.commands()), names(interface.channels()))
    }
}

/// Names as owned strings, sorted.
fn sorted(names: Vec<&'static str>) -> Vec<String> {
    let mut owned: Vec<String> = names.into_iter().map(str::to_owned).collect();
    owned.sort();
    owned
}

/// The names of the handlers that the latest starts returned: the plugin's,
/// and each feature's by its ID.
#[derive(Debug, Default)]
struct Handed {
    plugin: Option<Names>,
    features: BTreeMap<String, Names>,
}

type Record = Arc<Mutex<Handed>>;

/// The plugin, with the names of the handlers that its latest start and its
/// features' latest starts returned.
#[derive(Debug)]
struct Recorder {
    plugin: Arc<dyn Plugin>,
    handed: Record,
}

impl Plugin for Recorder {
    fn interface(&self) -> Interface {
        self.plugin.interface()
    }

    fn store_module(&self) -> Option<StoreModule> {
        self.plugin.store_module()
    }

    fn default_on(&self, context: DefaultContext) -> BoxFuture<'static, bool> {
        self.plugin.default_on(context)
    }

    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        let start = self.plugin.start(context);
        let handed = Arc::clone(&self.handed);
        Box::pin(async move {
            let mut started = start.await?;
            let names = Names::new(
                started.handlers.command_names(),
                started.handlers.channel_names(),
            );
            handed.lock().unwrap_or_else(PoisonError::into_inner).plugin = Some(names);
            started.instance = Box::new(RecordingInstance {
                instance: started.instance,
                handed,
            });
            Ok(started)
        })
    }
}

/// The plugin's running instance, which records the handlers of each feature
/// that it starts.
#[derive(Debug)]
struct RecordingInstance {
    instance: Box<dyn Instance>,
    handed: Record,
}

impl Instance for RecordingInstance {
    fn work(&self) -> BoxFuture<'static, Vec<WorkItem>> {
        self.instance.work()
    }

    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        self.instance.stop()
    }

    fn start_feature(
        &self,
        context: FeatureContext,
    ) -> BoxFuture<'static, Result<FeatureStarted, StartFailure>> {
        let feature = context.feature().to_string();
        let start = self.instance.start_feature(context);
        let handed = Arc::clone(&self.handed);
        Box::pin(async move {
            let started = start.await?;
            let names = Names::new(
                started.handlers.command_names(),
                started.handlers.channel_names(),
            );
            handed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .features
                .insert(feature, names);
            Ok(started)
        })
    }
}

/// Checks that the plugin of `entry` keeps the SDK's rules on a host set up
/// as `options`, which provides the services that its manifest declares
/// beyond the runtime's and the store's. Returns one sentence for each rule
/// that the plugin breaks, and nothing when it keeps them all.
///
/// # Panics
///
/// When the store refuses the plugin's module, or the host refuses the
/// plugin's manifest, which fails the test with the reason.
pub async fn conformance(entry: PluginEntry, options: HostOptions) -> Vec<String> {
    let manifest = match Manifest::parse(entry.manifest) {
        Ok(manifest) => manifest,
        Err(error) => return vec![format!("the manifest is refused: {error}")],
    };
    let interface = entry.plugin.interface();
    let mut problems = Vec::new();
    if let Err(error) = check_interface(manifest.id.as_str(), &interface) {
        problems.push(format!("the interface is refused: {error}"));
    }
    let declared = Names::new(interface.command_names(), interface.channel_names());
    let own = Names::of(&interface, None);
    let handed = Record::default();
    let recorder = Recorder {
        plugin: Arc::clone(&entry.plugin),
        handed: Arc::clone(&handed),
    };
    let hosted = HostedPlugin::new(
        PluginEntry::new(entry.manifest, Arc::new(recorder)),
        options,
    )
    .await;

    problems.extend(refusals_while_off(&hosted, &declared).await);
    let on = hosted.turn_on().await;
    if on.state != PluginState::On {
        problems.push(format!(
            "the plugin did not start: it is {:?} with the failure {:?}",
            on.state, on.failure
        ));
        return problems;
    }
    let started = handed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .plugin
        .clone();
    problems.extend(handler_problems("the start", &own, started.as_ref()));
    problems.extend(refusals_of_unknown_fields(&hosted, &own).await);
    for feature in manifest.features.keys() {
        let names = Names::of(&interface, Some(feature));
        problems.extend(feature_problems(&hosted, &handed, feature, &names).await);
    }
    problems.extend(stop_and_second_start(&hosted).await);
    problems
}

/// Every command and channel of the interface, refused with `plugin_off`
/// while the plugin is off.
async fn refusals_while_off(hosted: &HostedPlugin, declared: &Names) -> Vec<String> {
    let mut problems = Vec::new();
    for command in &declared.commands {
        let answer = hosted.invoke(command, json!({})).await;
        if !is_plugin_off(&answer) {
            problems.push(format!(
                "the command {command} answered {answer:?} while the plugin was off, not plugin_off"
            ));
        }
    }
    for channel in &declared.channels {
        let answer = hosted
            .open_channel(channel, Value::Null)
            .await
            .map(|_stream| ());
        if !is_plugin_off(&answer) {
            problems.push(format!(
                "the channel {channel} answered {answer:?} while the plugin was off, not plugin_off"
            ));
        }
    }
    problems
}

/// One feature, while its plugin is on: its commands and channels refused
/// with `feature_off` while it is off, its start, its handlers, its inputs,
/// its stop and second start, and its stop before its plugin's.
async fn feature_problems(
    hosted: &HostedPlugin,
    handed: &Record,
    feature: &str,
    names: &Names,
) -> Vec<String> {
    let mut problems = refusals_while_feature_off(hosted, feature, names).await;
    let on = hosted.turn_feature_on(feature).await;
    if on.state != PluginState::On {
        problems.push(format!(
            "the feature {feature} did not start: it is {:?} with the failure {:?}",
            on.state, on.failure
        ));
        return problems;
    }
    let started = handed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .features
        .get(feature)
        .cloned();
    let subject = format!("the start of the feature {feature}");
    problems.extend(handler_problems(&subject, names, started.as_ref()));
    problems.extend(refusals_of_unknown_fields(hosted, names).await);
    let off = hosted.turn_feature_off(feature).await;
    if off.state != PluginState::Off {
        problems.push(format!(
            "the feature {feature} did not stop: it is {:?}",
            off.state
        ));
    }
    let again = hosted.turn_feature_on(feature).await;
    if again.state != PluginState::On {
        problems.push(format!(
            "the feature {feature} did not start a second time: it is {:?} with the failure {:?}",
            again.state, again.failure
        ));
    }
    // Turning the plugin off stops the feature first; its choice stays.
    hosted.turn_off().await;
    let stopped = hosted.feature_view(feature);
    if stopped.state != PluginState::Off {
        problems.push(format!(
            "the feature {feature} did not stop with its plugin: it is {:?}",
            stopped.state
        ));
    }
    hosted.turn_on().await;
    hosted.turn_feature_off(feature).await;
    problems
}

/// Every command and channel of a feature, refused with `feature_off` while
/// the plugin is on and the feature is off.
async fn refusals_while_feature_off(
    hosted: &HostedPlugin,
    feature: &str,
    names: &Names,
) -> Vec<String> {
    let mut problems = Vec::new();
    for command in &names.commands {
        let answer = hosted.invoke(command, json!({})).await;
        if !is_feature_off(&answer) {
            problems.push(format!(
                "the command {command} answered {answer:?} while the feature {feature} was off, not feature_off"
            ));
        }
    }
    for channel in &names.channels {
        let answer = hosted
            .open_channel(channel, Value::Null)
            .await
            .map(|_stream| ());
        if !is_feature_off(&answer) {
            problems.push(format!(
                "the channel {channel} answered {answer:?} while the feature {feature} was off, not feature_off"
            ));
        }
    }
    problems
}

/// The handlers of a start, for every command and channel that the
/// interface names for it and none beside them.
fn handler_problems(subject: &str, declared: &Names, started: Option<&Names>) -> Vec<String> {
    let Some(started) = started else {
        return vec![format!("{subject} did not hand the host its handlers")];
    };
    let mut problems = Vec::new();
    if started.commands != declared.commands {
        problems.push(format!(
            "{subject} handed over handlers of the commands {:?}, and the interface lists {:?}",
            started.commands, declared.commands
        ));
    }
    if started.channels != declared.channels {
        problems.push(format!(
            "{subject} handed over handlers of the channels {:?}, and the interface lists {:?}",
            started.channels, declared.channels
        ));
    }
    problems
}

/// Every input of the commands and channels in `declared`, refused with
/// `invalid_input` when it holds an unknown field.
async fn refusals_of_unknown_fields(hosted: &HostedPlugin, declared: &Names) -> Vec<String> {
    let mut problems = Vec::new();
    for command in &declared.commands {
        let answer = hosted.invoke(command, json!({ PROBE_FIELD: 1 })).await;
        if !is_invalid_input(&answer) {
            problems.push(format!(
                "the command {command} answered {answer:?} to an input with an unknown field, not invalid_input"
            ));
        }
    }
    for channel in &declared.channels {
        let answer = hosted
            .open_channel(channel, json!({ PROBE_FIELD: 1 }))
            .await
            .map(|_stream| ());
        if !is_invalid_input(&answer) {
            problems.push(format!(
                "the channel {channel} answered {answer:?} to an input with an unknown field, not invalid_input"
            ));
        }
    }
    problems
}

/// The plugin stops, starts a second time and stops again.
async fn stop_and_second_start(hosted: &HostedPlugin) -> Vec<String> {
    let mut problems = Vec::new();
    let off = hosted.turn_off().await;
    if off.state != PluginState::Off {
        problems.push(format!("the plugin did not stop: it is {:?}", off.state));
    }
    let again = hosted.turn_on().await;
    if again.state != PluginState::On {
        problems.push(format!(
            "the plugin did not start a second time: it is {:?} with the failure {:?}",
            again.state, again.failure
        ));
    }
    hosted.turn_off().await;
    problems
}

/// Whether the host refused a call because the plugin is off.
fn is_plugin_off<T>(answer: &Result<T, CallFailure>) -> bool {
    matches!(
        answer,
        Err(CallFailure::Host(HostRefusal::PluginOff { .. }))
    )
}

/// Whether the host refused a call because its feature is off.
fn is_feature_off<T>(answer: &Result<T, CallFailure>) -> bool {
    matches!(
        answer,
        Err(CallFailure::Host(HostRefusal::FeatureOff { .. }))
    )
}

/// Whether the host refused a call's input.
fn is_invalid_input<T>(answer: &Result<T, CallFailure>) -> bool {
    matches!(
        answer,
        Err(CallFailure::Host(HostRefusal::InvalidInput { .. }))
    )
}
