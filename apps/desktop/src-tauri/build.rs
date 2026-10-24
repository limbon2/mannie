//! Tauri's build step: the access control list and the embedded context.
//!
//! Every app command gets permissions through `AppManifest::commands`, so a
//! command runs only when a capability grants it (ADR-0004). Validation builds
//! also compile the validation commands and the capability in
//! `capabilities/validation/`; other builds see only the capabilities at the
//! top of `capabilities/`.
//!
//! Each plugin becomes an inlined Tauri plugin (ADR-0034, decision 8): every
//! command and channel of its interface, as `task gen` wrote the interfaces
//! into `.generated/desktop/interfaces.json`, gets its own permission
//! `<id>:allow-<name>`, and a capability that this script writes grants the
//! plugin's commands to the main webview by its label. A grant for the
//! window would also cover a child webview inside it (spike S3). The fixture
//! plugins, which only validation builds bundle (ADR-0035, decision 9), get
//! their grants only there. The capabilities are copied into `OUT_DIR` with
//! the generated ones, and Tauri reads them from there.
//!
//! The script first compares the plugins' files with the fingerprint that
//! `task gen` wrote last, and refuses to build when they changed since, so a
//! build never mixes a launcher, a lock or grants of earlier plugins with
//! the plugins of now (ADR-0035, decision 3).

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tauri_build::{AppManifest, Attributes, DefaultPermissionRule, InlinedPlugin};

/// The repository's root, from the binary's folder.
const ROOT: &str = "../../..";

/// The fingerprint that `task gen` wrote last.
const FINGERPRINT: &str = "../../../.generated/desktop/fingerprint";

/// The files outside the plugins' folders that the fingerprint covers, as
/// `scripts/plugins/lib/fingerprint.mjs` lists them.
const FINGERPRINTED_FILES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "apps/desktop/core/Cargo.toml",
    "apps/desktop/src-tauri/Cargo.toml",
    ".generated/desktop/interfaces.json",
];

/// The folders that hold plugins, as the generator's `PLUGIN_FOLDERS` lists
/// them.
const PLUGIN_FOLDERS: &[&str] = &["plugins", "tests/fixtures/plugins"];

/// The folders that the walk of a plugin's folder skips, besides those whose
/// names start with a dot.
const SKIPPED_FOLDERS: &[&str] = &["generated", "node_modules", "target"];

/// The plugins' interfaces, which the plugin bindings program of `task gen`
/// writes (ADR-0035, decision 3).
const INTERFACES: &str = "../../../.generated/desktop/interfaces.json";

/// The routes of one plugin: its ID, which names its inlined Tauri plugin
/// and its permissions, and the names of its commands and channels.
struct PluginRoutes {
    id: &'static str,
    names: &'static [&'static str],
}

/// Why the interfaces cannot be read, with the step that writes them.
fn stale(problem: &str) -> String {
    format!("{INTERFACES} {problem}. Run `task gen`, which writes it.")
}

/// A string of the interfaces' JSON, kept for the rest of the build.
fn leaked(value: &serde_json::Value, what: &str) -> Result<&'static str, String> {
    let text = value
        .as_str()
        .ok_or_else(|| stale(&format!("has a {what} that is not a string")))?;
    Ok(String::leak(text.to_owned()))
}

/// The routes of one plugin of the interfaces, or none for a fixture plugin
/// outside a validation build.
fn routes_of(plugin: &serde_json::Value, validation: bool) -> Result<Option<PluginRoutes>, String> {
    let only_validation = plugin["validation"]
        .as_bool()
        .ok_or_else(|| stale("has a plugin without a validation flag"))?;
    if only_validation && !validation {
        return Ok(None);
    }
    let mut names: Vec<&'static str> = Vec::new();
    for key in ["commands", "channels"] {
        let list = plugin[key]
            .as_array()
            .ok_or_else(|| stale(&format!("has {key} that are not a list")))?;
        for name in list {
            names.push(leaked(name, key)?);
        }
    }
    Ok(Some(PluginRoutes {
        id: leaked(&plugin["id"], "id")?,
        names: Vec::leak(names),
    }))
}

