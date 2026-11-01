//! A stand-in for the core's service `terminals` (ADR-0053, decision 4):
//! terminals whose shell is a script, which speaks the terminal plugin's
//! markers with the nonce that the setup carries, so a plugin's tests run
//! without starting a shell.
//!
//! The scripted shell echoes what is typed and knows a few commands:
//! `echo <text>`, `lines <n>` (prints `line 1` to `line n`), `cd <folder>`,
//! `fail` (exits with 1), `sleep` (runs until Ctrl+C, then exits with 130),
//! `forge` (prints a marker with a wrong nonce), `exit <code>` (ends the
//! terminal), and anything else, which is not found (127). A host's terminal
//! starts without markers, as a remote login shell does, until the plugin
//! types a setup that carries a nonce.
//!
//! [`ScriptedHosts`] offers such shells as hosts of the point
//! `terminal-hosts`, so a test that contributes it reaches them through the
//! core's real registry and policy.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt as _;
use tokio::sync::{mpsc, watch};

use super::lock;
use crate::host::{BindRefusal, PluginBinding, ServiceProvider};
use crate::points::terminal_hosts::{
    OpenTerminal, TerminalConnection, TerminalHostEntry, TerminalHostSource, TerminalOpenFailure,
};
use crate::service::{BoundService, Service};
use crate::services::terminals::{
    LocalShell, OfferedHost, ShellKind, Terminal, TerminalEnding, TerminalHostList,
    TerminalHostRef, TerminalHostState, TerminalIoError, TerminalOpenError, TerminalOutput,
    TerminalSize, TerminalSpec, Terminals, TerminalsApi,
};
use crate::{BoxFuture, BoxStream};

/// The program of the fake's shell of this machine.
pub const FAKE_SHELL: &str = "/fake/bin/bash";

/// The folder that a fake terminal starts in without one.
pub const FAKE_HOME: &str = "/home/owner";

/// The user of a fake terminal.
pub const FAKE_USER: &str = "owner";

/// The most pieces of output that wait for the plugin.
const OUTPUT_QUEUE: usize = 4096;

/// A terminal that the fake opened, as a test reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedTerminal {
    /// The conversation it belongs to.
    pub conversation: String,
    /// Where it runs.
    pub host: TerminalHostRef,
    /// The folder it started in.
    pub folder: Option<PathBuf>,
    /// The setup it got.
    pub setup: Option<String>,
}

#[derive(Debug)]
struct FakeState {
    hosts: watch::Sender<TerminalHostList>,
    opened: Mutex<Vec<OpenedTerminal>>,
    refusal: Mutex<Option<TerminalOpenError>>,
    running: Mutex<Vec<bool>>,
}

/// The core's terminals with scripted shells: one shell of this machine,
/// [`FAKE_SHELL`], and the hosts that a test offers.
///
/// Cloning is cheap; clones share the terminals.
#[derive(Debug, Clone)]
pub struct FakeTerminals {
    state: Arc<FakeState>,
}

