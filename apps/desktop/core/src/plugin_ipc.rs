//! The plugins' commands and channels (ADR-0034, decision 8): one inlined
//! Tauri plugin for each hosted plugin, named by its manifest's ID, whose one
//! handler hands every call to the host's dispatch. The routes come from each
//! plugin's interface, as the build's grants do, so nothing here names a
//! plugin, and Tauri's access control has refused every call that the build
//! did not grant to the main webview before the handler runs.
//!
//! The generated transport sends a command's input as `{ input }`, and a
//! channel's as `{ input, on }`, where `on` names the webview's channel
//! (ADR-0034, decision 9). The handler answers every call, also one that the
//! host aborted, because Tauri never answers a call whose resolver was
//! dropped (spike S3). A channel's messages reach the webview in batches,
//! like the core's own subscriptions, until the host ends the channel, the
//! UI unsubscribes or the page loads again.

use std::str::FromStr;
use std::sync::Arc;

use futures_util::StreamExt;
use mannie_plugin_sdk::ipc::{ChannelMessage, HostRefusal, PluginStream};
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::plugin::PluginEntry;
use mannie_plugins::CallFailure;
use mannie_types::ipc::ChannelBatch;
use mannie_types::transcript::SubscriptionId;
use serde_json::Value;
use tauri::ipc::{Channel, Invoke, InvokeBody, InvokeError, JavaScriptChannelId};
use tauri::plugin::TauriPlugin;
use tauri::{Manager, Wry};

use crate::ipc::subscriptions::Page;
use crate::state::Core;

/// The channel through which a plugin's channel messages reach the webview.
type PluginChannel = Channel<ChannelBatch<ChannelMessage<Value>>>;

/// The routes of one plugin: its ID, which names its inlined Tauri plugin,
/// and the names of its commands and channels.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PluginRoutes {
    id: &'static str,
    commands: Vec<&'static str>,
    channels: Vec<&'static str>,
}

/// The routes of a plugin from its manifest and its interface, or none for a
/// manifest that the host refuses, such as one for another major of the SDK,
/// whose code is never called.
fn routes(entry: &PluginEntry) -> Option<PluginRoutes> {
    let manifest = Manifest::parse(entry.manifest).ok()?;
    let interface = entry.plugin.interface();
    // Tauri names a plugin with a `&'static str`. The list of plugins is fixed
    // for the process, so each ID is kept once, for as long as it runs.
    let id: &'static str = String::leak(manifest.id.to_string());
    Some(PluginRoutes {
        id,
        commands: interface.command_names(),
        channels: interface.channel_names(),
    })
}

/// The inlined Tauri plugins of `plugins`.
pub(crate) fn tauri_plugins(plugins: &[PluginEntry]) -> Vec<TauriPlugin<Wry>> {
    plugins.iter().filter_map(routes).map(forwarder).collect()
}

/// The inlined Tauri plugin of one plugin, named by its ID, so its calls are
/// `plugin:<id>|<name>`.
fn forwarder(routes: PluginRoutes) -> TauriPlugin<Wry> {
    let id = routes.id;
    let routes = Arc::new(routes);
    tauri::plugin::Builder::<Wry>::new(id)
        .invoke_handler(move |invoke| {
            forward(&routes, invoke);
            true
        })
        .build()
}

/// The host's refusal as the UI reads it.
fn refusal_json(refusal: HostRefusal) -> Value {
    CallFailure::Host(refusal).to_json()
}

/// The host's refusal as the answer of a call that the dispatch answers.
fn refusal(refusal: HostRefusal) -> InvokeError {
    InvokeError(refusal_json(refusal))
}

fn failure(failure: &CallFailure) -> InvokeError {
    InvokeError(failure.to_json())
}

/// Hands one call to the host's dispatch and answers it.
fn forward(routes: &PluginRoutes, invoke: Invoke<Wry>) {
    let plugin = routes.id;
    let Invoke {
        message, resolver, ..
    } = invoke;
    let name = message.command().to_owned();
    // The core is managed before the window opens, so every call finds it.
    let Some(core) = message
        .webview()
        .try_state::<Core>()
        .map(|core| core.inner().clone())
    else {
        tracing::error!(plugin, "a plugin's call came before the core started");
        resolver.reject(refusal_json(HostRefusal::HostServiceFailed));
        return;
    };
    let (input, on) = match message.payload() {
        InvokeBody::Json(Value::Object(payload)) => (
            payload.get("input").cloned().unwrap_or(Value::Null),
            payload.get("on").cloned(),
        ),
        // A body that is not a JSON object, such as raw bytes, never reaches
        // a plugin.
        _other => {
            resolver.reject(refusal_json(HostRefusal::InvalidInput { field: None }));
            return;
        }
    };
    if routes.commands.contains(&name.as_str()) {
        resolver.respond_async(async move { invoke_command(&core, plugin, &name, input).await });
        return;
    }
    if !routes.channels.contains(&name.as_str()) {
        // Tauri's access control refuses a name without a permission first,
        // so this answers only a list that differs from the build's.
        resolver.reject(refusal_json(HostRefusal::UnknownCommand));
        return;
    }
    let channel_id = on
        .as_ref()
        .and_then(Value::as_str)
        .and_then(|id| JavaScriptChannelId::from_str(id).ok());
    let Some(channel_id) = channel_id else {
        resolver.reject(refusal_json(HostRefusal::InvalidInput {
            field: Some("on".to_owned()),
        }));
        return;
    };
    let channel: PluginChannel = channel_id.channel_on(message.webview());
    let page = core.subscriptions().page();
    resolver.respond_async(async move {
        open_channel(&core, plugin, &name, input, channel, page).await
    });
}

