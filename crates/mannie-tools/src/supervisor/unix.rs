//! The Unix half of the process supervisor (ADR-0014, docs/features/
//! system-tools/commands.md): the shell, the wrapper that reports the
//! command's status, and the end of the shell's tree. The platform module
//! keeps every process that the command starts below the shell: Linux makes
//! the shell a child subreaper, FreeBSD makes it a reaper, and both flags
//! survive exec.
//!
//! A fixed wrapper runs the command, reports its status on descriptor 3, and
//! then waits on its standard input, so the shell outlives the tree until the
//! supervisor has ended it.
#![expect(
    unsafe_code,
    reason = "the process supervisor prepares the shell's tree and moves its status descriptor between fork and exec (ADR-0014)"
)]

use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use mannie_types::tools::ProcessRecord;
use rustix::io::FdFlags;
use rustix::pipe::PipeFlags;
use rustix::process::Pid;
use tokio::process::{Child, ChildStdin, Command};
use tokio::task::JoinHandle;

use super::CommandSpec;
#[cfg(target_os = "freebsd")]
use super::freebsd as tree;
#[cfg(target_os = "linux")]
use super::linux as tree;
use crate::programs::ProgramRun;

/// The wrapper that the shell runs. The command runs in a subshell, with
/// standard input from `/dev/null` and without the status descriptor; its
/// status goes to descriptor 3, which then closes; and the shell waits on
/// its standard input until the supervisor closes it.
const WRAPPER: &str = "(eval \"$1\") </dev/null 3>&-\n\
status=$?\n\
printf '%s\\n' \"$status\" >&3\n\
exec 3>&-\n\
read -r ignored\n\
exit \"$status\"\n";

/// The wrapper that runs an installed program (ADR-0038). The shell runs its
/// positional parameters, the program's path and its arguments, as one
/// command, so it never reads an argument as code. The program reads its
/// input from descriptor 4; the rest is the command wrapper's: the status on
/// descriptor 3, then a wait on standard input until the supervisor closes
/// it.
const PROGRAM_WRAPPER: &str = "(\"$@\") <&4 3>&- 4<&-\n\
status=$?\n\
exec 4<&-\n\
printf '%s\\n' \"$status\" >&3\n\
exec 3>&-\n\
read -r ignored\n\
exit \"$status\"\n";

/// The shell that holds a program's tree. It never reads the program's
/// arguments, so the system's `/bin/sh` serves on every Unix.
const PROGRAM_SHELL: &str = "/bin/sh";

/// The lowest descriptor of the copies that keep the status and input pipes
/// apart while they move to descriptors 3 and 4.
const SPARE_DESCRIPTORS: i32 = 10;

/// How long the processes of a stopped command have to end after `SIGTERM`.
pub(super) const TERM_GRACE: Duration = Duration::from_millis(500);

/// How often the supervisor looks again whether the processes have ended.
pub(super) const POLL: Duration = Duration::from_millis(20);

/// The most rounds of `SIGSTOP` and `SIGKILL`; a process in an
/// uninterruptible sleep cannot die until it wakes.
pub(super) const MAX_KILL_ROUNDS: u32 = 250;

/// A running command: its shell, the shell's standard input, and the status
/// that the wrapper reports. A program's run also holds the pipe of its input
/// until the input is sent.
#[derive(Debug)]
pub(super) struct Contained {
    shell: Pid,
    record: ProcessRecord,
    stdin: Option<ChildStdin>,
    status: Option<JoinHandle<Option<i32>>>,
    input: Option<OwnedFd>,
}

