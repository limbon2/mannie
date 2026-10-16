//! The master's connection to one worker (docs/features/workers/
//! pairing-and-link.md, the connection): TLS with the worker's pinned key,
//! HTTP/2 with keepalive, `Hello` with the versions and feature levels, and
//! a watch of the worker's tasks whose end shows that the link dropped. A
//! lost connection is tried again after a delay with jitter, from 1 second
//! up to 60.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use hyper_util::rt::TokioIo;
use mannie_link::codec::from_json;
use mannie_link::host::WorkerDetails;
use mannie_link::keys::KeyPin;
use mannie_link::levels::{self, Version};
use mannie_link::proto::session::HelloRequest;
use mannie_link::proto::session::session_service_client::SessionServiceClient;
use mannie_link::proto::tasks::WatchRequest;
use mannie_link::proto::tasks::tasks_service_client::TasksServiceClient;
use mannie_link::tls::{self, HANDSHAKE_TIMEOUT};
use mannie_link::worker::MAX_MESSAGE_BYTES;
use mannie_types::ids::TaskId;
use mannie_types::tasks::AssignedTaskState;
use rustls::ClientConfig;
use tokio::sync::broadcast;
use tonic::Code;
use tonic::transport::{Channel, Endpoint, Uri};

use crate::view::WorkerStatus;

/// How often HTTP/2 checks that an idle connection lives.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);

/// How long a keepalive may wait for its answer.
const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(10);

/// The first delay before a connection is tried again.
const FIRST_RETRY: Duration = Duration::from_secs(1);

/// The longest delay before a connection is tried again.
pub(crate) const LAST_RETRY: Duration = Duration::from_secs(60);

/// How many states a slow follower may fall behind before its watch ends.
const STATES_QUEUE: usize = 256;

/// The delay before the `failures`-th attempt after a failure: doubling
/// from one second, up to a minute, with a quarter of jitter either way so
/// the master's connections do not retry in step.
pub(crate) fn retry_delay(failures: u32, random: f64) -> Duration {
    let doubled = FIRST_RETRY.saturating_mul(1_u32 << failures.saturating_sub(1).min(6));
    let base = doubled.min(LAST_RETRY);
    let jitter = 0.75 + random.clamp(0.0, 1.0) / 2.0;
    base.mul_f64(jitter).min(LAST_RETRY)
}

/// Why a connection did not come up.
#[derive(Debug)]
pub(crate) enum ConnectError {
    /// The worker presented another key than the pinned one.
    KeyChanged,
    /// The worker answered, but no longer accepts this master's key.
    Unpaired,
    /// Nothing answered, or the link failed.
    Unreachable,
    /// The worker runs a version that this one does not work with.
    Incompatible(Box<Incompatible>),
}

/// What an incompatible worker said about itself.
#[derive(Debug)]
pub(crate) struct Incompatible {
    pub(crate) version: String,
    pub(crate) name: String,
    pub(crate) details: WorkerDetails,
}

/// Whether an error, or one of its sources, is the verifier's refusal of the
/// worker's key.
fn is_key_refusal(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(rustls::Error::InvalidCertificate(_reason)) =
            error.downcast_ref::<rustls::Error>()
        {
            return true;
        }
        // The source of an `io::Error` is the source of the error it wraps,
        // which skips the wrapped error itself: TLS's refusal arrives
        // wrapped in one.
        current = match error
            .downcast_ref::<io::Error>()
            .and_then(io::Error::get_ref)
        {
            Some(wrapped) => Some(wrapped),
            None => error.source(),
        };
    }
    false
}

/// A channel to a worker over the link's TLS, with the pin of the key that
/// the worker presented.
pub(crate) async fn channel(
    address: &str,
    config: Arc<ClientConfig>,
) -> Result<(Channel, KeyPin), ConnectError> {
    let seen: Arc<Mutex<Option<KeyPin>>> = Arc::default();
    let connector = {
        let address = address.to_owned();
        let seen = Arc::clone(&seen);
        tower::service_fn(move |_uri: Uri| {
            let address = address.clone();
            let config = Arc::clone(&config);
            let seen = Arc::clone(&seen);
            async move {
                let (stream, pin) = tls::connect(&address, config).await?;
                *seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(pin);
                Ok::<_, io::Error>(TokioIo::new(stream))
            }
        })
    };
    let channel = Endpoint::from_static("http://mannie-worker")
        .connect_timeout(HANDSHAKE_TIMEOUT)
        .http2_keep_alive_interval(KEEPALIVE_INTERVAL)
        .keep_alive_timeout(KEEPALIVE_TIMEOUT)
        .keep_alive_while_idle(true)
        .connect_with_connector(connector)
        .await
        .map_err(|error| {
            if is_key_refusal(&error) {
                ConnectError::KeyChanged
            } else {
                tracing::debug!(%error, "could not connect to a worker");
                ConnectError::Unreachable
            }
        })?;
    let pin = seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or(ConnectError::Unreachable)?;
    Ok((channel, pin))
}

/// What a worker said in `Hello`, once the master agreed to work with it.
/// Every package is at level 1 in this version, so no call checks the
/// agreed levels yet.
#[derive(Debug, Clone)]
pub(crate) struct Hello {
    pub(crate) name: String,
    pub(crate) details: WorkerDetails,
}

