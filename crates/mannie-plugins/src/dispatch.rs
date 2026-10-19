//! The dispatch of plugin calls (ADR-0034, decision 8): every call of a
//! plugin's command or channel goes through here, from the desktop app's
//! inlined Tauri plugins as from the tests, with no Tauri code.
//!
//! A call is refused, in this order, while Mannie quits, while the plugin is
//! not on, for a command or channel that the plugin does not have, for an
//! input larger than the spec allows, and for an input that does not decode
//! or that its checks refuse. Only then does the handler run, spawned into
//! the plugin's calls in flight. The desktop app answers `store_unavailable`
//! itself, since the host runs only once the store opened.
//!
//! Turning a plugin off refuses new calls, ends its channels, waits up to
//! five seconds for its calls in flight and aborts the rest. A call that was
//! aborted is answered with `plugin_off`, or `shutting_down` while Mannie
//! quits, because Tauri never answers a call whose answer was dropped
//! (spike S3).
//!
//! Each call leaves one log line with the plugin, the command, the outcome's
//! code and the duration, never the input or the output, and a decoding
//! error only with its category, because the decoder's message can quote
//! the input (T-6).

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{CommandError, SubscriptionEnd};
use mannie_plugin_sdk::ipc::{
    Answer, ChannelMessage, ErrorValue, Handlers, HostRefusal, InputFailure, Opened, PluginFailure,
    PluginStream,
};
use serde_json::Value;
use tokio::sync::{Notify, watch};
use tokio::task::{AbortHandle, JoinError, JoinHandle};
use tokio::time::Instant;

use crate::PluginHost;

/// How long turning a plugin off waits for its calls in flight before it
/// aborts them.
pub(crate) const STOP_GRACE: Duration = Duration::from_secs(5);

/// How long turning a plugin off waits for its aborted calls to end. A
/// handler that blocks its thread without awaiting cannot be aborted, and
/// the plugin then stops anyway, which the log records.
const ABORT_GRACE: Duration = Duration::from_secs(1);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only takes, replaces, inserts or removes.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why a call of a plugin's command or channel failed: a refusal of the
/// host, or the plugin's own error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallFailure {
    /// The host refused the call, or answered for a handler that did not.
    Host(HostRefusal),
    /// The plugin's error.
    Plugin(ErrorValue),
}

impl CallFailure {
    /// The code that the log records: the host's code, or the plugin's code
    /// for the log, such as `images.too_large`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Host(refusal) => refusal.code(),
            Self::Plugin(error) => error.code,
        }
    }

    /// The failure as it crosses IPC, a `CommandError<PluginFailure<_>>`
    /// (decision 9).
    pub fn to_command_error(&self) -> CommandError<PluginFailure<Value>> {
        match self {
            Self::Host(refusal) => CommandError {
                error: PluginFailure::Host(refusal.clone()),
                description: refusal.to_string(),
            },
            Self::Plugin(error) => CommandError {
                error: PluginFailure::Plugin(error.error.clone()),
                description: error.description.clone(),
            },
        }
    }

    /// The failure as JSON, as the UI receives it. A plugin error that is
    /// not an object tagged by its code, which the bindings exporter refuses,
    /// becomes `host_service_failed`.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self.to_command_error()).unwrap_or_else(|error| {
            tracing::error!(%error, code = self.code(), "a plugin's error did not serialize as an object tagged by its code");
            let failed = Self::Host(HostRefusal::HostServiceFailed).to_command_error();
            serde_json::to_value(failed).unwrap_or(Value::Null)
        })
    }
}

impl From<HostRefusal> for CallFailure {
    fn from(refusal: HostRefusal) -> Self {
        Self::Host(refusal)
    }
}

/// A plugin while it is on: the handlers of its start and the end of its
/// channels, which turning it off sets.
struct Live {
    handlers: Handlers,
    ends: watch::Sender<Option<SubscriptionEnd>>,
}

/// The calls of a plugin in flight, each with the handle that aborts it.
#[derive(Debug, Default)]
struct InFlight {
    tasks: Mutex<HashMap<u64, AbortHandle>>,
    /// Wakes the stop when the last call ended.
    emptied: Notify,
}