impl Contained {
    /// Starts the shell with the wrapper and the command's text, which it
    /// receives as an argument.
    pub(super) fn spawn(spec: &CommandSpec<'_>) -> io::Result<(Self, Child)> {
        let (status_read, status_write) = rustix::pipe::pipe_with(PipeFlags::CLOEXEC)?;
        let status_fd = status_write.as_raw_fd();
        let mut command = Command::new(spec.shell);
        command
            .arg("-c")
            .arg(WRAPPER)
            .arg("mannie")
            .arg(spec.text)
            .current_dir(spec.working_directory)
            .env_clear()
            .envs(spec.environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        // SAFETY: the closure runs in the child between fork and exec, where
        // only async-signal-safe calls are allowed. It makes three system
        // calls through rustix (prctl on Linux or procctl on FreeBSD, then
        // dup2 and fcntl), which take no locks and allocate nothing.
        unsafe {
            command.pre_exec(move || prepare_child(status_fd));
        }
        let mut child = command.spawn()?;
        // The shell holds its copy as descriptor 3; once it closes it, the
        // read below ends.
        drop(status_write);
        let contained = Self::watch(&mut child, status_read, None)?;
        Ok((contained, child))
    }

    /// Starts the shell with the program wrapper, the program's path and its
    /// arguments, each one argument of the shell (ADR-0038).
    pub(super) fn spawn_program(run: &ProgramRun) -> io::Result<(Self, Child)> {
        let (status_read, status_write) = rustix::pipe::pipe_with(PipeFlags::CLOEXEC)?;
        let (input_read, input_write) = rustix::pipe::pipe_with(PipeFlags::CLOEXEC)?;
        let status_fd = status_write.as_raw_fd();
        let input_fd = input_read.as_raw_fd();
        let mut command = Command::new(Path::new(PROGRAM_SHELL));
        command
            .arg("-c")
            .arg(PROGRAM_WRAPPER)
            .arg("mannie")
            .arg(&run.program)
            .args(&run.arguments)
            .current_dir(&run.working_directory)
            .env_clear()
            .envs(run.environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        // SAFETY: the closure runs in the child between fork and exec, where
        // only async-signal-safe calls are allowed. It makes system calls
        // through rustix (prctl on Linux or procctl on FreeBSD, then fcntl,
        // dup2 and close), which take no locks and allocate nothing.
        unsafe {
            command.pre_exec(move || prepare_program_child(status_fd, input_fd));
        }
        let mut child = command.spawn()?;
        // The shell holds its copies as descriptors 3 and 4.
        drop(status_write);
        drop(input_read);
        let contained = Self::watch(&mut child, status_read, Some(input_write))?;
        Ok((contained, child))
    }

    /// The running shell, with the read of the status that it will report.
    fn watch(child: &mut Child, status_read: OwnedFd, input: Option<OwnedFd>) -> io::Result<Self> {
        let shell_id = child.id().unwrap_or(0);
        let shell = Pid::from_raw(i32::try_from(shell_id).unwrap_or(0))
            .ok_or_else(|| io::Error::other("the shell has no process ID"))?;
        let record = ProcessRecord {
            pid: shell_id,
            start_time: tree::start_time(shell)
                .map(|start| start.to_string())
                .unwrap_or_default(),
        };
        let status = tokio::task::spawn_blocking(move || read_status(status_read));
        Ok(Self {
            shell,
            record,
            stdin: child.stdin.take(),
            status: Some(status),
            input,
        })
    }

    /// Gives a program its input on a blocking thread, while its output is
    /// read, so a program that writes before it reads cannot block on a full
    /// pipe. A program that ends without reading all of it closes the pipe,
    /// and the write fails with a broken pipe, which Rust programs receive as
    /// an error rather than a signal.
    pub(super) fn send_input(&mut self, _child: &mut Child, input: Vec<u8>) {
        let Some(pipe) = self.input.take() else {
            return;
        };
        tokio::task::spawn_blocking(move || {
            let mut file = fs::File::from(pipe);
            if let Err(error) = file.write_all(&input) {
                tracing::debug!(%error, "a program did not read all of its input");
            }
            // Dropping the file closes the program's input.
        });
    }

    /// The shell's process, for the journal and the recovery after a crash.
    pub(super) fn record(&self) -> ProcessRecord {
        self.record.clone()
    }

    /// Waits until the wrapper reports the command's status, or until the
    /// shell exits without one.
    pub(super) async fn finished(&mut self, child: &mut Child) -> Option<i32> {
        let Some(status) = self.status.as_mut() else {
            return child.wait().await.ok().and_then(|exit| exit.code());
        };
        tokio::select! {
            reported = status => {
                self.status = None;
                reported.ok().flatten()
            }
            exited = child.wait() => exited.ok().and_then(|exit| exit.code()),
        }
    }

    /// Ends what is left of the command's tree and then the shell. After a
    /// normal end, the shell exits once its standard input closes; after Stop
    /// or a timeout, it is killed.
    pub(super) async fn end(&mut self, child: &mut Child, graceful_shell: bool) {
        // Input that was never sent closes with the run.
        self.input = None;
        tree::end_tree(self.shell).await;
        if graceful_shell {
            self.stdin = None;
        } else if let Err(error) = child.start_kill() {
            tracing::warn!(%error, "the shell of a command was not killed");
        }
        if let Err(error) = child.wait().await {
            tracing::warn!(%error, "the shell of a command was not waited for");
        }
        if let Some(status) = self.status.take() {
            status.abort();
        }
    }
}

/// Runs in the child between fork and exec: makes the shell hold its
/// descendants and puts the status pipe at descriptor 3, open across exec.
fn prepare_child(status_fd: i32) -> io::Result<()> {
    tree::hold_descendants()?;
    // SAFETY: descriptor 3 is taken over only for `dup2`, which replaces
    // whatever it was, and is released again below without being closed.
    let mut three = unsafe { OwnedFd::from_raw_fd(3) };
    // SAFETY: `status_fd` is the write end of the status pipe, which stays
    // open in the child until exec.
    let status = unsafe { BorrowedFd::borrow_raw(status_fd) };
    let duplicated = rustix::io::dup2(status, &mut three);
    let three = three.into_raw_fd();
    duplicated?;
    // `dup2` onto the same descriptor keeps its close-on-exec flag, so the
    // flag is cleared in every case.
    // SAFETY: descriptor 3 is open: `dup2` just made it.
    let three = unsafe { BorrowedFd::borrow_raw(three) };
    rustix::io::fcntl_setfd(three, FdFlags::empty())?;
    Ok(())
}

/// Runs in the child between fork and exec: makes the shell hold its
/// descendants, puts the status pipe at descriptor 3 and the program's input
/// at descriptor 4, both open across exec. Both first move above
/// [`SPARE_DESCRIPTORS`], so placing one never replaces the other, whatever
/// numbers the pipes had.
fn prepare_program_child(status_fd: i32, input_fd: i32) -> io::Result<()> {
    tree::hold_descendants()?;
    // SAFETY: both descriptors are open in the child until exec: they are
    // the parent's pipe ends, which the fork copied.
    let status = unsafe { BorrowedFd::borrow_raw(status_fd) };
    // SAFETY: as above.
    let input = unsafe { BorrowedFd::borrow_raw(input_fd) };
    let status = rustix::io::fcntl_dupfd_cloexec(status, SPARE_DESCRIPTORS)?;
    let input = rustix::io::fcntl_dupfd_cloexec(input, SPARE_DESCRIPTORS)?;
    place(&status, 3)?;
    place(&input, 4)?;
    // The spare copies close here, and would close at exec anyway.
    Ok(())
}

/// Duplicates `descriptor` onto the number `target`, open across exec.
fn place(descriptor: &OwnedFd, target: i32) -> io::Result<()> {
    // SAFETY: `target` is taken over only for `dup2`, which replaces whatever
    // it was, and is released again below without being closed.
    let mut slot = unsafe { OwnedFd::from_raw_fd(target) };
    let duplicated = rustix::io::dup2(descriptor, &mut slot);
    let placed = slot.into_raw_fd();
    duplicated?;
    // SAFETY: `placed` is open: `dup2` just made it. A copy made by `dup2`
    // onto another number has no close-on-exec flag, and clearing it states
    // that the shell keeps it.
    let placed = unsafe { BorrowedFd::borrow_raw(placed) };
    rustix::io::fcntl_setfd(placed, FdFlags::empty())?;
    Ok(())
}

/// Reads the status that the wrapper reports, or `None` when the shell ended
/// without one.
fn read_status(pipe: OwnedFd) -> Option<i32> {
    let mut text = String::new();
    fs::File::from(pipe).read_to_string(&mut text).ok()?;
    text.trim().parse().ok()
}

/// Ends what is left of a command whose run ended with Mannie: the shell, when
/// it still runs with the start time the journal recorded, and its tree.
pub(super) async fn end_leftovers(record: &ProcessRecord) {
    let Some(shell) = i32::try_from(record.pid).ok().and_then(Pid::from_raw) else {
        return;
    };
    let Ok(recorded) = record.start_time.parse::<u64>() else {
        return;
    };
    if tree::start_time(shell) != Some(recorded) {
        return;
    }
    tree::end_tree(shell).await;
    tree::kill_if_same(shell, recorded);
}
