//! Mentions (ADR-0032; docs/features/chat/mentions.md): what the owner picks
//! after "@" in the message box. This module holds the references that a
//! message carries, the suggestions of the "@" list, the view of a sent
//! mention and the checks of a message's references. The kinds of mentions
//! come from the runtime's mention providers, and the UI words their codes
//! through its slot of mention kinds, so the core sends labels, codes and
//! numbers, never display text (ADR-0019).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::chat::ChatError;
use crate::ids::{ConversationId, ProjectId};

/// The most mentions that one message carries.
pub const MAX_MENTIONS: usize = 20;

/// The most entries that one provider suggests for a query.
pub const MAX_SUGGESTIONS: usize = 50;

/// The longest query that the "@" list filters by, in characters. A longer
/// query matches nothing.
pub const MAX_MENTION_QUERY_CHARACTERS: usize = 1_024;

/// The longest ID of a provider, in bytes.
pub const MAX_PROVIDER_ID_BYTES: usize = 64;

/// The longest key of a reference, in bytes. It leaves room for a path of
/// 32,767 characters, the longest that Windows takes.
pub const MAX_MENTION_KEY_BYTES: usize = 128 * 1024;

/// Where the owner writes the message that mentions something. A provider
/// decides from it whether it is offered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum MentionPlace {
    /// A conversation that exists.
    Conversation {
        /// The conversation.
        conversation_id: ConversationId,
    },
    /// A conversation that the message starts.
    NewConversation {
        /// The project it starts in, or `None` for a general conversation.
        project_id: Option<ProjectId>,
    },
}

/// The argument of `suggest_mentions`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SuggestMentions {
    /// Where the owner writes.
    pub place: MentionPlace,
    /// What the owner typed after "@", up to the caret.
    pub query: String,
}

/// The result of `suggest_mentions`: a group for each provider that is
/// offered in the place, in the order in which they were registered. No group
/// means that nothing can be mentioned there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MentionSuggestions {
    /// The groups.
    pub groups: Vec<MentionGroup>,
}

/// One provider's suggestions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MentionGroup {
    /// The provider's ID.
    pub provider: String,
    /// The entries that match, best first, at most [`MAX_SUGGESTIONS`].
    pub entries: Vec<MentionEntry>,
    /// Whether the provider holds more than it looked through, so that a
    /// longer query may find more.
    pub incomplete: bool,
    /// Why the provider listed nothing, as its code, or `None`.
    pub failure: Option<String>,
}

/// One entry of the "@" list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MentionEntry {
    /// What the entry names, as its provider knows it, such as a path.
    pub key: String,
    /// What the owner reads, such as the path or a title. It is content and
    /// is never translated.
    pub label: String,
    /// The text that choosing the entry puts into the message, with its "@".
    pub token: String,
    /// The provider's variant, which chooses the entry's icon, such as `file`
    /// or `folder`.
    pub variant: String,
    /// A warning about the entry as the provider's code, such as `secret`, or
    /// `None`.
    pub warning: Option<String>,
    /// Whether choosing the entry keeps the list open, to go deeper, as for
    /// a folder.
    pub expands: bool,
}

/// A mention that a message carries: which provider, and what it names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MentionRef {
    /// The provider's ID.
    pub provider: String,
    /// The key of the entry that the owner chose.
    pub key: String,
}

/// A sent mention as the owner sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MentionView {
    /// The provider's ID.
    pub provider: String,
    /// What the mention names, as its provider knows it.
    pub key: String,
    /// What the owner reads, such as the path.
    pub label: String,
    /// The provider's variant, which chooses the icon.
    pub variant: String,
    /// What the model got.
    pub summary: MentionSummary,
}

/// What the model got of a mention: the provider's code, such as `lines` or
/// `not_text`, with its numbers by name, which the UI words.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MentionSummary {
    /// The provider's code.
    pub code: String,
    /// The numbers that the code's words need, such as `total`.
    #[serde(default)]
    pub values: BTreeMap<String, u32>,
}

