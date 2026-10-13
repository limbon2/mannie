//! Batched channels (docs/architecture/desktop-app.md, streaming to the UI):
//! messages go over a Tauri channel in batches, at most one batch per frame
//! interval, with consecutive sequence numbers. The webview's frame rate sets
//! the pace, not the rate at which messages arrive.
//!
//! In M0 only the channel validations use it (V2, V3); the conversation stream
//! of M1 is its first feature.

use std::time::Duration;

use mannie_types::ipc::ChannelBatch;
use serde::Serialize;
use tauri::ipc::Channel;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

/// The longest a message waits before its batch is sent: one frame at 60 Hz.
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
}

/// Starts sending batches over `channel` every `interval`. The task ends once
/// every sender is dropped and the last batch is out, and returns what it sent.
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
    let task = tokio::spawn(forward(messages, interval, move |batch| {
        channel.send(batch).is_ok()
    }));
    (BatchSender { queue }, task)
}

/// Collects messages and hands them to `deliver` in batches, one per tick at
/// most. `deliver` returns `false` when the receiver is gone.
async fn forward<T, D>(
    mut messages: mpsc::Receiver<T>,
    interval: Duration,
    mut deliver: D,
) -> Result<ChannelTotals, ChannelClosed>
where
    D: FnMut(ChannelBatch<T>) -> bool,
{
    let mut ticks = tokio::time::interval(interval);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut pending = Vec::new();
    let mut totals = ChannelTotals::default();
    let mut senders_open = true;
    while senders_open || !pending.is_empty() {
        tokio::select! {
            message = messages.recv(), if senders_open => match message {
                Some(message) => pending.push(message),
                None => senders_open = false,
            },
            _ = ticks.tick() => {
                if pending.is_empty() {
                    continue;
                }
                let items = std::mem::take(&mut pending);
                let count = u32::try_from(items.len()).map_err(|_too_many| ChannelClosed)?;
                if !deliver(ChannelBatch { first_sequence: totals.messages, items }) {
                    return Err(ChannelClosed);
                }
                totals.messages = totals.messages.checked_add(count).ok_or(ChannelClosed)?;
                totals.batches += 1;
            }
        }
    }
    Ok(totals)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use mannie_types::ipc::ChannelBatch;
    use tokio::sync::mpsc;

    use super::{ChannelClosed, forward};

    #[tokio::test(start_paused = true)]
    async fn batches_carry_consecutive_sequence_numbers_and_every_message() {
        let (sender, messages) = mpsc::channel(64);
        let delivered = Arc::new(Mutex::new(Vec::<ChannelBatch<u32>>::new()));
        let sink = Arc::clone(&delivered);
        let task = tokio::spawn(forward(messages, Duration::from_millis(16), move |batch| {
            sink.lock().unwrap().push(batch);
            true
        }));

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
        for batch in batches.iter() {
            assert_eq!(batch.first_sequence, expected_sequence);
            expected_sequence += u32::try_from(batch.items.len()).unwrap();
        }
        let values: Vec<u32> = batches
            .iter()
            .flat_map(|batch| batch.items.clone())
            .collect();
        assert_eq!(values, (0..10).collect::<Vec<_>>());
        assert!(batches.len() > 1, "{} batches", batches.len());
        assert_eq!(totals.messages, 10);
        assert_eq!(totals.batches as usize, batches.len());
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
