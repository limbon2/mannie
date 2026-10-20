//! The process supervisor (ADR-0014, docs/features/system-tools/commands.md):
//! the shell that runs commands, the containment that ends every process a
//! command started when the command ends, and the capture of its output. It
//! also runs the installed programs of plugins without a shell, contained
//! the same way, with their output as bytes (ADR-0038).
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
use crate::programs::{PROGRAM_ERROR_BYTES, ProgramEnding};
use crate::programs::{ProgramOutput, ProgramRun};
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
use crate::stop::{StopReason, StopSignal};
use crate::system::ShellFacts;
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
pub(crate) use capture::Captured;
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
use capture::{ByteStream, Stream};

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

/// Runs an installed program contained, without a shell (ADR-0038): when it
/// ends, on its own, at its timeout or when its output passes its limit,
/// every process it started ends with it. Dropping the future ends them too.
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
pub(crate) async fn run_program(mut run: ProgramRun) -> std::io::Result<ProgramOutput> {
    let started = tokio::time::Instant::now();
    let input = std::mem::take(&mut run.input);
    let (contained, child) = platform::Contained::spawn_program(&run)?;
    let mut running = Running {
        parts: Some((contained, child)),
    };
    let (ending, exit_code, output, errors) = running.watch(&run, input).await;
    running.end(ending == ProgramEnding::Exited).await;
    let duration = started.elapsed();
    let output = ByteStream::finish(output).await;
    let errors = ByteStream::finish(errors).await;
    Ok(ProgramOutput {
        ending,
        exit_code,
        duration,
        output: output.bytes,
        output_total: output.total,
        errors: errors.bytes,
        errors_total: errors.total,
    })
}

/// No other system runs programs, as no other system runs commands.
#[cfg(not(any(windows, target_os = "linux", target_os = "freebsd")))]
pub(crate) async fn run_program(run: ProgramRun) -> std::io::Result<ProgramOutput> {
    let _ = run;
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "this system runs no programs",
    ))
}

/// A program's run while it lasts. A run whose future its caller dropped
/// still ends the program's whole tree: on Windows, closing the job's handle
/// would end it by itself, but on Unix only a walk of the shell's
/// descendants reaches the processes that left its process group, and the
/// walk must come before the shell ends, while they are still below it.
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
struct Running {
    parts: Option<(platform::Contained, tokio::process::Child)>,
}

#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
impl Running {
    /// Sends the input and waits until the program ends, its timeout passes
    /// or its output goes past its limit. Returns how it ended, its exit code
    /// and its two streams, which go on being read.
    async fn watch(
        &mut self,
        run: &ProgramRun,
        input: Vec<u8>,
    ) -> (
        ProgramEnding,
        Option<i32>,
        Option<ByteStream>,
        Option<ByteStream>,
    ) {
        let Some((contained, child)) = self.parts.as_mut() else {
            return (ProgramEnding::Exited, None, None, None);
        };
        contained.send_input(child, input);
        let output = child
            .stdout
            .take()
            .map(|pipe| ByteStream::capture(pipe, run.output_limit, true));
        let errors = child
            .stderr
            .take()
            .map(|pipe| ByteStream::capture(pipe, PROGRAM_ERROR_BYTES, false));
        let (ending, exit_code) = tokio::select! {
            // A program that ended counts as ended, even when its output
            // went past the limit at the same moment.
            biased;
            code = contained.finished(child) => (ProgramEnding::Exited, code),
            () = ByteStream::full(output.as_ref()) => (ProgramEnding::OutputFull, None),
            () = tokio::time::sleep(run.timeout) => (ProgramEnding::TimedOut, None),
        };
        (ending, exit_code, output, errors)
    }

    /// Ends what is left of the program's tree and waits for it.
    async fn end(mut self, graceful: bool) {
        if let Some((mut contained, mut child)) = self.parts.take() {
            contained.end(&mut child, graceful).await;
        }
    }
}

#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
impl Drop for Running {
    fn drop(&mut self) {
        let Some((mut contained, mut child)) = self.parts.take() else {
            return;
        };
        // Without a runtime, as while Mannie shuts down, the child's own kill
        // on drop ends the shell or the program, and on Windows the job's
        // handle ends the rest as it closes.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                contained.end(&mut child, false).await;
            });
        }
    }
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
