//! The harness with plugins of its own, written as plugins write theirs: a
//! hosted plugin starts off, the host refuses its calls until it is on and
//! then dispatches them, and the conformance check finds nothing in a plugin
//! that keeps the SDK's rules and reports each planted breach: a channel
//! without a handler, an input that takes an unknown field, and an error
//! code that is a host code.

use std::sync::Arc;

use futures_util::StreamExt as _;
use futures_util::stream;
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::{CallFailure, HostOptions, HostedPlugin};
use mannie_plugin_harness::types::PluginState;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::{FeatureContext, StartContext};
use mannie_plugin_sdk::error::{FEATURE_UNKNOWN, StartFailure};
use mannie_plugin_sdk::ipc::{
    ChannelMessage, ChannelSpec, CommandSpec, Handlers, HostRefusal, InputRefusal, Interface,
    PluginError, Validate,
};
use mannie_plugin_sdk::plugin::{
    FeatureInstance, FeatureStarted, Instance, Plugin, PluginEntry, Started,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use specta::Type;

/// The manifest of a plugin of the tests, without services.
macro_rules! manifest {
    ($id:literal) => {
        concat!(
            "id = \"",
            $id,
            "\"\nsdk = 1\norder = 10\n\n[rust]\nentry = \"mannie-",
            $id,
            "\"\n"
        )
    };
}

/// The argument of `greet`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Greet {
    /// Whom to greet.
    name: String,
}

impl Validate for Greet {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.name.trim().is_empty() {
            return Err(InputRefusal::new("name"));
        }
        Ok(())
    }
}

/// An argument that takes fields it does not know, which the rules refuse.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
struct LenientGreet {
    /// Whom to greet, if anyone.
    name: Option<String>,
}

impl Validate for LenientGreet {
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// Why `greet` failed.
#[derive(Debug, Clone, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
enum GreeterError {
    /// The greeter is hoarse.
    #[error("the greeter is hoarse")]
    Hoarse,
}

impl PluginError for GreeterError {
    fn code(&self) -> &'static str {
        "greeter.hoarse"
    }
}

/// An error whose code is the host's `plugin_off`, which the rules refuse.
#[derive(Debug, Clone, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
enum ShadowingError {
    /// Claims that the plugin is off.
    #[error("the plugin claims to be off")]
    PluginOff,
}

impl PluginError for ShadowingError {
    fn code(&self) -> &'static str {
        "greeter.plugin_off"
    }
}

/// `greet`: a greeting for a name.
enum GreetCommand {}

impl CommandSpec for GreetCommand {
    const NAME: &'static str = "greet";
    const DOCS: &'static str = "A greeting for a name.";
    type Input = Greet;
    type Output = String;
    type Error = GreeterError;
}

/// `greet` with an input that takes unknown fields.
enum LenientGreetCommand {}

impl CommandSpec for LenientGreetCommand {
    const NAME: &'static str = "greet";
    const DOCS: &'static str = "A greeting for a name, if any.";
    type Input = LenientGreet;
    type Output = String;
    type Error = GreeterError;
}

/// `greet` with an error that shadows a host code.
enum ShadowingGreetCommand {}

impl CommandSpec for ShadowingGreetCommand {
    const NAME: &'static str = "greet";
    const DOCS: &'static str = "A greeting for a name.";
    type Input = Greet;
    type Output = String;
    type Error = ShadowingError;
}

/// `ticks`: three numbers.
enum TicksChannel {}

impl ChannelSpec for TicksChannel {
    const NAME: &'static str = "ticks";
    const DOCS: &'static str = "Three numbers, then the end.";
    type Input = ();
    type Item = u32;
    type Error = GreeterError;
}

/// How a plugin of the tests breaks the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Breach {
    None,
    /// Its start hands over no handler of its channel.
    MissingChannelHandler,
    /// Its command takes unknown fields.
    LenientInput,
    /// Its error code is a host code.
    ShadowedCode,
}

#[derive(Debug)]
struct Greeter {
    breach: Breach,
}

#[derive(Debug)]
struct GreeterInstance;

impl Instance for GreeterInstance {
    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(std::future::ready(()))
    }
}

fn greeting(name: Option<String>) -> String {
    format!("Hello, {}!", name.unwrap_or_else(|| "nobody".to_owned()))
}

