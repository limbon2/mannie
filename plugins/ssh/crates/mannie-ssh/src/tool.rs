//! The tool `run_ssh_command` (plugins/ssh/docs/README.md, the tool;
//! ADR-0036): one command on a registered host, as an operation that the
//! core's policy decides on and its journal records.
//!
//! Prepare reads only the plugin's list of hosts and never touches the
//! network (T-17): it checks the arguments and returns the command on the
//! host as a remote-command access, with the host's endpoint and the
//! fingerprint of its pinned key. Execute acts only when the host still
//! matches the access that the core authorized, logs in, runs the command
//! without a terminal, and captures its output as `run_command` does.
//!
//! The core cancels a call by dropping its future (ADR-0034, decision 3). A
//! guard then asks the host to end the command, TERM and then KILL, and
//! closes its channel, as the timeout does. Not every server honors signals
//! (OpenSSH does since 7.9), so the result of a timeout says that processes
//! may still run.
//!
//! The tool `start_ssh_command` (`background.rs`) shares the checks, the
//! access, the channel and the guard.

use std::time::{Duration, Instant};

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::points::tools::{
    RemoteCommandAccess, Tool, ToolAccess, ToolCall, ToolExecution, ToolRefusal, ToolResult,
    ToolSpec,
};
use russh::client::Msg;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Sig};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::commands::{HostStatus, SshError};
use crate::hosts::{Connected, Hosts};
use crate::output::{Capture, Ending, Finished, Room, render};
use crate::schema::parameters;
use crate::store::HostRecord;
use crate::texts;

/// The tool's name.
pub const RUN_SSH_COMMAND: &str = "run_ssh_command";

/// The most bytes of a command.
const MAX_COMMAND_BYTES: usize = 100_000;

/// The timeout of a call that names none.
const DEFAULT_TIMEOUT_SECONDS: u32 = 60;

/// The longest timeout.
const MAX_TIMEOUT_SECONDS: u32 = 600;

/// How long a command gets to end after TERM before KILL.
pub(crate) const KILL_GRACE: Duration = Duration::from_millis(500);

/// How long the plugin waits for the channel to close after KILL.
pub(crate) const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// The arguments of `run_ssh_command`. The field documentation is what the
/// model reads about each argument.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Arguments {
    /// The name of a registered SSH host, as the list of SSH hosts gives it.
    host: String,
    /// The command, as the host's shell reads it: at most 100,000 bytes.
    command: String,
    /// Seconds before the command is ended, from 1 to 600; 60 when null.
    #[serde(default)]
    timeout_seconds: Option<u32>,
}

/// A call whose arguments passed the checks.
#[derive(Debug)]
struct Checked {
    host: HostRecord,
    command: String,
    timeout_seconds: u32,
}

/// The tool.
#[derive(Debug, Clone)]
pub(crate) struct RunSshCommand {
    hosts: Hosts,
}

impl RunSshCommand {
    pub(crate) fn new(hosts: Hosts) -> Self {
        Self { hosts }
    }
}

impl Tool for RunSshCommand {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            RUN_SSH_COMMAND,
            texts::TOOL_DESCRIPTION,
            parameters::<Arguments>(),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let prepared = check(&self.hosts, &call.arguments)
            .map(|checked| vec![access(&checked.host, &checked.command)]);
        Box::pin(std::future::ready(prepared))
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let hosts = self.hosts.clone();
        Box::pin(async move { run(&hosts, &execution).await })
    }
}

/// Reads and checks a call's arguments against the hosts as they are.
fn check(hosts: &Hosts, arguments: &str) -> Result<Checked, ToolRefusal> {
    let arguments: Arguments = serde_json::from_str(arguments)
        .map_err(|error| ToolRefusal::new(texts::invalid_arguments(&error.to_string())))?;
    check_command(&arguments.command)?;
    let timeout_seconds = arguments.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    if !(1..=MAX_TIMEOUT_SECONDS).contains(&timeout_seconds) {
        return Err(ToolRefusal::new(texts::TIMEOUT_OUT_OF_RANGE));
    }
    Ok(Checked {
        host: ready_host(hosts, &arguments.host)?,
        command: arguments.command,
        timeout_seconds,
    })
}

