//! Terminals on the registered hosts (ADR-0053, decision 3; plugins/ssh/
//! docs/README.md, terminals): the plugin offers its hosts through the point
//! `terminal-hosts`, and the core opens terminals there for the terminal
//! plugin, which never reads a host's record or its password.
//!
//! A terminal is a pseudo terminal and the account's login shell on a
//! channel of the host's pooled session, which the terminal leases for as
//! long as it runs, so the session is never closed for being idle meanwhile.
//! Opening one logs in, as testing a login does (ADR-0053, decision 6), and
//! only to a host that still shows the pinned key. A terminal that an
//! approved command opens names the endpoint and the key that the approval
//! named, and a host where either changed is refused before anything is
//! sent.
//!
//! The plugin reads the login shell once for each host, through
//! `printf '%s' "$SHELL"` on a channel of its own beside the terminal's, and
//! lists it with the host, so the terminal plugin types the setup that
//! reaches it. A host that does not answer is listed with a shell that no
//! setup reaches, and the next terminal there reads it again.
//!
//! The host's password is replaced in the output as its bytes arrive (T-71,
//! `redaction.rs`). Bytes at the end of a read that could start the password
//! wait for the next read, and at most [`HOLD_BACK`], so a password that the
//! host's reads cut is still replaced, and the echo of a key that starts the
//! password still shows at once.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::contracts::Secret;
use mannie_plugin_sdk::ipc::PluginError as _;
use mannie_plugin_sdk::points::terminal_hosts::{
    OpenTerminal, TerminalConnection, TerminalHostEntry, TerminalHostSource, TerminalOpenFailure,
    check_host_entry,
};
use mannie_plugin_sdk::services::terminals::{
    ShellKind, TerminalEnding, TerminalHostState, TerminalIoError, TerminalOutput, TerminalSize,
};
use mannie_plugin_sdk::{BoxFuture, BoxStream};
use russh::client::Msg;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
use tokio::sync::{mpsc, watch};

use crate::client::Session;
use crate::commands::{HostStatus, SshError};
use crate::hosts::{Connected, Hosts};
use crate::redaction::Redaction;
use crate::sessions::Lease;
use crate::store::HostRecord;
use crate::tool::open_channel;

/// The terminal type that the host's programs see, as on this computer
/// (ADR-0053, decision 1).
const TERM: &str = "xterm-256color";

/// The command that prints the account's login shell.
const SHELL_PROBE: &str = "printf '%s' \"$SHELL\"";

/// The most bytes of the login shell's path that the plugin reads.
const MAX_PROBE_BYTES: usize = 1024;

/// The longest that the plugin waits for the login shell, and for the host
/// to give the terminal its pseudo terminal and shell.
const OPEN_WAIT: Duration = Duration::from_secs(10);

/// The longest that bytes which could start the password wait for the next
/// read. The reads of one burst of output follow each other within a few
/// milliseconds, and a typed key's echo shows after this at the latest.
const HOLD_BACK: Duration = Duration::from_millis(50);

/// The most pieces of output that wait for the core, after which the plugin
/// reads the channel no further, so the host waits too.
const OUTPUT_QUEUE: usize = 64;

/// The ending of a terminal whose session Mannie closed: its host changed or
/// was removed, or the plugin stopped.
const SESSION_CLOSED: &str = "session_closed";

/// The ending of a terminal whose connection broke.
const CONNECTION_LOST: &str = "connection_lost";

/// The login shell of a host, as the plugin read it for the host's endpoint
/// and key.
#[derive(Debug, Clone, PartialEq, Eq)]
struct KnownShell {
    endpoint: String,
    fingerprint: String,
    kind: ShellKind,
    /// Whether the host answered, so later terminals need not ask again.
    answered: bool,
}

/// The registered hosts, as the point `terminal-hosts` offers them.
#[derive(Debug, Clone)]
pub(crate) struct SshTerminalHosts {
    hosts: Hosts,
    /// The login shells, by host ID.
    shells: Arc<watch::Sender<HashMap<String, KnownShell>>>,
}

impl SshTerminalHosts {
    pub(crate) fn new(hosts: Hosts) -> Self {
        Self {
            hosts,
            shells: Arc::new(watch::Sender::new(HashMap::new())),
        }
    }

