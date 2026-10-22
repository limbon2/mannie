//! The tool `start_ssh_command` (plugins/ssh/docs/README.md, the tool;
//! ADR-0041, decision 11): one command on a registered host that runs in the
//! background, past its call, until it ends on the host or is stopped.
//!
//! Its preparation returns the access of `run_ssh_command` in the
//! background, so the core's policy decides on it as on a local command in
//! the background. Its execution acts only when the host still matches the
//! authorized access and the core gave the call a ticket. It starts the
//! command as `run_ssh_command` does, waits until the host accepted it, and
//! hands the core the command through the service `background-commands`:
//! the stream of its lines, with the host's password replaced, and its stop.
//!
//! The command holds its session's lease while it runs, so idle closing
//! skips the session. Mannie closing the session, as when the owner edits or
//! removes the host, ends the command as its session closed. The stop asks
//! the host to end the command, TERM and then KILL, and closes its channel,
//! as the timeout of `run_ssh_command` does; a command that is dropped
//! before its end, such as one whose call the core let go, is ended the
//! same way.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::points::tools::{
    Tool, ToolAccess, ToolCall, ToolExecution, ToolRefusal, ToolResult, ToolSpec,
};
use mannie_plugin_sdk::services::background_commands::{
    BackgroundCommandsApi, RemoteCommand, RemoteEnding, RemoteOutput,
};
use russh::client::Msg;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Sig};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::watch;

use crate::hosts::{Connected, Hosts};
use crate::lines::StreamLines;
use crate::schema::parameters;
use crate::sessions::Lease;
use crate::store::HostRecord;
use crate::texts;
use crate::tool::{
    CLOSE_GRACE, KILL_GRACE, Running, check_command, open_channel, ready_host, remote_access,
    signal_text,
};

/// The tool's name.
pub const START_SSH_COMMAND: &str = "start_ssh_command";

/// How long the plugin waits for the host to accept a command.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(30);

/// The arguments of `start_ssh_command`. The field documentation is what the
/// model reads about each argument.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Arguments {
    /// The name of a registered SSH host, as the list of SSH hosts gives it.
    host: String,
    /// The command, as the host's shell reads it: at most 100,000 bytes.
    command: String,
}

/// The tool.
#[derive(Debug, Clone)]
pub(crate) struct StartSshCommand {
    hosts: Hosts,
    commands: Arc<dyn BackgroundCommandsApi>,
}

impl StartSshCommand {
    pub(crate) fn new(hosts: Hosts, commands: Arc<dyn BackgroundCommandsApi>) -> Self {
        Self { hosts, commands }
    }
}

impl Tool for StartSshCommand {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            START_SSH_COMMAND,
            texts::START_TOOL_DESCRIPTION,
            parameters::<Arguments>(),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let prepared = check(&self.hosts, &call.arguments)
            .map(|(host, command)| vec![access(&host, &command)]);
        Box::pin(std::future::ready(prepared))
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let hosts = self.hosts.clone();
        let commands = Arc::clone(&self.commands);
        Box::pin(async move { start(&hosts, commands.as_ref(), &execution).await })
    }
}

/// Reads and checks a call's arguments against the hosts as they are.
fn check(hosts: &Hosts, arguments: &str) -> Result<(HostRecord, String), ToolRefusal> {
    let arguments: Arguments = serde_json::from_str(arguments)
        .map_err(|error| ToolRefusal::new(texts::invalid_start_arguments(&error.to_string())))?;
    check_command(&arguments.command)?;
    let host = ready_host(hosts, &arguments.host)?;
    Ok((host, arguments.command))
}

/// The command on the host in the background, as the core's policy decides
/// on it.
fn access(host: &HostRecord, command: &str) -> ToolAccess {
    ToolAccess::RemoteCommand(remote_access(host, command).in_background())
}

