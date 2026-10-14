//! Batched channels (docs/architecture/desktop-app.md, streaming to the UI):
//! messages go over a Tauri channel in batches, at most one batch per frame
//! interval, with consecutive sequence numbers. The webview's frame rate sets
//! the pace, not the rate at which messages arrive.
//!
//! An idle channel does not wake up: the first message after a pause goes out
//! at once, and the messages after it wait for the rest of the interval.

use std::time::Duration;

use mannie_types::ipc::ChannelBatch;
use serde::Serialize;
use tauri::async_runtime::{self, JoinHandle};
use tauri::ipc::Channel;
use tokio::sync::mpsc;
use tokio::time::Instant;

/// The shortest time between two batches: one frame at 60 Hz.
pub(crate) const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// How many messages may wait for the next batch before senders wait too.
const QUEUE_LENGTH: usize = 4_096;

/// The webview no longer receives the channel, or the sequence numbers ran
/// out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the channel to the webview closed")]
pub(crate) struct ChannelClosed;

/// What a batched channel sent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ChannelTotals {
    pub(crate) messages: u32,
    pub(crate) batches: u32,
}

/// Queues messages for a batched channel.
#[derive(Debug)]
pub(crate) struct BatchSender<T> {
    queue: mpsc::Sender<T>,
}

impl<T> Clone for BatchSender<T> {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
        }
    }
}

impl<T> BatchSender<T> {
    /// Queues a message for the next batch.
    pub(crate) async fn send(&self, message: T) -> Result<(), ChannelClosed> {
        self.queue
            .send(message)
            .await
            .map_err(|_unsent_message| ChannelClosed)
    }

    /// Queues a message without waiting, for code outside async tasks. Fails
    /// when the channel ended or thousands of messages already wait.
    pub(crate) fn try_send(&self, message: T) -> Result<(), ChannelClosed> {
        self.queue
            .try_send(message)
            .map_err(|_unsent_message| ChannelClosed)
    }

    /// Whether the channel's task has ended.
    pub(crate) fn is_closed(&self) -> bool {
        self.queue.is_closed()
    }
}

/// Starts sending batches over `channel`, at most one every `interval`. The
/// task ends once every sender is dropped and the last batch is out, and
/// returns what it sent.
pub(crate) fn batched<T>(
    channel: Channel<ChannelBatch<T>>,
    interval: Duration,
) -> (
    BatchSender<T>,
    JoinHandle<Result<ChannelTotals, ChannelClosed>>,
)
where
    T: Serialize + Clone + Send + 'static,
{
    let (queue, messages) = mpsc::channel(QUEUE_LENGTH);
    // Tauri's runtime, which any thread can reach: synchronous commands run on
    // the main thread, where no Tokio runtime is current and `tokio::spawn`
    // panics.
    let task = async_runtime::spawn(forward(messages, interval, move |batch| {
        channel.send(batch).is_ok()
    }));
    (BatchSender { queue }, task)
}

