//! A command that runs in the background (ADR-0041,
//! docs/features/system-tools/background-commands.md): contained as every
//! command is, with its output in a [`LiveLog`], and without a timeout. It
//! outlives the tool call that started it, and ends when it exits on its own
//! or when its owner ends it. When it ends, everything that it started ends
//! with it, as ADR-0014 decision 8 says for every command.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
use std::time::Duration;

use mannie_types::tools::ProcessRecord;
use tokio::task::JoinHandle;

#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
use super::live;
use super::live::LiveLog;
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
use super::{CommandSpec, platform};

/// How long the end of a command waits for its streams to close after its
/// processes ended. Only a process outside the command's tree can keep them
/// open.
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// A command to run in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundSpec {
    /// The shell.
    pub shell: PathBuf,
    /// The command's text.
    pub text: String,
    /// The canonical working directory.
    pub working_directory: PathBuf,
    /// The variables that the command receives.
    pub environment: Vec<(OsString, OsString)>,
}

/// A command that runs in the background, with its tree and its log.
///
/// Dropping it ends its tree too, while a runtime runs. Without one, as
/// while a process shuts down, the shell's own kill on drop ends the shell,
/// and on Windows the job's handle ends the rest as it closes, but on Unix
/// the processes that left the shell's process group would outlive it: only
/// a walk made while the shell lives reaches them. The owner of a background
/// command therefore ends it with [`BackgroundProcess::end`] before the
/// runtime stops.
#[derive(Debug)]
pub struct BackgroundProcess {
    #[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
    parts: Option<(platform::Contained, tokio::process::Child)>,
    log: LiveLog,
    readers: Vec<JoinHandle<()>>,
    record: ProcessRecord,
}

impl BackgroundProcess {
    /// Starts the command in its shell, contained, and starts reading its
    /// output into its log.
    #[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
    pub async fn spawn(spec: &BackgroundSpec) -> io::Result<Self> {
        let command = CommandSpec {
            shell: &spec.shell,
            text: &spec.text,
            working_directory: &spec.working_directory,
            environment: &spec.environment,
        };
        let (contained, mut child) = spawn_contained(&command).await?;
        let log = LiveLog::new();
        let mut readers = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            readers.push(live::read_stream(log.clone(), stdout));
        }
        if let Some(stderr) = child.stderr.take() {
            readers.push(live::read_stream(log.clone(), stderr));
        }
        let record = contained.record();
        Ok(Self {
            parts: Some((contained, child)),
            log,
            readers,
            record,
        })
    }

    /// No other system runs commands (docs/features/system-tools/commands.md,
    /// the shell).
    #[cfg(not(any(windows, target_os = "linux", target_os = "freebsd")))]
    pub async fn spawn(spec: &BackgroundSpec) -> io::Result<Self> {
        let _ = spec;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this system runs no commands",
        ))
    }

    /// The shell's process, for the journal and the recovery after a crash.
    pub fn record(&self) -> &ProcessRecord {
        &self.record
    }

    /// The command's log, which its readers share.
    pub fn log(&self) -> &LiveLog {
        &self.log
    }

    /// Waits until the command ends on its own, and returns its exit code.
    pub async fn finished(&mut self) -> Option<i32> {
        #[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
        if let Some((contained, child)) = self.parts.as_mut() {
            return contained.finished(child).await;
        }
        std::future::pending().await
    }

    /// Ends what is left of the command's tree, and then its shell: after the
    /// command's own end, `graceful` lets the shell exit; otherwise the shell
    /// is killed, after its tree. Then waits for the streams to be read to
    /// their end, and ends the log.
    pub async fn end(mut self, graceful: bool) {
        #[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
        {
            if let Some((mut contained, mut child)) = self.parts.take() {
                contained.end(&mut child, graceful).await;
            }
            let readers = std::mem::take(&mut self.readers);
            let deadline = tokio::time::Instant::now() + DRAIN_TIMEOUT;
            for mut reader in readers {
                if tokio::time::timeout_at(deadline, &mut reader)
                    .await
                    .is_err()
                {
                    reader.abort();
                }
            }
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "freebsd")))]
        let _ = graceful;
        self.log.end();
    }
}

/// Windows writes the command to the shell asynchronously; Unix passes it
/// as an argument when it starts the shell.
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
fn spawn_contained(
    command: &CommandSpec<'_>,
) -> impl Future<Output = io::Result<(platform::Contained, tokio::process::Child)>> {
    #[cfg(windows)]
    {
        platform::Contained::spawn(command)
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        std::future::ready(platform::Contained::spawn(command))
    }
}

impl Drop for BackgroundProcess {
    fn drop(&mut self) {
        for reader in &self.readers {
            reader.abort();
        }
        #[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
        if let Some((mut contained, mut child)) = self.parts.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move {
                contained.end(&mut child, false).await;
            });
        }
    }
}
