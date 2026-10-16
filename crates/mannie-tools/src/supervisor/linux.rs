//! The Linux half of the process supervisor's tree (ADR-0014, docs/features/
//! system-tools/commands.md, Linux).
//!
//! The shell gets a process group of its own and becomes a child subreaper
//! between fork and exec, a flag that exec keeps. Every descendant that
//! detaches, with `setsid` or a double fork, is reparented to the shell, the
//! nearest living subreaper, so the shell's tree holds every process that the
//! command started.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;

use rustix::io::Errno;
use rustix::process::{Pid, PidfdFlags, Signal};

use super::unix::{MAX_KILL_ROUNDS, POLL, TERM_GRACE};

/// Runs in the child between fork and exec: makes the shell a child
/// subreaper, which exec keeps.
pub(super) fn hold_descendants() -> io::Result<()> {
    rustix::process::set_child_subreaper(Some(Pid::INIT))?;
    Ok(())
}

/// Ends every descendant of the shell: `SIGTERM`, up to 500 ms to end, then
/// `SIGSTOP` and `SIGKILL` until none is left. Stopping first keeps a process
/// from forking while its siblings die.
pub(super) async fn end_tree(shell: Pid) {
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

/// Kills the shell of a command that ended with Mannie, when it is still the
/// process that the journal recorded.
pub(super) fn kill_if_same(shell: Pid, recorded: u64) {
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
pub(super) fn start_time(pid: Pid) -> Option<u64> {
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
