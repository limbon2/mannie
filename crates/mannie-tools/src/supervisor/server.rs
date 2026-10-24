//! A program that runs as a server (ADR-0043, decision 5): contained as every
//! command is, without a timeout, with its standard input open for as long as
//! it runs, its output as bytes and its error output as a stream of its own.
//! It ends when it exits on its own or when its owner ends it: first its
//! input closes, so a server that reads to the end exits by itself, and after
//! a grace its whole tree ends, as ADR-0014 decision 8 says for every
//! command.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use mannie_types::tools::ProcessRecord;
use tokio::io::{AsyncRead, AsyncWrite};

#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
use super::platform;

/// A program to run as a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSpec {
    /// The program's canonical path: an `.exe` file, or on Windows a `.cmd`
    /// or `.bat` file, which the standard library starts through `cmd.exe`
    /// and refuses arguments that `cmd.exe` would read as code.
    pub program: PathBuf,
    /// Its arguments, exactly as it receives them.
    pub arguments: Vec<String>,
    /// The canonical working directory.
    pub working_directory: PathBuf,
    /// The variables that the program receives.
    pub environment: Vec<(OsString, OsString)>,
}

/// The server's standard input, which closes when it drops.
pub type ServerInput = Box<dyn AsyncWrite + Send + Unpin>;

/// One of the server's output streams.
pub type ServerStream = Box<dyn AsyncRead + Send + Unpin>;

/// A program that runs as a server, with its tree.
///
/// Dropping it ends its tree too, while a runtime runs, as a background
/// command's does; its owner ends it with [`ServerProcess::end`] before the
/// runtime stops.
#[derive(Debug)]
pub struct ServerProcess {
    #[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
    parts: Option<(platform::Contained, tokio::process::Child)>,
    input: Option<InputSlot>,
    output: Option<OutputSlot>,
    errors: Option<OutputSlot>,
    record: ProcessRecord,
}

/// The input before its owner takes it. `Debug` names it only.
struct InputSlot(ServerInput);

impl std::fmt::Debug for InputSlot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ServerInput")
    }
}

/// An output stream before its owner takes it. `Debug` names it only.
struct OutputSlot(ServerStream);

impl std::fmt::Debug for OutputSlot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ServerStream")
    }
}

impl ServerProcess {
    /// Starts the program, contained, with its input open.
    #[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
    pub fn spawn(spec: &ServerSpec) -> io::Result<Self> {
        let (contained, mut child, input) = platform::Contained::spawn_server(spec)?;
        let output = child
            .stdout
            .take()
            .map(|stdout| OutputSlot(Box::new(stdout)));
        let errors = child
            .stderr
            .take()
            .map(|stderr| OutputSlot(Box::new(stderr)));
        let record = contained.record();
        Ok(Self {
            parts: Some((contained, child)),
            input: Some(InputSlot(input)),
            output,
            errors,
            record,
        })
    }

    /// No other system runs commands (docs/features/system-tools/commands.md,
    /// the shell).
    #[cfg(not(any(windows, target_os = "linux", target_os = "freebsd")))]
    pub fn spawn(spec: &ServerSpec) -> io::Result<Self> {
        let _ = spec;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this system runs no commands",
        ))
    }

    /// The program's process, or on Unix the shell that holds its tree.
    pub fn record(&self) -> &ProcessRecord {
        &self.record
    }

    /// The server's standard input, which its owner takes once. Dropping it
    /// closes the input.
    pub fn take_input(&mut self) -> Option<ServerInput> {
        self.input.take().map(|slot| slot.0)
    }

    /// The server's standard output, which its owner takes once.
    pub fn take_output(&mut self) -> Option<ServerStream> {
        self.output.take().map(|slot| slot.0)
    }

    /// The server's error output, which its owner takes once.
    pub fn take_errors(&mut self) -> Option<ServerStream> {
        self.errors.take().map(|slot| slot.0)
    }

    /// Waits until the program exits on its own, and returns its exit code.
    pub async fn finished(&mut self) -> Option<i32> {
        #[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
        if let Some((contained, child)) = self.parts.as_mut() {
            return contained.finished(child).await;
        }
        std::future::pending().await
    }

    /// Ends the server: closes its input, if its owner did not take it,
    /// waits at most `grace` for it to exit on its own, then ends what is
    /// left of its tree. Returns its exit code when it exited in time.
    pub async fn end(mut self, grace: Duration) -> Option<i32> {
        self.input = None;
        #[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
        {
            let (mut contained, mut child) = self.parts.take()?;
            let exited = tokio::time::timeout(grace, contained.finished(&mut child))
                .await
                .ok()
                .flatten();
            contained.end(&mut child, exited.is_some()).await;
            exited
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "freebsd")))]
        {
            let _ = grace;
            None
        }
    }
}

#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
impl Drop for ServerProcess {
    fn drop(&mut self) {
        if let Some((mut contained, mut child)) = self.parts.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move {
                contained.end(&mut child, false).await;
            });
        }
    }
}