    /// The login shell that the plugin read for `host`, while the host has
    /// the same endpoint and key.
    fn known_shell(&self, host: &HostRecord) -> Option<KnownShell> {
        self.shells
            .borrow()
            .get(&host.id)
            .filter(|known| {
                known.endpoint == host.endpoint() && known.fingerprint == identity(host)
            })
            .cloned()
    }

    /// Records the login shell of `host`, which the hosts' stream then
    /// lists.
    fn learn_shell(&self, host: &HostRecord, kind: ShellKind, answered: bool) {
        let known = KnownShell {
            endpoint: host.endpoint(),
            fingerprint: identity(host),
            kind,
            answered,
        };
        self.shells.send_if_modified(|shells| {
            let is_new = shells.get(&host.id) != Some(&known);
            if is_new {
                shells.insert(host.id.clone(), known);
            }
            is_new
        });
    }

    async fn open_now(
        self,
        request: OpenTerminal,
    ) -> Result<Box<dyn TerminalConnection>, TerminalOpenFailure> {
        let host = self
            .hosts
            .by_id(&request.key)
            .ok_or(TerminalOpenFailure::UnknownHost)?;
        // An approved command names the host as the owner allowed it; one
        // whose endpoint or key changed since gets nothing, not even a
        // login.
        if let Some(expect) = &request.expect
            && (expect.endpoint != host.endpoint() || expect.identity != identity(&host))
        {
            tracing::info!(host = %host.id, "a terminal was refused, since its host changed after the approval");
            return Err(TerminalOpenFailure::HostChanged);
        }
        if let Some(code) = unavailable_code(host.status()) {
            return Err(failed(code));
        }
        let (connected, channel) = open_channel(&self.hosts, &host)
            .await
            .map_err(|error| {
                tracing::warn!(host = %host.id, error.code = error.code(), "a terminal could not reach its SSH host");
                failed(error_code(error))
            })?;
        let Connected { lease, password } = connected;
        let (mut reader, writer) = channel.split();
        let needs_probe = self.known_shell(&host).is_none_or(|known| !known.answered);
        let probing = async {
            if needs_probe {
                Some(login_shell(lease.session()).await)
            } else {
                None
            }
        };
        let starting = start_shell(&mut reader, &writer, request.size);
        let (probed, started) = tokio::join!(probing, starting);
        if let Some(probed) = probed {
            let answered = probed.is_some();
            let kind = probed.unwrap_or(ShellKind::Other);
            tracing::info!(host = %host.id, ?kind, answered, "the login shell of an SSH host");
            self.learn_shell(&host, kind, answered);
        }
        let early = match started {
            Ok(early) => early,
            Err(code) => {
                tracing::warn!(host = %host.id, code, "an SSH host gave a terminal no shell");
                if let Err(error) = writer.close().await {
                    tracing::debug!(%error, "the channel of a refused terminal was closed already");
                }
                return Err(failed(code));
            }
        };
        tracing::info!(host = %host.id, "a terminal opened on an SSH host");
        Ok(Box::new(SshTerminal::start(
            lease, password, reader, writer, early,
        )))
    }
}

impl TerminalHostSource for SshTerminalHosts {
    fn hosts(&self) -> BoxStream<'static, Vec<TerminalHostEntry>> {
        let records = self.hosts.follow();
        let shells = self.shells.subscribe();
        stream::unfold(
            (records, shells, true),
            |(mut records, mut shells, first)| async move {
                if !first {
                    let changed = tokio::select! {
                        changed = records.changed() => changed,
                        changed = shells.changed() => changed,
                    };
                    if changed.is_err() {
                        return None;
                    }
                }
                let entries = entries(&records.borrow_and_update(), &shells.borrow_and_update());
                Some((entries, (records, shells, false)))
            },
        )
        .boxed()
    }

    fn open(
        &self,
        request: OpenTerminal,
    ) -> BoxFuture<'static, Result<Box<dyn TerminalConnection>, TerminalOpenFailure>> {
        Box::pin(self.clone().open_now(request))
    }
}

