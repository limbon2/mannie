//! The runner of a server (`servers.rs`): the one task that owns a
//! server's connection. It connects, lists the tools and the resources,
//! lists them again when the server announces a change, and connects again
//! after a connection ended, at most [`MAX_RESTARTS`] times in
//! [`RESTART_WINDOW`], after which the server shows as failed until the
//! owner restarts it. Every change that it makes carries its generation, so a
//! runner that the owner replaced changes nothing.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mannie_plugin_sdk::contracts::UtcMillis;
use mannie_plugin_sdk::services::local_servers::{LocalServerError, ServerEnding};
use rmcp::service::{Peer, RoleClient};
use tokio::sync::{mpsc, watch};

use super::{
    Entry, LOCAL_START_TIMEOUT, MAX_RESTARTS, Prepared, REMOTE_START_TIMEOUT, RESTART_WINDOW,
    Servers,
};
use crate::client::{self, Change, ConnectError, Handler};
use crate::commands::{LogEvent, ServerFailure, ServerStatus};
use crate::store::StoredEndpoint;

/// How long a local server gets to end after its input closed, before the
/// core ends its tree.
const STOP_GRACE: Duration = Duration::from_secs(2);

/// How long the close of a connection may take.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

/// The announcements of changes that wait for a runner.
const CHANGES_QUEUE: usize = 8;

/// How a runner's connection ended.
enum Ending {
    /// The owner or the plugin stopped it.
    Stopped,
    /// The server ended it.
    Ended,
}

/// A server's connection, from its first connection to its stop, in one
/// task.
pub(super) async fn run(
    servers: Servers,
    server_id: String,
    generation: u64,
    mut stop: watch::Receiver<bool>,
) {
    let task = Task {
        servers,
        server_id,
        generation,
    };
    loop {
        let Some(open) = task.open(&mut stop).await else {
            return;
        };
        let Some(delay) = task.serve(open, &mut stop).await else {
            return;
        };
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            _stopped = stop.changed() => return,
        }
    }
}

/// What a runner's task works on: one server, in one generation.
struct Task {
    servers: Servers,
    server_id: String,
    generation: u64,
}

/// A connection that a runner follows.
struct Open {
    connected: client::Connected,
    peer: Peer<RoleClient>,
    announced: mpsc::Receiver<Change>,
}

impl Task {
    /// Changes what lives of the server, when the task is still its
    /// runner's.
    fn update(&self, change: impl FnOnce(&mut Entry, UtcMillis)) -> bool {
        self.servers
            .update_live(&self.server_id, self.generation, change)
    }

    /// Connects to the server and lists it, or records why not, which ends
    /// the runner.
    async fn open(&self, stop: &mut watch::Receiver<bool>) -> Option<Open> {
        self.update(|entry, now| {
            entry.live.status = ServerStatus::Starting;
            entry.live.log.event(now, LogEvent::Connecting);
        });
        let prepared = tokio::select! {
            prepared = self.servers.target(&self.server_id, self.generation) => prepared,
            _stopped = stop.changed() => return None,
        };
        let (target, record, redactions) = match prepared {
            Ok(prepared) => prepared,
            Err(Prepared::Removed) => return None,
            Err(Prepared::SignInNeeded) => {
                self.update(|entry, _now| {
                    entry.live.status = ServerStatus::SignInNeeded;
                    entry.live.runner = None;
                });
                return None;
            }
            Err(Prepared::Failed(failure)) => {
                fail(&self.servers, &self.server_id, self.generation, failure);
                return None;
            }
        };
        self.update(|entry, _now| entry.live.redactions = Arc::clone(&redactions));
        let timeout = match record.endpoint {
            StoredEndpoint::Program { .. } => LOCAL_START_TIMEOUT,
            StoredEndpoint::Url { .. } => REMOTE_START_TIMEOUT,
        };
        let (changes, announced) = mpsc::channel(CHANGES_QUEUE);
        let handler = Handler::new(changes.clone());
        let connected = tokio::select! {
            connected = client::connect(&target, record.era, &handler, timeout) => connected,
            _stopped = stop.changed() => return None,
        };
        let connected = match connected {
            Ok(connected) => connected,
            Err(error) => {
                connect_failed(
                    &self.servers,
                    &self.server_id,
                    self.generation,
                    error,
                    timeout,
                );
                return None;
            }
        };
        if record.era != Some(connected.era)
            && let Err(error) = self
                .servers
                .inner
                .store
                .set_era(&self.server_id, connected.era)
                .await
        {
            tracing::debug!(server = %self.server_id, error.code = error.code(), "the era of an MCP server was not stored");
        }
        let peer = connected.service.peer().clone();
        self.list(connected, peer, changes, announced).await
    }

