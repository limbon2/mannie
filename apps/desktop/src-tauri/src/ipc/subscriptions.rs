//! The UI's subscriptions (docs/architecture/desktop-app.md, streaming to the
//! UI): each one forwards a runtime subscription to a batched channel until
//! the UI unsubscribes, the subscription ends, or the page loads again.
//!
//! A Tauri channel accepts messages even after its page is gone, so a reload
//! must end the forwarders itself: the page's new script subscribes again from
//! its cursor, and a forwarder of the old page would otherwise run forever.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mannie_types::ipc::ChannelBatch;
use mannie_types::transcript::SubscriptionId;
use serde::Serialize;
use tauri::ipc::Channel;
use tokio::task::AbortHandle;

use super::channel::{self, BatchSender};

type Active = Arc<Mutex<HashMap<SubscriptionId, AbortHandle>>>;

fn lock(active: &Active) -> MutexGuard<'_, HashMap<SubscriptionId, AbortHandle>> {
    // The map only gains and loses entries, so it stays consistent even if a
    // holder panicked.
    active.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The forwarders of the current page.
#[derive(Debug, Default)]
pub(crate) struct Subscriptions {
    next: AtomicU32,
    /// Increases with every page load, so that a subscription that a page
    /// asked for just before it went away does not outlive it.
    page: AtomicU64,
    active: Active,
}

/// The page that a command came from, as `Subscriptions::page` counted it
/// when the command started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Page(u64);

impl Subscriptions {
    /// The current page.
    pub(crate) fn page(&self) -> Page {
        Page(self.page.load(Ordering::SeqCst))
    }

    /// Starts forwarding to `channel`: `forward` receives the channel's sender
    /// and runs until it returns. When the page that asked has gone away in
    /// the meantime, nothing starts, and the ID goes to a page that no longer
    /// listens.
    pub(crate) fn start<T, F, Fut>(
        &self,
        page: Page,
        channel: Channel<ChannelBatch<T>>,
        forward: F,
    ) -> SubscriptionId
    where
        T: Serialize + Clone + Send + 'static,
        F: FnOnce(BatchSender<T>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let id = SubscriptionId::new(self.next.fetch_add(1, Ordering::Relaxed));
        let mut active = lock(&self.active);
        // Checked under the lock that `cancel_all` takes, so a forwarder is
        // either cancelled by the reload or never started.
        if page != self.page() {
            tracing::debug!("a page asked for a subscription and went away before it started");
            return id;
        }
        let (sender, _sending) = channel::batched(channel, channel::FRAME_INTERVAL);
        let forwarding = forward(sender);
        let registry = Arc::clone(&self.active);
        let task = tokio::spawn(async move {
            forwarding.await;
            lock(&registry).remove(&id);
        });
        active.insert(id, task.abort_handle());
        id
    }

    /// Ends a subscription. Ending one that already ended changes nothing.
    pub(crate) fn cancel(&self, id: SubscriptionId) {
        let task = lock(&self.active).remove(&id);
        if let Some(task) = task {
            task.abort();
        }
    }

    /// Ends every subscription, because the page loads again or Mannie quits.
    pub(crate) fn cancel_all(&self) {
        let mut active = lock(&self.active);
        self.page.fetch_add(1, Ordering::SeqCst);
        for (_id, task) in active.drain() {
            task.abort();
        }
    }

    /// How many subscriptions are active.
    #[cfg(test)]
    fn count(&self) -> usize {
        lock(&self.active).len()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mannie_types::ipc::ChannelBatch;
    use tauri::ipc::{Channel, InvokeResponseBody};

    use super::Subscriptions;

    fn channel() -> Channel<ChannelBatch<u32>> {
        Channel::new(|_body: InvokeResponseBody| Ok(()))
    }

    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    #[tokio::test]
    async fn a_reload_ends_every_forwarder_and_refuses_the_old_pages_requests() {
        let subscriptions = Subscriptions::default();
        let page = subscriptions.page();

        let first = subscriptions.start(page, channel(), |_sender| std::future::pending::<()>());
        let second = subscriptions.start(page, channel(), |_sender| std::future::pending::<()>());
        assert_ne!(first, second);
        assert_eq!(subscriptions.count(), 2);

        subscriptions.cancel_all();
        subscriptions.start(page, channel(), |_sender| std::future::pending::<()>());
        assert_eq!(
            subscriptions.count(),
            0,
            "the old page is gone, so nothing started"
        );
        subscriptions.start(subscriptions.page(), channel(), |_sender| async {});
        settle().await;

        assert_eq!(
            subscriptions.count(),
            0,
            "the finished forwarder removed itself"
        );
    }

    #[tokio::test]
    async fn unsubscribing_ends_one_forwarder() {
        let subscriptions = Subscriptions::default();
        let page = subscriptions.page();
        let kept = subscriptions.start(page, channel(), |_sender| std::future::pending::<()>());
        let ended = subscriptions.start(page, channel(), |_sender| std::future::pending::<()>());

        subscriptions.cancel(ended);
        subscriptions.cancel(ended);

        assert_eq!(subscriptions.count(), 1);
        subscriptions.cancel(kept);
        assert_eq!(subscriptions.count(), 0);
    }
}