/// The hosts as the point lists them, with the login shells that the plugin
/// read for their endpoints and keys. A host whose fields break the point's
/// rules is left out.
fn entries(records: &[HostRecord], shells: &HashMap<String, KnownShell>) -> Vec<TerminalHostEntry> {
    records
        .iter()
        .filter_map(|host| {
            let mut entry = TerminalHostEntry::new(
                host.id.clone(),
                host.name.clone(),
                host.endpoint(),
                identity(host),
            );
            if let Some(code) = unavailable_code(host.status()) {
                entry = entry.with_state(TerminalHostState::Unavailable {
                    code: code.to_owned(),
                });
            }
            let shell = shells.get(&host.id).filter(|known| {
                known.endpoint == entry.endpoint && known.fingerprint == entry.identity
            });
            if let Some(known) = shell {
                entry = entry.with_shell(known.kind);
            }
            match check_host_entry(&entry) {
                Ok(()) => Some(entry),
                Err(field) => {
                    tracing::warn!(host = %host.id, field, "an SSH host is not offered to terminals");
                    None
                }
            }
        })
        .collect()
}

/// What identifies a host beyond its address: the fingerprint of its pinned
/// key, as the remote-command access names it.
fn identity(host: &HostRecord) -> String {
    host.key
        .as_ref()
        .map(|key| key.fingerprint.clone())
        .unwrap_or_default()
}

/// Why terminals do not open on a host now, or `None` when they do.
fn unavailable_code(status: HostStatus) -> Option<&'static str> {
    match status {
        HostStatus::Ready => None,
        HostStatus::KeyNotChecked => Some("key_not_checked"),
        HostStatus::KeyChanged => Some("key_changed"),
        HostStatus::PasswordMissing => Some("password_missing"),
    }
}

/// The code of a failure to reach or log in to a host, such as
/// `host_key_changed`.
fn error_code(error: SshError) -> &'static str {
    let code = error.code();
    code.strip_prefix("ssh.").unwrap_or(code)
}

fn failed(reason: &str) -> TerminalOpenFailure {
    TerminalOpenFailure::Failed {
        reason: reason.to_owned(),
    }
}

/// The account's login shell, read on a channel of its own, or `None` when
/// the host does not answer within [`OPEN_WAIT`].
async fn login_shell(session: &Session) -> Option<ShellKind> {
    let channel = session.channel_open_session().await.ok()?;
    let (mut reader, writer) = channel.split();
    let reading = async {
        writer.exec(true, SHELL_PROBE).await.ok()?;
        let mut output = Vec::new();
        loop {
            match reader.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    output.extend_from_slice(&data);
                    if output.len() > MAX_PROBE_BYTES {
                        return None;
                    }
                }
                Some(ChannelMsg::Failure) => return None,
                None | Some(ChannelMsg::Close) => break,
                Some(_other) => {}
            }
        }
        Some(shell_kind(&String::from_utf8_lossy(&output)))
    };
    let kind = tokio::time::timeout(OPEN_WAIT, reading)
        .await
        .ok()
        .flatten();
    if let Err(error) = writer.close().await {
        tracing::debug!(%error, "the channel of the login shell's probe was closed already");
    }
    kind
}

/// The kind of a login shell, by the name of its program: `bash`, `zsh`,
/// `fish`, a POSIX `sh` (also dash and ash) and PowerShell. Any other shell
/// is one that no setup reaches.
fn shell_kind(path: &str) -> ShellKind {
    let path = path.trim();
    let program = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let program = program.trim_start_matches('-').to_ascii_lowercase();
    let program = program.strip_suffix(".exe").unwrap_or(&program);
    match program {
        "bash" => ShellKind::Bash,
        "zsh" => ShellKind::Zsh,
        "fish" => ShellKind::Fish,
        "sh" | "dash" | "ash" => ShellKind::Sh,
        "pwsh" | "powershell" => ShellKind::PowerShell,
        _ => ShellKind::Other,
    }
}

/// Asks the host for a pseudo terminal of `size` and the login shell on it,
/// and returns the output that came before the host said yes, or the code of
/// the refusal.
async fn start_shell(
    reader: &mut ChannelReadHalf,
    writer: &ChannelWriteHalf<Msg>,
    size: TerminalSize,
) -> Result<Vec<u8>, &'static str> {
    let starting = async {
        let mut early = Vec::new();
        writer
            .request_pty(
                true,
                TERM,
                u32::from(size.columns),
                u32::from(size.rows),
                0,
                0,
                &[],
            )
            .await
            .map_err(|_error| "pty_refused")?;
        answer(reader, &mut early).await.ok_or("pty_refused")?;
        writer
            .request_shell(true)
            .await
            .map_err(|_error| "shell_refused")?;
        answer(reader, &mut early).await.ok_or("shell_refused")?;
        Ok(early)
    };
    tokio::time::timeout(OPEN_WAIT, starting)
        .await
        .unwrap_or(Err("timed_out"))
}