/// Calls a plugin's command.
async fn invoke_command(
    core: &Core,
    plugin: &str,
    command: &str,
    input: Value,
) -> Result<Value, InvokeError> {
    let host = core
        .plugins()
        .ok_or_else(|| refusal(HostRefusal::StoreUnavailable))?;
    host.invoke(plugin, command, input)
        .await
        .map_err(|refused| failure(&refused))
}

/// Opens a plugin's channel and forwards its messages to the webview until
/// the host ends it.
async fn open_channel(
    core: &Core,
    plugin: &str,
    name: &str,
    input: Value,
    channel: PluginChannel,
    page: Page,
) -> Result<SubscriptionId, InvokeError> {
    let host = core
        .plugins()
        .ok_or_else(|| refusal(HostRefusal::StoreUnavailable))?;
    let messages = host
        .open_channel(plugin, name, input)
        .await
        .map_err(|refused| failure(&refused))?;
    Ok(core
        .subscriptions()
        .start(page, channel, |sender| forward_messages(messages, sender)))
}

/// Sends a channel's messages until the host's end, which is the last.
async fn forward_messages(
    mut messages: PluginStream<ChannelMessage<Value>>,
    sender: crate::ipc::channel::BatchSender<ChannelMessage<Value>>,
) {
    while let Some(message) = messages.next().await {
        let ended = matches!(message, ChannelMessage::Ended { .. });
        if sender.send(message).await.is_err() || ended {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_plugin_sdk::BoxFuture;
    use mannie_plugin_sdk::context::StartContext;
    use mannie_plugin_sdk::error::StartFailure;
    use mannie_plugin_sdk::ipc::{ChannelSpec, CommandSpec, Interface, PluginError};
    use mannie_plugin_sdk::plugin::{Plugin, PluginEntry, Started};
    use serde::Serialize;

    use super::{PluginRoutes, routes};

    #[derive(Debug, Serialize, specta::Type, thiserror::Error)]
    #[serde(tag = "code", rename_all = "snake_case")]
    enum NotesError {
        #[error("the note is too long")]
        TooLong,
    }

    impl PluginError for NotesError {
        fn code(&self) -> &'static str {
            "notes.too_long"
        }
    }

    enum AddNote {}

    impl CommandSpec for AddNote {
        const NAME: &'static str = "add_note";
        const DOCS: &'static str = "Adds a note.";
        type Input = ();
        type Output = ();
        type Error = NotesError;
    }

    enum FollowNotes {}

    impl ChannelSpec for FollowNotes {
        const NAME: &'static str = "follow_notes";
        const DOCS: &'static str = "Follows the notes.";
        type Input = ();
        type Item = u32;
        type Error = NotesError;
    }

    /// A plugin with a command and a channel, whose start is never called.
    #[derive(Debug)]
    struct Notes;

    impl Plugin for Notes {
        fn interface(&self) -> Interface {
            Interface::new()
                .command::<AddNote>()
                .channel::<FollowNotes>()
        }

        fn start(
            &self,
            _context: StartContext,
        ) -> BoxFuture<'static, Result<Started, StartFailure>> {
            Box::pin(std::future::ready(Err(StartFailure::new("not_started"))))
        }
    }

    fn entry(manifest: &'static str) -> PluginEntry {
        PluginEntry::new(manifest, Arc::new(Notes))
    }

    #[test]
    fn a_plugin_s_routes_are_its_manifest_s_id_and_its_interface_s_names() {
        let routes = routes(&entry(
            "id = \"notes\"\nsdk = 1\norder = 10\n\n[rust]\nentry = \"mannie-notes\"\n",
        ));

        assert_eq!(
            routes,
            Some(PluginRoutes {
                id: "notes",
                commands: vec!["add_note"],
                channels: vec!["follow_notes"],
            })
        );
        assert_eq!(NotesError::TooLong.code(), "notes.too_long");
    }

    #[test]
    fn a_plugin_for_another_major_of_the_sdk_gets_no_routes() {
        let routes = routes(&entry(
            "id = \"notes\"\nsdk = 2\norder = 10\n\n[rust]\nentry = \"mannie-notes\"\n",
        ));

        assert_eq!(routes, None);
    }
}
