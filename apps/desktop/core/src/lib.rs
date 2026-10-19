//! The Mannie desktop app's library: the composition root of the master
//! (ADR-0008, ADR-0035).
//!
//! It starts logging, opens the store, starts the runtime, registers the IPC
//! commands with tauri-specta (ADR-0007) and one inlined Tauri plugin for each
//! plugin, whose calls go to the host's dispatch (ADR-0034, decision 8), and
//! creates the main window with the security baseline of ADR-0004. The logic
//! lives in the library crates; this crate wires them together and turns
//! their results into IPC responses.
//!
//! The binary in `apps/desktop/src-tauri` calls [`run`] with Tauri's context,
//! which its build script compiles, and with the plugins that `task gen`
//! found in `plugins/` (ADR-0035, decisions 3 and 4), so this crate names no
//! plugin.

mod commands;
mod credentials;
mod file_dialog;
mod folder_dialog;
mod ipc;
mod links;
mod plugin_ipc;
mod state;
mod window;

use std::sync::{Arc, OnceLock};

use mannie_plugin_sdk::plugin::PluginEntry;
use tauri::{Context, Manager, Wry};

pub use ipc::{BindingsError, export_bindings};

use crate::state::Core;

/// Runs the app with Tauri's `context` and the bundled `plugins` until the
/// owner quits it, then shuts the core down: active replies become
/// interrupted, and the store and the logs close.
pub fn run(context: Context<Wry>, plugins: Vec<PluginEntry>) -> anyhow::Result<()> {
    let commands = ipc::runtime_builder();
    // Debug builds keep the core's bindings in the source tree current
    // (ADR-0007); `task gen` writes the plugins' bindings.
    #[cfg(debug_assertions)]
    export_bindings(&ipc::source_bindings_directory())?;

    // The core outlives the event loop, so that quitting can shut it down
    // after the window is gone.
    let started: Arc<OnceLock<Core>> = Arc::default();
    let setup_core = Arc::clone(&started);
    let mut builder = tauri::Builder::default()
        // Registered first: a second start of Mannie hands over to the running
        // app before anything else happens.
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _working_directory| {
                window::show_main(app);
            },
        ));
    for plugin in plugin_ipc::tauri_plugins(&plugins) {
        builder = builder.plugin(plugin);
    }
    let app = builder
        .invoke_handler(commands.invoke_handler())
        .setup(move |app| {
            let core = state::start(app, plugins);
            if setup_core.set(core.clone()).is_err() {
                tracing::error!("the core started twice");
            }
            app.manage(core);
            window::create_main(app)?;
            Ok(())
        })
        .build(context)?;
    let exit_code = app.run_return(|_app, event| {
        if let tauri::RunEvent::ExitRequested { code, .. } = event {
            tracing::info!(?code, "the event loop ends");
        }
    });
    if let Some(core) = started.get() {
        tauri::async_runtime::block_on(core.shutdown());
    }
    if exit_code != 0 {
        anyhow::bail!("the event loop ended with code {exit_code}");
    }
    Ok(())
}
