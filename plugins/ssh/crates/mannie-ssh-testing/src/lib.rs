//! An SSH server for the SSH plugin's tests, on `127.0.0.1` and a port that
//! the system chooses, with an Ed25519 host key that it generates, or RSA
//! host keys from `keys/`, which only these tests use. It is
//! never shipped: the plugin's Rust tests start it in their process, and the
//! end-to-end tests start its example program.
//!
//! It records every authentication attempt, every command and every signal,
//! so a test can check that a scan sent no password and that Stop reached
//! the command. Commands are scripts that the tests name:
//!
//! - `echo <text>` prints the text and exits 0;
//! - `fail` prints to standard error and exits 3;
//! - `lines <n>` prints n numbered lines;
//! - `print-password` prints the account's password;
//! - `sleep` waits until a signal ends it;
//! - `ignore-signals` waits and ignores signals until its channel closes;
//! - `crash` ends with the signal SEGV and a core dump;
//! - `refuse` is refused before it starts;
//! - `drip <n>` prints n numbered lines, one every 50 ms, then waits until
//!   a signal ends it, as `journalctl -f` does;
//! - `count <n>` prints n numbered lines, one every 50 ms, then exits 0;
//! - `drip-password` prints the account's password cut across two writes,
//!   50 ms apart, then waits until a signal ends it;
//! - `printf '%s' "$SHELL"` prints the login shell, `/bin/bash`;
//! - anything else prints `ran: <command>` and exits 0.
//!
//! A shell on a PTY, as the terminal plugin opens one, echoes what is typed
//! and shows a prompt. It knows `echo <text>`, `print-password`, which
//! prints the account's password cut across two writes, and `exit <code>`.
//! A typed line that holds the terminal plugin's nonce turns on the
//! markers of its setup, which then mark the prompts and commands.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use russh::keys::ssh_encoding::bytes::Bytes;
use russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKey};
use russh::server::{self, Auth, ChannelOpenHandle, Handle, Msg, Session};
use russh::{Channel, ChannelId, MethodKind, MethodSet, Preferred, Sig};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// The account that the server lets in.
pub const USERNAME: &str = "deploy";

/// The account's password.
pub const PASSWORD: &str = "correct horse battery";

/// How the server takes the password.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Login {
    /// Password login, as OpenSSH's default on Linux.
    #[default]
    Password,
    /// Keyboard-interactive login only, which asks for the password in one
    /// hidden prompt, as FreeBSD's default sshd does through PAM.
    KeyboardInteractive,
}

/// The host keys that the server has, and the signatures it offers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HostKeys {
    /// An Ed25519 key that the server generates, and a new one on
    /// [`TestServer::replace_key`].
    #[default]
    Ed25519,
    /// A 2048-bit RSA key, signed with `rsa-sha2-512` only.
    Rsa,
    /// The same RSA key, signed with SHA-1 `ssh-rsa` only, as a server
    /// older than OpenSSH 7.2 does.
    RsaSha1Only,
    /// A 1024-bit RSA key, signed with `rsa-sha2-512`.
    WeakRsa,
    /// A generated Ed25519 key and the 2048-bit RSA key, with russh's
    /// default order of signatures.
    Ed25519AndRsa,
}

/// How a test server behaves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// How the server takes the password.
    pub login: Login,
    /// The server's host keys.
    pub host_keys: HostKeys,
}

/// A 2048-bit RSA host key, made with `ssh-keygen -t rsa -b 2048` for these
/// tests only. Generating one in a debug build takes too long.
const RSA_2048: &str = include_str!("../keys/rsa-2048");

/// A 1024-bit RSA host key, made with `ssh-keygen -t rsa -b 1024` for these
/// tests only.
const RSA_1024: &str = include_str!("../keys/rsa-1024");

