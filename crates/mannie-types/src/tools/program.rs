//! Runs of installed programs that a plugin's manifest declares, such as
//! git (ADR-0038, decisions 5 and 6): what a run does beyond the workspace,
//! which the owner decides on, and what a grant or a rule of a program
//! covers of it.

use serde::{Deserialize, Serialize};
use specta::Type;

/// The most items that an effect names, such as the files that a run
/// discards. The effect keeps the number of all of them.
pub const MAX_EFFECT_ITEMS: usize = 20;

/// What a run of an installed program throws away.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum DiscardedWork {
    /// Changes that no commit holds, in the work tree or the index.
    UncommittedChanges,
    /// Entries of the stash.
    StashEntries,
    /// Commits that no other branch or tag holds.
    Commits,
}

/// What a run of an installed program does beyond the workspace, which the
/// owner decides on (ADR-0038, decision 5). The list is closed, so the core
/// words each effect in an approval.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(
    tag = "effect",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ProgramEffect {
    /// The run reaches an endpoint on the network.
    Network {
        /// The endpoint without credentials, such as a remote's URL.
        endpoint: String,
    },
    /// The run starts a program that the folder's own files or
    /// configuration name, such as a hook of a repository.
    RunsFolderCode {
        /// The program's name, such as `pre-commit`.
        name: String,
        /// Where it comes from, such as `.git/hooks/pre-commit` or the
        /// repository's configuration.
        source: String,
    },
    /// The run throws work away.
    DiscardsWork {
        /// What it throws away.
        work: DiscardedWork,
        /// How much of it.
        count: u32,
        /// Its names, such as files or commits, at most
        /// [`MAX_EFFECT_ITEMS`].
        items: Vec<String>,
        /// Whether the plugin keeps a snapshot first, from which the owner
        /// can take the work back.
        snapshot: bool,
    },
    /// The run rewrites commits that a remote-tracking branch already holds.
    RewritesPublished {
        /// How many commits.
        count: u32,
        /// The remote-tracking branches that hold them, at most
        /// [`MAX_EFFECT_ITEMS`].
        places: Vec<String>,
    },
}

impl ProgramEffect {
    /// What a grant or a rule covers of the effect.
    pub fn key(&self) -> ProgramEffectKey {
        match self {
            Self::Network { endpoint } => ProgramEffectKey::Network {
                endpoint: endpoint.clone(),
            },
            Self::RunsFolderCode { name, source } => ProgramEffectKey::RunsFolderCode {
                name: name.clone(),
                source: source.clone(),
            },
            Self::DiscardsWork { work, .. } => ProgramEffectKey::DiscardsWork { work: *work },
            Self::RewritesPublished { .. } => ProgramEffectKey::RewritesPublished,
        }
    }
}

/// What a grant or a rule of an installed program covers of an effect
/// (ADR-0038, decision 6): an endpoint by itself, a program that the folder
/// names by its name and source and not by its content, so a hook whose
/// content changed stays covered (D37), and discarded or rewritten work by
/// its kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(
    tag = "effect",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ProgramEffectKey {
    /// An endpoint that a run reaches.
    Network {
        /// The endpoint without credentials.
        endpoint: String,
    },
    /// A program that the folder names and a run starts.
    RunsFolderCode {
        /// The program's name.
        name: String,
        /// Where it comes from.
        source: String,
    },
    /// Work of a kind that a run throws away.
    DiscardsWork {
        /// What it throws away.
        work: DiscardedWork,
    },
    /// Commits that a remote-tracking branch holds, which a run rewrites.
    RewritesPublished,
}

/// The keys of effects, sorted and without duplicates, as a grant or a rule
/// keeps them.
pub fn effect_keys(effects: &[ProgramEffect]) -> Vec<ProgramEffectKey> {
    let mut keys: Vec<ProgramEffectKey> = effects.iter().map(ProgramEffect::key).collect();
    keys.sort();
    keys.dedup();
    keys
}

/// An installed program as a rule or a rule's proposal names it: the plugin
/// that runs it, its name in the plugin's manifest and the path that the core
/// found, which decide what the rule covers, and the keys of the effects
/// that the rule covers (ADR-0038, decision 6).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProgramTarget {
    /// The plugin whose tool runs the program.
    pub plugin: String,
    /// The program's name in the plugin's manifest, such as `git`.
    pub name: String,
    /// The program's canonical path.
    pub program: String,
    /// The keys of the effects that the rule covers, sorted.
    pub effects: Vec<ProgramEffectKey>,
}

#[cfg(test)]
mod tests {
    use super::{DiscardedWork, ProgramEffect, ProgramEffectKey, effect_keys};

    fn hook(name: &str) -> ProgramEffect {
        ProgramEffect::RunsFolderCode {
            name: name.to_owned(),
            source: format!(".git/hooks/{name}"),
        }
    }

    #[test]
    fn an_effect_keeps_its_key_without_what_a_grant_does_not_compare() {
        let discards = ProgramEffect::DiscardsWork {
            work: DiscardedWork::UncommittedChanges,
            count: 3,
            items: vec!["a.txt".to_owned()],
            snapshot: true,
        };
        let rewrites = ProgramEffect::RewritesPublished {
            count: 2,
            places: vec!["origin/main".to_owned()],
        };

        assert_eq!(
            discards.key(),
            ProgramEffectKey::DiscardsWork {
                work: DiscardedWork::UncommittedChanges
            }
        );
        assert_eq!(rewrites.key(), ProgramEffectKey::RewritesPublished);
        assert_eq!(
            hook("pre-commit").key(),
            ProgramEffectKey::RunsFolderCode {
                name: "pre-commit".to_owned(),
                source: ".git/hooks/pre-commit".to_owned()
            }
        );
    }

    #[test]
    fn the_keys_of_effects_are_sorted_and_unique() {
        let effects = [
            hook("pre-commit"),
            ProgramEffect::Network {
                endpoint: "https://example.com/repo.git".to_owned(),
            },
            hook("commit-msg"),
            hook("pre-commit"),
        ];

        let keys = effect_keys(&effects);

        assert_eq!(
            keys,
            [
                ProgramEffectKey::Network {
                    endpoint: "https://example.com/repo.git".to_owned()
                },
                hook("commit-msg").key(),
                hook("pre-commit").key(),
            ]
        );
    }

    #[test]
    fn effects_serialize_with_their_name_and_fields() {
        let discards = ProgramEffect::DiscardsWork {
            work: DiscardedWork::StashEntries,
            count: 1,
            items: vec!["stash@{0}".to_owned()],
            snapshot: false,
        };

        assert_eq!(
            serde_json::to_value(&discards).unwrap(),
            serde_json::json!({
                "effect": "discards_work",
                "work": "stash_entries",
                "count": 1,
                "items": ["stash@{0}"],
                "snapshot": false,
            })
        );
        assert_eq!(
            serde_json::to_value(ProgramEffectKey::RewritesPublished).unwrap(),
            serde_json::json!({ "effect": "rewrites_published" })
        );
    }
}
