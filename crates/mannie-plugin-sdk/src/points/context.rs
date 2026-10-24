//! Context providers (ADR-0046, decision 2): a plugin's provider gives the
//! context of the conversations that have its keys attached, such as the
//! passages of the owner's documents that apply to a message.
//!
//! A conversation holds attachments, each a provider's ID, one of its keys
//! and the label that the provider gave the key (decision 1). The core asks
//! [`ContextProvider::describe`] for the label of each key that the webview
//! sends, and keeps the attachment only when the provider knows the key.
//! Before the first model call of a reply to an owner's message that asks
//! for it, the core asks [`ContextProvider::gather`] with the message, the
//! owner's earlier messages and the keys, and keeps what it returns with the
//! message, where the model reads it (ADR-0049, decisions 1 and 2). Before a
//! run's first call it asks [`ContextProvider::outline`] what the keys hold,
//! which the model's instructions show (decision 3).
//!
//! The model cites an item by copying its cite link, [`cite_link`], which a
//! provider's tools write the same way (decision 4).
//!
//! The core keeps at most the request's number of items, leaves out an item
//! whose key is longer than [`MAX_ITEM_KEY_BYTES`], and cuts the texts that
//! pass the request's room, so a provider that returns more loses its last
//! items. It gives up on a provider after [`GATHER_TIMEOUT`].
//!
//! A provider's ID follows the rule `Owned`, which
//! [`owned_by`](crate::ids::owned_by) checks: the plugin's ID, or the
//! plugin's ID, a dot and more (ADR-0034, decision 6).
//!
//! [`MAX_ITEM_KEY_BYTES`]: crate::contracts::MAX_ITEM_KEY_BYTES

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::BoxFuture;
use crate::contracts::{ContextLocation, ConversationId};
use crate::contributions::{ExtensionPoint, IdRule};

/// The point of plugins' context providers, whose entries are named by their
/// IDs.
#[derive(Debug)]
pub enum ContextProviders {}

impl ExtensionPoint for ContextProviders {
    const NAME: &'static str = "context-providers";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Owned;
    type Entry = PluginContextProvider;

    fn id(entry: &PluginContextProvider) -> &str {
        &entry.id
    }
}

/// How long the core waits for one provider's items before it keeps the
/// failure `timed_out` instead.
pub const GATHER_TIMEOUT: Duration = Duration::from_secs(120);

/// How long the core waits for one provider's outline before the model's
/// instructions say that it could not be read.
pub const OUTLINE_TIMEOUT: Duration = Duration::from_secs(10);

/// The most earlier messages of the owner that a request carries.
pub const MAX_EARLIER_MESSAGES: usize = 2;

/// The most characters of one earlier message that a request carries.
pub const MAX_EARLIER_MESSAGE_CHARACTERS: usize = 4_000;

/// The most characters of a cite link's label.
const MAX_CITE_LABEL_CHARACTERS: usize = 120;

/// The scheme of cite links, whose addresses name a provider and its key.
pub const CITE_SCHEME: &str = "cite:";

/// The Markdown link that cites an item: `[label](cite:<provider>/<key>)`,
/// which the model copies into its answer and the transcript renders as a
/// citation (ADR-0049, decision 4). The label loses the characters that
/// would end it, and is cut to 120 characters.
pub fn cite_link(provider: &str, key: &str, label: &str) -> String {
    let clean: String = label
        .chars()
        .filter(|character| !matches!(character, '[' | ']' | '\\') && !character.is_control())
        .take(MAX_CITE_LABEL_CHARACTERS)
        .collect();
    format!("[{}]({CITE_SCHEME}{provider}/{key})", clean.trim())
}

/// A kind of context (ADR-0046, decision 2).
pub trait ContextProvider: Send + Sync + fmt::Debug + 'static {
    /// The label of the attachment that `key` names, such as a notebook's
    /// name, or the provider's code, such as `notebook_not_found`, for a key
    /// that it does not hold. The key comes from the webview, so the provider
    /// checks it as untrusted input (ADR-0004).
    fn describe(&self, key: String) -> BoxFuture<'static, Result<String, String>>;

    /// What applies to the owner's message from the request's attachments,
    /// best first, or the provider's code for why it gave nothing, such as a
    /// server that cannot be reached. No items is an answer: nothing in the
    /// attachments applies.
    fn gather(
        &self,
        request: ContextRequest,
    ) -> BoxFuture<'static, Result<Vec<ContextItem>, String>>;

    /// What the attachments `keys` hold, as text for the model, such as each
    /// notebook with its documents and their IDs, which its tools take, or
    /// the provider's code for why it cannot say. An empty text says nothing.
    /// A provider that outlines nothing keeps this default.
    fn outline(&self, keys: Vec<String>) -> BoxFuture<'static, Result<String, String>> {
        drop(keys);
        Box::pin(std::future::ready(Ok(String::new())))
    }
}