impl InFlight {
    /// Waits until no call is in flight or `deadline` passed, and says
    /// whether none is.
    async fn settle(&self, deadline: Instant) -> bool {
        loop {
            let emptied = self.emptied.notified();
            tokio::pin!(emptied);
            // Registered before the check, so a call that ends in between
            // still wakes this wait.
            emptied.as_mut().enable();
            if lock(&self.tasks).is_empty() {
                return true;
            }
            if tokio::time::timeout_at(deadline, emptied).await.is_err() {
                return lock(&self.tasks).is_empty();
            }
        }
    }

    /// Aborts every call in flight and returns how many there were.
    fn abort_all(&self) -> usize {
        let tasks = lock(&self.tasks);
        for task in tasks.values() {
            task.abort();
        }
        tasks.len()
    }
}

/// Takes a call out of the calls in flight when its task ends, also when
/// it is aborted, which drops it.
struct CallGuard {
    in_flight: Arc<InFlight>,
    id: u64,
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        let mut tasks = lock(&self.in_flight.tasks);
        tasks.remove(&self.id);
        if tasks.is_empty() {
            self.in_flight.emptied.notify_waiters();
        }
    }
}

/// The calls of one plugin: its handlers while it is on, and the calls in
/// flight.
#[derive(Default)]
pub(crate) struct Calls {
    live: Mutex<Option<Arc<Live>>>,
    in_flight: Arc<InFlight>,
    next: AtomicU64,
}

impl std::fmt::Debug for Calls {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let live = lock(&self.live).is_some();
        let in_flight = lock(&self.in_flight.tasks).len();
        formatter
            .debug_struct("Calls")
            .field("live", &live)
            .field("in_flight", &in_flight)
            .finish_non_exhaustive()
    }
}

impl Calls {
    /// Lets calls reach the handlers of a start that the core accepted.
    pub(crate) fn open(&self, handlers: Handlers) {
        let live = Live {
            handlers,
            ends: watch::Sender::new(None),
        };
        *lock(&self.live) = Some(Arc::new(live));
    }

    /// Refuses new calls, ends the plugin's channels with `end`, waits up to
    /// [`STOP_GRACE`] for the calls in flight and aborts the rest. Returns
    /// once none runs, or after [`ABORT_GRACE`] for calls that an abort does
    /// not reach.
    pub(crate) async fn close(&self, plugin: &str, end: SubscriptionEnd) {
        let Some(live) = lock(&self.live).take() else {
            return;
        };
        live.ends.send_replace(Some(end));
        if self.in_flight.settle(Instant::now() + STOP_GRACE).await {
            return;
        }
        let aborted = self.in_flight.abort_all();
        tracing::warn!(
            plugin,
            aborted,
            "calls of a plugin that is turning off did not end in time and were aborted"
        );
        if !self.in_flight.settle(Instant::now() + ABORT_GRACE).await {
            tracing::error!(
                plugin,
                "aborted calls of a plugin still run; the plugin stops anyway"
            );
        }
    }

    fn live(&self) -> Option<Arc<Live>> {
        lock(&self.live).clone()
    }

    /// Spawns `call` into the calls in flight, unless the start that `live`
    /// belongs to was turned off since the call was admitted.
    fn spawn<T: Send + 'static>(
        &self,
        live: &Arc<Live>,
        call: BoxFuture<'static, T>,
    ) -> Option<JoinHandle<T>> {
        // The handlers' lock is held while the call joins the calls in
        // flight, so turning off either refuses the call or waits for it.
        let current = lock(&self.live);
        let same_start = current
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, live));
        if !same_start {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let guard = CallGuard {
            in_flight: Arc::clone(&self.in_flight),
            id,
        };
        // The calls' lock is held until the handle is in, so a call that
        // ends at once takes itself out only after it was put in.
        let mut tasks = lock(&self.in_flight.tasks);
        let task = tokio::spawn(async move {
            let _guard = guard;
            call.await
        });
        tasks.insert(id, task.abort_handle());
        Some(task)
    }
}

/// Counts the bytes of a value as JSON and stops at the limit, without
/// keeping them.
struct Measure {
    bytes: usize,
    limit: usize,
}

