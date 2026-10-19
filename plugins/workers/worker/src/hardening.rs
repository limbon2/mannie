//! Keeps the worker's memory, which holds credential leases, from other
//! processes (plugins/workers/docs/service.md, hardening; ADR-0010). On
//! Linux the worker turns off core dumps and marks itself not dumpable, so
//! another process of its account cannot read its memory through `ptrace` or
//! `/proc`. On FreeBSD it turns off core dumps and tracing. Both flags reset
//! when a command's program starts, so the commands that the worker runs are
//! not affected; the core limit of zero stays with them. Windows has no
//! equivalent for a process of the same account, which the security
//! architecture records as a residual risk.

/// Applies the hardening of this system, and logs what could not be applied.
pub(crate) fn apply() {
    #[cfg(unix)]
    {
        use rustix::process::{Resource, Rlimit, setrlimit};

        let no_core = Rlimit {
            current: Some(0),
            maximum: Some(0),
        };
        if let Err(error) = setrlimit(Resource::Core, no_core) {
            tracing::warn!(%error, "could not turn off core dumps");
        }
    }
    #[cfg(target_os = "linux")]
    {
        use rustix::process::{DumpableBehavior, set_dumpable_behavior};

        if let Err(error) = set_dumpable_behavior(DumpableBehavior::NotDumpable) {
            tracing::warn!(%error, "could not mark the worker as not dumpable");
        }
    }
    #[cfg(target_os = "freebsd")]
    if let Err(error) = disable_tracing() {
        tracing::warn!(%error, "could not turn off tracing of the worker");
    }
}

/// Turns off tracing of the worker, which covers ptrace(2), ktrace(2), the
/// debugging sysctls and core dumps, until it starts another program. The
/// call goes through libc: rustix's `set_dumpable_behavior` passes the value
/// where procctl(2) expects a pointer to it, and fails with EFAULT.
#[cfg(target_os = "freebsd")]
#[expect(
    unsafe_code,
    reason = "procctl(PROC_TRACE_CTL), whose rustix binding passes its argument wrongly (ADR-0010)"
)]
fn disable_tracing() -> std::io::Result<()> {
    let mut disable = libc::PROC_TRACE_CTL_DISABLE;
    // SAFETY: PROC_TRACE_CTL reads one int, which `disable` is and which
    // outlives the call.
    let result = unsafe {
        libc::procctl(
            libc::P_PID,
            libc::id_t::from(std::process::id()),
            libc::PROC_TRACE_CTL,
            (&raw mut disable).cast(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(test, unix))]
mod tests;