/// Says `Hello`, and checks the worker's version and levels.
pub(crate) async fn hello(channel: &Channel, master_version: &str) -> Result<Hello, ConnectError> {
    let mut client = SessionServiceClient::new(channel.clone())
        .max_decoding_message_size(MAX_MESSAGE_BYTES)
        .max_encoding_message_size(MAX_MESSAGE_BYTES);
    let response = client
        .hello(HelloRequest {
            version: master_version.to_owned(),
            levels: levels::levels(),
        })
        .await
        .map_err(|status| match status.code() {
            Code::PermissionDenied => ConnectError::Unpaired,
            _other => ConnectError::Unreachable,
        })?
        .into_inner();
    let details: WorkerDetails = from_json(&response.details_json, "the worker's details")
        .map_err(|_invalid| ConnectError::Unreachable)?;
    let agreed = levels::agree(&response.levels);
    let compatible = match (
        Version::parse(master_version),
        Version::parse(&response.version),
    ) {
        (Some(master), Some(worker)) => levels::compatible(master, worker),
        _ => false,
    };
    if !compatible || !levels::covers_every_package(&agreed) {
        return Err(ConnectError::Incompatible(Box::new(Incompatible {
            version: response.version,
            name: response.name,
            details,
        })));
    }
    Ok(Hello {
        name: response.name,
        details,
    })
}

/// A working connection to a worker: its channel, and the states of its
/// tasks for the runtime's follower.
#[derive(Debug)]
pub(crate) struct Connection {
    pub(crate) channel: Channel,
    states: broadcast::Sender<AssignedTaskState>,
    /// The latest state of each active task, for a follower that watches
    /// after the connection came up.
    active: Mutex<HashMap<TaskId, AssignedTaskState>>,
}

impl Connection {
    pub(crate) fn new(channel: Channel) -> Self {
        Self {
            channel,
            states: broadcast::Sender::new(STATES_QUEUE),
            active: Mutex::default(),
        }
    }

    fn active(&self) -> MutexGuard<'_, HashMap<TaskId, AssignedTaskState>> {
        self.active.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The states of the worker's tasks: the latest state of each active
    /// task, then every change. The stream ends when the connection ends or
    /// the follower falls too far behind; the follower then watches again
    /// and asks about each task that it follows.
    pub(crate) fn watch(&self) -> BoxStream<'static, AssignedTaskState> {
        let (snapshot, receiver) = {
            let active = self.active();
            let mut snapshot: Vec<AssignedTaskState> = active.values().cloned().collect();
            snapshot.sort_by_key(|state| state.version);
            (snapshot, self.states.subscribe())
        };
        let live = futures_util::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.ok().map(|state| (state, receiver))
        });
        futures_util::stream::iter(snapshot).chain(live).boxed()
    }

    /// Follows the worker's tasks until the link drops: the sentinel of the
    /// connection.
    pub(crate) async fn follow(&self) {
        let mut client = TasksServiceClient::new(self.channel.clone())
            .max_decoding_message_size(MAX_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_MESSAGE_BYTES);
        let mut stream = match client.watch(WatchRequest {}).await {
            Ok(response) => response.into_inner(),
            Err(status) => {
                tracing::debug!(%status, "could not watch a worker's tasks");
                return;
            }
        };
        loop {
            match stream.message().await {
                Ok(Some(message)) => {
                    let Ok(state) =
                        from_json::<AssignedTaskState>(&message.state_json, "a task's state")
                    else {
                        continue;
                    };
                    {
                        let mut active = self.active();
                        if state.state.is_active() {
                            active.insert(state.task_id.clone(), state.clone());
                        } else {
                            active.remove(&state.task_id);
                        }
                        // Sent under the lock, so a new watch never misses a
                        // state between its snapshot and its subscription.
                        let _receivers = self.states.send(state);
                    }
                }
                Ok(None) => return,
                Err(status) => {
                    tracing::debug!(%status, "the watch of a worker's tasks ended");
                    return;
                }
            }
        }
    }
}

/// The status that a failed connection shows.
pub(crate) fn status_of(error: &ConnectError) -> WorkerStatus {
    match error {
        ConnectError::KeyChanged => WorkerStatus::KeyChanged,
        ConnectError::Unpaired => WorkerStatus::Unpaired,
        ConnectError::Unreachable => WorkerStatus::Offline,
        ConnectError::Incompatible(_) => WorkerStatus::Incompatible,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::retry_delay;

    #[test]
    fn retries_double_from_a_second_up_to_a_minute_with_jitter() {
        assert_eq!(retry_delay(1, 0.5), Duration::from_secs(1));
        assert_eq!(retry_delay(2, 0.5), Duration::from_secs(2));
        assert_eq!(retry_delay(4, 0.5), Duration::from_secs(8));
        assert_eq!(retry_delay(30, 0.5), Duration::from_secs(60));
        assert_eq!(retry_delay(1, 0.0), Duration::from_millis(750));
        assert_eq!(retry_delay(1, 1.0), Duration::from_millis(1_250));
        assert!(retry_delay(30, 1.0) <= Duration::from_secs(60));
    }
}