impl io::Write for Measure {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self.bytes.saturating_add(bytes.len());
        if self.bytes > self.limit {
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Refuses an input larger than `max_bytes` as JSON. Tauri already parsed
/// the body, so the input is measured as it would be written.
fn check_size(input: &Value, max_bytes: u32) -> Result<(), CallFailure> {
    let mut measure = Measure {
        bytes: 0,
        limit: usize::try_from(max_bytes).unwrap_or(usize::MAX),
    };
    // Writing a JSON value fails only when the measure stops it.
    serde_json::to_writer(&mut measure, input)
        .map_err(|_too_large| CallFailure::Host(HostRefusal::InputTooLarge { max_bytes }))
}

fn plugin_off(plugin: &str) -> CallFailure {
    CallFailure::Host(HostRefusal::plugin_off(plugin))
}

/// The refusal of an input that did not reach the handler. A decoding
/// error is logged by its category alone.
fn refused_input(plugin: &str, name: &str, failure: InputFailure) -> CallFailure {
    let field = match failure {
        InputFailure::Undecodable { category } => {
            tracing::warn!(
                plugin,
                command = name,
                category,
                "the input of a plugin's call did not decode"
            );
            None
        }
        InputFailure::Refused(refusal) => Some(refusal.field),
        _other => None,
    };
    CallFailure::Host(HostRefusal::InvalidInput { field })
}

/// Logs the outcome of a call, without its input or output.
fn record(plugin: &str, name: &str, failure: Option<&CallFailure>, started: Instant) {
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match failure {
        None => tracing::info!(
            plugin,
            command = name,
            outcome = "ok",
            duration_ms,
            "a plugin's call ended"
        ),
        Some(failure) => tracing::warn!(
            plugin,
            command = name,
            outcome = failure.code(),
            duration_ms,
            "a plugin's call failed"
        ),
    }
}

/// The next message of a channel.
enum Next {
    Item(Value),
    Ended(SubscriptionEnd),
}

/// The plugin's items until the host ends the channel, then the host's end.
/// A stream of the plugin that ends before ends the channel with `lagged`,
/// so the UI subscribes again.
fn until_ended(
    items: PluginStream<Value>,
    ends: watch::Receiver<Option<SubscriptionEnd>>,
) -> PluginStream<ChannelMessage<Value>> {
    stream::unfold(Some((items, ends)), |state| async move {
        let (mut items, mut ends) = state?;
        let next = tokio::select! {
            biased;
            changed = ends.wait_for(Option::is_some) => Next::Ended(match changed {
                Ok(end) => (*end).unwrap_or(SubscriptionEnd::PluginOff),
                // The start's handlers are gone, so the plugin stopped.
                Err(_closed) => SubscriptionEnd::PluginOff,
            }),
            item = items.next() => item.map_or(Next::Ended(SubscriptionEnd::Lagged), Next::Item),
        };
        match next {
            Next::Item(item) => Some((ChannelMessage::Item { item }, Some((items, ends)))),
            Next::Ended(reason) => Some((ChannelMessage::Ended { reason }, None)),
        }
    })
    .boxed()
}

impl PluginHost {
    /// Calls the command `command` of the plugin `plugin` with `input`, as
    /// the plugin's UI does through its inlined Tauri plugin, and returns
    /// the output as JSON.
    pub async fn invoke(
        &self,
        plugin: &str,
        command: &str,
        input: Value,
    ) -> Result<Value, CallFailure> {
        let started = Instant::now();
        let outcome = self.invoke_now(plugin, command, input).await;
        record(plugin, command, outcome.as_ref().err(), started);
        outcome
    }

    /// Opens the channel `channel` of the plugin `plugin` with `input`. The
    /// stream carries the plugin's items as JSON and ends with the host's end
    /// message: `plugin_off` when the plugin stops, `shutdown` when Mannie
    /// quits.
    pub async fn open_channel(
        &self,
        plugin: &str,
        channel: &str,
        input: Value,
    ) -> Result<PluginStream<ChannelMessage<Value>>, CallFailure> {
        let started = Instant::now();
        let outcome = self.open_channel_now(plugin, channel, input).await;
        record(plugin, channel, outcome.as_ref().err(), started);
        outcome
    }

    async fn invoke_now(
        &self,
        plugin: &str,
        command: &str,
        input: Value,
    ) -> Result<Value, CallFailure> {
        let (calls, live) = self.admit(plugin)?;
        let handler = live
            .handlers
            .find_command(command)
            .ok_or(HostRefusal::UnknownCommand)?;
        check_size(&input, handler.max_input_bytes())?;
        let call = handler
            .prepare(input)
            .map_err(|failure| refused_input(plugin, command, failure))?;
        let task = calls.spawn(&live, call).ok_or_else(|| plugin_off(plugin))?;
        drop(live);
        match task.await {
            Ok(Answer::Output(output)) => Ok(output),
            Ok(Answer::Error(error)) => Err(CallFailure::Plugin(error)),
            Ok(_unserializable) => Err(HostRefusal::HostServiceFailed.into()),
            Err(ended) => Err(self.ended_early(plugin, command, &ended)),
        }
    }

    async fn open_channel_now(
        &self,
        plugin: &str,
        channel: &str,
        input: Value,
    ) -> Result<PluginStream<ChannelMessage<Value>>, CallFailure> {
        let (calls, live) = self.admit(plugin)?;
        let handler = live
            .handlers
            .find_channel(channel)
            .ok_or(HostRefusal::UnknownCommand)?;
        check_size(&input, handler.max_input_bytes())?;
        let opening = handler
            .prepare(input)
            .map_err(|failure| refused_input(plugin, channel, failure))?;
        // Subscribed before the channel opens, so turning off meanwhile
        // still ends it.
        let ends = live.ends.subscribe();
        let task = calls
            .spawn(&live, opening)
            .ok_or_else(|| plugin_off(plugin))?;
        drop(live);
        match task.await {
            Ok(Opened::Items(items)) => Ok(until_ended(items, ends)),
            Ok(Opened::Error(error)) => Err(CallFailure::Plugin(error)),
            Ok(_unserializable) => Err(HostRefusal::HostServiceFailed.into()),
            Err(ended) => Err(self.ended_early(plugin, channel, &ended)),
        }
    }

    /// The calls and the handlers of a plugin that is on, while Mannie does
    /// not quit.
    fn admit(&self, plugin: &str) -> Result<(&Calls, Arc<Live>), CallFailure> {
        if self.shutting_down() {
            return Err(HostRefusal::ShuttingDown.into());
        }
        let calls = self.calls(plugin).ok_or_else(|| plugin_off(plugin))?;
        let live = calls.live().ok_or_else(|| plugin_off(plugin))?;
        Ok((calls, live))
    }

    /// The answer for a call whose task ended without answering: aborted,
    /// because the plugin was turned off or Mannie quits, or panicked.
    fn ended_early(&self, plugin: &str, name: &str, ended: &JoinError) -> CallFailure {
        if ended.is_cancelled() {
            if self.shutting_down() {
                return HostRefusal::ShuttingDown.into();
            }
            return plugin_off(plugin);
        }
        tracing::error!(plugin, command = name, "a plugin's handler panicked");
        HostRefusal::HostServiceFailed.into()
    }
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::ipc::{ErrorValue, HostRefusal};
    use serde_json::json;

    use super::{CallFailure, check_size};

    #[test]
    fn an_input_is_measured_as_json_up_to_its_limit() {
        let input = json!({ "name": "abc" });
        let length = u32::try_from(input.to_string().len()).unwrap();

        assert_eq!(check_size(&input, length), Ok(()));
        assert_eq!(
            check_size(&input, length - 1),
            Err(CallFailure::Host(HostRefusal::InputTooLarge {
                max_bytes: length - 1
            }))
        );
    }

    #[test]
    fn a_failure_crosses_ipc_with_its_origin_and_code() {
        let host = CallFailure::Host(HostRefusal::plugin_off("images"));
        let plugin = CallFailure::Plugin(ErrorValue::new(
            "images.not_found",
            json!({ "code": "not_found" }),
            "the chosen file is no longer available",
        ));
        let odd = CallFailure::Plugin(ErrorValue::new("images.odd", json!("odd"), "odd"));

        assert_eq!(host.code(), "plugin_off");
        assert_eq!(plugin.code(), "images.not_found");
        assert_eq!(
            host.to_json(),
            json!({ "origin": "host", "code": "plugin_off", "plugin": "images", "description": "the plugin images is off" })
        );
        assert_eq!(
            plugin.to_json(),
            json!({ "origin": "plugin", "code": "not_found", "description": "the chosen file is no longer available" })
        );
        assert_eq!(
            odd.to_json(),
            json!({ "origin": "host", "code": "host_service_failed", "description": "the host could not complete the call" })
        );
    }
}