/// Waits for the host's answer to a request, keeping any output that comes
/// first, and says whether it was yes.
async fn answer(reader: &mut ChannelReadHalf, early: &mut Vec<u8>) -> Option<()> {
    loop {
        match reader.wait().await? {
            ChannelMsg::Success => return Some(()),
            ChannelMsg::Failure | ChannelMsg::Close => return None,
            ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                early.extend_from_slice(&data);
            }
            _other => {}
        }
    }
}

/// A terminal on a host. Dropping it without closing it closes its channel.
pub(crate) struct SshTerminal {
    writer: Arc<ChannelWriteHalf<Msg>>,
    output: Option<mpsc::Receiver<TerminalOutput>>,
    ending: watch::Receiver<Option<TerminalEnding>>,
    /// Set once the core closes the terminal, so its end reads as closed.
    closing: Arc<AtomicBool>,
    pump: tokio::task::JoinHandle<()>,
}

impl fmt::Debug for SshTerminal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SshTerminal")
            .field("channel", &self.writer.id())
            .finish_non_exhaustive()
    }
}

impl SshTerminal {
    /// Starts reading the terminal's channel, with the output that came
    /// before its shell started.
    fn start(
        lease: Lease,
        password: Secret,
        reader: ChannelReadHalf,
        writer: ChannelWriteHalf<Msg>,
        early: Vec<u8>,
    ) -> Self {
        let (sender, output) = mpsc::channel(OUTPUT_QUEUE);
        let (ended, ending) = watch::channel(None);
        let closing = Arc::new(AtomicBool::new(false));
        let pump = Pump {
            lease,
            redaction: Redaction::new(password),
            sender,
            ended,
            closing: Arc::clone(&closing),
            exit_code: None,
        };
        Self {
            writer: Arc::new(writer),
            output: Some(output),
            ending,
            closing,
            pump: tokio::spawn(pump.run(reader, early)),
        }
    }
}

impl TerminalConnection for SshTerminal {
    fn output(&mut self) -> BoxStream<'static, TerminalOutput> {
        let Some(output) = self.output.take() else {
            return stream::empty().boxed();
        };
        stream::unfold(output, |mut output| async move {
            output.recv().await.map(|item| (item, output))
        })
        .boxed()
    }

    fn write(&self, bytes: Vec<u8>) -> BoxFuture<'static, Result<(), TerminalIoError>> {
        // The core hands writes over one at a time, so they reach the host
        // in order.
        let writer = Arc::clone(&self.writer);
        Box::pin(async move {
            writer
                .data_bytes(bytes)
                .await
                .map_err(|_error| TerminalIoError::Ended)
        })
    }

    fn resize(&self, size: TerminalSize) -> BoxFuture<'static, Result<(), TerminalIoError>> {
        let writer = Arc::clone(&self.writer);
        Box::pin(async move {
            writer
                .window_change(u32::from(size.columns), u32::from(size.rows), 0, 0)
                .await
                .map_err(|_error| TerminalIoError::Ended)
        })
    }

    fn close(self: Box<Self>, grace: Duration) -> BoxFuture<'static, TerminalEnding> {
        // Closing the channel hangs up the shell: the server closes the
        // pseudo terminal, which sends the shell SIGHUP.
        self.closing.store(true, Ordering::SeqCst);
        let writer = Arc::clone(&self.writer);
        let mut ending = self.ending.clone();
        let pump = self.pump.abort_handle();
        Box::pin(async move {
            if let Err(error) = writer.eof().await {
                tracing::debug!(%error, "the end of a terminal's input did not reach the host");
            }
            if let Err(error) = writer.close().await {
                tracing::debug!(%error, "the channel of a terminal was closed already");
            }
            let ended = tokio::time::timeout(grace, ending.wait_for(Option::is_some))
                .await
                .ok()
                .and_then(|seen| seen.ok().and_then(|ending| ending.clone()));
            ended.unwrap_or_else(|| {
                pump.abort();
                TerminalEnding::Closed
            })
        })
    }
}

