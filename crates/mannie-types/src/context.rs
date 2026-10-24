//! Attached context (ADR-0046): the attachments of a conversation, which name
//! keys of context providers, and the items that the providers gather for an
//! owner's message, as the core keeps them in a part `context`. The UI words
//! the providers' codes through its slot of context kinds, so the core sends
//! labels, codes and numbers, never display text (ADR-0019).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::chat::ChatError;
use crate::ids::ConversationId;
use crate::mentions::{INVALID_KEY, MAX_PROVIDER_ID_BYTES};

/// The most attachments of one conversation.
pub const MAX_ATTACHMENTS: usize = 8;

/// The longest key of an attachment, in bytes.
pub const MAX_ATTACHMENT_KEY_BYTES: usize = 256;

/// The longest label of an attachment that the core keeps, in characters. A
/// provider's longer label is cut.
pub const MAX_ATTACHMENT_LABEL_CHARACTERS: usize = 200;

/// The most items of one provider's part.
pub const MAX_CONTEXT_ITEMS: usize = 20;

/// The longest key of an item, in bytes. The core leaves out an item with a
/// longer one.
pub const MAX_ITEM_KEY_BYTES: usize = 256;

/// The longest title of an item that the core keeps, in characters.
pub const MAX_ITEM_TITLE_CHARACTERS: usize = 300;

/// The most headings of an item's location that the core keeps.
pub const MAX_LOCATION_HEADINGS: usize = 8;

/// The longest heading of an item's location that the core keeps, in
/// characters.
pub const MAX_HEADING_CHARACTERS: usize = 200;

/// The failure of a part whose provider's plugin was off.
pub const PLUGIN_OFF: &str = "plugin_off";

/// The failure of a part whose provider took longer than the core waits.
pub const TIMED_OUT: &str = "timed_out";

/// An attachment of a conversation: a provider's key, with the label that the
/// provider gave it when the owner attached it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// The provider's ID.
    pub provider: String,
    /// The provider's key, such as a notebook's ID.
    pub key: String,
    /// What the owner reads, such as the notebook's name. It is content and
    /// is never translated.
    pub label: String,
}

/// An attachment as the webview names it: a provider and its key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentRef {
    /// The provider's ID.
    pub provider: String,
    /// The provider's key.
    pub key: String,
}

/// The argument of `attach_context` and `detach_context`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChangeAttachment {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// What to attach or detach.
    pub attachment: AttachmentRef,
}

/// Where an item stands in what it comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ContextLocation {
    /// A page, counted from 1.
    Page {
        /// The page's number.
        number: u32,
    },
    /// The headings that the item stands under, the outermost first. They
    /// are content and are never translated.
    Headings {
        /// The headings.
        path: Vec<String>,
    },
}

/// An attachment that a part `context` answers, as it was when the provider
/// was asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AttachedKey {
    /// The provider's key.
    pub key: String,
    /// The label that the attachment had.
    pub label: String,
}

/// An item of a part `context`, as the core keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContextItemView {
    /// The item's number among the items of its message, from 1, which the
    /// model cites as `[n]`.
    pub number: u32,
    /// The provider's key of the item, which its kind of context opens.
    pub key: String,
    /// What the item comes from, such as a document's name. It is content.
    pub title: String,
    /// Where the item stands in it.
    pub location: Option<ContextLocation>,
    /// The text that the model read.
    pub text: String,
}

/// What one provider gave for an owner's message: the content of a part
/// `context`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContextView {
    /// The provider's ID.
    pub provider: String,
    /// The conversation's attachments of the provider that it was asked
    /// about, in the order that the owner attached them.
    pub attachments: Vec<AttachedKey>,
    /// The items, in the order of their numbers. None when nothing applied
    /// or the provider failed.
    pub items: Vec<ContextItemView>,
    /// Why the provider gave nothing: its own code, [`PLUGIN_OFF`] or
    /// [`TIMED_OUT`]; `None` when it answered.
    pub failure: Option<String>,
}

/// Checks an attachment that the webview names: a provider's ID and a key
/// within their limits.
pub fn check_attachment(attachment: &AttachmentRef) -> Result<(), ChatError> {
    if attachment.provider.is_empty() || attachment.provider.len() > MAX_PROVIDER_ID_BYTES {
        return Err(ChatError::ContextUnavailable {
            provider: attachment.provider.clone(),
        });
    }
    if attachment.key.is_empty() || attachment.key.len() > MAX_ATTACHMENT_KEY_BYTES {
        return Err(ChatError::ContextRefused {
            provider: attachment.provider.clone(),
            reason: INVALID_KEY.to_owned(),
        });
    }
    Ok(())
}

