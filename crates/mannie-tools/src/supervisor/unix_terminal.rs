//! The Linux half of a terminal's shell (ADR-0053, decision 1): a pseudo
//! terminal from rustix, and the shell in a new session whose controlling
//! terminal it is. The shell becomes a child subreaper between fork and exec,
//! as a command's shell does (ADR-0014, decision 3), so every process that it
//! starts stays in its tree.
//!
//! Ending hangs up the shell's process group, which an interactive shell
//! passes on to its jobs, and after a grace ends what is left of the tree
//! and the shell. The reading thread then sees the terminal close.
#![expect(
    unsafe_code,
    reason = "a terminal's shell gets its session and controlling terminal between fork and exec (ADR-0053)"
)]

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::process::Stdio;
use std::time::Duration;

use mannie_types::tools::ProcessRecord;
use rustix::fs::{Mode, OFlags};
use rustix::process::{Pid, Signal};
use rustix::pty::OpenptFlags;
use rustix::termios::Winsize;
use tokio::process::{Child, Command};

use super::linux as tree;
use super::terminal::TerminalLaunch;

/// A shell in a pseudo terminal, with the controlling side of the terminal.
#[derive(Debug)]
pub(super) struct Pty {
    shell: Pid,
    child: Child,
    controller: OwnedFd,
    record: ProcessRecord,
}

impl Pty {
    /// Starts the shell in a new pseudo terminal of the launch's size, and
    /// returns it with the terminal's output to read and its input to write.
    pub(super) fn spawn(launch: &TerminalLaunch) -> io::Result<(Self, File, File)> {
        let controller =
            rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)?;
        rustix::pty::grantpt(&controller)?;
        rustix::pty::unlockpt(&controller)?;
        rustix::termios::tcsetwinsize(&controller, size(launch.columns, launch.rows))?;
        let name = rustix::pty::ptsname(&controller, Vec::new())?;
        let terminal = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let mut command = Command::new(&launch.program);
        command
            .args(&launch.arguments)
            .current_dir(&launch.working_directory)
            .env_clear()
            .envs(launch.environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::from(terminal.try_clone()?))
            .stdout(Stdio::from(terminal.try_clone()?))
            .stderr(Stdio::from(terminal))
            .kill_on_drop(true);
        // SAFETY: the closure runs in the child between fork and exec, where
        // only async-signal-safe calls are allowed. It makes three system
        // calls through rustix (setsid, the ioctl that takes a controlling
        // terminal, and prctl), which take no locks and allocate nothing.
        unsafe {
            command.pre_exec(prepare_child);
        }
        // The parent's copies of the terminal's side close with `command`, so
        // the controlling side reads the end once the shell's tree closes it.
        let child = command.spawn()?;
        drop(command);
        let shell_id = child.id().unwrap_or(0);
        let shell = Pid::from_raw(i32::try_from(shell_id).unwrap_or(0))
            .ok_or_else(|| io::Error::other("the shell has no process ID"))?;
        let record = ProcessRecord {
            pid: shell_id,
            start_time: tree::start_time(shell)
                .map(|start| start.to_string())
                .unwrap_or_default(),
        };
        let reader = File::from(controller.try_clone()?);
        let writer = File::from(controller.try_clone()?);
        let pty = Self {
            shell,
            child,
            controller,
            record,
        };
        Ok((pty, reader, writer))
    }

    /// The shell's process.
    pub(super) fn record(&self) -> ProcessRecord {
        self.record.clone()
    }

    /// Changes the terminal's size; the kernel tells the shell's foreground
    /// programs with `SIGWINCH`.
    pub(super) fn resize(&self, columns: u16, rows: u16) -> io::Result<()> {
        rustix::termios::tcsetwinsize(&self.controller, size(columns, rows))?;
        Ok(())
    }

    /// Waits until the shell exits, and returns its exit code.
    pub(super) async fn finished(&mut self) -> Option<i32> {
        self.child
            .wait()
            .await
            .ok()
            .and_then(|status| status.code())
    }

    /// Hangs up the shell's process group, waits at most `grace` for the
    /// shell to exit, then ends what is left of its tree and the shell.
    pub(super) async fn end(mut self, grace: Duration) -> Option<i32> {
        let processes = self
            .record
            .start_time
            .parse()
            .ok()
            .map(|start| tree::ProcessTree::capture(self.shell, start));
        // The shell leads its own process group. The child holds the PID
        // until it is reaped, so hang up the group only before that happens.
        if self.child.id().is_some()
            && let Err(error) = rustix::process::kill_process_group(self.shell, Signal::HUP)
            && error != rustix::io::Errno::SRCH
        {
            tracing::debug!(%error, "the shell of a terminal was not hung up");
        }
        let exited = tokio::time::timeout(grace, self.finished())
            .await
            .ok()
            .flatten();
        if let Some(processes) = processes {
            processes.end().await;
        }
        if exited.is_none() {
            if let Err(error) = self.child.start_kill() {
                tracing::debug!(%error, "the shell of a terminal was not killed");
            }
            self.finished().await;
        }
        exited
    }
}

/// Runs in the child between fork and exec: a new session, the terminal on
/// standard input as its controlling terminal, and the shell holding its
/// descendants.
fn prepare_child() -> io::Result<()> {
    rustix::process::setsid()?;
    // SAFETY: descriptor 0 is the terminal's side, which the standard library
    // placed there before this closure runs, and which stays open until exec.
    let input = unsafe { BorrowedFd::borrow_raw(0) };
    rustix::process::ioctl_tiocsctty(input.as_fd())?;
    tree::hold_descendants()?;
    Ok(())
}

/// A size in the terminal's terms.
fn size(columns: u16, rows: u16) -> Winsize {
    Winsize {
        ws_row: rows.max(1),
        ws_col: columns.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}