/// Starts an authorized call's command and hands it to the core.
async fn start(
    hosts: &Hosts,
    commands: &dyn BackgroundCommandsApi,
    execution: &ToolExecution,
) -> ToolResult {
    let (host, command) = match check(hosts, &execution.arguments) {
        Ok(checked) => checked,
        Err(refusal) => return ToolResult::new(texts::not_run(&refusal.detail)),
    };
    // The core authorized these exact accesses. A host whose name, endpoint
    // or key changed since is not the one that the owner allowed.
    if !execution.authorized.contains(&access(&host, &command)) {
        return ToolResult::new(texts::not_run(texts::HOST_CHANGED));
    }
    let Some(ticket) = execution.background.clone() else {
        return ToolResult::new(texts::not_run(texts::NO_TICKET));
    };
    tracing::info!(host = %host.id, "an SSH command starts in the background");
    tracing::debug!(host = %host.id, command = %command, "the command of an SSH call in the background");
    let (connected, channel) = match open_channel(hosts, &host).await {
        Ok(opened) => opened,
        Err(reason) => return ToolResult::new(texts::not_run(reason)),
    };
    let (mut reader, writer) = channel.split();
    if writer.exec(true, command.as_bytes()).await.is_err() {
        return ToolResult::new(texts::not_run(texts::EXEC_REFUSED));
    }
    // The command reads no input.
    if let Err(error) = writer.eof().await {
        tracing::debug!(%error, "the end of a command's input did not reach the host");
    }
    // From here on, a call that is dropped asks the host to end the command.
    let mut running = Running::new(writer);
    let early = match accepted(&mut reader).await {
        Ok(early) => early,
        Err(Refused::Refused) => {
            running.close_refused().await;
            return ToolResult::new(texts::not_run(texts::EXEC_REFUSED));
        }
        Err(Refused::NoAnswer) => {
            drop(running);
            return ToolResult::new(texts::not_run(texts::TIMED_OUT));
        }
    };
    let Some(writer) = running.take() else {
        return ToolResult::new(texts::not_run(texts::EXEC_REFUSED));
    };
    let command = SshCommand::new(reader, early, writer, connected);
    match commands.start(ticket, Box::new(command)).await {
        Ok(started) => {
            tracing::info!(host = %host.id, number = started.number, "an SSH command runs in the background");
            ToolResult::new(texts::started_in_background(started.number, &host))
        }
        Err(error) => {
            tracing::warn!(host = %host.id, %error, "the core did not take an SSH command, so it was ended");
            ToolResult::new(texts::not_started(&error)).with_failure(texts::NOT_STARTED)
        }
    }
}

/// Why the host did not accept a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refused {
    /// It refused it, or closed the channel first.
    Refused,
    /// It did not answer in time.
    NoAnswer,
}

/// Waits until the host accepted the command, and returns the messages that
/// came before its answer, which the command's output reads first.
async fn accepted(reader: &mut ChannelReadHalf) -> Result<VecDeque<ChannelMsg>, Refused> {
    let mut early = VecDeque::new();
    let answer = async {
        loop {
            match reader.wait().await {
                Some(ChannelMsg::Success) => return Ok(()),
                None | Some(ChannelMsg::Failure | ChannelMsg::Close) => {
                    return Err(Refused::Refused);
                }
                Some(message) => early.push_back(message),
            }
        }
    };
    match tokio::time::timeout(ACCEPT_TIMEOUT, answer).await {
        Ok(Ok(())) => Ok(early),
        Ok(Err(refused)) => Err(refused),
        Err(_elapsed) => Err(Refused::NoAnswer),
    }
}

/// A command that runs on a host in the background, which the plugin hands
/// the core.
pub(crate) struct SshCommand {
    /// The output, until the core takes it.
    output: Option<Output>,
    /// The channel, until the command ends or is stopped.
    writer: Option<ChannelWriteHalf<Msg>>,
    /// How the command ended, once its channel closed.
    ended: watch::Receiver<Option<RemoteEnding>>,
    /// The session's lease, which the command holds while it runs.
    lease: Arc<Lease>,
}

impl fmt::Debug for SshCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SshCommand")
            .field("lease", &self.lease)
            .field("ended", &*self.ended.borrow())
            .finish_non_exhaustive()
    }
}

impl SshCommand {
    fn new(
        reader: ChannelReadHalf,
        early: VecDeque<ChannelMsg>,
        writer: ChannelWriteHalf<Msg>,
        connected: Connected,
    ) -> Self {
        let Connected { lease, password } = connected;
        let lease = Arc::new(lease);
        let (ended, ended_receiver) = watch::channel(None);
        let output = Output {
            reader,
            early,
            stdout: StreamLines::new(password.clone()),
            stderr: StreamLines::new(password),
            reported: None,
            queued: VecDeque::new(),
            ended,
            lease: Arc::clone(&lease),
            closed: false,
        };
        Self {
            output: Some(output),
            writer: Some(writer),
            ended: ended_receiver,
            lease,
        }
    }
}

impl RemoteCommand for SshCommand {
    fn output(&mut self) -> BoxStream<'static, RemoteOutput> {
        let Some(output) = self.output.take() else {
            return futures_util::stream::empty().boxed();
        };
        futures_util::stream::unfold(output, |mut output| async move {
            output.next().await.map(|item| (item, output))
        })
        .boxed()
    }

    fn stop(mut self: Box<Self>) -> BoxFuture<'static, RemoteEnding> {
        let writer = self.writer.take();
        let ended = self.ended.clone();
        let lease = Arc::clone(&self.lease);
        Box::pin(async move {
            let ending = match writer {
                Some(writer) => end(&writer, ended).await,
                None => ended.borrow().clone().unwrap_or(RemoteEnding::Closed),
            };
            // The session stays leased until the command ended.
            drop(lease);
            ending
        })
    }
}

