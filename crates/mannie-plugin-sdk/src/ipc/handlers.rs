//! The handlers of a plugin's commands and channels, which its start hands
//! the host, and the form in which the host's dispatch calls them.
//!
//! A plugin registers a typed handler for each spec. The SDK erases its
//! types at once: the host hands it the input as JSON, and the handler
//! decodes it, runs its checks, and returns a future that the host spawns
//! into the plugin's calls in flight. The host never sees a plugin type, and
//! the plugin never sees a host type (decision 3).

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use futures_util::StreamExt;
use serde_json::Value;

use super::{ChannelSpec, CommandSpec, InputRefusal, PluginError, PluginStream, Validate};
use crate::BoxFuture;

/// The handlers of a plugin's commands and channels, which exist only
/// between its start and its stop (decision 4). Each spec of the plugin's
/// interface gets one handler; a second handler for a name replaces the
/// first.
#[derive(Default)]
pub struct Handlers {
    commands: BTreeMap<&'static str, Arc<dyn ErasedCommand>>,
    channels: BTreeMap<&'static str, Arc<dyn ErasedChannel>>,
}

impl Handlers {
    /// No handlers.
    pub fn new() -> Self {
        Self::default()
    }

    /// The handlers with `handler` for the command `C`.
    #[must_use]
    pub fn command<C, F, Fut>(mut self, handler: F) -> Self
    where
        C: CommandSpec,
        F: Fn(C::Input) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<C::Output, C::Error>> + Send + 'static,
    {
        let erased = TypedCommand::<C, F> {
            handler,
            spec: PhantomData,
        };
        self.commands.insert(C::NAME, Arc::new(erased));
        self
    }

    /// The handlers with `handler` for the channel `C`, which opens the
    /// channel's stream.
    #[must_use]
    pub fn channel<C, F, Fut>(mut self, handler: F) -> Self
    where
        C: ChannelSpec,
        F: Fn(C::Input) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<PluginStream<C::Item>, C::Error>> + Send + 'static,
    {
        let erased = TypedChannel::<C, F> {
            handler,
            spec: PhantomData,
        };
        self.channels.insert(C::NAME, Arc::new(erased));
        self
    }

    /// The names of the commands with a handler, in order.
    pub fn command_names(&self) -> Vec<&'static str> {
        self.commands.keys().copied().collect()
    }

    /// The names of the channels with a handler, in order.
    pub fn channel_names(&self) -> Vec<&'static str> {
        self.channels.keys().copied().collect()
    }
}

impl fmt::Debug for Handlers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Handlers")
            .field("commands", &self.command_names())
            .field("channels", &self.channel_names())
            .finish()
    }
}

impl Handlers {
    /// The handler of the command `name`, if the plugin has one.
    pub fn find_command(&self, name: &str) -> Option<CommandHandler> {
        self.commands.get(name).map(|erased| CommandHandler {
            erased: Arc::clone(erased),
        })
    }

    /// The handler of the channel `name`, if the plugin has one.
    pub fn find_channel(&self, name: &str) -> Option<ChannelHandler> {
        self.channels.get(name).map(|erased| ChannelHandler {
            erased: Arc::clone(erased),
        })
    }
}

/// Why an input did not reach a handler.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputFailure {
    /// The input did not decode as the spec's input, such as one with an
    /// unknown field. It keeps only the category of the decoder's error,
    /// whose message can quote the input (T-6).
    Undecodable {
        /// The category: `syntax`, `data`, `eof` or `io`.
        category: &'static str,
    },
    /// The input decoded and its checks refused it.
    Refused(InputRefusal),
}

/// A plugin's own error as JSON, with what the host logs and the UI's
/// envelope carries.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ErrorValue {
    /// The error's stable code for the log, from [`PluginError::code`].
    pub code: &'static str,
    /// The error as JSON, tagged by its `code`.
    pub error: Value,
    /// What went wrong, in English, for logs.
    pub description: String,
}

impl ErrorValue {
    /// A plugin's error with its code for the log, its JSON and its
    /// description, as a test or a later adapter builds one.
    pub fn new(code: &'static str, error: Value, description: impl Into<String>) -> Self {
        Self {
            code,
            error,
            description: description.into(),
        }
    }

    /// The error `error` as JSON, or `None` when it does not serialize.
    fn of<E: PluginError>(error: &E) -> Option<Self> {
        match serde_json::to_value(error) {
            Ok(json) => Some(Self {
                code: error.code(),
                error: json,
                description: error.to_string(),
            }),
            Err(serialize) => {
                tracing::error!(error = %serialize, code = error.code(), "a plugin's error did not serialize");
                None
            }
        }
    }
}

/// What a command's handler answered.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Answer {
    /// The output, as JSON.
    Output(Value),
    /// The plugin's error.
    Error(ErrorValue),
    /// The output or the error did not serialize, which the log records.
    Unserializable,
}

/// What a channel's handler answered.
#[non_exhaustive]
pub enum Opened {
    /// The channel's items, as JSON. An item that does not serialize is
    /// left out, and the log records it.
    Items(PluginStream<Value>),
    /// The plugin's error.
    Error(ErrorValue),
    /// The error did not serialize, which the log records.
    Unserializable,
}

