//! Validation commands (V1, V2, V3, and the end-to-end tests of M1). Only
//! validation builds compile them, and only the validation capability grants
//! them; a release has neither.
//!
//! The WebDriver tests cannot press the window's close button or reach the
//! tray, so these commands call the same code that those do
//! (docs/features/window-and-tray.md, tests).

use std::time::Duration;

use mannie_logging::ErrorChain;
use mannie_types::clock::{Clock, SystemClock};
use mannie_types::ids::ConversationId;
use mannie_types::ipc::{ChannelBatch, CommandError};
use mannie_types::validation::{
    ChannelLoad, ChannelLoadSummary, SeedConversation, ValidationDelta, ValidationError,
    ValidationTrayItem, ValidationWindowState,
};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tokio::time::{Instant, MissedTickBehavior};

use crate::ipc::channel::{self, BatchSender};
use crate::state::{Core, FolderChoice};
use crate::window::{MAIN_WINDOW, tray};

/// Sets what the next folder dialog answers instead of opening: a folder, or
/// nothing for a dialog that the owner closed. The end-to-end tests cannot
/// click in a dialog of the system.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn validation_set_folder_choice(core: State<'_, Core>, folder: Option<String>) {
    let choice = match folder {
        Some(folder) => FolderChoice::Folder(folder),
        None => FolderChoice::Closed,
    };
    core.set_folder_choice(choice);
}

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

/// Closes the main window the way its close button does: Tauri emits the
/// close request, and the core decides what happens.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn validation_close_main_window(
    app: AppHandle,
) -> Result<(), CommandError<ValidationError>> {
    let window = app
        .get_webview_window(MAIN_WINDOW)
        .ok_or(ValidationError::Failed)?;
    window.close().map_err(|error| {
        tracing::warn!(%error, "could not close the main window");
        CommandError::new(ValidationError::Failed)
    })
}

/// Whether the main window is visible and the tray exists.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn validation_window_state(
    app: AppHandle,
) -> Result<ValidationWindowState, CommandError<ValidationError>> {
    let window = app
        .get_webview_window(MAIN_WINDOW)
        .ok_or(ValidationError::Failed)?;
    let visible = window.is_visible().map_err(|error| {
        tracing::warn!(%error, "could not read whether the main window is visible");
        CommandError::new(ValidationError::Failed)
    })?;
    Ok(ValidationWindowState {
        visible,
        tray: tray::exists(&app),
    })
}

/// Runs a menu item of the tray, as a click on it does.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn validation_tray_menu(app: AppHandle, item: ValidationTrayItem) {
    let item = match item {
        ValidationTrayItem::Open => tray::OPEN_ITEM,
        ValidationTrayItem::Quit => tray::QUIT_ITEM,
    };
    tray::on_menu_item(&app, item);
}

/// The ID of Mannie's process, so that a test can kill it as a crash would.
#[tauri::command]
#[specta::specta]
pub(crate) fn validation_process_id() -> u32 {
    std::process::id()
}

/// Seeds a long conversation for the performance checks (NFR-PERF-04).
#[tauri::command]
#[specta::specta]
pub(crate) async fn validation_seed_conversation(
    core: State<'_, Core>,
    input: SeedConversation,
) -> Result<ConversationId, CommandError<ValidationError>> {
    input.check()?;
    let runtime = core.runtime().ok_or(ValidationError::CoreUnavailable)?;
    runtime
        .seed_conversation(input.turns)
        .await
        .map_err(|error| {
            tracing::error!(error = %ErrorChain(&error), "could not seed a conversation");
            CommandError::new(ValidationError::Failed)
        })
}

/// The links that `open_link` would have opened, in order.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn validation_opened_links(core: State<'_, Core>) -> Vec<String> {
    core.opened_links()
}