/// The routes of every plugin that this build bundles, in the manifests'
/// order, with the fixture plugins only in a validation build. The build
/// script runs once, so leaking the names that Tauri keeps as `&'static str`
/// costs nothing.
fn plugin_routes(validation: bool) -> Result<Vec<PluginRoutes>, String> {
    let text = fs::read_to_string(INTERFACES)
        .map_err(|error| stale(&format!("cannot be read ({error})")))?;
    let interfaces: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| stale(&format!("is not valid JSON ({error})")))?;
    let plugins = interfaces
        .as_array()
        .ok_or_else(|| stale("is not a list"))?;
    let mut routes = Vec::new();
    for plugin in plugins {
        routes.extend(routes_of(plugin, validation)?);
    }
    Ok(routes)
}

/// The `Cargo.toml` files below a folder, relative to the root.
fn cargo_manifests(root: &Path, folder: &str, found: &mut Vec<String>) -> Result<(), String> {
    let entries = fs::read_dir(root.join(folder))
        .map_err(|error| format!("cannot list {folder} ({error})"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot list {folder} ({error})"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = format!("{folder}/{name}");
        let kind = entry
            .file_type()
            .map_err(|error| format!("cannot read {path} ({error})"))?;
        let skipped = SKIPPED_FOLDERS.contains(&name.as_str()) || name.starts_with('.');
        if kind.is_dir() {
            if !skipped {
                cargo_manifests(root, &path, found)?;
            }
        } else if name == "Cargo.toml" {
            found.push(path);
        }
    }
    Ok(())
}

/// The files that the fingerprint covers, relative to the root, sorted, as
/// the generator lists them: the fixed files, and in each folder of a plugin
/// its `plugin.toml`, its `Cargo.lock` and every `Cargo.toml` below it.
fn fingerprinted_files(root: &Path) -> Result<Vec<String>, String> {
    let mut files: Vec<String> = FINGERPRINTED_FILES
        .iter()
        .map(|&file| file.to_owned())
        .collect();
    for parent in PLUGIN_FOLDERS {
        let Ok(entries) = fs::read_dir(root.join(parent)) else {
            continue;
        };
        for entry in entries {
            let entry = entry.map_err(|error| format!("cannot list {parent} ({error})"))?;
            let folder = format!("{parent}/{}", entry.file_name().to_string_lossy());
            let is_plugin = root.join(&folder).join("plugin.toml").is_file();
            if !is_plugin {
                continue;
            }
            files.push(format!("{folder}/plugin.toml"));
            files.push(format!("{folder}/Cargo.lock"));
            cargo_manifests(root, &folder, &mut files)?;
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// The fingerprint of the files as they are now, in the generator's format:
/// a line `<sha256>  <path>` for each, or `missing  <path>`.
fn fingerprint(root: &Path, files: &[String]) -> String {
    let mut text = String::new();
    for file in files {
        let hash = fs::read(root.join(file)).map_or_else(
            |_missing| "missing".to_owned(),
            |bytes| format!("{:x}", Sha256::digest(bytes)),
        );
        writeln!(text, "{hash}  {file}").expect("writing to a String cannot fail");
    }
    text
}

/// Whether the plugins' files are as `task gen` left them. Cargo runs this
/// script again when one of the files changes.
fn check_fingerprint() -> Result<(), String> {
    let root = Path::new(ROOT);
    let files = fingerprinted_files(root)?;
    println!("cargo:rerun-if-changed={FINGERPRINT}");
    for file in &files {
        // Cargo runs a build script every time when a watched file is
        // missing, so only the files that exist are watched.
        if root.join(file).is_file() {
            println!("cargo:rerun-if-changed={ROOT}/{file}");
        }
    }
    let recorded = fs::read_to_string(FINGERPRINT).map_err(|_missing| {
        "`task gen` has not written the fingerprint of the plugins. Run `task gen`.".to_owned()
    })?;
    if recorded.replace("\r\n", "\n") != fingerprint(root, &files) {
        return Err("The plugins changed since `task gen` ran. Run `task gen`.".to_owned());
    }
    Ok(())
}

/// Commands of every build.
const APP_COMMANDS: &[&str] = &[
    "add_project",
    "add_project_from_path",
    "answer_close_question",
    "check_link",
    "compact_conversation",
    "confirm_quit",
    "create_provider_profile",
    "decide_approval",
    "delete_command_rule",
    "delete_conversation",
    "delete_program_rule",
    "delete_provider_profile",
    "delete_remote_command_rule",
    "dismiss_window_request",
    "get_chat_settings",
    "get_checkpoint",
    "get_child_agents_settings",
    "get_interface_settings",
    "get_model_defaults",
    "get_startup_status",
    "get_tools_settings",
    "get_window_settings",
    "list_command_rules",
    "list_conversation_grants",
    "list_mention_providers",
    "list_program_rules",
    "list_provider_profiles",
    "list_remote_command_rules",
    "list_shared_grants",
    "load_earlier_turns",
    "open_link",
    "plugin_work",
    "read_image",
    "export_conversation",
    "read_tool_output",
    "remove_project",
    "rename_conversation",
    "rename_project",
    "retry_reply",
    "revoke_conversation_grant",
    "revoke_shared_grant",
    "send_message",
    "set_conversation_profile",
    "set_default_provider_profile",
    "set_full_access",
    "set_plugin_enabled",
    "set_tray_labels",
    "start_conversation",
    "stop_background_command",
    "stop_reply",
    "subscribe_command_output",
    "subscribe_conversation",
    "subscribe_conversations",
    "subscribe_plugins",
    "subscribe_window_requests",
    "suggest_mentions",
    "test_provider_profile",
    "unsubscribe",
    "update_chat_settings",
    "update_child_agents_settings",
    "update_interface_settings",
    "update_provider_profile",
    "update_tools_settings",
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
    "validation_set_file_choice",
    "validation_set_folder_choice",
    "validation_tray_menu",
    "validation_ungranted_command",
    "validation_window_state",
];

/// The label of the webview that receives the plugins' grants.
const MAIN_WEBVIEW: &str = "main";

/// Copies the capabilities in `from` into `to`, in name order.
fn copy_capabilities(from: &Path, to: &Path) {
    let mut files: Vec<PathBuf> = fs::read_dir(from)
        .expect("the capabilities folder can be read")
        .map(|entry| entry.expect("a capability's entry can be read").path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    for file in files {
        let name = file.file_name().expect("a capability has a file name");
        fs::copy(&file, to.join(name)).expect("a capability can be copied into OUT_DIR");
    }
}

/// The capability that grants every command and channel of a plugin to the
/// main webview.
fn plugin_capability(routes: &PluginRoutes) -> String {
    let capability = format!(
        r#"{{
  "identifier": "plugin-{id}",
  "description": "Generated by build.rs: the main webview may call every command and channel of the plugin {id} (ADR-0034, decision 8).",
  "webviews": ["{webview}"],
  "permissions": ["{id}:default"]
}}
"#,
        id = routes.id,
        webview = MAIN_WEBVIEW,
    );
    capability
}

fn main() {
    let validation = std::env::var_os("CARGO_FEATURE_VALIDATION").is_some();
    let commands: &'static [&'static str] = if validation {
        // The build script runs once, so leaking the joined list costs nothing.
        let all: Vec<&'static str> = APP_COMMANDS
            .iter()
            .chain(VALIDATION_COMMANDS)
            .copied()
            .collect();
        Vec::leak(all)
    } else {
        APP_COMMANDS
    };

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"))
        .join("capabilities");
    if out.exists() {
        fs::remove_dir_all(&out).expect("the capabilities of an earlier build can be removed");
    }
    fs::create_dir_all(&out).expect("OUT_DIR can hold the capabilities");
    copy_capabilities(Path::new("capabilities"), &out);
    if validation {
        copy_capabilities(Path::new("capabilities/validation"), &out);
    }

    println!("cargo:rerun-if-changed=capabilities");
    println!("cargo:rerun-if-changed={INTERFACES}");
    let plugins = match check_fingerprint().and_then(|()| plugin_routes(validation)) {
        Ok(plugins) => plugins,
        Err(problem) => {
            // Cargo fails the build after the script, with this message.
            println!("cargo::error={problem}");
            return;
        }
    };
    let mut attributes = Attributes::new();
    for routes in plugins {
        attributes = attributes.plugin(
            routes.id,
            InlinedPlugin::new()
                .commands(routes.names)
                .default_permission(DefaultPermissionRule::AllowAllCommands),
        );
        fs::write(
            out.join(format!("plugin-{}.json", routes.id)),
            plugin_capability(&routes),
        )
        .expect("a plugin's capability can be written into OUT_DIR");
    }

    // The pattern is the folder in OUT_DIR, which holds only the capabilities
    // of this build.
    let pattern = format!("{}/*.json", out.display());
    let attributes = attributes
        .capabilities_path_pattern(String::leak(pattern))
        .app_manifest(AppManifest::new().commands(commands));
    tauri_build::try_build(attributes)
        .expect("Tauri's build step needs a valid configuration and capabilities");
}
