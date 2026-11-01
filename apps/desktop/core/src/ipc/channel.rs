//! Batched channels (docs/architecture/desktop-app.md, streaming to the UI):
//! messages go over a Tauri channel in batches, on average at most one batch
//! per frame interval, with consecutive sequence numbers. The webview's frame
//! rate sets the pace, not the rate at which messages arrive.
//!
//! An idle channel does not wake up. After a pause, up to [`BURST`] batches
//! go out at once, without a timer, and the batches after them wait for their
//! share of the interval.

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

/// How many batches may go one after another after a pause. A keystroke's
/// echo can come in pieces a frame apart, as `ConPTY` sends it, and a piece
/// that waits for a timer waits on Windows until the system's next tick,
/// up to 15.6 ms later.
const BURST: u32 = 4;

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

/// Starts sending batches over `channel`, on average at most one every
/// `interval`. The task ends once every sender is dropped and the last batch
/// is out, and returns what it sent.
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

/// The pace of a channel's batches, by the generic cell rate algorithm: on
/// average at most one per interval, and up to [`BURST`] one after another
/// after a pause. A batch that went late, as after a timer that woke up at
/// the system's next tick, does not push the batches after it back.
#[derive(Debug, Clone, Copy)]
struct Pace {
    interval: Duration,
    /// When the next batch would go if every batch had gone at the steady
    /// rate.
    steady: Instant,
}

impl Pace {
    fn new(interval: Duration, now: Instant) -> Self {
        Self {
            interval,
            steady: now,
        }
    }

    /// When the next batch may go, or `None` when it may go now.
    fn wait_until(&self, now: Instant) -> Option<Instant> {
        let allowed = self.steady.checked_sub(self.interval * (BURST - 1))?;
        (allowed > now).then_some(allowed)
    }

    /// Notes a batch that went at `now`.
    fn sent(&mut self, now: Instant) {
        self.steady = self.steady.max(now) + self.interval;
    }
}

/// Collects messages and hands them to `deliver` in batches, at the pace of
/// [`Pace`]. `deliver` returns `false` when the receiver is gone.
async fn forward<T, D>(
    mut messages: mpsc::Receiver<T>,
    interval: Duration,
    mut deliver: D,
) -> Result<ChannelTotals, ChannelClosed>
where
    D: FnMut(ChannelBatch<T>) -> bool,
{
    let mut totals = ChannelTotals::default();
    let mut pace = Pace::new(interval, Instant::now());
    let mut senders_open = true;
    while senders_open {
        // An idle channel waits here without waking up.
        let Some(first) = messages.recv().await else {
            break;
        };
        let mut pending = vec![first];
        if let Some(due) = pace.wait_until(Instant::now()) {
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
        } else {
            // A batch that may go now goes without a timer, with every
            // message that already waits.
            while let Ok(message) = messages.try_recv() {
                pending.push(message);
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
        pace.sent(Instant::now());
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

    use super::{BURST, ChannelClosed, Pace, forward};

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

    /// Lets the forwarding task run without letting time pass.
    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
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
    async fn after_a_pause_a_burst_goes_at_once_without_a_timer() {
        let (sender, messages) = mpsc::channel(64);
        let (delivered, task) = start(messages);
        let started = Instant::now();

        for value in 0..BURST {
            sender.send(value).await.unwrap();
            settle().await;
        }
        let burst = delivered.lock().unwrap().len();
        sender.send(BURST).await.unwrap();
        settle().await;
        let held = delivered.lock().unwrap().len();
        tokio::time::sleep(Duration::from_millis(16)).await;
        drop(sender);
        task.await.unwrap().unwrap();

        let batches = delivered.lock().unwrap();
        assert_eq!(
            burst, BURST as usize,
            "each piece went in a batch of its own"
        );
        assert!(
            batches[..burst].iter().all(|(at, _batch)| *at == started),
            "no piece of the burst waited"
        );
        assert_eq!(held, burst, "the piece after the burst waits for its share");
        assert_eq!(batches.last().unwrap().1.items, [BURST]);
        assert_eq!(
            batches.last().unwrap().0 - started,
            Duration::from_millis(16)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_steady_stream_never_runs_ahead_of_one_batch_per_interval() {
        let (sender, messages) = mpsc::channel(64);
        let (delivered, task) = start(messages);
        let started = Instant::now();

        for value in 0..400 {
            sender.send(value).await.unwrap();
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        drop(sender);
        task.await.unwrap().unwrap();

        let batches = delivered.lock().unwrap();
        for (index, (at, _batch)) in batches.iter().enumerate() {
            // Past the burst, each batch waits for its share of the interval.
            let shares = u32::try_from(index).unwrap().saturating_sub(BURST - 1);
            assert!(
                *at >= started + Duration::from_millis(16) * shares,
                "batch {index} went after {:?}",
                *at - started
            );
        }
        let values: Vec<u32> = batches
            .iter()
            .flat_map(|(_at, batch)| batch.items.clone())
            .collect();
        assert_eq!(values, (0..400).collect::<Vec<_>>());
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_message_after_a_pause_goes_at_once() {
        let (sender, messages) = mpsc::channel(64);
        let (delivered, task) = start(messages);

        for value in 0..40 {
            sender.send(value).await.unwrap();
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
        let resumed = Instant::now();
        sender.send(40).await.unwrap();
        settle().await;
        drop(sender);
        task.await.unwrap().unwrap();

        let batches = delivered.lock().unwrap();
        assert_eq!(batches.last().unwrap().1.items, [40]);
        assert_eq!(batches.last().unwrap().0, resumed);
    }

    #[test]
    fn a_late_batch_does_not_push_the_batches_after_it_back() {
        let interval = Duration::from_millis(16);
        let start = Instant::now();
        let mut pace = Pace::new(interval, start);
        for _ in 0..BURST {
            assert_eq!(pace.wait_until(start), None);
            pace.sent(start);
        }
        let due = start + interval;
        assert_eq!(pace.wait_until(start), Some(due));

        // A timer that woke up at the system's next tick, 15 ms late.
        let late = due + Duration::from_millis(15);
        pace.sent(late);

        assert_eq!(pace.wait_until(late), Some(due + interval));
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