impl Default for FakeTerminals {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeTerminals {
    /// The fake with its shell of this machine and no hosts.
    pub fn new() -> Self {
        let shell = LocalShell {
            program: PathBuf::from(FAKE_SHELL),
            name: "bash".to_owned(),
            kind: ShellKind::Bash,
            default: true,
        };
        let (hosts, _receiver) = watch::channel(TerminalHostList {
            shells: vec![shell],
            hosts: Vec::new(),
        });
        Self {
            state: Arc::new(FakeState {
                hosts,
                opened: Mutex::new(Vec::new()),
                refusal: Mutex::new(None),
                running: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Offers a host of `plugin` under `key`, whose login shell is `shell`.
    pub fn offer_host(
        &self,
        plugin: &str,
        key: &str,
        label: &str,
        endpoint: &str,
        shell: ShellKind,
    ) {
        self.state.hosts.send_modify(|list| {
            list.hosts
                .retain(|host| !(host.plugin == plugin && host.key == key));
            list.hosts.push(OfferedHost {
                plugin: plugin.to_owned(),
                key: key.to_owned(),
                label: label.to_owned(),
                endpoint: endpoint.to_owned(),
                identity: "SHA256:fake".to_owned(),
                state: TerminalHostState::Ready,
                shell: Some(shell),
            });
        });
    }

    /// The terminals that the fake opened, oldest first.
    pub fn opened(&self) -> Vec<OpenedTerminal> {
        lock(&self.state.opened).clone()
    }

    /// Refuses the next terminal with `error`.
    pub fn refuse_next(&self, error: TerminalOpenError) {
        *lock(&self.state.refusal) = Some(error);
    }

    /// Whether the plugin marked a command as running in each terminal, by
    /// the order of opening.
    pub fn running(&self) -> Vec<bool> {
        lock(&self.state.running).clone()
    }
}

impl ServiceProvider for FakeTerminals {
    fn name(&self) -> &'static str {
        Terminals::NAME
    }

    fn major(&self) -> u16 {
        Terminals::MAJOR
    }

    fn bind(&self, _binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        let api: Arc<dyn TerminalsApi> = Arc::new(self.clone());
        Ok(BoundService::new::<Terminals>(api))
    }
}

impl TerminalsApi for FakeTerminals {
    fn hosts(&self) -> BoxStream<'static, TerminalHostList> {
        let receiver = self.state.hosts.subscribe();
        futures_util::stream::unfold((receiver, true), |(mut receiver, first)| async move {
            if !first && receiver.changed().await.is_err() {
                return None;
            }
            let list = receiver.borrow_and_update().clone();
            Some((list, (receiver, false)))
        })
        .boxed()
    }

    fn open(
        &self,
        spec: TerminalSpec,
    ) -> BoxFuture<'static, Result<Box<dyn Terminal>, TerminalOpenError>> {
        let opened = self.open_now(&spec);
        Box::pin(std::future::ready(opened))
    }
}

impl FakeTerminals {
    fn open_now(&self, spec: &TerminalSpec) -> Result<Box<dyn Terminal>, TerminalOpenError> {
        let refusal = lock(&self.state.refusal).take();
        if let Some(refusal) = refusal {
            return Err(refusal);
        }
        let list = self.state.hosts.borrow().clone();
        match &spec.host {
            TerminalHostRef::ThisMachine { shell } => {
                if !list.shells.iter().any(|known| known.program == *shell) {
                    return Err(TerminalOpenError::ShellNotListed);
                }
            }
            TerminalHostRef::Plugin { plugin, key } => {
                let Some(host) = list
                    .hosts
                    .iter()
                    .find(|host| host.plugin == *plugin && host.key == *key)
                else {
                    return Err(TerminalOpenError::HostNotFound);
                };
                if let Some(expect) = &spec.expect
                    && (expect.endpoint != host.endpoint || expect.identity != host.identity)
                {
                    return Err(TerminalOpenError::HostChanged);
                }
            }
        }
        let nonce = spec.setup.as_deref().and_then(nonce_in);
        let folder = spec.folder.as_ref().map_or_else(
            || FAKE_HOME.to_owned(),
            |folder| folder.to_string_lossy().into_owned(),
        );
        let index = {
            let mut opened = lock(&self.state.opened);
            opened.push(OpenedTerminal {
                conversation: spec.conversation.clone(),
                host: spec.host.clone(),
                folder: spec.folder.clone(),
                setup: spec.setup.clone(),
            });
            opened.len() - 1
        };
        lock(&self.state.running).push(false);
        Ok(Box::new(ScriptedTerminal::start(
            nonce,
            folder,
            Some((Arc::clone(&self.state), index)),
        )))
    }
}

/// Hosts of the point `terminal-hosts` whose terminals are scripted shells
/// without markers until a setup is typed, as remote login shells are.
#[derive(Debug, Clone)]
pub struct ScriptedHosts {
    hosts: Vec<TerminalHostEntry>,
    opened: Arc<Mutex<Vec<String>>>,
}

impl ScriptedHosts {
    /// Hosts with these keys, names and endpoints, whose login shell is
    /// bash and whose identity is `SHA256:fake`.
    pub fn new(hosts: &[(&str, &str, &str)]) -> Self {
        let hosts = hosts
            .iter()
            .map(|(key, label, endpoint)| {
                TerminalHostEntry::new(
                    (*key).to_owned(),
                    (*label).to_owned(),
                    (*endpoint).to_owned(),
                    "SHA256:fake".to_owned(),
                )
                .with_shell(ShellKind::Bash)
            })
            .collect();
        Self {
            hosts,
            opened: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The keys of the hosts whose terminals opened, oldest first.
    pub fn opened(&self) -> Vec<String> {
        lock(&self.opened).clone()
    }
}

impl TerminalHostSource for ScriptedHosts {
    fn hosts(&self) -> BoxStream<'static, Vec<TerminalHostEntry>> {
        // A stream that ends empties the list, so this one stays open.
        futures_util::stream::once(std::future::ready(self.hosts.clone()))
            .chain(futures_util::stream::pending())
            .boxed()
    }

    fn open(
        &self,
        request: OpenTerminal,
    ) -> BoxFuture<'static, Result<Box<dyn TerminalConnection>, TerminalOpenFailure>> {
        let opened = match self.hosts.iter().find(|host| host.key == request.key) {
            None => Err(TerminalOpenFailure::UnknownHost),
            Some(host)
                if request.expect.as_ref().is_some_and(|expect| {
                    expect.endpoint != host.endpoint || expect.identity != host.identity
                }) =>
            {
                Err(TerminalOpenFailure::HostChanged)
            }
            Some(_host) => {
                lock(&self.opened).push(request.key.clone());
                let connection: Box<dyn TerminalConnection> =
                    Box::new(ScriptedTerminal::start(None, FAKE_HOME.to_owned(), None));
                Ok(connection)
            }
        };
        Box::pin(std::future::ready(opened))
    }
}

/// The first run of 32 hexadecimal digits of a setup: its nonce.
fn nonce_in(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut start = 0;
    while start + 32 <= bytes.len() {
        let run = bytes[start..]
            .iter()
            .take_while(|byte| byte.is_ascii_hexdigit())
            .count();
        if run >= 32 {
            return Some(text[start..start + 32].to_owned());
        }
        start += run.max(1);
    }
    None
}

/// The scripted shell's state.
#[derive(Debug)]
struct Shell {
    nonce: Option<String>,
    line: String,
    folder: String,
    sleeping: bool,
    ended: bool,
    sender: mpsc::Sender<TerminalOutput>,
}

impl Shell {
    fn send(&self, text: &str) {
        let _sent = self.sender.try_send(TerminalOutput::Bytes {
            bytes: text.as_bytes().to_vec(),
        });
    }

