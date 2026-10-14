//! Tauri's build step: the access control list and the embedded context.
//!
//! Every app command gets permissions through `AppManifest::commands`, so a
//! command runs only when a capability grants it (ADR-0004). Validation builds
//! also compile the validation commands and the capability in
//! `capabilities/validation/`; other builds see only the capabilities at the
//! top of `capabilities/`.

/// Commands of every build.
const APP_COMMANDS: &[&str] = &[
    "answer_close_question",
    "check_link",
    "compact_conversation",
    "confirm_quit",
    "create_provider_profile",
    "delete_conversation",
    "delete_provider_profile",
    "dismiss_window_request",
    "get_chat_settings",
    "get_checkpoint",
    "get_interface_settings",
    "get_model_defaults",
    "get_startup_status",
    "get_window_settings",
    "list_provider_profiles",
    "load_earlier_turns",
    "open_link",
    "rename_conversation",
    "retry_reply",
    "send_message",
    "set_conversation_profile",
    "set_default_provider_profile",
    "set_tray_labels",
    "start_conversation",
    "stop_reply",
    "subscribe_conversation",
    "subscribe_conversations",
    "subscribe_window_requests",
    "test_provider_profile",
    "unsubscribe",
    "update_chat_settings",
    "update_interface_settings",
    "update_provider_profile",
    "update_window_settings",
];

/// Commands of validation builds. `validation_ungranted_command` has a
/// permission but no grant, so the validations can show that a command without
/// a grant is refused (V1).
const VALIDATION_COMMANDS: &[&str] = &[
    "stream_validation_events",
    "validation_close_main_window",
    "validation_opened_links",
    "validation_process_id",
    "validation_seed_conversation",
    "validation_tray_menu",
    "validation_ungranted_command",
    "validation_window_state",
];

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