/// Collects messages and hands them to `deliver` in batches, at most one per
/// interval. `deliver` returns `false` when the receiver is gone.
async fn forward<T, D>(
    mut messages: mpsc::Receiver<T>,
    interval: Duration,
    mut deliver: D,
) -> Result<ChannelTotals, ChannelClosed>
where
    D: FnMut(ChannelBatch<T>) -> bool,
{
    let mut totals = ChannelTotals::default();
    let mut next_batch_at = Instant::now();
    let mut senders_open = true;
    while senders_open {
        // An idle channel waits here without waking up.
        let Some(first) = messages.recv().await else {
            break;
        };
        let mut pending = vec![first];
        let due = next_batch_at.max(Instant::now());
        loop {
            tokio::select! {
                // The deadline comes first, so a steady stream of messages
                // cannot hold a batch back.
                biased;
                () = tokio::time::sleep_until(due) => break,
                message = messages.recv() => {
                    let Some(message) = message else {
                        senders_open = false;
                        break;
                    };
                    pending.push(message);
                }
            }
        }
        let count = u32::try_from(pending.len()).map_err(|_too_many| ChannelClosed)?;
        if !deliver(ChannelBatch {
            first_sequence: totals.messages,
            items: pending,
        }) {
            return Err(ChannelClosed);
        }
        totals.messages = totals.messages.checked_add(count).ok_or(ChannelClosed)?;
        totals.batches += 1;
        next_batch_at = Instant::now() + interval;
    }
    Ok(totals)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use mannie_types::ipc::ChannelBatch;
    use tokio::sync::mpsc;
    use tokio::time::Instant;

    use super::{ChannelClosed, forward};

    type Delivered = Arc<Mutex<Vec<(Instant, ChannelBatch<u32>)>>>;

    fn start(
        messages: mpsc::Receiver<u32>,
    ) -> (
        Delivered,
        tokio::task::JoinHandle<Result<super::ChannelTotals, ChannelClosed>>,
    ) {
        let delivered = Delivered::default();
        let sink = Arc::clone(&delivered);
        let task = tokio::spawn(forward(messages, Duration::from_millis(16), move |batch| {
            sink.lock().unwrap().push((Instant::now(), batch));
            true
        }));
        (delivered, task)
    }

    #[test]
    fn a_channel_starts_on_a_thread_without_an_async_runtime() {
        // As a synchronous command does, on the main thread.
        let channel = tauri::ipc::Channel::<ChannelBatch<u32>>::new(|_body| Ok(()));

        let (sender, _sending) = super::batched(channel, Duration::from_millis(16));

        assert_eq!(sender.try_send(1), Ok(()));
    }

    #[tokio::test(start_paused = true)]
    async fn batches_carry_consecutive_sequence_numbers_and_every_message() {
        let (sender, messages) = mpsc::channel(64);
        let (delivered, task) = start(messages);

        for value in 0..10 {
            sender.send(value).await.unwrap();
            if value % 3 == 2 {
                tokio::time::sleep(Duration::from_millis(16)).await;
            }
        }
        drop(sender);
        let totals = task.await.unwrap().unwrap();

        let batches = delivered.lock().unwrap();
        let mut expected_sequence = 0;
        for (_at, batch) in batches.iter() {
            assert_eq!(batch.first_sequence, expected_sequence);
            expected_sequence += u32::try_from(batch.items.len()).unwrap();
        }
        let values: Vec<u32> = batches
            .iter()
            .flat_map(|(_at, batch)| batch.items.clone())
            .collect();
        assert_eq!(values, (0..10).collect::<Vec<_>>());
        assert!(batches.len() > 1, "{} batches", batches.len());
        assert_eq!(totals.messages, 10);
        assert_eq!(totals.batches as usize, batches.len());
    }

    #[tokio::test(start_paused = true)]
    async fn batches_keep_the_interval_and_the_first_after_a_pause_goes_at_once() {
        let (sender, messages) = mpsc::channel(64);
        let (delivered, task) = start(messages);
        let started = Instant::now();

        for value in 0..40 {
            sender.send(value).await.unwrap();
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
        let quiet = delivered.lock().unwrap().len();
        let resumed = Instant::now();
        sender.send(40).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1)).await;
        drop(sender);
        task.await.unwrap().unwrap();

        let batches = delivered.lock().unwrap();
        for pair in batches[..quiet].windows(2) {
            assert!(
                pair[1].0 - pair[0].0 >= Duration::from_millis(16),
                "{:?}",
                pair[1].0 - pair[0].0
            );
        }
        assert!(
            batches[0].0 - started < Duration::from_millis(1),
            "the first message waits for nothing"
        );
        assert_eq!(batches.last().unwrap().1.items, [40]);
        assert!(batches.last().unwrap().0 - resumed < Duration::from_millis(1));
    }

    #[tokio::test(start_paused = true)]
    async fn a_closed_receiver_stops_the_channel() {
        let (sender, messages) = mpsc::channel(4);
        let task = tokio::spawn(forward(
            messages,
            Duration::from_millis(16),
            |_batch: ChannelBatch<u32>| false,
        ));

        sender.send(1).await.unwrap();
        drop(sender);

        assert_eq!(task.await.unwrap(), Err(ChannelClosed));
    }
}