/// What the server saw, which tests read.
#[derive(Debug, Default)]
struct Seen {
    auth_attempts: AtomicUsize,
    commands: Mutex<Vec<String>>,
    signals: Mutex<Vec<String>>,
    /// The sizes of PTYs, as their requests and changes gave them.
    pty_sizes: Mutex<Vec<(u32, u32)>>,
    /// The lines that shells on PTYs read.
    shell_lines: Mutex<Vec<String>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only reads or replaces a value.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A running test server. Dropping it stops accepting connections.
#[derive(Debug)]
pub struct TestServer {
    address: SocketAddr,
    options: Options,
    config: Arc<Mutex<Arc<server::Config>>>,
    seen: Arc<Seen>,
    accept: JoinHandle<()>,
}

impl TestServer {
    /// Starts a server on `127.0.0.1` and a free port.
    ///
    /// # Errors
    ///
    /// When the port cannot be bound or the key cannot be made.
    pub async fn start(options: Options) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let config = Arc::new(Mutex::new(Arc::new(config(options, host_keys(options)?))));
        let seen = Arc::new(Seen::default());
        let accept = tokio::spawn(accept(
            listener,
            Arc::clone(&config),
            Arc::clone(&seen),
            options,
        ));
        Ok(Self {
            address,
            options,
            config,
            seen,
            accept,
        })
    }

    /// The server's port.
    pub fn port(&self) -> u16 {
        self.address.port()
    }

    /// The host key that the server shows now: its first one, which is
    /// the Ed25519 key when it has both.
    ///
    /// # Panics
    ///
    /// Never: the server always has a key.
    pub fn key(&self) -> PublicKey {
        let config = Arc::clone(&lock(&self.config));
        config
            .keys
            .first()
            .map(|key| key.public_key().clone())
            .expect("the server has a key")
    }

    /// The SHA-256 fingerprint of the host key, as `ssh-keygen -l` prints it.
    pub fn fingerprint(&self) -> String {
        self.key().fingerprint(HashAlg::Sha256).to_string()
    }

    /// Shows a new host key from the next connection on, as a server that
    /// was reinstalled does, or an attacker in its place.
    ///
    /// # Errors
    ///
    /// When the key cannot be made.
    pub fn replace_key(&self) -> std::io::Result<()> {
        *lock(&self.config) = Arc::new(config(self.options, host_keys(self.options)?));
        Ok(())
    }

    /// How many times a client tried to log in, with any method.
    pub fn auth_attempts(&self) -> usize {
        self.seen.auth_attempts.load(Ordering::SeqCst)
    }

    /// The commands that clients ran, in order.
    pub fn commands(&self) -> Vec<String> {
        lock(&self.seen.commands).clone()
    }

    /// The signals that reached commands, by name, in order.
    pub fn signals(&self) -> Vec<String> {
        lock(&self.seen.signals).clone()
    }

    /// The sizes of PTYs that clients asked for, columns then rows, in order.
    pub fn pty_sizes(&self) -> Vec<(u32, u32)> {
        lock(&self.seen.pty_sizes).clone()
    }

    /// The lines that shells on PTYs read, in order.
    pub fn shell_lines(&self) -> Vec<String> {
        lock(&self.seen.shell_lines).clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

fn new_key() -> std::io::Result<PrivateKey> {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).map_err(std::io::Error::other)
}

fn fixed_key(openssh: &str) -> std::io::Result<PrivateKey> {
    PrivateKey::from_openssh(openssh).map_err(std::io::Error::other)
}

/// The host keys of `options`, the one that [`TestServer::key`] shows
/// first.
fn host_keys(options: Options) -> std::io::Result<Vec<PrivateKey>> {
    Ok(match options.host_keys {
        HostKeys::Ed25519 => vec![new_key()?],
        HostKeys::Rsa | HostKeys::RsaSha1Only => vec![fixed_key(RSA_2048)?],
        HostKeys::WeakRsa => vec![fixed_key(RSA_1024)?],
        HostKeys::Ed25519AndRsa => vec![new_key()?, fixed_key(RSA_2048)?],
    })
}

/// The host key signatures that the server offers, or `None` for russh's
/// default order.
fn host_key_algorithms(host_keys: HostKeys) -> Option<&'static [Algorithm]> {
    match host_keys {
        HostKeys::Ed25519 | HostKeys::Ed25519AndRsa => None,
        HostKeys::Rsa | HostKeys::WeakRsa => Some(&[Algorithm::Rsa {
            hash: Some(HashAlg::Sha512),
        }]),
        HostKeys::RsaSha1Only => Some(&[Algorithm::Rsa { hash: None }]),
    }
}

fn config(options: Options, keys: Vec<PrivateKey>) -> server::Config {
    let mut config = server::Config {
        keys,
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..server::Config::default()
    };
    if options.login == Login::KeyboardInteractive {
        config.methods = MethodSet::from(&[MethodKind::KeyboardInteractive][..]);
    }
    if let Some(algorithms) = host_key_algorithms(options.host_keys) {
        config.preferred = Preferred {
            key: Cow::Borrowed(algorithms),
            ..Preferred::DEFAULT
        };
    }
    config
}

