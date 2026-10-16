//! The process supervisor (ADR-0014, docs/features/system-tools/commands.md):
//! the shell that runs commands, the containment that ends every process a
//! command started when the command ends, and the capture of its output.
//!
//! Its platform modules are the only code in Mannie that uses `unsafe`, and
//! each unsafe block says why it is sound.

#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
mod capture;
#[cfg(target_os = "freebsd")]
mod freebsd;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
use std::ffi::OsString;
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
use std::path::Path;
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
use std::time::Duration;

#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
use mannie_types::tools::CommandEnding;
use mannie_types::tools::ProcessRecord;

#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
use crate::stop::{StopReason, StopSignal};
use crate::system::ShellFacts;
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
pub(crate) use capture::Captured;
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
use capture::Stream;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use unix as platform;
#[cfg(windows)]
use windows as platform;

/// The shell that runs commands on this platform, or `None` where commands
/// are not available yet.
pub(crate) fn shell() -> Option<ShellFacts> {
    platform_shell()
}

/// Windows PowerShell 5.1 from the system folder, never from `PATH`.
#[cfg(windows)]
fn platform_shell() -> Option<ShellFacts> {
    let system = windows::system_directory().ok()?;
    Some(ShellFacts {
        name: "Windows PowerShell 5.1".to_owned(),
        path: system.join(r"WindowsPowerShell\v1.0\powershell.exe"),
    })
}

/// `/bin/sh`, with the shell it links to, such as `dash`.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "Windows can fail to find its shell"
)]
fn platform_shell() -> Option<ShellFacts> {
    let path = std::path::PathBuf::from("/bin/sh");
    let name = match std::fs::read_link(&path) {
        Ok(target) => format!("sh ({})", target.display()),
        Err(_not_a_link) => "sh".to_owned(),
    };
    Some(ShellFacts { name, path })
}

/// No other system runs commands (docs/features/system-tools/commands.md,
/// the shell).
#[cfg(not(any(windows, target_os = "linux", target_os = "freebsd")))]
fn platform_shell() -> Option<ShellFacts> {
    None
}

/// A command to run.
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
#[derive(Debug)]
pub(crate) struct CommandSpec<'a> {
    /// The shell.
    pub(crate) shell: &'a Path,
    /// The command's text.
    pub(crate) text: &'a str,
    /// The canonical working directory.
    pub(crate) working_directory: &'a Path,
    /// The variables the command receives.
    pub(crate) environment: &'a [(OsString, OsString)],
    /// How long the command may run.
    pub(crate) timeout: Duration,
}

/// A command that ended, with every process it started.
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
#[derive(Debug)]
pub(crate) struct Finished {
    /// How it ended.
    pub(crate) ending: CommandEnding,
    /// Its exit code, when it ended on its own.
    pub(crate) exit_code: Option<i32>,
    /// How long it ran.
    pub(crate) duration: Duration,
    /// Its standard output.
    pub(crate) stdout: Captured,
    /// Its standard error.
    pub(crate) stderr: Captured,
}

/// Runs a command contained: when it ends, on its own, at its timeout or by
/// Stop, every process it started ends with it. `on_start` receives the
/// shell's process as soon as it runs, for the journal.
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
pub(crate) async fn run(
    spec: &CommandSpec<'_>,
    stop: &StopSignal,
    on_start: impl FnOnce(ProcessRecord),
) -> std::io::Result<Finished> {
    let started = tokio::time::Instant::now();
    // The Windows shell receives the command through its standard input,
    // which the spawn writes; the Unix shell receives it as an argument.
    #[cfg(windows)]
    let (mut contained, mut child) = platform::Contained::spawn(spec).await?;
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let (mut contained, mut child) = platform::Contained::spawn(spec)?;
    on_start(contained.record());
    let stdout = child.stdout.take().map(Stream::capture);
    let stderr = child.stderr.take().map(Stream::capture);
    let (ending, exit_code) = tokio::select! {
        code = contained.finished(&mut child) => (CommandEnding::Exited, code),
        () = tokio::time::sleep(spec.timeout) => (CommandEnding::TimedOut, None),
        () = stop.stopped() => (ending_of(stop.reason()), None),
    };
    contained
        .end(&mut child, ending == CommandEnding::Exited)
        .await;
    let duration = started.elapsed();
    Ok(Finished {
        ending,
        exit_code,
        duration,
        stdout: Stream::finish(stdout).await,
        stderr: Stream::finish(stderr).await,
    })
}

/// Ends what is left of a command whose run ended with Mannie, as a best
/// effort (docs/features/system-tools/commands.md, crash recovery). On
/// Windows the job ended with Mannie, so nothing is left.
pub async fn end_leftover_command(record: &ProcessRecord) {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    unix::end_leftovers(record).await;
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    let _ = record;
}

/// How a stopped command ended.
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
fn ending_of(reason: Option<StopReason>) -> CommandEnding {
    match reason {
        Some(StopReason::Interrupted) => CommandEnding::Interrupted,
        Some(StopReason::Stopped) | None => CommandEnding::Stopped,
    }
}