impl Plugin for Greeter {
    fn interface(&self) -> Interface {
        let interface = match self.breach {
            Breach::LenientInput => Interface::new().command::<LenientGreetCommand>(),
            Breach::ShadowedCode => Interface::new().command::<ShadowingGreetCommand>(),
            Breach::None | Breach::MissingChannelHandler => {
                Interface::new().command::<GreetCommand>()
            }
        };
        interface.channel::<TicksChannel>()
    }

    fn start(&self, _context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        let handlers = match self.breach {
            Breach::LenientInput => Handlers::new().command::<LenientGreetCommand, _, _>(|input| {
                std::future::ready(Ok(greeting(input.name)))
            }),
            Breach::ShadowedCode => {
                Handlers::new().command::<ShadowingGreetCommand, _, _>(|input| {
                    std::future::ready(Ok(greeting(Some(input.name))))
                })
            }
            Breach::None | Breach::MissingChannelHandler => Handlers::new()
                .command::<GreetCommand, _, _>(|input| {
                    std::future::ready(Ok(greeting(Some(input.name))))
                }),
        };
        let handlers = if self.breach == Breach::MissingChannelHandler {
            handlers
        } else {
            handlers.channel::<TicksChannel, _, _>(|()| {
                std::future::ready(Ok(stream::iter([1, 2, 3]).boxed()))
            })
        };
        Box::pin(std::future::ready(Ok(
            Started::new(GreeterInstance).with_handlers(handlers)
        )))
    }
}

fn greeter(breach: Breach) -> PluginEntry {
    PluginEntry::new(manifest!("greeter"), Arc::new(Greeter { breach }))
}

#[tokio::test]
async fn a_hosted_plugin_starts_off_and_answers_once_it_is_on() {
    let hosted = HostedPlugin::new(greeter(Breach::None), HostOptions::new()).await;

    let while_off = hosted.invoke("greet", json!({ "name": "Ada" })).await;
    let view = hosted.turn_on().await;
    let greeting = hosted.invoke("greet", json!({ "name": "Ada" })).await;
    let blank = hosted.invoke("greet", json!({ "name": " " })).await;
    let ticks: Vec<ChannelMessage<Value>> = hosted
        .open_channel("ticks", Value::Null)
        .await
        .unwrap()
        .collect()
        .await;

    assert_eq!(hosted.id(), "greeter");
    assert_eq!(
        while_off,
        Err(CallFailure::Host(HostRefusal::plugin_off("greeter")))
    );
    assert_eq!(view.state, PluginState::On);
    assert_eq!(greeting, Ok(json!("Hello, Ada!")));
    assert_eq!(
        blank,
        Err(CallFailure::Host(HostRefusal::InvalidInput {
            field: Some("name".to_owned())
        }))
    );
    assert_eq!(ticks.len(), 4, "three items and the end: {ticks:?}");
    assert_eq!(hosted.turn_off().await.state, PluginState::Off);
}

#[tokio::test]
async fn a_plugin_that_keeps_the_rules_conforms() {
    assert_eq!(
        conformance(greeter(Breach::None), HostOptions::new()).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn the_check_reports_a_channel_without_a_handler() {
    let problems = conformance(greeter(Breach::MissingChannelHandler), HostOptions::new()).await;

    assert_eq!(problems.len(), 2, "{problems:#?}");
    assert!(
        problems[0].contains("handlers of the channels []"),
        "{problems:#?}"
    );
    assert!(
        problems[1].contains("the channel ticks answered"),
        "{problems:#?}"
    );
}

#[tokio::test]
async fn the_check_reports_an_input_that_takes_an_unknown_field() {
    let problems = conformance(greeter(Breach::LenientInput), HostOptions::new()).await;

    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert!(
        problems[0].starts_with("the command greet answered Ok(String(\"Hello, nobody!\"))"),
        "{problems:#?}"
    );
}

#[tokio::test]
async fn the_check_reports_a_code_of_the_host() {
    let problems = conformance(greeter(Breach::ShadowedCode), HostOptions::new()).await;

    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert!(
        problems[0].starts_with("the interface is refused"),
        "{problems:#?}"
    );
}

/// `farewell`: a farewell for a name, a command of the feature `farewell`.
enum FarewellCommand {}

impl CommandSpec for FarewellCommand {
    const NAME: &'static str = "farewell";
    const DOCS: &'static str = "A farewell for a name.";
    type Input = Greet;
    type Output = String;
    type Error = GreeterError;
}

/// How a plugin with a feature breaks the rules of features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeatureBreach {
    None,
    /// The feature's start hands over no handler of its command.
    MissingHandler,
    /// The plugin's instance does not start the feature that its manifest
    /// lists.
    NotStarted,
}

/// A plugin with the optional feature `farewell` (ADR-0055).
#[derive(Debug)]
struct Parting {
    breach: FeatureBreach,
}

#[derive(Debug)]
struct PartingInstance {
    breach: FeatureBreach,
}

#[derive(Debug)]
struct FarewellInstance;

impl FeatureInstance for FarewellInstance {
    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(std::future::ready(()))
    }
}