/// Serves each connection with the configuration of the moment it arrives.
async fn accept(
    listener: TcpListener,
    config: Arc<Mutex<Arc<server::Config>>>,
    seen: Arc<Seen>,
    options: Options,
) {
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            return;
        };
        let current = Arc::clone(&lock(&config));
        let connection = Connection {
            seen: Arc::clone(&seen),
            options,
            waiting: HashMap::new(),
            ended: HashMap::new(),
            shells: HashMap::new(),
        };
        tokio::spawn(async move {
            if let Ok(session) = server::run_stream(current, stream, connection).await {
                // A session ends with an error when the client goes away,
                // which a test does on purpose.
                drop(session.await);
            }
        });
    }
}

/// A command that waits for a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waiting {
    /// A signal ends it.
    UntilSignal,
    /// It ignores signals.
    Forever,
}

/// How long a command that writes over time waits between its writes.
const WRITE_PAUSE: Duration = Duration::from_millis(50);

/// One client's connection.
struct Connection {
    seen: Arc<Seen>,
    options: Options,
    waiting: HashMap<ChannelId, Waiting>,
    /// Set for each command that writes over time once its channel ended,
    /// so it writes no more.
    ended: HashMap<ChannelId, Arc<AtomicBool>>,
    /// The shells on PTYs, by channel.
    shells: HashMap<ChannelId, PtyShell>,
}

/// A shell on a PTY: the line being typed, and the nonce of the markers once
/// a setup gave one.
#[derive(Debug, Default)]
struct PtyShell {
    line: String,
    nonce: Option<String>,
}

impl PtyShell {
    fn marker(&self, kind: &str) -> String {
        self.nonce
            .as_ref()
            .map(|nonce| format!("\x1b]633;{kind};{nonce}\x07"))
            .unwrap_or_default()
    }

    fn prompt(&self) -> String {
        let mut prompt = self.marker("P;Cwd=/home/deploy");
        prompt.push_str(&self.marker("A"));
        prompt.push_str("deploy$ ");
        prompt.push_str(&self.marker("B"));
        prompt
    }
}

/// The first run of 32 hexadecimal digits of a line: the nonce of a typed
/// setup.
fn nonce_in(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let mut start = 0;
    while start + 32 <= bytes.len() {
        let run = bytes[start..]
            .iter()
            .take_while(|byte| byte.is_ascii_hexdigit())
            .count();
        if run >= 32 {
            return Some(line[start..start + 32].to_owned());
        }
        start += run.max(1);
    }
    None
}

/// What a command that writes over time does after its writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterWrites {
    /// It waits until a signal ends it.
    Wait,
    /// It exits 0.
    Exit,
}

/// Writes `writes`, one every [`WRITE_PAUSE`], until the channel ends, and
/// then exits when `after` says so.
async fn write_over_time(
    handle: Handle,
    channel: ChannelId,
    writes: Vec<String>,
    after: AfterWrites,
    ended: Arc<AtomicBool>,
) {
    for text in writes {
        tokio::time::sleep(WRITE_PAUSE).await;
        if ended.load(Ordering::SeqCst) || handle.data(channel, Bytes::from(text)).await.is_err() {
            return;
        }
    }
    if after == AfterWrites::Exit && !ended.swap(true, Ordering::SeqCst) {
        // A channel that the client closed meanwhile takes none of them.
        let _status = handle.exit_status_request(channel, 0).await;
        let _eof = handle.eof(channel).await;
        let _closed = handle.close(channel).await;
    }
}

impl Connection {
    /// Starts a shell on a PTY with its first prompt.
    fn start_shell(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        session.channel_success(channel)?;
        let shell = PtyShell::default();
        session.data(channel, Bytes::from(shell.prompt()))?;
        self.shells.insert(channel, shell);
        Ok(())
    }