impl Drop for SshCommand {
    fn drop(&mut self) {
        let has_ended = self.ended.borrow().is_some();
        let Some(writer) = self.writer.take() else {
            return;
        };
        if has_ended {
            return;
        }
        let ended = self.ended.clone();
        let lease = Arc::clone(&self.lease);
        tokio::spawn(async move {
            end(&writer, ended).await;
            drop(lease);
        });
    }
}

/// Asks the host to end a command, TERM and then KILL, each with a grace for
/// its channel to close, then closes the channel, and says how the command
/// ended. A server that ignores signals still loses the channel.
async fn end(
    writer: &ChannelWriteHalf<Msg>,
    mut ended: watch::Receiver<Option<RemoteEnding>>,
) -> RemoteEnding {
    if let Err(error) = writer.signal(Sig::TERM).await {
        tracing::debug!(%error, "TERM did not reach the host");
    }
    if let Some(ending) = ended_within(&mut ended, KILL_GRACE).await {
        return ending;
    }
    if let Err(error) = writer.signal(Sig::KILL).await {
        tracing::debug!(%error, "KILL did not reach the host");
    }
    if let Some(ending) = ended_within(&mut ended, CLOSE_GRACE).await {
        return ending;
    }
    if let Err(error) = writer.close().await {
        tracing::debug!(%error, "the channel of a command was closed already");
    }
    RemoteEnding::Closed
}

/// How the command ended, when its channel closes within `grace`. Once the
/// output was let go, the end is not known, and the grace passes all the
/// same.
async fn ended_within(
    ended: &mut watch::Receiver<Option<RemoteEnding>>,
    grace: Duration,
) -> Option<RemoteEnding> {
    // The watch's guard stays inside the wait, so the future stays `Send`.
    let waiting = async {
        ended
            .wait_for(Option::is_some)
            .await
            .ok()
            .map(|ending| ending.clone())
    };
    match tokio::time::timeout(grace, waiting).await {
        Ok(Some(ending)) => ending,
        Ok(None) => {
            tokio::time::sleep(grace).await;
            None
        }
        Err(_elapsed) => None,
    }
}

/// The output of a command: its channel's messages, as lines and its end.
struct Output {
    reader: ChannelReadHalf,
    /// The messages that came before the host accepted the command.
    early: VecDeque<ChannelMsg>,
    stdout: StreamLines,
    stderr: StreamLines,
    /// What the host reported of the command's end.
    reported: Option<RemoteEnding>,
    /// What the stream hands on before it reads again.
    queued: VecDeque<RemoteOutput>,
    /// Tells the stop how the command ended, once its channel closed.
    ended: watch::Sender<Option<RemoteEnding>>,
    lease: Arc<Lease>,
    closed: bool,
}

impl Output {
    /// The next lines, or the command's end, or `None` after the end.
    async fn next(&mut self) -> Option<RemoteOutput> {
        loop {
            if let Some(item) = self.queued.pop_front() {
                return Some(item);
            }
            if self.closed {
                return None;
            }
            let message = match self.early.pop_front() {
                Some(message) => Some(message),
                None => self.reader.wait().await,
            };
            self.read(message);
        }
    }

    /// Keeps what a message of the channel says.
    fn read(&mut self, message: Option<ChannelMsg>) {
        match message {
            Some(ChannelMsg::Data { data }) => {
                let lines = self.stdout.push(&data);
                self.queue(lines);
            }
            Some(ChannelMsg::ExtendedData { data, ext: 1 }) => {
                let lines = self.stderr.push(&data);
                self.queue(lines);
            }
            Some(ChannelMsg::ExitStatus { exit_status }) => {
                self.reported = Some(RemoteEnding::Exited {
                    status: exit_status,
                });
            }
            Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                self.reported = Some(RemoteEnding::Signaled {
                    signal: signal_text(&signal_name),
                });
            }
            None | Some(ChannelMsg::Close) => self.close(),
            Some(_other) => {}
        }
    }

    fn queue(&mut self, lines: Vec<String>) {
        if !lines.is_empty() {
            self.queued.push_back(RemoteOutput::lines(lines));
        }
    }

    /// Hands on the last lines and how the command ended.
    fn close(&mut self) {
        let mut lines = self.stdout.finish();
        lines.extend(self.stderr.finish());
        self.queue(lines);
        let ending = self.reported.take().unwrap_or_else(|| {
            if self.lease.closed_by_mannie() {
                RemoteEnding::SessionClosed
            } else {
                RemoteEnding::Closed
            }
        });
        self.ended.send_replace(Some(ending.clone()));
        self.queued.push_back(RemoteOutput::ended(ending));
        self.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Arguments;
    use crate::schema::parameters;

    #[test]
    fn the_schema_requires_the_host_and_the_command_only() {
        let schema = parameters::<Arguments>();

        assert_eq!(schema["required"], json!(["command", "host"]));
        assert_eq!(schema["additionalProperties"], json!(false));
        assert!(schema["properties"].get("timeout_seconds").is_none());
    }
}