impl Instance for PartingInstance {
    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(std::future::ready(()))
    }

    fn start_feature(
        &self,
        _context: FeatureContext,
    ) -> BoxFuture<'static, Result<FeatureStarted, StartFailure>> {
        let handlers = match self.breach {
            FeatureBreach::None => Handlers::new().command::<FarewellCommand, _, _>(|input| {
                std::future::ready(Ok(format!("Goodbye, {}!", input.name)))
            }),
            FeatureBreach::MissingHandler => Handlers::new(),
            FeatureBreach::NotStarted => {
                return Box::pin(std::future::ready(Err(StartFailure::new(FEATURE_UNKNOWN))));
            }
        };
        Box::pin(std::future::ready(Ok(FeatureStarted::new(
            FarewellInstance,
        )
        .with_handlers(handlers))))
    }
}

impl Plugin for Parting {
    fn interface(&self) -> Interface {
        Interface::new()
            .command::<GreetCommand>()
            .feature("farewell", Interface::command::<FarewellCommand>)
    }

    fn start(&self, _context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        let handlers = Handlers::new().command::<GreetCommand, _, _>(|input| {
            std::future::ready(Ok(greeting(Some(input.name))))
        });
        let instance = PartingInstance {
            breach: self.breach,
        };
        Box::pin(std::future::ready(Ok(
            Started::new(instance).with_handlers(handlers)
        )))
    }
}

fn parting(breach: FeatureBreach) -> PluginEntry {
    let manifest = concat!(manifest!("parting"), "\n[features.farewell]\n");
    PluginEntry::new(manifest, Arc::new(Parting { breach }))
}

#[tokio::test]
async fn a_hosted_feature_answers_while_it_is_on() {
    let hosted = HostedPlugin::new(parting(FeatureBreach::None), HostOptions::new()).await;
    hosted.turn_on().await;

    let while_off = hosted.invoke("farewell", json!({ "name": "Ada" })).await;
    let view = hosted.turn_feature_on("farewell").await;
    let farewell = hosted.invoke("farewell", json!({ "name": "Ada" })).await;
    let off = hosted.turn_feature_off("farewell").await;

    assert_eq!(
        while_off,
        Err(CallFailure::Host(HostRefusal::feature_off(
            "parting", "farewell"
        )))
    );
    assert_eq!(view.state, PluginState::On);
    assert_eq!(farewell, Ok(json!("Goodbye, Ada!")));
    assert_eq!(off.state, PluginState::Off);
}

#[tokio::test]
async fn a_plugin_whose_feature_keeps_the_rules_conforms() {
    assert_eq!(
        conformance(parting(FeatureBreach::None), HostOptions::new()).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn the_check_reports_a_features_command_without_a_handler() {
    let problems = conformance(parting(FeatureBreach::MissingHandler), HostOptions::new()).await;

    assert!(
        problems.iter().any(|problem| problem
            .contains("the start of the feature farewell handed over handlers of the commands []")),
        "{problems:#?}"
    );
}

#[tokio::test]
async fn the_check_reports_a_feature_that_the_plugin_does_not_start() {
    let problems = conformance(parting(FeatureBreach::NotStarted), HostOptions::new()).await;

    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert!(
        problems[0].contains("the feature farewell did not start")
            && problems[0].contains("feature_unknown"),
        "{problems:#?}"
    );
}
