//! The `tools` settings section (docs/features/system-tools/
//! operations-and-approvals.md, settings).

use serde::{Deserialize, Deserializer, Serialize};
use specta::Type;

use crate::settings::SettingsSection;

use super::commands::ToolsError;

/// The most environment variables the owner can add for commands.
pub const MAX_ENVIRONMENT_NAMES: usize = 50;

/// The longest name of an environment variable Mannie accepts.
const MAX_ENV_NAME_CHARACTERS: usize = 128;

/// Parts of a variable name that mark a secret. A name with one of them
/// between its underscores never reaches a command (AGENTS.md: secrets never
/// reach child process environments). The build's check of plugin manifests
/// reads the list from the SDK's `plugin-rules.json`.
pub const SECRET_NAME_PARTS: [&str; 8] = [
    "KEY",
    "APIKEY",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "CREDENTIAL",
    "CREDENTIALS",
];

/// Whether a name looks like a secret's: one of its words, between the
/// characters that are not ASCII letters or digits, is one of
/// [`SECRET_NAME_PARTS`] in any case, as in `GITHUB_TOKEN`, `apiKey` or
/// `x-api-key`.
pub fn looks_like_secret(name: &str) -> bool {
    name.split(|character: char| !character.is_ascii_alphanumeric())
        .any(|part| SECRET_NAME_PARTS.contains(&part.to_ascii_uppercase().as_str()))
}

/// The starts of the names of variables that make programs load libraries,
/// such as `LD_PRELOAD` and `DYLD_INSERT_LIBRARIES`. An installed program
/// that a plugin runs never receives one by the plugin's choice (ADR-0038,
/// decision 7).
pub const LOADER_NAME_PREFIXES: [&str; 2] = ["LD_", "DYLD_"];

/// Whether a variable's name makes programs load libraries, in any case.
pub fn is_loader_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    LOADER_NAME_PREFIXES
        .iter()
        .any(|prefix| upper.starts_with(prefix))
}

/// The name of an environment variable that commands receive. Names that look
/// like a secret are refused.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Type)]
#[serde(transparent)]
pub struct EnvName(String);

impl EnvName {
    /// Checks the name: a letter or an underscore, then letters, digits,
    /// underscores or parentheses, as in `ProgramFiles(x86)`.
    pub fn parse(text: &str) -> Result<Self, ToolsError> {
        let invalid = || ToolsError::EnvironmentNameInvalid {
            name: text.chars().take(MAX_ENV_NAME_CHARACTERS).collect(),
        };
        let mut characters = text.chars();
        let starts_well = characters
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_');
        let continues_well = characters
            .all(|character| character.is_ascii_alphanumeric() || "_()".contains(character));
        let has_valid_length = text.len() <= MAX_ENV_NAME_CHARACTERS;
        if !(starts_well && continues_well && has_valid_length) {
            return Err(invalid());
        }
        if looks_like_secret(text) {
            return Err(ToolsError::EnvironmentNameSecret {
                name: text.to_owned(),
            });
        }
        Ok(Self(text.to_owned()))
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for EnvName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// The tools settings section. A section stored before projects replaced
/// the default workspace still has `defaultWorkspace`, which reading
/// ignores (ADR-0023).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolsSettings {
    /// Whether changes inside a conversation's workspace run without asking.
    /// Off by default (owner decision of 2026-10-14).
    pub allow_changes_in_workspace: bool,
    /// Environment variables that commands receive besides the built-in ones.
    pub command_environment: Vec<EnvName>,
    /// Whether a new conversation starts in full access, which its own switch
    /// changes afterwards. Off by default (ADR-0029, decision 2).
    #[serde(default)]
    pub full_access_for_new_conversations: bool,
    /// Whether a background command's own end wakes an idle agent with a
    /// reply of Mannie's own (ADR-0041, decision 8). On by default (D40),
    /// also in a section stored before it existed.
    #[serde(default = "wake_by_default")]
    pub wake_on_command_end: bool,
}

/// Wake-ups are on unless the owner turns them off.
const fn wake_by_default() -> bool {
    true
}

impl Default for ToolsSettings {
    fn default() -> Self {
        Self {
            allow_changes_in_workspace: false,
            command_environment: Vec::new(),
            full_access_for_new_conversations: false,
            wake_on_command_end: wake_by_default(),
        }
    }
}

impl SettingsSection for ToolsSettings {
    const KEY: &'static str = "tools";
    const VERSION: u32 = 1;
}

/// The argument of `update_tools_settings`: the complete new section, checked
/// by the core.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateToolsSettings {
    /// Whether changes inside the workspace run without asking.
    pub allow_changes_in_workspace: bool,
    /// The names of extra environment variables for commands.
    pub command_environment: Vec<String>,
    /// Whether new conversations start in full access.
    pub full_access_for_new_conversations: bool,
    /// Whether a background command's own end wakes an idle agent.
    #[serde(default = "wake_by_default")]
    pub wake_on_command_end: bool,
}

