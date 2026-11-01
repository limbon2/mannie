//! Commands of the plugins feature (docs/features/plugins.md): the plugins on
//! a channel, turning one or one of its optional features on or off, and the
//! work that turning one off lets go (ADR-0055, decision 9). The host checks
//! every request; these commands find the host and log refusals by their
//! code.

use mannie_plugins::PluginHost;
use mannie_types::ipc::{ChannelBatch, CommandError};
use mannie_types::plugins::{
    PluginFeatureRef, PluginListUpdate, PluginRef, PluginView, PluginWork, PluginsError,
    SetPluginEnabled, SetPluginFeatureEnabled,
};
use mannie_types::transcript::{SubscriptionEnd, SubscriptionId};
use tauri::State;
use tauri::ipc::Channel;

use crate::state::Core;

fn host(core: &Core) -> Result<&PluginHost, PluginsError> {
    core.plugins().ok_or(PluginsError::StoreUnavailable)
}

/// Logs a refused or failed command by its code, and wraps the error for the
/// UI.
fn refused(command: &'static str) -> impl FnOnce(PluginsError) -> CommandError<PluginsError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "a plugins command was refused"
        );
        CommandError::new(error)
    }
}

/// Follows the plugins: the list first, then the list after each change of a
/// plugin's state or of the owner's choice.
#[tauri::command]
#[specta::specta]
pub(crate) async fn subscribe_plugins(
    core: State<'_, Core>,
    updates: Channel<ChannelBatch<PluginListUpdate>>,
) -> Result<SubscriptionId, CommandError<PluginsError>> {
    let page = core.subscriptions().page();
    let mut plugins = host(&core)
        .map_err(refused("subscribe_plugins"))?
        .subscribe();
    Ok(core
        .subscriptions()
        .start(page, updates, |sender| async move {
            loop {
                let current = plugins.borrow_and_update().clone();
                if sender
                    .send(PluginListUpdate::Plugins { plugins: current })
                    .await
                    .is_err()
                {
                    break;
                }
                if plugins.changed().await.is_err() {
                    // The host is gone: Mannie quits.
                    let _sent = sender
                        .send(PluginListUpdate::Ended {
                            reason: SubscriptionEnd::Shutdown,
                        })
                        .await;
                    break;
                }
            }
        }))
}

/// Turns a plugin on or off, as the owner's lasting choice. The answer comes
/// once the plugin started or stopped; the channel shows the steps between.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_plugin_enabled(
    core: State<'_, Core>,
    input: SetPluginEnabled,
) -> Result<PluginView, CommandError<PluginsError>> {
    host(&core)
        .map_err(refused("set_plugin_enabled"))?
        .set_enabled(&input.plugin_id, input.enabled)
        .await
        .map_err(refused("set_plugin_enabled"))
}

/// The work that a plugin has in progress, which the switch names before it
/// turns the plugin off.
#[tauri::command]
#[specta::specta]
pub(crate) async fn plugin_work(
    core: State<'_, Core>,
    input: PluginRef,
) -> Result<PluginWork, CommandError<PluginsError>> {
    host(&core)
        .map_err(refused("plugin_work"))?
        .work(&input.plugin_id)
        .await
        .map_err(refused("plugin_work"))
}

/// Turns an optional feature of a plugin on or off, as the owner's lasting
/// choice, which outlives the plugin's own switch. The answer is the
/// plugin's view once the feature started or stopped; the channel shows the
/// steps between.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_plugin_feature_enabled(
    core: State<'_, Core>,
    input: SetPluginFeatureEnabled,
) -> Result<PluginView, CommandError<PluginsError>> {
    host(&core)
        .map_err(refused("set_plugin_feature_enabled"))?
        .set_feature_enabled(&input.plugin_id, &input.feature_id, input.enabled)
        .await
        .map_err(refused("set_plugin_feature_enabled"))
}

/// The work that an optional feature has in progress, which its switch names
/// before it turns the feature off.
#[tauri::command]
#[specta::specta]
pub(crate) async fn plugin_feature_work(
    core: State<'_, Core>,
    input: PluginFeatureRef,
) -> Result<PluginWork, CommandError<PluginsError>> {
    host(&core)
        .map_err(refused("plugin_feature_work"))?
        .feature_work(&input.plugin_id, &input.feature_id)
        .await
        .map_err(refused("plugin_feature_work"))
}