    fn marker(&self, kind: &str) -> String {
        self.nonce
            .as_ref()
            .map(|nonce| format!("\x1b]633;{kind};{nonce}\x07"))
            .unwrap_or_default()
    }

    fn prompt(&self) {
        let folder = self.folder.replace('\\', "\\\\").replace(';', "\\x3b");
        let mut text = self.marker(&format!("P;Cwd={folder}"));
        text.push_str(&self.marker(&format!("P;User={FAKE_USER}")));
        text.push_str(&self.marker("A"));
        let _written = write!(text, "{}$ ", self.folder);
        text.push_str(&self.marker("B"));
        self.send(&text);
    }

    fn end(&mut self, ending: TerminalEnding) {
        if self.ended {
            return;
        }
        self.ended = true;
        let _sent = self.sender.try_send(TerminalOutput::Ended { ending });
    }

    fn input(&mut self, bytes: &[u8]) {
        for character in String::from_utf8_lossy(bytes).chars() {
            if self.ended {
                return;
            }
            if self.sleeping {
                if character == '\x03' {
                    self.sleeping = false;
                    self.send("^C\r\n");
                    self.finish(130);
                }
                continue;
            }
            match character {
                '\r' => self.enter(),
                '\x03' => {
                    self.line.clear();
                    self.send("^C\r\n");
                    self.prompt();
                }
                '\x7f' | '\x08' => {
                    if self.line.pop().is_some() {
                        self.send("\x08 \x08");
                    }
                }
                character if !character.is_control() => {
                    self.line.push(character);
                    self.send(&character.to_string());
                }
                _ => {}
            }
        }
    }

    fn finish(&mut self, code: i32) {
        let text = self.marker(&format!("D;{code}"));
        self.send(&text);
        self.prompt();
    }