/// Checks the references of a message before any provider reads them: at
/// most [`MAX_MENTIONS`] different ones, with IDs and keys within their
/// limits. Returns them without repetitions, in the order of the message.
pub fn check_mentions(mentions: Vec<MentionRef>) -> Result<Vec<MentionRef>, ChatError> {
    let mut distinct: Vec<MentionRef> = Vec::with_capacity(mentions.len().min(MAX_MENTIONS));
    for mention in mentions {
        if mention.provider.is_empty() || mention.provider.len() > MAX_PROVIDER_ID_BYTES {
            return Err(ChatError::MentionUnavailable {
                provider: cut_id(&mention.provider),
            });
        }
        if mention.key.is_empty() || mention.key.len() > MAX_MENTION_KEY_BYTES {
            return Err(ChatError::MentionRefused {
                provider: mention.provider,
                reason: INVALID_KEY.to_owned(),
            });
        }
        if distinct.contains(&mention) {
            continue;
        }
        if distinct.len() == MAX_MENTIONS {
            return Err(ChatError::TooManyMentions {
                max: u32::try_from(MAX_MENTIONS).unwrap_or(u32::MAX),
            });
        }
        distinct.push(mention);
    }
    Ok(distinct)
}

/// The code of a refusal for a key that no provider could hold: empty or too
/// long.
pub const INVALID_KEY: &str = "invalid_key";

/// A provider's ID as an error names it: its first bytes, cut at a character.
fn cut_id(id: &str) -> String {
    let mut end = id.len().min(MAX_PROVIDER_ID_BYTES);
    while !id.is_char_boundary(end) {
        end -= 1;
    }
    id[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        INVALID_KEY, MAX_MENTION_KEY_BYTES, MAX_MENTIONS, MentionPlace, MentionRef, MentionSummary,
        check_mentions,
    };
    use crate::chat::ChatError;
    use crate::ids::ProjectId;

    fn file(key: &str) -> MentionRef {
        MentionRef {
            provider: "files".to_owned(),
            key: key.to_owned(),
        }
    }

    #[test]
    fn references_lose_their_repetitions_and_keep_their_order() {
        let checked = check_mentions(vec![file("b.rs"), file("a.rs"), file("b.rs")]).unwrap();

        assert_eq!(checked, [file("b.rs"), file("a.rs")]);
    }

    #[test]
    fn a_message_carries_at_most_twenty_different_mentions() {
        let twenty: Vec<MentionRef> = (0..MAX_MENTIONS)
            .map(|n| file(&format!("{n}.rs")))
            .collect();
        let mut repeated = twenty.clone();
        repeated.push(file("0.rs"));
        let mut too_many = twenty.clone();
        too_many.push(file("extra.rs"));

        assert_eq!(check_mentions(twenty).unwrap().len(), MAX_MENTIONS);
        assert_eq!(check_mentions(repeated).unwrap().len(), MAX_MENTIONS);
        assert_eq!(
            check_mentions(too_many),
            Err(ChatError::TooManyMentions { max: 20 })
        );
    }

    #[test]
    fn empty_or_oversized_ids_and_keys_are_refused() {
        let no_provider = MentionRef {
            provider: String::new(),
            key: "a.rs".to_owned(),
        };
        let long_provider = MentionRef {
            provider: "é".repeat(40),
            key: "a.rs".to_owned(),
        };

        assert_eq!(
            check_mentions(vec![no_provider]),
            Err(ChatError::MentionUnavailable {
                provider: String::new()
            })
        );
        let Err(ChatError::MentionUnavailable { provider }) = check_mentions(vec![long_provider])
        else {
            panic!("a long provider ID must be refused");
        };
        assert_eq!(provider, "é".repeat(32));
        for key in [String::new(), "a".repeat(MAX_MENTION_KEY_BYTES + 1)] {
            assert_eq!(
                check_mentions(vec![file(&key)]),
                Err(ChatError::MentionRefused {
                    provider: "files".to_owned(),
                    reason: INVALID_KEY.to_owned()
                })
            );
        }
    }

    #[test]
    fn places_and_summaries_serialize_for_the_ui() {
        let project = ProjectId::parse("0192f5e0-0000-7000-8000-000000000001").unwrap();
        let place = MentionPlace::NewConversation {
            project_id: Some(project),
        };
        let summary = MentionSummary {
            code: "lines".to_owned(),
            values: [("total".to_owned(), 42)].into_iter().collect(),
        };

        assert_eq!(
            serde_json::to_value(&place).unwrap(),
            serde_json::json!({
                "kind": "new_conversation",
                "projectId": "0192f5e0-0000-7000-8000-000000000001"
            })
        );
        assert_eq!(
            serde_json::to_value(&summary).unwrap(),
            serde_json::json!({ "code": "lines", "values": { "total": 42 } })
        );
        assert_eq!(
            serde_json::from_value::<MentionSummary>(serde_json::json!({ "code": "unreadable" }))
                .unwrap(),
            MentionSummary {
                code: "unreadable".to_owned(),
                values: std::collections::BTreeMap::new()
            }
        );
    }
}