/// Checks a command's text: not blank, within its size and without NUL,
/// which SSH cannot carry.
pub(crate) fn check_command(command: &str) -> Result<(), ToolRefusal> {
    if command.trim().is_empty() {
        return Err(ToolRefusal::new(texts::BLANK_COMMAND));
    }
    if command.len() > MAX_COMMAND_BYTES {
        return Err(ToolRefusal::new(texts::LONG_COMMAND));
    }
    if command.contains('\0') {
        return Err(ToolRefusal::new(texts::NUL_IN_COMMAND));
    }
    Ok(())
}

/// The host with this name, when it can run commands.
pub(crate) fn ready_host(hosts: &Hosts, name: &str) -> Result<HostRecord, ToolRefusal> {
    let Some(host) = hosts.by_name(name) else {
        return Err(ToolRefusal::new(texts::unknown_host(
            name,
            &hosts.records(),
        )));
    };
    match host.status() {
        HostStatus::Ready => {}
        HostStatus::KeyNotChecked => {
            return Err(ToolRefusal::new(texts::key_not_checked(&host.name)));
        }
        HostStatus::KeyChanged => return Err(ToolRefusal::new(texts::key_changed(&host.name))),
        HostStatus::PasswordMissing => {
            return Err(ToolRefusal::new(texts::password_missing(&host.name)));
        }
    }
    Ok(host)
}

/// The command on the host, as the core's policy decides on it.
fn access(host: &HostRecord, command: &str) -> ToolAccess {
    ToolAccess::RemoteCommand(remote_access(host, command))
}

/// The command on the host, with the host's ID as the target, its name as
/// the label, its endpoint and the fingerprint of its trusted key.
pub(crate) fn remote_access(host: &HostRecord, command: &str) -> RemoteCommandAccess {
    let identity = host
        .key
        .as_ref()
        .map(|key| key.fingerprint.clone())
        .unwrap_or_default();
    RemoteCommandAccess::new(
        host.id.clone(),
        host.name.clone(),
        host.endpoint(),
        identity,
        command.to_owned(),
    )
}

/// Runs an authorized call.
async fn run(hosts: &Hosts, execution: &ToolExecution) -> ToolResult {
    let checked = match check(hosts, &execution.arguments) {
        Ok(checked) => checked,
        Err(refusal) => return ToolResult::new(texts::not_run(&refusal.detail)),
    };
    // The core authorized these exact accesses. A host whose name, endpoint
    // or key changed since is not the one that the owner allowed.
    let expected = access(&checked.host, &checked.command);
    if !execution.authorized.contains(&expected) {
        return ToolResult::new(texts::not_run(texts::HOST_CHANGED));
    }
    tracing::info!(host = %checked.host.id, "an SSH command runs");
    tracing::debug!(host = %checked.host.id, command = %checked.command, "the command of an SSH call");
    let started = Instant::now();
    let (connected, channel) = match open_channel(hosts, &checked.host).await {
        Ok(opened) => opened,
        Err(reason) => return ToolResult::new(texts::not_run(reason)),
    };
    let (mut reader, writer) = channel.split();
    if writer.exec(true, checked.command.as_bytes()).await.is_err() {
        return ToolResult::new(texts::not_run(texts::EXEC_REFUSED));
    }
    // The command reads no input.
    if let Err(error) = writer.eof().await {
        tracing::debug!(%error, "the end of a command's input did not reach the host");
    }
    let mut running = Running::new(writer);
    let watched = watch(&mut reader, &mut running, checked.timeout_seconds).await;
    let Some(watched) = watched else {
        running.close_refused().await;
        return ToolResult::new(texts::not_run(texts::EXEC_REFUSED));
    };
    let password = connected.password.expose();
    let finished = Finished {
        ending: watched.ending,
        duration: started.elapsed(),
        stdout: watched.stdout.finish().redacted(password),
        stderr: watched.stderr.finish().redacted(password),
    };
    tracing::info!(host = %checked.host.id, ending = ?finished.ending, "an SSH command ended");
    let result_limit = usize::try_from(execution.result_limit).unwrap_or(usize::MAX);
    render(
        &texts::ran_on(&checked.host),
        checked.timeout_seconds,
        &finished,
        Room {
            result_limit,
            archive_ids: &execution.archive_ids,
        },
    )
}

