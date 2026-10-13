//! Tauri's build step: the access control list and the embedded context.
//!
//! Every app command gets permissions through `AppManifest::commands`, so a
//! command runs only when a capability grants it (ADR-0004). Validation builds
//! also compile the validation commands and the capability in
//! `capabilities/validation/`; other builds see only the capabilities at the
//! top of `capabilities/`.

/// Commands of every build.
const APP_COMMANDS: &[&str] = &[
    "get_interface_settings",
    "get_startup_status",
    "update_interface_settings",
];

/// Commands of validation builds. `validation_ungranted_command` has a
/// permission but no grant, so the validations can show that a command without
/// a grant is refused (V1).
const VALIDATION_COMMANDS: &[&str] = &["stream_validation_events", "validation_ungranted_command"];

fn main() {
    let validation = std::env::var_os("CARGO_FEATURE_VALIDATION").is_some();
    let (commands, capabilities): (&'static [&'static str], _) = if validation {
        // The build script runs once, so leaking the joined list costs nothing.
        let all: Vec<&'static str> = APP_COMMANDS
            .iter()
            .chain(VALIDATION_COMMANDS)
            .copied()
            .collect();
        (Vec::leak(all), "./capabilities/**/*.json")
    } else {
        (APP_COMMANDS, "./capabilities/*.json")
    };
    let attributes = tauri_build::Attributes::new()
        .capabilities_path_pattern(capabilities)
        .app_manifest(tauri_build::AppManifest::new().commands(commands));
    println!("cargo:rerun-if-changed=capabilities");
    tauri_build::try_build(attributes)
        .expect("Tauri's build step needs a valid configuration and capabilities");
}
