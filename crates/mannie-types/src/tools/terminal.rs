//! Commands that a plugin's tool types into a terminal (ADR-0053, decision
//! 5): where a terminal runs, as the access, a grant and a rule name it, and
//! the rules for such commands as the settings list them.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::clock::UtcMillis;
use crate::ids::RuleId;

/// Where a terminal runs, as a terminal command's access, a grant and a rule
/// name it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TerminalHost {
    /// This machine, in one of its shells.
    ThisMachine {
        /// The canonical path of the shell's program. A grant or a rule
        /// covers only the same shell, since the same words mean other
        /// things in another shell.
        shell: String,
    },
    /// A host that a plugin offers to terminals, such as a host that the
    /// owner registered with the SSH plugin.
    Remote {
        /// The plugin that offers the host.
        plugin: String,
        /// The plugin's key for the host, which stays while the host exists.
        target: String,
        /// The host's name, as the owner gave it, for display only.
        label: String,
        /// Where the terminal runs, such as `deploy@build-box.lan:22`.
        endpoint: String,
    },
}

impl TerminalHost {
    /// Whether two hosts are the same for a grant or a rule: the same shell
    /// on this machine, or the same plugin, target and endpoint elsewhere. A
    /// host's name is for display only, so renaming a host keeps its grants
    /// and rules.
    pub fn same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::ThisMachine { shell }, Self::ThisMachine { shell: other_shell }) => {
                shell == other_shell
            }
            (
                Self::Remote {
                    plugin,
                    target,
                    endpoint,
                    ..
                },
                Self::Remote {
                    plugin: other_plugin,
                    target: other_target,
                    endpoint: other_endpoint,
                    ..
                },
            ) => plugin == other_plugin && target == other_target && endpoint == other_endpoint,
            _ => false,
        }
    }

    /// Whether the host is a remote one, whose shell may read a backslash as
    /// an escape whatever this machine runs.
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote { .. })
    }
}

/// The terminals that a grant or a rule covers: those of one host, typed into
/// by one plugin's tools.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalTarget {
    /// The plugin whose tool types the commands.
    pub plugin: String,
    /// Where the terminals run.
    pub host: TerminalHost,
}

impl TerminalTarget {
    /// Whether this target covers commands that `plugin` types on `host`.
    pub fn covers(&self, plugin: &str, host: &TerminalHost) -> bool {
        self.plugin == plugin && self.host.same_as(host)
    }
}

/// A rule for commands that a plugin's tool types into the terminals of one
/// host, as the settings list it. It lives apart from the rules of commands
/// and of remote hosts, so a rule of one never matches a command of another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCommandRule {
    /// The rule.
    pub id: RuleId,
    /// The host and the plugin that the rule applies to.
    pub target: TerminalTarget,
    /// The words a command must start with.
    pub words: Vec<String>,
    /// When the owner saved it.
    pub created_at: UtcMillis,
}

/// The argument of `delete_terminal_command_rule`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalRuleRef {
    /// The rule.
    pub rule_id: RuleId,
}

#[cfg(test)]
mod tests {
    use super::{TerminalHost, TerminalTarget};

    fn ssh(label: &str, endpoint: &str) -> TerminalHost {
        TerminalHost::Remote {
            plugin: "ssh".to_owned(),
            target: "0199e0f2-0000-7000-8000-0000000000aa".to_owned(),
            label: label.to_owned(),
            endpoint: endpoint.to_owned(),
        }
    }

    #[test]
    fn a_host_is_the_same_whatever_its_name_and_never_across_kinds() {
        let shell = TerminalHost::ThisMachine {
            shell: "C:\\Windows\\System32\\cmd.exe".to_owned(),
        };
        let other_shell = TerminalHost::ThisMachine {
            shell: "C:\\Program Files\\PowerShell\\7\\pwsh.exe".to_owned(),
        };

        assert!(
            ssh("build-box", "deploy@build-box:22").same_as(&ssh("renamed", "deploy@build-box:22"))
        );
        assert!(
            !ssh("build-box", "deploy@build-box:22")
                .same_as(&ssh("build-box", "root@build-box:22"))
        );
        assert!(shell.same_as(&shell.clone()));
        assert!(!shell.same_as(&other_shell));
        assert!(!shell.same_as(&ssh("build-box", "deploy@build-box:22")));
    }

    #[test]
    fn a_target_covers_only_its_plugin_and_host() {
        let target = TerminalTarget {
            plugin: "terminal".to_owned(),
            host: ssh("build-box", "deploy@build-box:22"),
        };

        assert!(target.covers("terminal", &ssh("other name", "deploy@build-box:22")));
        assert!(!target.covers("other", &ssh("build-box", "deploy@build-box:22")));
    }

    #[test]
    fn hosts_serialize_with_their_kind() {
        assert_eq!(
            serde_json::to_value(TerminalHost::ThisMachine {
                shell: "/bin/bash".to_owned()
            })
            .unwrap(),
            serde_json::json!({ "kind": "this_machine", "shell": "/bin/bash" })
        );
        assert_eq!(
            serde_json::to_value(ssh("build-box", "deploy@build-box:22")).unwrap(),
            serde_json::json!({
                "kind": "remote",
                "plugin": "ssh",
                "target": "0199e0f2-0000-7000-8000-0000000000aa",
                "label": "build-box",
                "endpoint": "deploy@build-box:22",
            })
        );
    }
}