/// A session with the host and a channel on it. A session that was open
/// before and fails to open a channel may have been closed by the host
/// meanwhile, such as while the computer slept, so it is replaced once:
/// nothing has run at that point.
pub(crate) async fn open_channel(
    hosts: &Hosts,
    host: &HostRecord,
) -> Result<(Connected, russh::Channel<Msg>), &'static str> {
    let connected = hosts.connect(host).await.map_err(reason)?;
    match connected.lease.session().channel_open_session().await {
        Ok(channel) => return Ok((connected, channel)),
        Err(error) if !connected.lease.reused() => {
            tracing::warn!(host = %host.id, %error, "a new SSH session opened no channel");
            return Err(texts::HANDSHAKE_FAILED);
        }
        Err(error) => {
            tracing::debug!(host = %host.id, %error, "a reused SSH session opened no channel; opening a new one");
        }
    }
    drop(connected);
    hosts.sessions().forget(&host.id);
    let connected = hosts.connect(host).await.map_err(reason)?;
    let channel = connected
        .lease
        .session()
        .channel_open_session()
        .await
        .map_err(|error| {
            tracing::warn!(host = %host.id, %error, "a new SSH session opened no channel");
            texts::HANDSHAKE_FAILED
        })?;
    Ok((connected, channel))
}

/// Why a call did not reach the host's shell.
pub(crate) fn reason(error: SshError) -> &'static str {
    match error {
        SshError::HostNotFound => texts::HOST_REMOVED,
        SshError::Unreachable => texts::UNREACHABLE,
        SshError::TimedOut => texts::TIMED_OUT,
        SshError::HostKeyUnsupported => texts::KEY_UNSUPPORTED,
        SshError::HostKeyChanged => texts::KEY_CHANGED,
        SshError::HostKeyNotTrusted => texts::KEY_NOT_TRUSTED,
        SshError::AuthFailed => texts::AUTH_FAILED,
        SshError::AuthMethodUnsupported => texts::AUTH_UNSUPPORTED,
        SshError::PasswordMissing => texts::PASSWORD_MISSING,
        SshError::CredentialStoreUnavailable => texts::CREDENTIAL_STORE_UNAVAILABLE,
        SshError::HandshakeFailed
        | SshError::NameTaken
        | SshError::ScanOutdated
        | SshError::StorageFailed => texts::HANDSHAKE_FAILED,
    }
}

/// A command that runs on a host. Dropping it before the command ended,
/// as the core does when the owner stops the reply, asks the host to end
/// the command and closes its channel.
pub(crate) struct Running {
    writer: Option<ChannelWriteHalf<Msg>>,
}

impl Running {
    /// The command whose channel `writer` writes to.
    pub(crate) fn new(writer: ChannelWriteHalf<Msg>) -> Self {
        Self {
            writer: Some(writer),
        }
    }

    /// Takes the channel, which is then no longer the guard's to end.
    pub(crate) fn take(&mut self) -> Option<ChannelWriteHalf<Msg>> {
        self.writer.take()
    }

    /// Closes the channel of a command that the host refused to start, so
    /// there is nothing to end.
    pub(crate) async fn close_refused(&mut self) {
        if let Some(writer) = self.writer.take()
            && let Err(error) = writer.close().await
        {
            tracing::debug!(%error, "the channel of a refused command was closed already");
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(writer) = self.writer.take() {
            tokio::spawn(async move {
                end(&writer).await;
            });
        }
    }
}

/// Asks the host to end a command, TERM and then KILL, and closes its
/// channel. A server that ignores signals still loses the channel.
async fn end(writer: &ChannelWriteHalf<Msg>) {
    if let Err(error) = writer.signal(Sig::TERM).await {
        tracing::debug!(%error, "TERM did not reach the host");
    }
    tokio::time::sleep(KILL_GRACE).await;
    if let Err(error) = writer.signal(Sig::KILL).await {
        tracing::debug!(%error, "KILL did not reach the host");
    }
    if let Err(error) = writer.close().await {
        tracing::debug!(%error, "the channel of a command was closed already");
    }
}

/// What a command produced while the plugin watched it.
struct Watched {
    ending: Ending,
    stdout: Capture,
    stderr: Capture,
}

/// Watches a command until its channel closes or its time runs out, and
/// keeps its output. `None` means that the host refused to start it.
async fn watch(
    reader: &mut ChannelReadHalf,
    running: &mut Running,
    timeout_seconds: u32,
) -> Option<Watched> {
    let mut watched = Watched {
        ending: Ending::Unknown,
        stdout: Capture::default(),
        stderr: Capture::default(),
    };
    let deadline = tokio::time::sleep(Duration::from_secs(u64::from(timeout_seconds)));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            message = reader.wait() => match message {
                None | Some(ChannelMsg::Close) => break,
                Some(ChannelMsg::Failure) => return None,
                Some(message) => keep(&mut watched, message),
            },
            () = &mut deadline => {
                let writer = running.writer.take();
                drain_ending(reader, &mut watched, writer).await;
                watched.ending = Ending::TimedOut;
                return Some(watched);
            }
        }
    }
    // The command ended on its own, so nothing is left to end.
    running.writer = None;
    Some(watched)
}

