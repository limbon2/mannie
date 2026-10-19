//! The executions of plugins' tools that run now, so that turning a plugin
//! off waits for them and aborts the rest (ADR-0034, decision 8).
//!
//! A call joins only while the plugin's start that offered its tool is still
//! registered, under the registry's read lock (`Registry::while_registered`).
//! Turning the plugin off swaps the registry under its write lock before it
//! drains, so it either finds the execution here or keeps it from starting.
//!
//! An abort fires the execution's signal. The run that executes the call
//! drops the plugin's future, records how the call ended and takes the
//! execution out, which wakes the drain.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use mannie_tools::{StopReason, StopSignal};
use tokio::sync::Notify;
use tokio::time::Instant;

/// How long turning a plugin off waits for its aborted executions to end.
/// The run drops an aborted execution at once, so this only covers a run that
/// is busy storing.
const ABORT_GRACE: Duration = Duration::from_secs(1);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only inserts, removes or reads.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One execution of a plugin's tool.
#[derive(Debug)]
struct Running {
    /// The plugin's ID.
    plugin: String,
    /// Fires when turning the plugin off aborts the execution.
    abort: StopSignal,
}

/// The executions of plugins' tools that run now.
#[derive(Debug, Default)]
pub(crate) struct PluginExecutions {
    running: Mutex<HashMap<u64, Running>>,
    /// Wakes a drain when an execution ends.
    ended: Notify,
    next: AtomicU64,
}

/// An execution in the registry, until the run drops it.
#[derive(Debug)]
pub(crate) struct ExecutionGuard {
    executions: Arc<PluginExecutions>,
    id: u64,
    /// Fires when turning the plugin off aborts the execution.
    pub(crate) abort: StopSignal,
}

impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        lock(&self.executions.running).remove(&self.id);
        self.executions.ended.notify_waiters();
    }
}

impl PluginExecutions {
    /// Adds an execution of the plugin's tool. The caller holds the
    /// registry's read lock, so the plugin's start is still registered.
    pub(crate) fn join(self: &Arc<Self>, plugin: &str) -> ExecutionGuard {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let abort = StopSignal::new();
        lock(&self.running).insert(
            id,
            Running {
                plugin: plugin.to_owned(),
                abort: abort.clone(),
            },
        );
        ExecutionGuard {
            executions: Arc::clone(self),
            id,
            abort,
        }
    }

    /// How many executions of the plugin run.
    fn count(&self, plugin: &str) -> usize {
        lock(&self.running)
            .values()
            .filter(|running| running.plugin == plugin)
            .count()
    }

    /// Waits until no execution of the plugin runs or `deadline` passed, and
    /// says whether none runs.
    async fn settle(&self, plugin: &str, deadline: Instant) -> bool {
        loop {
            let ended = self.ended.notified();
            tokio::pin!(ended);
            // Registered before the check, so an execution that ends in
            // between still wakes this wait.
            ended.as_mut().enable();
            if self.count(plugin) == 0 {
                return true;
            }
            if tokio::time::timeout_at(deadline, ended).await.is_err() {
                return self.count(plugin) == 0;
            }
        }
    }

    /// Fires the abort of every execution of the plugin, and returns how many
    /// there were.
    fn abort(&self, plugin: &str) -> usize {
        let running = lock(&self.running);
        let mut aborted = 0;
        for execution in running.values().filter(|running| running.plugin == plugin) {
            execution.abort.stop(StopReason::Interrupted);
            aborted += 1;
        }
        aborted
    }

    /// Waits up to `grace` for the plugin's executions to end, then aborts the
    /// rest and waits for their runs to let them go. The plugin's
    /// contributions are gone already, so no execution of it starts meanwhile.
    pub(crate) async fn drain(&self, plugin: &str, grace: Duration) {
        if self.settle(plugin, Instant::now() + grace).await {
            return;
        }
        let aborted = self.abort(plugin);
        tracing::warn!(
            plugin,
            aborted,
            "tool calls of a plugin that is turning off did not end in time and were aborted"
        );
        if !self.settle(plugin, Instant::now() + ABORT_GRACE).await {
            tracing::error!(
                plugin,
                "aborted tool calls of a plugin still run; the plugin stops anyway"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::PluginExecutions;

    #[tokio::test]
    async fn a_drain_returns_at_once_without_executions_of_the_plugin() {
        let executions = Arc::new(PluginExecutions::default());
        let other = executions.join("other");

        tokio::time::timeout(
            Duration::from_secs(1),
            executions.drain("ssh", Duration::from_secs(5)),
        )
        .await
        .expect("nothing of the plugin runs");

        assert!(!other.abort.is_stopped());
    }

    #[tokio::test]
    async fn a_drain_waits_for_an_execution_that_ends_in_time_without_aborting_it() {
        let executions = Arc::new(PluginExecutions::default());
        let running = executions.join("ssh");
        let abort = running.abort.clone();

        let drain = tokio::spawn({
            let executions = Arc::clone(&executions);
            async move { executions.drain("ssh", Duration::from_secs(5)).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(running);
        drain.await.unwrap();

        assert!(!abort.is_stopped());
    }

    #[tokio::test]
    async fn a_drain_aborts_what_did_not_end_in_time() {
        let executions = Arc::new(PluginExecutions::default());
        let running = executions.join("ssh");
        let abort = running.abort.clone();

        let drain = tokio::spawn({
            let executions = Arc::clone(&executions);
            async move { executions.drain("ssh", Duration::from_millis(20)).await }
        });
        abort.stopped().await;
        // The run lets the aborted execution go.
        drop(running);
        drain.await.unwrap();

        assert!(abort.is_stopped());
    }
}
