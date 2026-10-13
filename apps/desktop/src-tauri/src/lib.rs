//! The Mannie desktop app: the composition root of the master (ADR-0008).
//!
//! It starts logging, opens the store, registers the IPC commands with
//! tauri-specta (ADR-0007), and creates the main window with the security
//! baseline of ADR-0004. The logic lives in the library crates; this crate
//! wires them together and turns their results into IPC responses.

mod commands;
mod ipc;
mod state;
mod window;

pub use ipc::{BindingsError, export_bindings};

/// Runs the app until the owner quits it.
pub fn run() -> anyhow::Result<()> {
    let commands = ipc::runtime_builder();
    // Debug builds keep the bindings in the source tree current (ADR-0007).
    #[cfg(debug_assertions)]
    export_bindings(&ipc::source_bindings_directory())?;

    let app = tauri::Builder::default()
        // Registered first: a second start of Mannie hands over to the running
        // app before anything else happens.
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _working_directory| {
                window::show_main(app);
            },
        ))
        .invoke_handler(commands.invoke_handler())
        .setup(|app| {
            state::start(app);
            window::create_main(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())?;
    app.run(|_app, event| {
        if let tauri::RunEvent::Exit = event {
            tracing::info!("Mannie is quitting");
        }
    });
    Ok(())
}
