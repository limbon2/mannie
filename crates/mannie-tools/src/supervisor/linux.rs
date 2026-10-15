//! The Linux half of the process supervisor (ADR-0014, docs/features/
//! system-tools/commands.md, Linux).
//!
//! The shell gets a process group of its own and becomes a child subreaper
//! between fork and exec, a flag that exec keeps. Every descendant that
//! detaches, with `setsid` or a double fork, is reparented to the shell, the
//! nearest living subreaper, so the shell's tree holds every process that the
//! command started. A fixed wrapper runs the command, reports its status on
//! descriptor 3, and then waits on its standard input, so the shell outlives
//! the tree until the supervisor has ended it.
#![expect(
    unsafe_code,
    reason = "the process supervisor makes the shell a child subreaper and moves its status descriptor between fork and exec (ADR-0014)"
)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};
use std::process::Stdio;
use std::time::Duration;

use mannie_types::tools::ProcessRecord;
use rustix::io::{Errno, FdFlags};
use rustix::pipe::PipeFlags;
use rustix::process::{Pid, PidfdFlags, Signal};
use tokio::process::{Child, ChildStdin, Command};
use tokio::task::JoinHandle;

use super::CommandSpec;

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
const TERM_GRACE: Duration = Duration::from_millis(500);

/// How often the supervisor looks again whether the processes have ended.
const POLL: Duration = Duration::from_millis(20);

/// The most rounds of `SIGSTOP` and `SIGKILL`; a process in an
/// uninterruptible sleep cannot die until it wakes.
const MAX_KILL_ROUNDS: u32 = 250;

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
        // calls through rustix, prctl, dup2 and fcntl, which take no locks and
        // allocate nothing.
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
            start_time: start_time(shell)
                .map(|ticks| ticks.to_string())
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
        end_tree(self.shell).await;
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

/// Runs in the child between fork and exec: makes the shell a child
/// subreaper and puts the status pipe at descriptor 3, open across exec.
fn prepare_child(status_fd: i32) -> io::Result<()> {
    rustix::process::set_child_subreaper(Some(Pid::INIT))?;
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

/// Ends every descendant of the shell: `SIGTERM`, up to 500 ms to end, then
/// `SIGSTOP` and `SIGKILL` until none is left. Stopping first keeps a process
/// from forking while its siblings die.
async fn end_tree(shell: Pid) {
    let tree = descendants(shell);
    if tree.is_empty() {
        return;
    }
    signal_all(&tree, Signal::TERM);
    let deadline = tokio::time::Instant::now() + TERM_GRACE;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(POLL).await;
        if descendants(shell).is_empty() {
            return;
        }
    }
    for _ in 0..MAX_KILL_ROUNDS {
        let tree = descendants(shell);
        if tree.is_empty() {
            return;
        }
        signal_all(&tree, Signal::STOP);
        signal_all(&tree, Signal::KILL);
        tokio::time::sleep(POLL).await;
    }
    tracing::warn!("processes of a command did not end");
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
    if start_time(shell) != Some(recorded) {
        return;
    }
    end_tree(shell).await;
    signal(shell, recorded, Signal::KILL);
}

/// A process of the tree: its ID and start time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Member {
    pid: Pid,
    start_time: u64,
}

/// The living descendants of a process, from `/proc`. Zombies are skipped:
/// they have ended, and signals cannot reach them.
fn descendants(root: Pid) -> Vec<Member> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut children: HashMap<i32, Vec<Member>> = HashMap::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
            .and_then(Pid::from_raw)
        else {
            continue;
        };
        let Some(stat) = read_stat(pid) else {
            continue;
        };
        if stat.state == 'Z' {
            continue;
        }
        children.entry(stat.parent).or_default().push(Member {
            pid,
            start_time: stat.start_time,
        });
    }
    let mut tree = Vec::new();
    let mut seen = HashSet::new();
    let mut pending = vec![root.as_raw_pid()];
    while let Some(parent) = pending.pop() {
        for member in children.get(&parent).into_iter().flatten() {
            if seen.insert(member.pid) {
                tree.push(*member);
                pending.push(member.pid.as_raw_pid());
            }
        }
    }
    tree
}

/// The fields of `/proc/<pid>/stat` that the supervisor reads.
struct Stat {
    state: char,
    parent: i32,
    start_time: u64,
}

/// Reads a process's state, parent and start time. The command name, in
/// parentheses, may itself contain spaces and parentheses, so the fields are
/// counted after its last closing parenthesis.
fn read_stat(pid: Pid) -> Option<Stat> {
    let text = fs::read_to_string(format!("/proc/{}/stat", pid.as_raw_pid())).ok()?;
    let after_name = &text[text.rfind(')')? + 1..];
    let fields: Vec<&str> = after_name.split_whitespace().collect();
    Some(Stat {
        state: fields.first()?.chars().next()?,
        parent: fields.get(1)?.parse().ok()?,
        // Field 22 of the file; the first field after the name is field 3.
        start_time: fields.get(19)?.parse().ok()?,
    })
}

/// A process's start time in clock ticks since boot.
fn start_time(pid: Pid) -> Option<u64> {
    read_stat(pid).map(|stat| stat.start_time)
}

fn signal_all(tree: &[Member], signal_number: Signal) {
    for member in tree {
        signal(member.pid, member.start_time, signal_number);
    }
}

/// Signals a process only when it is still the one the walk found. A process
/// file descriptor pins the process, and its start time, read after the
/// descriptor was opened, proves that the ID was not reused.
fn signal(pid: Pid, expected_start: u64, signal_number: Signal) {
    match rustix::process::pidfd_open(pid, PidfdFlags::empty()) {
        Ok(pidfd) => {
            if start_time(pid) == Some(expected_start)
                && let Err(error) = rustix::process::pidfd_send_signal(&pidfd, signal_number)
                && error != Errno::SRCH
            {
                tracing::warn!(%error, "a process of a command was not signaled");
            }
        }
        // Kernels before 5.3 lack process file descriptors: check the start
        // time right before the signal instead.
        Err(Errno::NOSYS) => {
            if start_time(pid) == Some(expected_start)
                && let Err(error) = rustix::process::kill_process(pid, signal_number)
                && error != Errno::SRCH
            {
                tracing::warn!(%error, "a process of a command was not signaled");
            }
        }
        Err(_ended) => {}
    }
}