    fn enter(&mut self) {
        let line = std::mem::take(&mut self.line);
        self.send("\r\n");
        // A typed setup gives the shell its markers, then clears the screen.
        if self.nonce.is_none()
            && line.contains("__mannie_n")
            && let Some(nonce) = nonce_in(&line)
        {
            self.nonce = Some(nonce);
            self.send("\x1b[H\x1b[2J");
            self.prompt();
            return;
        }
        let command = line.trim().to_owned();
        if command.is_empty() {
            self.prompt();
            return;
        }
        let escaped = command.replace('\\', "\\\\").replace(';', "\\x3b");
        let mut start = self.marker(&format!("E;{escaped}"));
        start.push_str(&self.marker("C"));
        self.send(&start);
        let (word, rest) = command.split_once(' ').unwrap_or((command.as_str(), ""));
        match word {
            "echo" => {
                self.send(&format!("{rest}\r\n"));
                self.finish(0);
            }
            "lines" => {
                let count: u32 = rest.trim().parse().unwrap_or(0);
                let mut text = String::new();
                for line in 1..=count {
                    let _written = write!(text, "line {line}\r\n");
                }
                self.send(&text);
                self.finish(0);
            }
            "cd" => {
                rest.trim().clone_into(&mut self.folder);
                self.finish(0);
            }
            "fail" => {
                self.send("error: it failed\r\n");
                self.finish(1);
            }
            "sleep" => self.sleeping = true,
            "forge" => {
                self.send("\x1b]633;D;0;00000000000000000000000000000000\x07forged\r\n");
                self.finish(0);
            }
            "exit" => {
                let code = rest.trim().parse().unwrap_or(0);
                self.end(TerminalEnding::Exited { code });
            }
            _ => {
                self.send(&format!("{word}: command not found\r\n"));
                self.finish(127);
            }
        }
    }
}

/// A terminal whose shell is a script.
#[derive(Debug)]
struct ScriptedTerminal {
    shell: Arc<Mutex<Shell>>,
    output: Option<mpsc::Receiver<TerminalOutput>>,
    /// The fake's record of running commands, and this terminal's place in
    /// it, for a terminal of the fake service.
    fake: Option<(Arc<FakeState>, usize)>,
}

impl ScriptedTerminal {
    /// A shell that starts at its prompt, with markers when `nonce` is set.
    fn start(nonce: Option<String>, folder: String, fake: Option<(Arc<FakeState>, usize)>) -> Self {
        let (sender, receiver) = mpsc::channel(OUTPUT_QUEUE);
        let shell = Arc::new(Mutex::new(Shell {
            nonce,
            line: String::new(),
            folder,
            sleeping: false,
            ended: false,
            sender,
        }));
        lock(&shell).prompt();
        Self {
            shell,
            output: Some(receiver),
            fake,
        }
    }

    fn output_stream(&mut self) -> BoxStream<'static, TerminalOutput> {
        match self.output.take() {
            Some(receiver) => futures_util::stream::unfold(receiver, |mut receiver| async move {
                receiver.recv().await.map(|item| (item, receiver))
            })
            .boxed(),
            None => futures_util::stream::empty().boxed(),
        }
    }

    fn write_now(&self, bytes: &[u8]) -> BoxFuture<'static, Result<(), TerminalIoError>> {
        let mut shell = lock(&self.shell);
        let written = if shell.ended {
            Err(TerminalIoError::Ended)
        } else {
            shell.input(bytes);
            Ok(())
        };
        Box::pin(std::future::ready(written))
    }

    fn resize_now(&self) -> BoxFuture<'static, Result<(), TerminalIoError>> {
        let ended = lock(&self.shell).ended;
        Box::pin(std::future::ready(if ended {
            Err(TerminalIoError::Ended)
        } else {
            Ok(())
        }))
    }

    fn close_now(&self) -> BoxFuture<'static, TerminalEnding> {
        lock(&self.shell).end(TerminalEnding::Closed);
        Box::pin(std::future::ready(TerminalEnding::Closed))
    }
}

impl TerminalConnection for ScriptedTerminal {
    fn output(&mut self) -> BoxStream<'static, TerminalOutput> {
        self.output_stream()
    }

    fn write(&self, bytes: Vec<u8>) -> BoxFuture<'static, Result<(), TerminalIoError>> {
        self.write_now(&bytes)
    }

    fn resize(&self, _size: TerminalSize) -> BoxFuture<'static, Result<(), TerminalIoError>> {
        self.resize_now()
    }

    fn close(self: Box<Self>, _grace: Duration) -> BoxFuture<'static, TerminalEnding> {
        self.close_now()
    }
}

impl Terminal for ScriptedTerminal {
    fn output(&mut self) -> BoxStream<'static, TerminalOutput> {
        self.output_stream()
    }

    fn write(&self, bytes: Vec<u8>) -> BoxFuture<'static, Result<(), TerminalIoError>> {
        self.write_now(&bytes)
    }

    fn resize(&self, _size: TerminalSize) -> BoxFuture<'static, Result<(), TerminalIoError>> {
        self.resize_now()
    }

    fn set_running(&self, running: bool) {
        if let Some((fake, index)) = &self.fake
            && let Some(flag) = lock(&fake.running).get_mut(*index)
        {
            *flag = running;
        }
    }

    fn close(self: Box<Self>, _grace: Duration) -> BoxFuture<'static, TerminalEnding> {
        self.close_now()
    }
}

#[cfg(test)]
mod tests {
    use super::nonce_in;

    #[test]
    fn the_nonce_is_the_first_run_of_32_hexadecimal_digits() {
        let nonce = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            nonce_in(&format!("__mannie_n='{nonce}'")),
            Some(nonce.to_owned())
        );
        assert_eq!(nonce_in("deadbeef only"), None);
    }
}