/// Keeps what a message of the channel says.
fn keep(watched: &mut Watched, message: ChannelMsg) {
    match message {
        ChannelMsg::Data { data } => watched.stdout.push(&data),
        ChannelMsg::ExtendedData { data, ext: 1 } => watched.stderr.push(&data),
        ChannelMsg::ExitStatus { exit_status } => watched.ending = Ending::Exited(exit_status),
        ChannelMsg::ExitSignal {
            signal_name,
            core_dumped,
            ..
        } => {
            watched.ending = Ending::Signaled {
                signal: signal_text(&signal_name),
                core_dumped,
            };
        }
        _other => {}
    }
}

/// Ends a command whose time ran out: TERM, the output that still comes
/// within a short grace, KILL, the rest within another, and then the channel
/// closes.
async fn drain_ending(
    reader: &mut ChannelReadHalf,
    watched: &mut Watched,
    writer: Option<ChannelWriteHalf<Msg>>,
) {
    let Some(writer) = writer else {
        return;
    };
    if let Err(error) = writer.signal(Sig::TERM).await {
        tracing::debug!(%error, "TERM did not reach the host");
    }
    if drain(reader, watched, KILL_GRACE).await {
        return;
    }
    if let Err(error) = writer.signal(Sig::KILL).await {
        tracing::debug!(%error, "KILL did not reach the host");
    }
    if drain(reader, watched, CLOSE_GRACE).await {
        return;
    }
    if let Err(error) = writer.close().await {
        tracing::debug!(%error, "the channel of a command was closed already");
    }
}

/// Keeps the channel's output for up to `grace`, and says whether the
/// channel closed meanwhile.
async fn drain(reader: &mut ChannelReadHalf, watched: &mut Watched, grace: Duration) -> bool {
    let draining = async {
        loop {
            match reader.wait().await {
                None | Some(ChannelMsg::Close) => return,
                Some(message) => keep(watched, message),
            }
        }
    };
    tokio::time::timeout(grace, draining).await.is_ok()
}

/// A signal's name as SSH sends it, such as `TERM`.
pub(crate) fn signal_text(signal: &Sig) -> String {
    let name = match signal {
        Sig::ABRT => "ABRT",
        Sig::ALRM => "ALRM",
        Sig::FPE => "FPE",
        Sig::HUP => "HUP",
        Sig::ILL => "ILL",
        Sig::INT => "INT",
        Sig::KILL => "KILL",
        Sig::PIPE => "PIPE",
        Sig::QUIT => "QUIT",
        Sig::SEGV => "SEGV",
        Sig::TERM => "TERM",
        Sig::USR1 => "USR1",
        Sig::Custom(name) => name.as_str(),
    };
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .take(32)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Arguments, signal_text};
    use crate::schema::parameters;

    #[test]
    fn the_schema_requires_every_argument_and_lets_the_timeout_be_null() {
        let schema = parameters::<Arguments>();

        assert_eq!(
            schema["required"],
            json!(["command", "host", "timeout_seconds"])
        );
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(
            schema["properties"]["timeout_seconds"]["type"],
            json!(["integer", "null"])
        );
        assert!(
            schema["properties"]["host"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("registered SSH host"))
        );
        assert!(schema.get("$schema").is_none());
    }

    #[test]
    fn a_signal_is_named_as_ssh_names_it_and_a_custom_one_is_cleaned() {
        assert_eq!(signal_text(&russh::Sig::TERM), "TERM");
        assert_eq!(
            signal_text(&russh::Sig::Custom("WINCH\n".to_owned())),
            "WINCH"
        );
    }
}