    /// Echoes what a client typed into a shell, and runs each line.
    fn type_into_shell(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        let Some(shell) = self.shells.get_mut(&channel) else {
            return Ok(());
        };
        let mut lines = Vec::new();
        let mut echo = String::new();
        for character in String::from_utf8_lossy(data).chars() {
            match character {
                '\r' => {
                    echo.push_str("\r\n");
                    lines.push(std::mem::take(&mut shell.line));
                }
                character if !character.is_control() => {
                    shell.line.push(character);
                    echo.push(character);
                }
                _ => {}
            }
        }
        if !echo.is_empty() {
            session.data(channel, Bytes::from(echo))?;
        }
        for line in lines {
            lock(&self.seen.shell_lines).push(line.clone());
            if self.run_line(channel, &line, session)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Runs a line of a shell, and says whether the shell ended.
    fn run_line(
        &mut self,
        channel: ChannelId,
        line: &str,
        session: &mut Session,
    ) -> Result<bool, russh::Error> {
        let Some(shell) = self.shells.get_mut(&channel) else {
            return Ok(true);
        };
        if shell.nonce.is_none()
            && line.contains("__mannie_n")
            && let Some(nonce) = nonce_in(line)
        {
            shell.nonce = Some(nonce);
            let mut text = String::from("\x1b[H\x1b[2J");
            text.push_str(&shell.prompt());
            session.data(channel, Bytes::from(text))?;
            return Ok(false);
        }
        let command = line.trim();
        if let Some(code) = command.strip_prefix("exit") {
            let code = code.trim().parse().unwrap_or(0);
            self.shells.remove(&channel);
            session.exit_status_request(channel, code)?;
            session.eof(channel)?;
            session.close(channel)?;
            return Ok(true);
        }
        if command.is_empty() {
            session.data(channel, Bytes::from(shell.prompt()))?;
            return Ok(false);
        }
        let mut start = shell.marker(&format!("E;{command}"));
        start.push_str(&shell.marker("C"));
        if !start.is_empty() {
            session.data(channel, Bytes::from(start))?;
        }
        if command == "print-password" {
            // The password arrives cut across two writes, as output does.
            let (first, rest) = PASSWORD.split_at(7);
            session.data(channel, Bytes::from(format!("the password is {first}")))?;
            session.data(channel, Bytes::from(format!("{rest}\r\n")))?;
        } else if let Some(text) = command.strip_prefix("echo ") {
            session.data(channel, Bytes::from(format!("{text}\r\n")))?;
        } else {
            session.data(channel, Bytes::from(format!("{command}: not found\r\n")))?;
        }
        let mut end = shell.marker("D;0");
        end.push_str(&shell.prompt());
        session.data(channel, Bytes::from(end))?;
        Ok(false)
    }

    fn password_login(&self, user: &str, password: &str) -> Auth {
        self.seen.auth_attempts.fetch_add(1, Ordering::SeqCst);
        if self.options.login == Login::KeyboardInteractive {
            return Auth::Reject {
                proceed_with_methods: Some(MethodSet::from(&[MethodKind::KeyboardInteractive][..])),
                partial_success: false,
            };
        }
        let is_right = user == USERNAME && password == PASSWORD;
        if is_right {
            Auth::Accept
        } else {
            Auth::reject()
        }
    }

    fn keyboard_interactive_login(
        &self,
        user: &str,
        response: Option<server::Response<'_>>,
    ) -> Auth {
        self.seen.auth_attempts.fetch_add(1, Ordering::SeqCst);
        let Some(mut answers) = response else {
            return Auth::Partial {
                name: Cow::Borrowed(""),
                instructions: Cow::Borrowed(""),
                prompts: Cow::Owned(vec![(Cow::Borrowed("Password for deploy@test: "), false)]),
            };
        };
        let answer = answers.next().map(|answer| answer.to_vec());
        let is_right = user == USERNAME && answer.as_deref() == Some(PASSWORD.as_bytes());
        if is_right {
            Auth::Accept
        } else {
            Auth::reject()
        }
    }

    fn run(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        let command = String::from_utf8_lossy(data).into_owned();
        lock(&self.seen.commands).push(command.clone());
        if command == "refuse" {
            session.channel_failure(channel)?;
            return Ok(());
        }
        session.channel_success(channel)?;
        if let Some((writes, after)) = writes_over_time(&command) {
            if after == AfterWrites::Wait {
                self.waiting.insert(channel, Waiting::UntilSignal);
            }
            let ended = Arc::new(AtomicBool::new(false));
            self.ended.insert(channel, Arc::clone(&ended));
            tokio::spawn(write_over_time(
                session.handle(),
                channel,
                writes,
                after,
                ended,
            ));
            return Ok(());
        }
        match command.as_str() {
            "sleep" => {
                self.waiting.insert(channel, Waiting::UntilSignal);
                return Ok(());
            }
            "ignore-signals" => {
                self.waiting.insert(channel, Waiting::Forever);
                return Ok(());
            }
            "fail" => {
                session.extended_data(channel, 1, Bytes::from_static(b"failed\n"))?;
                session.exit_status_request(channel, 3)?;
            }
            "print-password" => {
                session.data(
                    channel,
                    Bytes::from(format!("the password is {PASSWORD}\n")),
                )?;
                session.exit_status_request(channel, 0)?;
            }
            "crash" => {
                session.exit_signal_request(channel, Sig::SEGV, true, "crashed", "en")?;
            }
            other => {
                session.data(channel, Bytes::from(output_of(other)))?;
                session.exit_status_request(channel, 0)?;
            }
        }
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }

    fn take_signal(
        &mut self,
        channel: ChannelId,
        signal: Sig,
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        lock(&self.seen.signals).push(signal_name(&signal));
        if self.waiting.get(&channel) == Some(&Waiting::UntilSignal) {
            self.waiting.remove(&channel);
            if let Some(ended) = self.ended.remove(&channel) {
                ended.store(true, Ordering::SeqCst);
            }
            session.exit_signal_request(channel, signal, false, "ended", "en")?;
            session.eof(channel)?;
            session.close(channel)?;
        }
        Ok(())
    }
}

/// The writes of a command that writes over time, and what it does after
/// them, or `None` for another command.
fn writes_over_time(command: &str) -> Option<(Vec<String>, AfterWrites)> {
    let numbered = |count: &str| -> Vec<String> {
        let count: usize = count.parse().unwrap_or(0);
        (1..=count).map(|line| format!("line {line}\n")).collect()
    };
    if command == "drip-password" {
        let (start, end) = PASSWORD.split_at(7);
        let writes = vec![format!("the password is {start}"), format!("{end}\n")];
        return Some((writes, AfterWrites::Wait));
    }
    if let Some(count) = command.strip_prefix("drip ") {
        return Some((numbered(count), AfterWrites::Wait));
    }
    let count = command.strip_prefix("count ")?;
    Some((numbered(count), AfterWrites::Exit))
}

/// What `echo`, `lines` and any other command print.
fn output_of(command: &str) -> String {
    if command == "printf '%s' \"$SHELL\"" {
        return "/bin/bash".to_owned();
    }
    if let Some(text) = command.strip_prefix("echo ") {
        return format!("{text}\n");
    }
    let Some(count) = command.strip_prefix("lines ") else {
        return format!("ran: {command}\n");
    };
    let count: usize = count.parse().unwrap_or(0);
    let mut output = String::new();
    for line in 1..=count {
        // Writing to a String cannot fail.
        let _written = writeln!(output, "line {line}");
    }
    output
}

impl server::Handler for Connection {
    type Error = russh::Error;

    fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> impl Future<Output = Result<Auth, Self::Error>> + Send {
        std::future::ready(Ok(self.password_login(user, password)))
    }

    fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<server::Response<'a>>,
    ) -> impl Future<Output = Result<Auth, Self::Error>> + Send {
        std::future::ready(Ok(self.keyboard_interactive_login(user, response)))
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        std::future::ready(self.run(channel, data, session))
    }

    fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        lock(&self.seen.pty_sizes).push((col_width, row_height));
        std::future::ready(session.channel_success(channel))
    }

    fn window_change_request(
        &mut self,
        _channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        lock(&self.seen.pty_sizes).push((col_width, row_height));
        std::future::ready(Ok(()))
    }

    fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        std::future::ready(self.start_shell(channel, session))
    }

    fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        std::future::ready(self.type_into_shell(channel, data, session))
    }

    fn signal(
        &mut self,
        channel: ChannelId,
        signal: Sig,
        session: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        std::future::ready(self.take_signal(channel, signal, session))
    }

    fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.waiting.remove(&channel);
        self.shells.remove(&channel);
        if let Some(ended) = self.ended.remove(&channel) {
            ended.store(true, Ordering::SeqCst);
        }
        std::future::ready(Ok(()))
    }
}

/// A signal's name.
fn signal_name(signal: &Sig) -> String {
    match signal {
        Sig::TERM => "TERM".to_owned(),
        Sig::KILL => "KILL".to_owned(),
        Sig::INT => "INT".to_owned(),
        Sig::HUP => "HUP".to_owned(),
        other => format!("{other:?}"),
    }
}