    /// Lists a new connection's tools and resources and marks the server as
    /// connected. A server that does not list its tools fails.
    async fn list(
        &self,
        connected: client::Connected,
        peer: Peer<RoleClient>,
        changes: mpsc::Sender<Change>,
        announced: mpsc::Receiver<Change>,
    ) -> Option<Open> {
        let info = connected.info.clone();
        let tools = if info.tools {
            client::list_tools(&peer).await
        } else {
            Ok(Vec::new())
        };
        let Ok(tools) = tools else {
            if let Some(process) = connected.process {
                process.stop(Duration::ZERO).await;
            }
            fail(
                &self.servers,
                &self.server_id,
                self.generation,
                ServerFailure::Protocol,
            );
            return None;
        };
        self.servers
            .record_tools(&self.server_id, self.generation, tools)
            .await;
        if info.resources {
            self.servers
                .record_resources(&self.server_id, self.generation, &peer)
                .await;
        }
        let version = info.version.clone();
        self.update(|entry, now| {
            entry.live.status = ServerStatus::Connected;
            entry.live.failure = None;
            entry.live.peer = Some(peer.clone());
            entry.live.info = Some(info.clone());
            entry.live.log.event(now, LogEvent::Connected { version });
        });
        client::follow_changes(peer.clone(), connected.era, &info, changes);
        tracing::info!(server = %self.server_id, era = ?connected.era, "an MCP server connected");
        Some(Open {
            connected,
            peer,
            announced,
        })
    }

    /// Follows a connection until it ends or the runner stops, listing the
    /// server again after each change that it announces. Returns the pause
    /// before the next connection, or `None` when the runner ends.
    async fn serve(&self, open: Open, stop: &mut watch::Receiver<bool>) -> Option<Duration> {
        let Open {
            connected,
            peer,
            mut announced,
        } = open;
        let client::Connected {
            service, process, ..
        } = connected;
        let mut cancel = Some(service.cancellation_token());
        let mut waiting = std::pin::pin!(service.waiting());
        let ending = loop {
            tokio::select! {
                _reason = &mut waiting => break Ending::Ended,
                Some(change) = announced.recv() => self.relist(change, &peer).await,
                _stopped = stop.changed() => {
                    if let Some(cancel) = cancel.take() {
                        cancel.cancel();
                    }
                    if tokio::time::timeout(CLOSE_TIMEOUT, &mut waiting).await.is_err() {
                        tracing::debug!(server = %self.server_id, "the connection of an MCP server closed late");
                    }
                    break Ending::Stopped;
                }
            }
        };
        let exit = match process {
            Some(process) => process.stop(STOP_GRACE).await,
            None => None,
        };
        if matches!(ending, Ending::Stopped) {
            self.update(|entry, now| {
                entry.live.status = ServerStatus::Off;
                entry.live.disconnect();
                entry.live.log.event(now, LogEvent::Stopped);
            });
            return None;
        }
        let event = match exit {
            Some(ServerEnding::Exited { code }) => LogEvent::Exited {
                exit_code: Some(code),
            },
            Some(_ended) => LogEvent::Exited { exit_code: None },
            None => LogEvent::Disconnected,
        };
        tracing::warn!(server = %self.server_id, "the connection of an MCP server ended");
        restart_delay(&self.servers, &self.server_id, self.generation, event)
    }

    /// Lists what a server announced as changed.
    async fn relist(&self, change: Change, peer: &Peer<RoleClient>) {
        match change {
            Change::Tools => match client::list_tools(peer).await {
                Ok(tools) => {
                    self.servers
                        .record_tools(&self.server_id, self.generation, tools)
                        .await;
                }
                Err(error) => {
                    tracing::debug!(server = %self.server_id, %error, "an MCP server did not list its changed tools");
                }
            },
            Change::Resources => {
                self.servers
                    .record_resources(&self.server_id, self.generation, peer)
                    .await;
            }
        }
    }
}