impl fmt::Debug for Opened {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Items(_items) => formatter.write_str("Items"),
            Self::Error(error) => formatter.debug_tuple("Error").field(error).finish(),
            Self::Unserializable => formatter.write_str("Unserializable"),
        }
    }
}

/// A command's handler, as the host's dispatch calls it.
#[derive(Clone)]
pub struct CommandHandler {
    erased: Arc<dyn ErasedCommand>,
}

impl CommandHandler {
    /// The most bytes of the input as JSON.
    pub fn max_input_bytes(&self) -> u32 {
        self.erased.max_input_bytes()
    }

    /// Decodes and checks `input`, and returns the handler's call, which
    /// runs once it is polled.
    pub fn prepare(&self, input: Value) -> Result<BoxFuture<'static, Answer>, InputFailure> {
        self.erased.prepare(input)
    }
}

impl fmt::Debug for CommandHandler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandHandler")
            .field("max_input_bytes", &self.max_input_bytes())
            .finish_non_exhaustive()
    }
}

/// A channel's handler, as the host's dispatch calls it.
#[derive(Clone)]
pub struct ChannelHandler {
    erased: Arc<dyn ErasedChannel>,
}

impl ChannelHandler {
    /// The most bytes of the input as JSON.
    pub fn max_input_bytes(&self) -> u32 {
        self.erased.max_input_bytes()
    }

    /// Decodes and checks `input`, and returns the handler's opening of the
    /// channel, which runs once it is polled.
    pub fn prepare(&self, input: Value) -> Result<BoxFuture<'static, Opened>, InputFailure> {
        self.erased.prepare(input)
    }
}

impl fmt::Debug for ChannelHandler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChannelHandler")
            .field("max_input_bytes", &self.max_input_bytes())
            .finish_non_exhaustive()
    }
}

/// A command's handler without its types.
trait ErasedCommand: Send + Sync {
    fn max_input_bytes(&self) -> u32;

    fn prepare(&self, input: Value) -> Result<BoxFuture<'static, Answer>, InputFailure>;
}

/// A channel's handler without its types.
trait ErasedChannel: Send + Sync {
    fn max_input_bytes(&self) -> u32;

    fn prepare(&self, input: Value) -> Result<BoxFuture<'static, Opened>, InputFailure>;
}

/// Decodes an input and runs its checks.
fn decode<I>(input: Value) -> Result<I, InputFailure>
where
    I: serde::de::DeserializeOwned + Validate,
{
    let decoded: I = serde_json::from_value(input).map_err(|error| InputFailure::Undecodable {
        category: category(&error),
    })?;
    decoded.validate().map_err(InputFailure::Refused)?;
    Ok(decoded)
}

/// The category of a decoder's error, without its message.
fn category(error: &serde_json::Error) -> &'static str {
    match error.classify() {
        serde_json::error::Category::Io => "io",
        serde_json::error::Category::Syntax => "syntax",
        serde_json::error::Category::Data => "data",
        serde_json::error::Category::Eof => "eof",
    }
}

struct TypedCommand<C, F> {
    handler: F,
    /// The spec, by type alone; `fn() -> C` keeps the handler `Send` and
    /// `Sync` whatever `C` is.
    spec: PhantomData<fn() -> C>,
}

impl<C, F, Fut> ErasedCommand for TypedCommand<C, F>
where
    C: CommandSpec,
    F: Fn(C::Input) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<C::Output, C::Error>> + Send + 'static,
{
    fn max_input_bytes(&self) -> u32 {
        C::MAX_INPUT_BYTES
    }

    fn prepare(&self, input: Value) -> Result<BoxFuture<'static, Answer>, InputFailure> {
        let call = (self.handler)(decode::<C::Input>(input)?);
        Ok(Box::pin(async move {
            match call.await {
                Ok(output) => match serde_json::to_value(output) {
                    Ok(json) => Answer::Output(json),
                    Err(error) => {
                        tracing::error!(%error, command = C::NAME, "the output of a plugin's command did not serialize");
                        Answer::Unserializable
                    }
                },
                Err(error) => ErrorValue::of(&error).map_or(Answer::Unserializable, Answer::Error),
            }
        }))
    }
}

struct TypedChannel<C, F> {
    handler: F,
    /// The spec, by type alone, as in [`TypedCommand`].
    spec: PhantomData<fn() -> C>,
}

impl<C, F, Fut> ErasedChannel for TypedChannel<C, F>
where
    C: ChannelSpec,
    F: Fn(C::Input) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<PluginStream<C::Item>, C::Error>> + Send + 'static,
{
    fn max_input_bytes(&self) -> u32 {
        C::MAX_INPUT_BYTES
    }

    fn prepare(&self, input: Value) -> Result<BoxFuture<'static, Opened>, InputFailure> {
        let opening = (self.handler)(decode::<C::Input>(input)?);
        Ok(Box::pin(async move {
            match opening.await {
                Ok(items) => Opened::Items(items.filter_map(item_json::<C>).boxed()),
                Err(error) => ErrorValue::of(&error).map_or(Opened::Unserializable, Opened::Error),
            }
        }))
    }
}