/// A plugin's context provider, under its ID.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PluginContextProvider {
    /// The plugin's ID, or the plugin's ID, a dot and more.
    pub id: String,
    /// The provider.
    pub provider: Arc<dyn ContextProvider>,
}

impl PluginContextProvider {
    /// A provider under `id`.
    pub fn new(id: impl Into<String>, provider: Arc<dyn ContextProvider>) -> Self {
        Self {
            id: id.into(),
            provider,
        }
    }
}

/// What the core asks a provider for: the items of a conversation's
/// attachments that apply to the owner's message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ContextRequest {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The keys of the conversation's attachments of the provider, in the
    /// order the owner attached them.
    pub keys: Vec<String>,
    /// The owner's message, as the owner wrote it.
    pub message: String,
    /// The owner's messages of the turns before, newest first, at most
    /// [`MAX_EARLIER_MESSAGES`], each cut to
    /// [`MAX_EARLIER_MESSAGE_CHARACTERS`], so that a follow-up keeps its
    /// subject (ADR-0049, decision 2). A request of the first version reads
    /// as none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub earlier_messages: Vec<String>,
    /// The most bytes of the items' texts together that the core keeps.
    pub max_bytes: u32,
    /// The most items that the core keeps.
    pub max_items: u32,
}

impl ContextRequest {
    /// A request for the items of `keys` that apply to `message`, within
    /// `max_bytes` of text and `max_items` items, without earlier messages.
    pub fn new(
        conversation_id: ConversationId,
        keys: Vec<String>,
        message: String,
        max_bytes: u32,
        max_items: u32,
    ) -> Self {
        Self {
            conversation_id,
            keys,
            message,
            earlier_messages: Vec::new(),
            max_bytes,
            max_items,
        }
    }

    /// The same request with the owner's earlier messages, newest first.
    #[must_use]
    pub fn with_earlier_messages(mut self, earlier_messages: Vec<String>) -> Self {
        self.earlier_messages = earlier_messages;
        self
    }
}

/// An item that a provider gives, such as a passage of a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ContextItem {
    /// The provider's key of the item, which the provider's kind of context
    /// opens in the UI.
    pub key: String,
    /// What the item comes from, such as a document's name.
    pub title: String,
    /// Where the item stands in it.
    pub location: Option<ContextLocation>,
    /// The text that the model reads.
    pub text: String,
}

impl ContextItem {
    /// An item with these parts. The texts are easy to swap, so callers pass
    /// named locals.
    pub fn new(
        key: String,
        title: String,
        location: Option<ContextLocation>,
        text: String,
    ) -> Self {
        Self {
            key,
            title,
            location,
            text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ContextItem, ContextRequest, cite_link};
    use crate::contracts::{ContextLocation, ConversationId};

    #[test]
    fn a_cite_link_names_the_provider_and_key_with_a_clean_label() {
        assert_eq!(
            cite_link("notebooks", "s1:4", "Guide.pdf p. 12"),
            "[Guide.pdf p. 12](cite:notebooks/s1:4)"
        );
        assert_eq!(
            cite_link("notebooks", "s1:4", " [Guide]\\\n "),
            "[Guide](cite:notebooks/s1:4)"
        );
        let long = "x".repeat(300);
        assert_eq!(
            cite_link("notebooks", "s1:4", &long).len(),
            "[](cite:notebooks/s1:4)".len() + 120
        );
    }

    #[test]
    fn a_request_carries_the_earlier_messages_and_reads_without_them() {
        let conversation = ConversationId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap();
        let request =
            ContextRequest::new(conversation, Vec::new(), "And the second?".to_owned(), 1, 1)
                .with_earlier_messages(vec!["How do lenses work?".to_owned()]);

        let json = serde_json::to_value(&request).unwrap();
        let mut first_version = json.clone();
        first_version
            .as_object_mut()
            .unwrap()
            .remove("earlierMessages");

        assert_eq!(json["earlierMessages"][0], "How do lenses work?");
        let read: ContextRequest = serde_json::from_value(first_version).unwrap();
        assert_eq!(read.earlier_messages, Vec::<String>::new());
    }

    #[test]
    fn the_constructors_keep_each_argument_in_its_field() {
        let conversation = ConversationId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap();
        let key = "s1:4".to_owned();
        let title = "Optics.pdf".to_owned();
        let text = "Light bends.".to_owned();

        let request = ContextRequest::new(
            conversation.clone(),
            vec!["n1".to_owned()],
            "Why does light bend?".to_owned(),
            4_096,
            6,
        );
        let item = ContextItem::new(key, title, Some(ContextLocation::Page { number: 3 }), text);

        assert_eq!(request.conversation_id, conversation);
        assert_eq!(request.keys, ["n1"]);
        assert_eq!(request.message, "Why does light bend?");
        assert_eq!((request.max_bytes, request.max_items), (4_096, 6));
        assert_eq!(item.key, "s1:4");
        assert_eq!(item.title, "Optics.pdf");
        assert_eq!(item.location, Some(ContextLocation::Page { number: 3 }));
        assert_eq!(item.text, "Light bends.");
    }
}