/// Records a connection that ended and returns the pause before the next
/// one, or `None` when the server ended too often, which fails it.
fn restart_delay(
    servers: &Servers,
    server_id: &str,
    generation: u64,
    event: LogEvent,
) -> Option<Duration> {
    let mut delay = None;
    servers.update_live(server_id, generation, |entry, now| {
        entry.live.disconnect();
        entry.live.log.event(now, event);
        let started = Instant::now();
        while entry
            .live
            .restarts
            .front()
            .is_some_and(|ended| started.duration_since(*ended) > RESTART_WINDOW)
        {
            entry.live.restarts.pop_front();
        }
        if entry.live.restarts.len() >= MAX_RESTARTS {
            let restarts = u32::try_from(entry.live.restarts.len()).unwrap_or(u32::MAX);
            let failure = ServerFailure::Crashed { restarts };
            entry.live.status = ServerStatus::Failed;
            entry.live.failure = Some(failure.clone());
            entry.live.runner = None;
            entry.live.log.event(now, LogEvent::Failed { failure });
            return;
        }
        let pause = Duration::from_secs(1 << entry.live.restarts.len());
        entry.live.restarts.push_back(started);
        entry.live.status = ServerStatus::Starting;
        entry.live.log.event(
            now,
            LogEvent::Restarting {
                seconds: u32::try_from(pause.as_secs()).unwrap_or(u32::MAX),
            },
        );
        delay = Some(pause);
    });
    delay
}

/// Marks a server as failed, which ends its runner.
fn fail(servers: &Servers, server_id: &str, generation: u64, failure: ServerFailure) {
    tracing::warn!(server = %server_id, ?failure, "an MCP server failed");
    servers.update_live(server_id, generation, |entry, now| {
        entry.live.status = ServerStatus::Failed;
        entry.live.failure = Some(failure.clone());
        entry.live.runner = None;
        entry.live.disconnect();
        entry.live.log.event(now, LogEvent::Failed { failure });
    });
}

/// Records a failed connection.
fn connect_failed(
    servers: &Servers,
    server_id: &str,
    generation: u64,
    error: ConnectError,
    timeout: Duration,
) {
    let status = match &error {
        ConnectError::SignInNeeded => Some(ServerStatus::SignInNeeded),
        ConnectError::AccessNeeded => Some(ServerStatus::AccessNeeded),
        _other => None,
    };
    if let Some(status) = status {
        servers.update_live(server_id, generation, |entry, _now| {
            entry.live.status = status;
            entry.live.runner = None;
            entry.live.disconnect();
        });
        return;
    }
    let failure = match error {
        ConnectError::Local(error) => local_failure(error),
        ConnectError::Exited(exit_code) => ServerFailure::Exited { exit_code },
        ConnectError::TimedOut => ServerFailure::TimedOut {
            seconds: u32::try_from(timeout.as_secs()).unwrap_or(u32::MAX),
        },
        ConnectError::Unreachable => ServerFailure::Unreachable,
        ConnectError::HttpStatus(status) => ServerFailure::HttpStatus { status },
        ConnectError::Protocol | ConnectError::SignInNeeded | ConnectError::AccessNeeded => {
            ServerFailure::Protocol
        }
    };
    fail(servers, server_id, generation, failure);
}

/// What a refused start of a local server means for the owner.
fn local_failure(error: LocalServerError) -> ServerFailure {
    match error {
        LocalServerError::ProgramNotFound { .. } => ServerFailure::ProgramNotFound,
        LocalServerError::ProgramRefused { .. } => ServerFailure::ProgramRefused,
        LocalServerError::WorkingDirectoryInvalid => ServerFailure::WorkingDirectoryInvalid,
        LocalServerError::ArgumentRefused => ServerFailure::ArgumentRefused,
        LocalServerError::VariableRefused { name } => ServerFailure::VariableRefused { name },
        LocalServerError::TooMany { limit } => ServerFailure::TooManyServers { limit },
        _other => ServerFailure::SpawnFailed,
    }
}
