//! Commands of the Workers plugin (docs/features/workers/pairing-and-link.md,
//! contracts; docs/features/workers/plugin.md): the paired workers on a
//! channel, pairing a worker with the code that its host printed, naming one,
//! and removing one. They are thin calls into the plugin's crate (ADR-0030,
//! decision 10): the hub checks every request, and while Workers is off every
//! command refuses with `plugin_off` and the channel ends with that reason.

use mannie_types::ipc::{ChannelBatch, CommandError};
use mannie_types::transcript::{SubscriptionEnd, SubscriptionId};
use mannie_workers::WorkersPlugin;
use mannie_workers::commands::{PairWorker, RenameWorker, WorkerRef, WorkersError};
use mannie_workers::hub::Hub;
use mannie_workers::view::{WorkerListUpdate, WorkerView};
use tauri::State;
use tauri::ipc::Channel;

use crate::state::Core;

fn plugin(core: &Core) -> Result<&WorkersPlugin, WorkersError> {
    core.workers().ok_or(WorkersError::StoreUnavailable)
}

fn hub(core: &Core) -> Result<Hub, WorkersError> {
    plugin(core)?.hub()
}

/// Logs a refused or failed command by its code, and wraps the error for the
/// UI.
fn refused(command: &'static str) -> impl FnOnce(WorkersError) -> CommandError<WorkersError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "a workers command was refused"
        );
        CommandError::new(error)
    }
}

/// Follows the paired workers: the list first, then the list after each
/// change of a worker or its status, until Workers stops.
#[tauri::command]
#[specta::specta]
pub(crate) async fn subscribe_workers(
    core: State<'_, Core>,
    updates: Channel<ChannelBatch<WorkerListUpdate>>,
) -> Result<SubscriptionId, CommandError<WorkersError>> {
    let page = core.subscriptions().page();
    let subscription = plugin(&core)
        .and_then(WorkersPlugin::subscribe)
        .map_err(refused("subscribe_workers"))?;
    let mut workers = subscription.workers;
    let mut on = subscription.on;
    Ok(core
        .subscriptions()
        .start(page, updates, |sender| async move {
            loop {
                let current = workers.borrow_and_update().clone();
                if sender
                    .send(WorkerListUpdate::Workers { workers: current })
                    .await
                    .is_err()
                {
                    break;
                }
                let reason = tokio::select! {
                    changed = workers.changed() => match changed {
                        Ok(()) => continue,
                        // The hub stopped: Workers stops or Mannie quits.
                        Err(_closed) => SubscriptionEnd::Shutdown,
                    },
                    turned = on.wait_for(|on| !*on) => match turned {
                        Ok(_off) => SubscriptionEnd::PluginOff,
                        Err(_closed) => SubscriptionEnd::Shutdown,
                    },
                };
                let _sent = sender.send(WorkerListUpdate::Ended { reason }).await;
                break;
            }
        }))
}

/// Pairs with the worker at the input's address with the code that
/// `mannie-worker pair` printed on its host.
#[tauri::command]
#[specta::specta]
pub(crate) async fn pair_worker(
    core: State<'_, Core>,
    input: PairWorker,
) -> Result<WorkerView, CommandError<WorkersError>> {
    hub(&core)
        .map_err(refused("pair_worker"))?
        .pair(input)
        .await
        .map_err(refused("pair_worker"))
}

/// Gives a worker the name that the owner chose in Mannie, or with a blank
/// name goes back to the name in the worker's configuration.
#[tauri::command]
#[specta::specta]
pub(crate) async fn rename_worker(
    core: State<'_, Core>,
    input: RenameWorker,
) -> Result<WorkerView, CommandError<WorkersError>> {
    hub(&core)
        .map_err(refused("rename_worker"))?
        .rename(&input.worker_id, &input.name)
        .await
        .map_err(refused("rename_worker"))
}

/// Removes a worker: it is unpaired when it can be reached, and the master
/// forgets it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn remove_worker(
    core: State<'_, Core>,
    input: WorkerRef,
) -> Result<(), CommandError<WorkersError>> {
    hub(&core)
        .map_err(refused("remove_worker"))?
        .remove(&input.worker_id)
        .await
        .map_err(refused("remove_worker"))
}
