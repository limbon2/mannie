//! The check that a plugin keeps the SDK's rules (ADR-0034, decision 1),
//! through the real host on the core's test runtime: its manifest, its
//! module of the store, whose migrations and listed statements the store
//! prepares under its authorizer when it opens, the codes of its interface,
//! none of which may equal a host code, the host's refusals while it is off,
//! its start with contributions whose IDs the core's consumers take, a
//! handler for every command and channel of its interface and none beside
//! them, inputs that refuse an unknown field, and its stop and second start.
//!
//! An input that refuses an unknown field may also refuse the probe because
//! it lacks a field that it needs; the plugin's own tests check its inputs
//! with valid values.

use std::sync::{Arc, Mutex, PoisonError};

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::bindings::check_interface;
use mannie_plugin_sdk::context::{DefaultContext, StartContext};
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{HostRefusal, Interface};
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::plugin::{Plugin, PluginEntry, Started};
use mannie_plugin_sdk::services::store::StoreModule;
use mannie_plugins::CallFailure;
use mannie_types::plugins::PluginState;
use serde_json::{Value, json};

use crate::host::{HostOptions, HostedPlugin};

/// The field of the probe that every input must refuse.
const PROBE_FIELD: &str = "conformanceProbe";

/// The names of the commands and of the channels of an interface or a start,
/// each sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

/// Names as owned strings, sorted.
fn sorted(names: Vec<&'static str>) -> Vec<String> {
    let mut owned: Vec<String> = names.into_iter().map(str::to_owned).collect();
    owned.sort();
    owned
}

/// The plugin, with the names of the handlers that its latest start returned.
#[derive(Debug)]
struct Recorder {
    plugin: Arc<dyn Plugin>,
    handlers: Arc<Mutex<Option<Names>>>,
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
        let handlers = Arc::clone(&self.handlers);
        Box::pin(async move {
            let started = start.await?;
            let names = Names::new(
                started.handlers.command_names(),
                started.handlers.channel_names(),
            );
            *handlers.lock().unwrap_or_else(PoisonError::into_inner) = Some(names);
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
    let handlers = Arc::new(Mutex::new(None));
    let recorder = Recorder {
        plugin: Arc::clone(&entry.plugin),
        handlers: Arc::clone(&handlers),
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
    let started = handlers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    problems.extend(handler_problems(&declared, started.as_ref()));
    problems.extend(refusals_of_unknown_fields(&hosted, &declared).await);
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

/// The handlers of the start, for every command and channel of the interface
/// and none beside them.
fn handler_problems(declared: &Names, started: Option<&Names>) -> Vec<String> {
    let Some(started) = started else {
        return vec!["the plugin started without handing the host its handlers".to_owned()];
    };
    let mut problems = Vec::new();
    if started.commands != declared.commands {
        problems.push(format!(
            "the start handed over handlers of the commands {:?}, and the interface lists {:?}",
            started.commands, declared.commands
        ));
    }
    if started.channels != declared.channels {
        problems.push(format!(
            "the start handed over handlers of the channels {:?}, and the interface lists {:?}",
            started.channels, declared.channels
        ));
    }
    problems
}

/// Every input of the plugin's commands and channels, refused with
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

/// Whether the host refused a call's input.
fn is_invalid_input<T>(answer: &Result<T, CallFailure>) -> bool {
    matches!(
        answer,
        Err(CallFailure::Host(HostRefusal::InvalidInput { .. }))
    )
}
