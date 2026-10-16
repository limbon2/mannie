//! The FreeBSD half of the process supervisor's tree (ADR-0014, docs/
//! features/system-tools/commands.md, FreeBSD).
//!
//! The shell gets a process group of its own and becomes a reaper with
//! `procctl(PROC_REAP_ACQUIRE)` between fork and exec; exec leaves the reaper
//! status alone. Every descendant that detaches, with `setsid`, `daemon(8)`
//! or a double fork, is reparented to the shell, its nearest reaper, so
//! `procctl(PROC_REAP_KILL)` on the shell reaches every process that the
//! command started, in one call that the kernel makes for the whole tree.
#![expect(
    unsafe_code,
    reason = "the process supervisor reads a process's start time from the kernel with sysctl, which rustix does not cover, and signals a tree with procctl, whose rustix wrapper panics on the kernel's answer (ADR-0014)"
)]

use std::io;
use std::mem::{MaybeUninit, size_of};

use rustix::io::Errno;
use rustix::process::{IdType, Pid, PidInfoFlags, ReaperStatusFlags, Signal};

use super::unix::{MAX_KILL_ROUNDS, POLL, TERM_GRACE};

/// Runs in the child between fork and exec: makes the shell a reaper, which
/// exec keeps.
pub(super) fn hold_descendants() -> io::Result<()> {
    rustix::process::set_reaper_status(true)?;
    Ok(())
}

/// Ends every descendant of the shell: `SIGTERM`, up to 500 ms to end, then
/// `SIGSTOP` and `SIGKILL` until none is left. Stopping first keeps a process
/// from forking while its siblings die.
pub(super) async fn end_tree(shell: Pid) {
    if !holds_living_descendants(shell) {
        return;
    }
    signal_tree(shell, Signal::TERM);
    let deadline = tokio::time::Instant::now() + TERM_GRACE;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(POLL).await;
        if !holds_living_descendants(shell) {
            return;
        }
    }
    for _ in 0..MAX_KILL_ROUNDS {
        if !holds_living_descendants(shell) {
            return;
        }
        signal_tree(shell, Signal::STOP);
        signal_tree(shell, Signal::KILL);
        tokio::time::sleep(POLL).await;
    }
    tracing::warn!("processes of a command did not end");
}

/// Kills the shell of a command that ended with Mannie, when it is still the
/// process that the journal recorded. The start time is read again right
/// before the signal, so a reused process ID is not signaled.
pub(super) fn kill_if_same(shell: Pid, recorded: u64) {
    if start_time(shell) != Some(recorded) {
        return;
    }
    if let Err(error) = rustix::process::kill_process(shell, Signal::KILL)
        && error != Errno::SRCH
    {
        tracing::warn!(%error, "the shell of a command was not killed");
    }
}

/// How many descendants the shell's tree holds, zombies included, or `None`
/// when the shell is not a reaper of its own. `PROC_REAP_KILL` and
/// `PROC_REAP_GETPIDS` on a process that is not a reaper act on that
/// process's reaper, such as `init`, so the supervisor never makes them for a
/// shell that does not own its reaper status. The shell is the supervisor's
/// child and is waited for only after its tree ended, so its process ID is
/// not reused meanwhile.
fn tree_size(shell: Pid) -> Option<usize> {
    let status = rustix::process::get_reaper_status(Some((IdType::Pid, shell))).ok()?;
    let owned = status.flags.contains(ReaperStatusFlags::OWNED) && status.reaper == shell;
    owned.then_some(status.descendants)
}

/// Whether any descendant of the shell still runs. Zombies and processes
/// that are exiting have ended; signals cannot reach them.
fn holds_living_descendants(shell: Pid) -> bool {
    // Most commands leave nothing behind; the list of descendants is read
    // only when there are some.
    if tree_size(shell).is_none_or(|size| size == 0) {
        return false;
    }
    let Ok(descendants) = rustix::process::get_reaper_pids(Some((IdType::Pid, shell))) else {
        return false;
    };
    descendants.iter().any(|info| {
        !info
            .flags
            .intersects(PidInfoFlags::ZOMBIE | PidInfoFlags::EXITING)
    })
}