impl Drop for SshTerminal {
    fn drop(&mut self) {
        let is_closed = self.closing.swap(true, Ordering::SeqCst);
        if is_closed || self.ending.borrow().is_some() {
            return;
        }
        let writer = Arc::clone(&self.writer);
        tokio::spawn(async move {
            if let Err(error) = writer.close().await {
                tracing::debug!(%error, "the channel of a dropped terminal was closed already");
            }
        });
    }
}

/// What reads a terminal's channel: the lease on the session, the password's
/// redaction, and where the output goes.
struct Pump {
    lease: Lease,
    redaction: Redaction,
    sender: mpsc::Sender<TerminalOutput>,
    ended: watch::Sender<Option<TerminalEnding>>,
    closing: Arc<AtomicBool>,
    exit_code: Option<i32>,
}

impl Pump {
    /// Hands the core the output, without the password, until the channel
    /// closes, then how the terminal ended, and lets go of the session.
    async fn run(mut self, mut reader: ChannelReadHalf, early: Vec<u8>) {
        let mut closed = false;
        let mut open = self.send_known(&early).await;
        while open {
            let message = if self.redaction.holds_back() {
                match tokio::time::timeout(HOLD_BACK, reader.wait()).await {
                    Ok(message) => message,
                    Err(_quiet) => {
                        let held = self.redaction.finish();
                        open = self.send(held).await;
                        continue;
                    }
                }
            } else {
                reader.wait().await
            };
            match message {
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    open = self.send_known(&data).await;
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    self.exit_code = Some(i32::try_from(exit_status).unwrap_or(i32::MAX));
                }
                Some(ChannelMsg::Close) => {
                    closed = true;
                    break;
                }
                None => break,
                Some(_other) => {}
            }
        }
        let held = self.redaction.finish();
        if open {
            self.send(held).await;
        }
        let ending = self.ending(closed);
        tracing::info!(ending = ?ending, "a terminal on an SSH host ended");
        self.ended.send_replace(Some(ending.clone()));
        if open
            && self
                .sender
                .send(TerminalOutput::Ended { ending })
                .await
                .is_err()
        {
            tracing::debug!("nobody read the end of a terminal on an SSH host");
        }
    }

    /// Hands over the bytes of a read that are known not to be the
    /// password, and says whether the core still reads.
    async fn send_known(&mut self, bytes: &[u8]) -> bool {
        let known = self.redaction.push(bytes);
        self.send(known).await
    }

    async fn send(&self, bytes: Vec<u8>) -> bool {
        if bytes.is_empty() {
            return true;
        }
        self.sender
            .send(TerminalOutput::Bytes { bytes })
            .await
            .is_ok()
    }

    /// How the terminal ended: with its shell's exit code, closed by the
    /// core or the host, or lost with its session.
    fn ending(&self, closed: bool) -> TerminalEnding {
        if let Some(code) = self.exit_code {
            return TerminalEnding::Exited { code };
        }
        if self.closing.load(Ordering::SeqCst) {
            return TerminalEnding::Closed;
        }
        if self.lease.closed_by_mannie() {
            return TerminalEnding::Lost {
                code: SESSION_CLOSED.to_owned(),
            };
        }
        if closed {
            return TerminalEnding::Closed;
        }
        TerminalEnding::Lost {
            code: CONNECTION_LOST.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::services::terminals::ShellKind;

    use super::shell_kind;

    #[test]
    fn a_login_shell_is_known_by_its_programs_name() {
        assert_eq!(shell_kind("/bin/bash"), ShellKind::Bash);
        assert_eq!(shell_kind("/usr/local/bin/zsh\n"), ShellKind::Zsh);
        assert_eq!(shell_kind("/usr/bin/fish"), ShellKind::Fish);
        assert_eq!(shell_kind("/bin/sh"), ShellKind::Sh);
        assert_eq!(shell_kind("/bin/dash"), ShellKind::Sh);
        assert_eq!(shell_kind("-bash"), ShellKind::Bash);
        assert_eq!(
            shell_kind("/opt/microsoft/powershell/7/pwsh"),
            ShellKind::PowerShell
        );
        assert_eq!(
            shell_kind("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
            ShellKind::PowerShell
        );
        assert_eq!(shell_kind("/bin/tcsh"), ShellKind::Other);
        assert_eq!(shell_kind(""), ShellKind::Other);
    }
}
