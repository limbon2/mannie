//! Validation commands (V1, V2, V3). Only validation builds compile them, and
//! only the validation capability grants them; a release has neither.

use std::time::Duration;

use mannie_types::clock::{Clock, SystemClock};
use mannie_types::ipc::{ChannelBatch, CommandError};
use mannie_types::validation::{ChannelLoad, ChannelLoadSummary, ValidationDelta, ValidationError};
use tauri::ipc::Channel;
use tokio::time::{Instant, MissedTickBehavior};

use crate::ipc::channel::{self, BatchSender};

/// Streams a synthetic load to the webview: `load.runs` simulated runs that
/// each send `load.deltas_per_second` deltas for `load.duration_millis`, over
/// one batched channel (V2, V3).
#[tauri::command]
#[specta::specta]
pub(crate) async fn stream_validation_events(
    load: ChannelLoad,
    events: Channel<ChannelBatch<ValidationDelta>>,
) -> Result<ChannelLoadSummary, CommandError<ValidationError>> {
    load.check()?;
    let (sender, sending) = channel::batched(events, channel::FRAME_INTERVAL);
    let duration = Duration::from_millis(u64::from(load.duration_millis));
    let runs: Vec<_> = (0..load.runs)
        .map(|run| {
            tokio::spawn(stream_run(
                run,
                load.deltas_per_second,
                duration,
                sender.clone(),
            ))
        })
        .collect();
    drop(sender);
    for run in runs {
        run.await
            .map_err(|_failed| ValidationError::ChannelClosed)??;
    }
    let totals = sending
        .await
        .map_err(|_failed| ValidationError::ChannelClosed)?
        .map_err(|_closed| {
            tracing::warn!("the webview stopped receiving a validation stream");
            ValidationError::ChannelClosed
        })?;
    Ok(ChannelLoadSummary {
        deltas: totals.messages,
        batches: totals.batches,
    })
}

/// One simulated run: deltas at an even rate, each stamped with its creation
/// time so the webview can measure how long it took to arrive.
async fn stream_run(
    run: u8,
    deltas_per_second: u16,
    duration: Duration,
    sender: BatchSender<ValidationDelta>,
) -> Result<(), ValidationError> {
    let mut ticks = tokio::time::interval(Duration::from_secs(1) / u32::from(deltas_per_second));
    ticks.set_missed_tick_behavior(MissedTickBehavior::Burst);
    let started = Instant::now();
    let mut index = 0_u32;
    while started.elapsed() < duration {
        ticks.tick().await;
        let delta = ValidationDelta {
            run,
            index,
            created_at: epoch_millis(),
            text: format!(" token{index}"),
        };
        sender
            .send(delta)
            .await
            .map_err(|_closed| ValidationError::ChannelClosed)?;
        index += 1;
    }
    Ok(())
}

#[expect(
    clippy::cast_precision_loss,
    reason = "milliseconds since 1970 stay far below 2^53, which f64 represents exactly"
)]
fn epoch_millis() -> f64 {
    SystemClock.now().as_millis() as f64
}

/// Has a permission but no grant in any capability, so the webview can never
/// call it. The validations call it to show that Tauri refuses a command
/// without a grant (V1).
#[tauri::command]
#[specta::specta]
pub(crate) fn validation_ungranted_command() -> bool {
    tracing::error!("a command without a grant ran; the capability configuration is broken");
    true
}