impl UpdateToolsSettings {
    /// Checks every field and turns the request into the section to store.
    pub fn into_settings(self) -> Result<ToolsSettings, ToolsError> {
        if self.command_environment.len() > MAX_ENVIRONMENT_NAMES {
            return Err(ToolsError::TooManyEnvironmentNames {
                max: u32::try_from(MAX_ENVIRONMENT_NAMES).unwrap_or(u32::MAX),
            });
        }
        let mut command_environment = self
            .command_environment
            .iter()
            .map(|name| EnvName::parse(name.trim()))
            .collect::<Result<Vec<_>, _>>()?;
        command_environment.sort();
        command_environment.dedup();
        Ok(ToolsSettings {
            allow_changes_in_workspace: self.allow_changes_in_workspace,
            command_environment,
            full_access_for_new_conversations: self.full_access_for_new_conversations,
            wake_on_command_end: self.wake_on_command_end,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{EnvName, ToolsSettings, UpdateToolsSettings};
    use crate::tools::ToolsError;

    #[test]
    fn environment_names_are_checked_and_secrets_refused() {
        for name in [
            "CARGO_HOME",
            "ProgramFiles(x86)",
            "_PRIVATE_DIR",
            "KEYBOARD_LAYOUT",
        ] {
            assert_eq!(EnvName::parse(name).unwrap().as_str(), name);
        }
        for name in ["", "1PATH", "A-B", "PATH VAR"] {
            assert!(
                matches!(
                    EnvName::parse(name),
                    Err(ToolsError::EnvironmentNameInvalid { .. })
                ),
                "{name:?}"
            );
        }
        for name in [
            "OPENAI_API_KEY",
            "GITHUB_TOKEN",
            "db_password",
            "AWS_SECRET_ACCESS_KEY",
        ] {
            assert!(
                matches!(
                    EnvName::parse(name),
                    Err(ToolsError::EnvironmentNameSecret { .. })
                ),
                "{name:?}"
            );
        }
    }

    #[test]
    fn an_update_becomes_the_section_with_sorted_unique_names() {
        let settings = UpdateToolsSettings {
            allow_changes_in_workspace: true,
            command_environment: vec![
                "RUSTUP_HOME".to_owned(),
                " CARGO_HOME ".to_owned(),
                "RUSTUP_HOME".to_owned(),
            ],
            full_access_for_new_conversations: true,
            wake_on_command_end: false,
        }
        .into_settings()
        .unwrap();

        assert_eq!(
            settings
                .command_environment
                .iter()
                .map(EnvName::as_str)
                .collect::<Vec<_>>(),
            ["CARGO_HOME", "RUSTUP_HOME"]
        );
        assert!(settings.allow_changes_in_workspace);
        assert!(settings.full_access_for_new_conversations);
        assert!(!ToolsSettings::default().allow_changes_in_workspace);
        assert!(!ToolsSettings::default().full_access_for_new_conversations);
        assert!(!settings.wake_on_command_end);
        assert!(ToolsSettings::default().wake_on_command_end);
    }

    #[test]
    fn a_section_from_before_wake_ups_reads_with_them_on() {
        let stored = serde_json::json!({
            "allowChangesInWorkspace": false,
            "commandEnvironment": [],
            "fullAccessForNewConversations": false,
        });

        let settings: ToolsSettings = serde_json::from_value(stored).unwrap();

        assert!(settings.wake_on_command_end);
    }

    #[test]
    fn a_section_from_before_full_access_reads_with_it_off() {
        let stored = serde_json::json!({
            "allowChangesInWorkspace": true,
            "commandEnvironment": [],
        });

        let settings = serde_json::from_value::<ToolsSettings>(stored).unwrap();

        assert!(!settings.full_access_for_new_conversations);
    }

    #[test]
    fn a_stored_section_with_a_secret_name_does_not_read() {
        let stored = serde_json::json!({
            "allowChangesInWorkspace": false,
            "commandEnvironment": ["OPENAI_API_KEY"],
        });

        serde_json::from_value::<ToolsSettings>(stored).unwrap_err();
    }

    #[test]
    fn a_section_with_the_removed_default_workspace_still_reads() {
        let stored = serde_json::json!({
            "defaultWorkspace": "/home/owner/projects/app",
            "allowChangesInWorkspace": true,
            "commandEnvironment": ["CARGO_HOME"],
        });

        let settings = serde_json::from_value::<ToolsSettings>(stored).unwrap();

        assert!(settings.allow_changes_in_workspace);
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            serde_json::json!({
                "allowChangesInWorkspace": true,
                "commandEnvironment": ["CARGO_HOME"],
                "fullAccessForNewConversations": false,
                "wakeOnCommandEnd": true,
            })
        );
    }
}