/// The argument of `procctl(PROC_REAP_KILL)`: `struct procctl_reaper_kill`
/// of `<sys/procctl.h>`, which the libc crate does not declare.
#[repr(C)]
struct ReaperKill {
    /// `rk_sig`: the signal to send.
    signal: libc::c_int,
    /// `rk_flags`: `REAPER_KILL_*` flags; none signals every descendant.
    flags: libc::c_uint,
    /// `rk_subtree`: the subtree to signal, with `REAPER_KILL_SUBTREE`.
    subtree: libc::pid_t,
    /// `rk_killed`, set by the kernel: how many processes it signaled.
    killed: libc::c_uint,
    /// `rk_fpid`, set by the kernel: the first process that it could not
    /// signal, or -1 when it signaled every one.
    first_failed: libc::pid_t,
    /// `rk_pad`, which must be zero.
    pad: [libc::c_uint; 15],
}

/// Signals every descendant of the shell, in the kernel, so no process can
/// fork away between finding it and signaling it. The call goes through libc:
/// rustix's `reaper_kill` makes a `Pid` of `rk_fpid`, which is -1 after every
/// process was signaled, and that panics in debug builds.
fn signal_tree(shell: Pid, signal: Signal) {
    if tree_size(shell).is_none() {
        return;
    }
    let mut request = ReaperKill {
        signal: signal.as_raw(),
        flags: 0,
        subtree: 0,
        killed: 0,
        first_failed: 0,
        pad: [0; 15],
    };
    // SAFETY: PROC_REAP_KILL reads and writes one `procctl_reaper_kill`,
    // which `request` is, in its C layout, and which outlives the call.
    let result = unsafe {
        libc::procctl(
            libc::P_PID,
            libc::id_t::from(shell.as_raw_pid()),
            libc::PROC_REAP_KILL,
            (&raw mut request).cast(),
        )
    };
    if result != 0 {
        let error = io::Error::last_os_error();
        // ESRCH: nothing was left to signal.
        if error.raw_os_error() != Some(libc::ESRCH) {
            tracing::warn!(%error, "the processes of a command were not signaled");
        }
        return;
    }
    // A process that changed its user, as through a setuid program, cannot
    // be signaled by the worker's account.
    if request.first_failed != -1 {
        tracing::warn!(
            process = request.first_failed,
            signaled = request.killed,
            "a process of a command could not be signaled"
        );
    }
}

/// A process's start time, in microseconds since the epoch, or `None` when
/// it does not run or has become a zombie.
pub(super) fn start_time(pid: Pid) -> Option<u64> {
    let raw = pid.as_raw_pid();
    let mut name = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, raw];
    let mut info = MaybeUninit::<libc::kinfo_proc>::zeroed();
    let mut length = size_of::<libc::kinfo_proc>();
    // SAFETY: `name` names four integers, as the length says; `info` is
    // writable memory of the size that `length` gives, which the kernel
    // fills up to `length` and updates; no new value is set.
    let result = unsafe {
        libc::sysctl(
            name.as_mut_ptr(),
            4,
            info.as_mut_ptr().cast(),
            &raw mut length,
            std::ptr::null(),
            0,
        )
    };
    if result != 0 || length != size_of::<libc::kinfo_proc>() {
        return None;
    }
    // SAFETY: the kernel filled the whole structure, as `length` shows, and
    // every field of `kinfo_proc` is valid for any bit pattern.
    let info = unsafe { info.assume_init() };
    if info.ki_pid != raw || info.ki_stat == libc::SZOMB {
        return None;
    }
    let seconds = u64::try_from(info.ki_start.tv_sec).ok()?;
    let microseconds = u64::try_from(info.ki_start.tv_usec).ok()?;
    Some(seconds * 1_000_000 + microseconds)
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::CommandExt;

    use rustix::process::{IdType, Pid, ReaperStatusFlags};

    use super::{hold_descendants, start_time};

    /// The containment rests on this: procctl(2) does not say whether exec
    /// keeps the reaper status, and the kernel's exec leaves it alone.
    #[test]
    fn a_process_keeps_its_reaper_status_across_exec() {
        let mut command = std::process::Command::new("/bin/sleep");
        command.arg("30");
        // SAFETY: the closure runs between fork and exec and makes one
        // system call, procctl, which takes no locks and allocates nothing.
        unsafe {
            command.pre_exec(hold_descendants);
        }
        // `spawn` returns once exec succeeded.
        let mut child = command.spawn().unwrap();
        let pid = Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap();
        let status = rustix::process::get_reaper_status(Some((IdType::Pid, pid)));
        let started = start_time(pid);
        child.kill().unwrap();
        child.wait().unwrap();

        let status = status.unwrap();
        assert!(status.flags.contains(ReaperStatusFlags::OWNED));
        assert_eq!(status.reaper, pid);
        assert!(started.is_some());
    }

    #[test]
    fn a_process_that_does_not_run_has_no_start_time() {
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap();
        child.wait().unwrap();

        assert_eq!(start_time(pid), None);
    }
}
