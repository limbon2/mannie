//! The slots of child agents (docs/features/child-agents/tasks.md, slots): a
//! pool on the machine that limits how many children run at once.
//!
//! The pool is a semaphore, which hands out permits in the order they were
//! asked for, and drops a waiter that stops waiting without losing a permit.
//! A larger limit adds permits at once. A smaller one takes the permits that
//! are free and owes the rest: a running child's permit pays the debt when the
//! child ends, instead of going back.

use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// The pool of slots of child agents. Cloning is cheap; clones share the
/// pool.
#[derive(Debug, Clone)]
pub(crate) struct ChildSlots {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    semaphore: Arc<Semaphore>,
    limit: Mutex<Limit>,
}

#[derive(Debug)]
struct Limit {
    /// How many children may run at once.
    current: u32,
    /// Permits that the pool owes after its limit went down: running
    /// children give them up when they end.
    debt: u32,
}

/// A child's slot. Dropping it frees the slot, or pays the pool's debt.
#[derive(Debug)]
pub(crate) struct ChildSlot {
    inner: Arc<Inner>,
    permit: Option<OwnedSemaphorePermit>,
}

fn usize_of(count: u32) -> usize {
    usize::try_from(count).unwrap_or(usize::MAX)
}

impl ChildSlots {
    /// A pool of `limit` slots.
    pub(crate) fn new(limit: u32) -> Self {
        Self {
            inner: Arc::new(Inner {
                semaphore: Arc::new(Semaphore::new(usize_of(limit))),
                limit: Mutex::new(Limit {
                    current: limit,
                    debt: 0,
                }),
            }),
        }
    }

    /// Waits for a free slot. Waiters get slots in the order they asked.
    pub(crate) async fn acquire(&self) -> ChildSlot {
        let semaphore = Arc::clone(&self.inner.semaphore);
        // The pool never closes its semaphore.
        let permit = semaphore.acquire_owned().await.ok();
        ChildSlot {
            inner: Arc::clone(&self.inner),
            permit,
        }
    }

    /// Takes a free slot without waiting, if there is one.
    pub(crate) fn try_acquire(&self) -> Option<ChildSlot> {
        let semaphore = Arc::clone(&self.inner.semaphore);
        semaphore.try_acquire_owned().ok().map(|permit| ChildSlot {
            inner: Arc::clone(&self.inner),
            permit: Some(permit),
        })
    }

    /// Changes how many children may run at once. A larger limit lets
    /// waiting children start at once; a smaller one takes effect as running
    /// children end.
    pub(crate) fn set_limit(&self, limit: u32) {
        let mut current = self
            .inner
            .limit
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if limit > current.current {
            let added = limit - current.current;
            let paid = added.min(current.debt);
            current.debt -= paid;
            self.inner.semaphore.add_permits(usize_of(added - paid));
        } else if limit < current.current {
            let removed = current.current - limit;
            let forgotten = self.inner.semaphore.forget_permits(usize_of(removed));
            let forgotten = u32::try_from(forgotten).unwrap_or(removed);
            current.debt += removed.saturating_sub(forgotten);
        }
        current.current = limit;
    }

    /// How many slots are free now, for tests.
    #[cfg(test)]
    fn available(&self) -> usize {
        self.inner.semaphore.available_permits()
    }
}

impl Drop for ChildSlot {
    fn drop(&mut self) {
        let Some(permit) = self.permit.take() else {
            return;
        };
        let mut limit = self
            .inner
            .limit
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if limit.debt > 0 {
            limit.debt -= 1;
            permit.forget();
        } else {
            drop(permit);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::ChildSlots;

    #[tokio::test]
    async fn a_pool_hands_out_its_slots_in_the_order_they_were_asked_for() {
        let slots = ChildSlots::new(1);
        let first = slots.acquire().await;
        let second = tokio::spawn({
            let slots = slots.clone();
            async move { slots.acquire().await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let third = tokio::spawn({
            let slots = slots.clone();
            async move { slots.acquire().await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;

        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(5), second)
            .await
            .unwrap()
            .unwrap();

        assert!(!third.is_finished(), "the third waits behind the second");
        drop(second);
        let third = tokio::time::timeout(Duration::from_secs(5), third)
            .await
            .unwrap()
            .unwrap();
        drop(third);
        assert_eq!(slots.available(), 1);
    }

    #[tokio::test]
    async fn a_larger_limit_starts_waiting_children_at_once() {
        let slots = ChildSlots::new(1);
        let _running = slots.acquire().await;
        let waiting = tokio::spawn({
            let slots = slots.clone();
            async move { slots.acquire().await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiting.is_finished());

        slots.set_limit(2);

        let started = tokio::time::timeout(Duration::from_secs(5), waiting).await;
        assert!(started.is_ok(), "the waiting child starts");
    }

    #[tokio::test]
    async fn a_smaller_limit_takes_effect_as_running_children_end() {
        let slots = ChildSlots::new(3);
        let first = slots.acquire().await;
        let second = slots.acquire().await;

        slots.set_limit(1);

        assert_eq!(slots.available(), 0, "the free slot is gone");
        assert!(slots.try_acquire().is_none());
        drop(first);
        assert_eq!(slots.available(), 0, "the first slot paid the debt");
        drop(second);
        assert_eq!(slots.available(), 1);
        slots.set_limit(2);
        assert_eq!(slots.available(), 2);
    }

    #[tokio::test]
    async fn a_waiter_that_stops_waiting_loses_no_slot() {
        let slots = ChildSlots::new(1);
        let running = slots.acquire().await;
        let waiting = tokio::spawn({
            let slots = slots.clone();
            async move { slots.acquire().await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;

        waiting.abort();
        assert!(waiting.await.unwrap_err().is_cancelled());
        drop(running);

        assert_eq!(slots.available(), 1);
        assert!(slots.try_acquire().is_some());
    }
}
