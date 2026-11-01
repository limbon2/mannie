//! A shell attached to a pseudo console, for a terminal (ADR-0053, decision
//! 1): contained as every command is (ADR-0014), without a timeout, with its
//! input and output as bytes and a size that changes. It ends when its shell
//! exits on its own, or when its owner ends it: the pseudo console closes
//! first, which hangs up the shell, and after a grace what is left of its
//! tree ends.
//!
//! One thread reads the pseudo console and one writes to it, because both
//! Windows' pipes and a PTY's descriptor block; the reader hands its chunks
//! to an async channel and the writer takes its bytes from one, in order.
//! Terminals are built for Windows and Linux, the desktop's platforms (D6).

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use mannie_types::tools::ProcessRecord;
use tokio::sync::{mpsc, oneshot};

#[cfg(target_os = "linux")]
use super::unix_terminal as platform;
#[cfg(windows)]
use super::windows_terminal as platform;

/// How many chunks of output wait for their reader before the reading thread
/// waits too. A chunk holds at most [`READ_CHUNK_BYTES`].
const OUTPUT_QUEUE: usize = 64;

/// The most bytes of one read from the pseudo console.
pub(super) const READ_CHUNK_BYTES: usize = 64 * 1024;

/// How many writes wait for the writing thread.
const INPUT_QUEUE: usize = 256;

/// A shell to start in a pseudo console.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLaunch {
    /// The shell's canonical path.
    pub program: PathBuf,
    /// Its arguments, exactly as it receives them.
    pub arguments: Vec<OsString>,
    /// The canonical folder that it starts in.
    pub working_directory: PathBuf,
    /// The variables that it receives.
    pub environment: Vec<(OsString, OsString)>,
    /// The terminal's columns.
    pub columns: u16,
    /// The terminal's rows.
    pub rows: u16,
}

/// A shell that runs in a pseudo console, with its tree.
///
/// Dropping it ends its tree too, while a runtime runs; its owner ends it
/// with [`TerminalProcess::end`] before the runtime stops.
#[derive(Debug)]
pub struct TerminalProcess {
    #[cfg(any(windows, target_os = "linux"))]
    pty: Option<platform::Pty>,
    output: Option<mpsc::Receiver<Vec<u8>>>,
    input: TerminalInput,
    record: ProcessRecord,
}

/// The input of a terminal's shell, which writes in the order of the calls.
#[derive(Debug, Clone)]
pub struct TerminalInput {
    writes: mpsc::Sender<(Vec<u8>, oneshot::Sender<io::Result<()>>)>,
}

impl TerminalInput {
    /// Writes bytes to the shell's input after every earlier write, and
    /// resolves once they are written. A shell that ended takes nothing.
    pub async fn write(&self, bytes: Vec<u8>) -> io::Result<()> {
        let (done, written) = oneshot::channel();
        self.writes
            .send((bytes, done))
            .await
            .map_err(|_closed| ended())?;
        written.await.map_err(|_dropped| ended())?
    }
}

fn ended() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "the terminal ended")
}

impl TerminalProcess {
    /// Starts the shell in a new pseudo console, contained.
    #[cfg(any(windows, target_os = "linux"))]
    pub fn spawn(launch: &TerminalLaunch) -> io::Result<Self> {
        let (pty, console_output, console_input) = platform::Pty::spawn(launch)?;
        let record = pty.record();
        let (chunks, output) = mpsc::channel(OUTPUT_QUEUE);
        std::thread::Builder::new()
            .name("terminal-output".to_owned())
            .spawn(move || read_output(console_output, &chunks))?;
        let (writes, queued) = mpsc::channel(INPUT_QUEUE);
        std::thread::Builder::new()
            .name("terminal-input".to_owned())
            .spawn(move || write_input(console_input, queued))?;
        Ok(Self {
            pty: Some(pty),
            output: Some(output),
            input: TerminalInput { writes },
            record,
        })
    }

    /// No other system runs terminals yet (ADR-0053, decision 1).
    #[cfg(not(any(windows, target_os = "linux")))]
    pub fn spawn(launch: &TerminalLaunch) -> io::Result<Self> {
        let _ = launch;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this system runs no terminals",
        ))
    }

    /// The shell's process.
    pub fn record(&self) -> &ProcessRecord {
        &self.record
    }

    /// The shell's output as chunks of bytes, in order, which its owner
    /// takes once. The channel closes when the pseudo console closes.
    pub fn take_output(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> {
        self.output.take()
    }

    /// The shell's input.
    pub fn input(&self) -> TerminalInput {
        self.input.clone()
    }

    /// Changes the pseudo console's size, which the shell and its programs
    /// learn of as a resized terminal.
    pub fn resize(&self, columns: u16, rows: u16) -> io::Result<()> {
        #[cfg(any(windows, target_os = "linux"))]
        if let Some(pty) = &self.pty {
            return pty.resize(columns, rows);
        }
        let _ = (columns, rows);
        Err(ended())
    }

    /// Waits until the shell exits on its own, and returns its exit code.
    pub async fn finished(&mut self) -> Option<i32> {
        #[cfg(any(windows, target_os = "linux"))]
        if let Some(pty) = self.pty.as_mut() {
            return pty.finished().await;
        }
        std::future::pending().await
    }

    /// Ends the terminal: closes the pseudo console, which hangs up the
    /// shell, waits at most `grace` for it to exit, then ends what is left of
    /// its tree. Returns the shell's exit code when it exited in time.
    pub async fn end(mut self, grace: Duration) -> Option<i32> {
        #[cfg(any(windows, target_os = "linux"))]
        {
            let pty = self.pty.take()?;
            pty.end(grace).await
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let _ = grace;
            None
        }
    }
}

#[cfg(any(windows, target_os = "linux"))]
impl Drop for TerminalProcess {
    fn drop(&mut self) {
        if let Some(pty) = self.pty.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move {
                pty.end(Duration::ZERO).await;
            });
        }
    }
}

/// Reads the pseudo console until it closes, and hands each chunk on. The
/// thread waits while the chunks wait for their reader, so a reader that
/// stopped reading slows the shell rather than Mannie's memory.
#[cfg(any(windows, target_os = "linux"))]
fn read_output(mut reader: impl io::Read, chunks: &mpsc::Sender<Vec<u8>>) {
    let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
    loop {
        let read = match reader.read(&mut buffer) {
            // A PTY whose shell and programs closed it reads `EIO` on Linux,
            // and a closed pipe ends with zero bytes or an error on Windows.
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        if chunks.blocking_send(buffer[..read].to_vec()).is_err() {
            return;
        }
    }
}

/// Writes the queued bytes to the pseudo console in order, until every
/// sender drops or a write fails.
#[cfg(any(windows, target_os = "linux"))]
fn write_input(
    mut writer: impl io::Write,
    mut queued: mpsc::Receiver<(Vec<u8>, oneshot::Sender<io::Result<()>>)>,
) {
    while let Some((bytes, done)) = queued.blocking_recv() {
        let written = writer.write_all(&bytes).and_then(|()| writer.flush());
        let failed = written.is_err();
        // The caller may have stopped waiting; the bytes are written anyway.
        drop(done.send(written));
        if failed {
            return;
        }
    }
}