/// Checks the attachments that a new conversation starts with: each within
/// its limits, and at most [`MAX_ATTACHMENTS`] different ones. Returns them
/// without repetitions, in their order.
pub fn check_attachments(attachments: Vec<AttachmentRef>) -> Result<Vec<AttachmentRef>, ChatError> {
    let mut distinct: Vec<AttachmentRef> =
        Vec::with_capacity(attachments.len().min(MAX_ATTACHMENTS));
    for attachment in attachments {
        check_attachment(&attachment)?;
        if distinct.contains(&attachment) {
            continue;
        }
        if distinct.len() == MAX_ATTACHMENTS {
            return Err(ChatError::TooManyAttachments {
                max: u32::try_from(MAX_ATTACHMENTS).unwrap_or(u32::MAX),
            });
        }
        distinct.push(attachment);
    }
    Ok(distinct)
}

/// A provider's text cut to `max_characters` characters, without control
/// characters other than line breaks and tabs, which become spaces.
pub fn clean_context_text(text: &str, max_characters: usize) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(max_characters)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        AttachmentRef, ContextItemView, ContextLocation, ContextView, MAX_ATTACHMENT_KEY_BYTES,
        MAX_ATTACHMENTS, check_attachment, check_attachments, clean_context_text,
    };
    use crate::chat::ChatError;

    fn notebook(key: &str) -> AttachmentRef {
        AttachmentRef {
            provider: "notebooks".to_owned(),
            key: key.to_owned(),
        }
    }

    #[test]
    fn an_attachment_has_a_provider_and_a_key_within_their_limits() {
        assert_eq!(check_attachment(&notebook("n1")), Ok(()));
        assert_eq!(
            check_attachment(&notebook(&"k".repeat(MAX_ATTACHMENT_KEY_BYTES))),
            Ok(())
        );
        for key in [String::new(), "k".repeat(MAX_ATTACHMENT_KEY_BYTES + 1)] {
            assert_eq!(
                check_attachment(&notebook(&key)),
                Err(ChatError::ContextRefused {
                    provider: "notebooks".to_owned(),
                    reason: "invalid_key".to_owned(),
                })
            );
        }
        let nobody = AttachmentRef {
            provider: String::new(),
            key: "n1".to_owned(),
        };
        assert_eq!(
            check_attachment(&nobody),
            Err(ChatError::ContextUnavailable {
                provider: String::new()
            })
        );
    }

    #[test]
    fn a_new_conversation_takes_at_most_eight_different_attachments() {
        let eight: Vec<AttachmentRef> = (0..MAX_ATTACHMENTS)
            .map(|index| notebook(&format!("n{index}")))
            .collect();
        let mut repeated = eight.clone();
        repeated.push(notebook("n0"));
        let mut too_many = eight.clone();
        too_many.push(notebook("n9"));

        assert_eq!(check_attachments(eight.clone()), Ok(eight.clone()));
        assert_eq!(check_attachments(repeated), Ok(eight));
        assert_eq!(
            check_attachments(too_many),
            Err(ChatError::TooManyAttachments { max: 8 })
        );
    }

    #[test]
    fn texts_lose_their_control_characters_and_are_cut() {
        assert_eq!(
            clean_context_text(" Physics\nnotes\u{7} ", 100),
            "Physics notes"
        );
        assert_eq!(clean_context_text("Grüße aus Köln", 5), "Grüße");
    }

    #[test]
    fn a_part_crosses_ipc_with_its_items_and_their_locations() {
        let part = ContextView {
            provider: "notebooks".to_owned(),
            attachments: vec![super::AttachedKey {
                key: "n1".to_owned(),
                label: "Physics".to_owned(),
            }],
            items: vec![
                ContextItemView {
                    number: 1,
                    key: "s1:4".to_owned(),
                    title: "Optics.pdf".to_owned(),
                    location: Some(ContextLocation::Page { number: 3 }),
                    text: "Light bends.".to_owned(),
                },
                ContextItemView {
                    number: 2,
                    key: "s2:0".to_owned(),
                    title: "Notes.md".to_owned(),
                    location: Some(ContextLocation::Headings {
                        path: vec!["Waves".to_owned()],
                    }),
                    text: "Waves carry energy.".to_owned(),
                },
            ],
            failure: None,
        };

        assert_eq!(
            serde_json::to_value(&part).unwrap(),
            serde_json::json!({
                "provider": "notebooks",
                "attachments": [{ "key": "n1", "label": "Physics" }],
                "items": [
                    {
                        "number": 1,
                        "key": "s1:4",
                        "title": "Optics.pdf",
                        "location": { "kind": "page", "number": 3 },
                        "text": "Light bends."
                    },
                    {
                        "number": 2,
                        "key": "s2:0",
                        "title": "Notes.md",
                        "location": { "kind": "headings", "path": ["Waves"] },
                        "text": "Waves carry energy."
                    }
                ],
                "failure": null
            })
        );
    }
}