/// An item of the channel `C` as JSON, or `None` when it does not
/// serialize.
fn item_json<C: ChannelSpec>(item: C::Item) -> std::future::Ready<Option<Value>> {
    std::future::ready(match serde_json::to_value(item) {
        Ok(json) => Some(json),
        Err(error) => {
            tracing::error!(%error, channel = C::NAME, "an item of a plugin's channel did not serialize and was left out");
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use futures_util::{FutureExt, StreamExt, stream};
    use serde::{Deserialize, Serialize};

    use super::{Answer, ErrorValue, Handlers, InputFailure, Opened};
    use crate::ipc::{ChannelSpec, CommandSpec, InputRefusal, PluginError, Validate};

    #[derive(Debug, Deserialize, specta::Type)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Greet {
        name: String,
    }

    impl Validate for Greet {
        fn validate(&self) -> Result<(), InputRefusal> {
            if self.name.is_empty() {
                return Err(InputRefusal::new("name"));
            }
            Ok(())
        }
    }

    #[derive(Debug, Serialize, specta::Type, thiserror::Error)]
    #[serde(tag = "code", rename_all = "snake_case")]
    enum GreetError {
        #[error("nobody is called {name}")]
        Stranger { name: String },
    }

    impl PluginError for GreetError {
        fn code(&self) -> &'static str {
            "greeter.stranger"
        }
    }

    enum GreetCommand {}

    impl CommandSpec for GreetCommand {
        const NAME: &'static str = "greet";
        const DOCS: &'static str = "Greets someone.";
        const MAX_INPUT_BYTES: u32 = 100;
        type Input = Greet;
        type Output = String;
        type Error = GreetError;
    }

    enum CountChannel {}

    impl ChannelSpec for CountChannel {
        const NAME: &'static str = "count";
        const DOCS: &'static str = "Counts to three.";
        type Input = ();
        type Item = u32;
        type Error = GreetError;
    }

    fn handlers() -> Handlers {
        Handlers::new()
            .command::<GreetCommand, _, _>(|input: Greet| async move {
                if input.name == "nobody" {
                    return Err(GreetError::Stranger { name: input.name });
                }
                Ok(format!("hello {}", input.name))
            })
            .channel::<CountChannel, _, _>(|()| async { Ok(stream::iter([1, 2, 3]).boxed()) })
    }

    fn answer(input: serde_json::Value) -> Result<Answer, InputFailure> {
        let handler = handlers().find_command("greet").unwrap();
        handler
            .prepare(input)
            .map(|call| call.now_or_never().unwrap())
    }

    #[test]
    fn a_command_decodes_its_input_and_serializes_its_output_and_its_error() {
        assert_eq!(
            answer(serde_json::json!({ "name": "Ada" })),
            Ok(Answer::Output(serde_json::json!("hello Ada")))
        );
        assert_eq!(
            answer(serde_json::json!({ "name": "nobody" })),
            Ok(Answer::Error(ErrorValue {
                code: "greeter.stranger",
                error: serde_json::json!({ "code": "stranger", "name": "nobody" }),
                description: "nobody is called nobody".to_owned(),
            }))
        );
    }

    #[test]
    fn an_input_with_an_unknown_field_or_that_breaks_a_rule_never_reaches_the_handler() {
        assert_eq!(
            answer(serde_json::json!({ "name": "Ada", "extra": 1 })).unwrap_err(),
            InputFailure::Undecodable { category: "data" }
        );
        assert_eq!(
            answer(serde_json::json!("Ada")).unwrap_err(),
            InputFailure::Undecodable { category: "data" }
        );
        assert_eq!(
            answer(serde_json::json!({ "name": "" })).unwrap_err(),
            InputFailure::Refused(InputRefusal::new("name"))
        );
    }

    #[test]
    fn a_channel_opens_its_items_as_json() {
        let handler = handlers().find_channel("count").unwrap();
        let opened = handler
            .prepare(serde_json::Value::Null)
            .unwrap()
            .now_or_never()
            .unwrap();

        let Opened::Items(items) = opened else {
            panic!("the channel did not open: {opened:?}");
        };
        let items: Vec<serde_json::Value> = items.collect().now_or_never().unwrap();
        assert_eq!(items, [1, 2, 3].map(|n| serde_json::json!(n)));
    }

    #[test]
    fn the_handlers_list_their_names_and_limits_and_find_nothing_else() {
        let handlers = handlers();

        assert_eq!(handlers.command_names(), ["greet"]);
        assert_eq!(handlers.channel_names(), ["count"]);
        assert_eq!(
            handlers.find_command("greet").unwrap().max_input_bytes(),
            100
        );
        assert_eq!(
            handlers.find_channel("count").unwrap().max_input_bytes(),
            64 * 1024
        );
        assert!(handlers.find_command("count").is_none());
        assert!(handlers.find_channel("greet").is_none());
        assert_eq!(
            format!("{handlers:?}"),
            r#"Handlers { commands: ["greet"], channels: ["count"] }"#
        );
    }
}
