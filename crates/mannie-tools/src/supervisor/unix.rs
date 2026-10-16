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
use std::io::{self, Read};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};
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

/// How long the processes of a stopped command have to end after `SIGTERM`.
pub(super) const TERM_GRACE: Duration = Duration::from_millis(500);

/// How often the supervisor looks again whether the processes have ended.
pub(super) const POLL: Duration = Duration::from_millis(20);

/// The most rounds of `SIGSTOP` and `SIGKILL`; a process in an
/// uninterruptible sleep cannot die until it wakes.
pub(super) const MAX_KILL_ROUNDS: u32 = 250;

/// A running command: its shell, the shell's standard input, and the status
/// that the wrapper reports.
#[derive(Debug)]
pub(super) struct Contained {
    shell: Pid,
    record: ProcessRecord,
    stdin: Option<ChildStdin>,
    status: Option<JoinHandle<Option<i32>>>,
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
        Ok((
            Self {
                shell,
                record,
                stdin: child.stdin.take(),
                status: Some(status),
            },
            child,
        ))
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
