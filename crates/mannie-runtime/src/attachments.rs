//! Attached context (ADR-0046): the attachments of conversations, which name
//! keys of the context providers that plugins contribute through the SDK's
//! `points::context`, and the gathering of what the providers give for the
//! owner's message, which the run that answers it stores as parts `context`
//! of the message before its first model call (decision 3). A message
//! gathers from the conversation's home and from the attachments that it
//! names with "@", with the owner's earlier messages, and the model reads the
//! items with their cite links (ADR-0049, decisions 1, 2 and 4). The outlines
//! of the attachments go to the instructions (decision 3).
//!
//! The core checks what a provider returns: it keeps at most the request's
//! items, leaves out an item without a usable key or text, cleans titles and
//! locations, and cuts the texts that pass the room of one tool result, which
//! the providers of one message share. Logs name providers, counts and codes,
//! never the text of an item.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint};
use mannie_plugin_sdk::host::{ContributionRefusal, ID_TAKEN};
use mannie_plugin_sdk::points::context::{
    ContextItem, ContextProvider, ContextProviders, ContextRequest, MAX_EARLIER_MESSAGE_CHARACTERS,
    MAX_EARLIER_MESSAGES, OUTLINE_TIMEOUT, cite_link,
};
use mannie_store::{Change, Durability, NewContext, NewMention};
use mannie_types::chat::ChatError;
use mannie_types::context::{
    AttachedKey, Attachment, AttachmentRef, ChangeAttachment, ContextItemView, ContextLocation,
    ContextView, MAX_ATTACHMENT_LABEL_CHARACTERS, MAX_CONTEXT_ITEMS, MAX_HEADING_CHARACTERS,
    MAX_ITEM_KEY_BYTES, MAX_ITEM_TITLE_CHARACTERS, MAX_LOCATION_HEADINGS, PLUGIN_OFF, TIMED_OUT,
    check_attachment, check_attachments, clean_context_text,
};
use mannie_types::ids::ConversationId;
use mannie_types::mentions::{ATTACHED, ATTACHED_VARIANT, MentionRef, MentionSummary, MentionView};

use crate::conversations::{refuse_task_conversation, store_failure};
use crate::plugins::Snapshot;
use crate::{Runtime, texts};

/// How long the core waits for a provider's label of a key.
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(15);

/// The longest failure code of a provider that the core keeps. A provider's
/// code that is longer, or holds other characters than lower-case letters,
/// digits, `_` and `.`, becomes [`PROVIDER_FAILED`].
const MAX_CODE_BYTES: usize = 64;

/// The failure of a provider whose own code the core does not keep.
const PROVIDER_FAILED: &str = "provider_failed";

/// Refuses a plugin's context providers whose IDs another plugin's providers
/// have. The coordinator of the SDK made sure that each ID is the plugin's
/// (the rule `Owned`), and the core has no providers of its own.
pub(crate) fn check_context_providers(
    snapshot: &Snapshot,
    contributions: &Contributions,
) -> Result<(), ContributionRefusal> {
    for provider in contributions.entries::<ContextProviders>() {
        let id = provider.id.as_str();
        if snapshot.find::<ContextProviders>(id).is_some() {
            return Err(ContributionRefusal::new(
                ContextProviders::NAME,
                ID_TAKEN,
                format!("a context provider with the ID {id} is registered already"),
            ));
        }
    }
    Ok(())
}

/// A provider's code as the core keeps it.
fn clean_code(code: &str) -> String {
    let is_code = !code.is_empty()
        && code.len() <= MAX_CODE_BYTES
        && code.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.".contains(&byte)
        });
    if is_code {
        code.to_owned()
    } else {
        PROVIDER_FAILED.to_owned()
    }
}

/// The attachments of one provider, in the order of their first one.
fn by_provider(attachments: &[Attachment]) -> Vec<(String, Vec<AttachedKey>)> {
    let mut groups: Vec<(String, Vec<AttachedKey>)> = Vec::new();
    for attachment in attachments {
        let key = AttachedKey {
            key: attachment.key.clone(),
            label: attachment.label.clone(),
        };
        match groups
            .iter_mut()
            .find(|(provider, _keys)| *provider == attachment.provider)
        {
            Some((_provider, keys)) => keys.push(key),
            None => groups.push((attachment.provider.clone(), vec![key])),
        }
    }
    groups
}

/// The providers of attachments, in the order of their first attachment.
pub(crate) fn providers_of(attachments: &[Attachment]) -> Vec<String> {
    by_provider(attachments)
        .into_iter()
        .map(|(provider, _keys)| provider)
        .collect()
}

/// An item's location as the core keeps it: a page from 1, or at most
/// [`MAX_LOCATION_HEADINGS`] headings that are not blank, each cut to
/// [`MAX_HEADING_CHARACTERS`].
fn clean_location(location: Option<ContextLocation>) -> Option<ContextLocation> {
    match location? {
        ContextLocation::Page { number } => {
            (number > 0).then_some(ContextLocation::Page { number })
        }
        ContextLocation::Headings { path } => {
            let path: Vec<String> = path
                .iter()
                .map(|heading| clean_context_text(heading, MAX_HEADING_CHARACTERS))
                .filter(|heading| !heading.is_empty())
                .take(MAX_LOCATION_HEADINGS)
                .collect();
            (!path.is_empty()).then_some(ContextLocation::Headings { path })
        }
    }
}

/// An item's text without control characters other than line breaks and
/// tabs, and without white space at its ends.
fn clean_item_text(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The first `max_bytes` bytes of a text, cut back to a character's start.
fn cut_to(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

/// The items of a provider that the core keeps, numbered on from `numbers`:
/// at most [`MAX_CONTEXT_ITEMS`], whose texts together stay within
/// `max_bytes`.
fn kept_items(
    items: Vec<ContextItem>,
    max_bytes: usize,
    numbers: &mut u32,
) -> Vec<ContextItemView> {
    let mut room = max_bytes;
    let mut kept = Vec::new();
    for item in items {
        if kept.len() == MAX_CONTEXT_ITEMS || room == 0 {
            break;
        }
        if item.key.is_empty() || item.key.len() > MAX_ITEM_KEY_BYTES {
            tracing::warn!(
                "a context provider gave an item without a usable key, which is left out"
            );
            continue;
        }
        let text = clean_item_text(&item.text);
        if text.is_empty() {
            continue;
        }
        let text = cut_to(text, room);
        room -= text.len();
        let title = clean_context_text(&item.title, MAX_ITEM_TITLE_CHARACTERS);
        // A title is the provider's; one that is blank shows the key.
        let title = if title.is_empty() {
            clean_context_text(&item.key, MAX_ITEM_TITLE_CHARACTERS)
        } else {
            title
        };
        *numbers += 1;
        kept.push(ContextItemView {
            number: *numbers,
            key: item.key,
            title,
            location: clean_location(item.location),
            text,
        });
    }
    kept
}

/// The labels of a part's attachments.
fn labels(context: &ContextView) -> Vec<&str> {
    context
        .attachments
        .iter()
        .map(|attachment| attachment.label.as_str())
        .collect()
}

/// Where an item stands, as the model reads it.
fn place(location: Option<&ContextLocation>) -> Option<String> {
    match location? {
        ContextLocation::Page { number } => Some(texts::context_page(*number)),
        ContextLocation::Headings { path } => Some(texts::context_headings(path)),
    }
}

/// What the model reads of a part in full: its items, or the line that says
/// why it holds none. The heading of a message's excerpts comes before its
/// first part with items.
pub(crate) fn context_text(context: &ContextView) -> String {
    let labels = labels(context);
    match context.failure.as_deref() {
        Some(PLUGIN_OFF) => return texts::context_plugin_off(&labels),
        Some(TIMED_OUT) => return texts::context_timed_out(&labels),
        Some(code) => return texts::context_failed(&labels, code),
        None => {}
    }
    if context.items.is_empty() {
        return texts::context_nothing(&labels);
    }
    context
        .items
        .iter()
        .map(|item| {
            let place = place(item.location.as_ref());
            let cite = cite_link(&context.provider, &item.key, &item_cite_label(item));
            texts::context_item(
                item.number,
                &item.title,
                place.as_deref(),
                &cite,
                &item.text,
            )
        })
        .collect::<Vec<String>>()
        .join("\n\n")
}

/// The label of an item's cite link: its title, with its page or its last
/// heading.
fn item_cite_label(item: &ContextItemView) -> String {
    match &item.location {
        Some(ContextLocation::Page { number }) => {
            texts::cite_label(&item.title, Some(*number), &[])
        }
        Some(ContextLocation::Headings { path }) => texts::cite_label(&item.title, None, path),
        None => texts::cite_label(&item.title, None, &[]),
    }
}

/// The owner's earlier messages as a request carries them: at most
/// [`MAX_EARLIER_MESSAGES`], newest first, each cut to
/// [`MAX_EARLIER_MESSAGE_CHARACTERS`] (ADR-0049, decision 2).
pub(crate) fn earlier_for_request(earlier: Vec<String>) -> Vec<String> {
    earlier
        .into_iter()
        .take(MAX_EARLIER_MESSAGES)
        .map(|message| {
            message
                .chars()
                .take(MAX_EARLIER_MESSAGE_CHARACTERS)
                .collect()
        })
        .collect()
}

/// What the model reads of the part of an older message: a line for its
/// items, which went with that message, or the line of a part without items.
pub(crate) fn context_line(context: &ContextView) -> String {
    let numbers = context.items.first().zip(context.items.last());
    match numbers {
        Some((first, last)) => {
            texts::context_not_repeated(&labels(context), first.number, last.number)
        }
        None => context_text(context),
    }
}

impl Runtime {
    /// The context provider with this ID, while its plugin is on.
    fn context_provider(&self, id: &str) -> Option<Arc<dyn ContextProvider>> {
        self.contributions()
            .find::<ContextProviders>(id)
            .map(|registered| Arc::clone(&registered.entry.provider))
    }

    /// An attachment with the label that its provider gives its key, which
    /// the provider checks: the key came from the webview (ADR-0004).
    async fn describe_attachment(
        &self,
        attachment: &AttachmentRef,
    ) -> Result<Attachment, ChatError> {
        check_attachment(attachment)?;
        let provider = attachment.provider.clone();
        let source =
            self.context_provider(&provider)
                .ok_or_else(|| ChatError::ContextUnavailable {
                    provider: provider.clone(),
                })?;
        let described =
            tokio::time::timeout(DESCRIBE_TIMEOUT, source.describe(attachment.key.clone())).await;
        let label = match described {
            Ok(Ok(label)) => label,
            Ok(Err(code)) => {
                let reason = clean_code(&code);
                tracing::info!(provider, reason, "a context provider refused a key");
                return Err(ChatError::ContextRefused { provider, reason });
            }
            Err(_elapsed) => {
                tracing::warn!(
                    provider,
                    "a context provider did not describe a key in time"
                );
                return Err(ChatError::ContextRefused {
                    provider,
                    reason: TIMED_OUT.to_owned(),
                });
            }
        };
        let label = clean_context_text(&label, MAX_ATTACHMENT_LABEL_CHARACTERS);
        Ok(Attachment {
            provider,
            key: attachment.key.clone(),
            // A label is the provider's; one that is blank shows the key.
            label: if label.is_empty() {
                attachment.key.clone()
            } else {
                label
            },
        })
    }

    /// The attachments that a new conversation starts with, each with the
    /// label that its provider gives it.
    pub(crate) async fn describe_attachments(
        &self,
        attachments: Vec<AttachmentRef>,
    ) -> Result<Vec<Attachment>, ChatError> {
        let attachments = check_attachments(attachments)?;
        let mut described = Vec::with_capacity(attachments.len());
        for attachment in &attachments {
            described.push(self.describe_attachment(attachment).await?);
        }
        Ok(described)
    }

    /// Attaches a provider's key to a conversation, after the provider
    /// described it (ADR-0046, decision 1). Its next replies gather what the
    /// key gives. Attaching a key again takes the provider's new label.
    pub async fn attach_context(&self, input: ChangeAttachment) -> Result<(), ChatError> {
        let summary = self
            .inner
            .store
            .conversation_summary(&input.conversation_id)
            .await
            .map_err(store_failure)?
            .ok_or(ChatError::ConversationNotFound)?;
        refuse_task_conversation(&summary)?;
        let attachment = self.describe_attachment(&input.attachment).await?;
        let provider = attachment.provider.clone();
        self.inner
            .hubs
            .apply(
                &self.inner.store,
                &input.conversation_id,
                vec![Change::Attach { attachment }],
                Durability::Full,
            )
            .await
            .map_err(store_failure)?;
        tracing::info!(conversation = %input.conversation_id, provider, "the owner attached context to a conversation");
        Ok(())
    }

    /// Detaches a key from a conversation. Its provider need not be on, so
    /// the attachments of a plugin that is off can go too. The conversation's
    /// home stays (ADR-0051).
    pub async fn detach_context(&self, input: ChangeAttachment) -> Result<(), ChatError> {
        check_attachment(&input.attachment)?;
        let summary = self
            .inner
            .store
            .conversation_summary(&input.conversation_id)
            .await
            .map_err(store_failure)?
            .ok_or(ChatError::ConversationNotFound)?;
        refuse_task_conversation(&summary)?;
        if summary.home.as_ref() == Some(&input.attachment) {
            return Err(ChatError::HomeAttachment);
        }
        let provider = input.attachment.provider.clone();
        self.inner
            .hubs
            .apply(
                &self.inner.store,
                &input.conversation_id,
                vec![Change::Detach {
                    attachment: input.attachment,
                }],
                Durability::Full,
            )
            .await
            .map_err(store_failure)?;
        tracing::info!(conversation = %input.conversation_id, provider, "the owner detached context from a conversation");
        Ok(())
    }

    /// What the providers of a conversation's attachments give for the
    /// owner's message, one part for each provider in the order of the
    /// attachments, with the items numbered across them. The providers work
    /// at once and share `room` bytes of text. A provider whose plugin is
    /// off, that fails or that takes longer than the configured timeout,
    /// [`GATHER_TIMEOUT`](mannie_plugin_sdk::points::context::GATHER_TIMEOUT)
    /// outside tests, gets a part with its failure.
    pub(crate) async fn gather_context(
        &self,
        conversation: &ConversationId,
        attachments: &[Attachment],
        message: &str,
        earlier: &[String],
        room: usize,
    ) -> Vec<NewContext> {
        let groups = by_provider(attachments);
        let share = room / groups.len().max(1);
        let max_bytes = u32::try_from(share).unwrap_or(u32::MAX);
        let max_items = u32::try_from(MAX_CONTEXT_ITEMS).unwrap_or(u32::MAX);
        let snapshot = self.contributions();
        let timeout = self.inner.config.gather_timeout;
        let gatherings = groups.iter().map(|(provider_id, keys)| {
            let provider = snapshot
                .find::<ContextProviders>(provider_id)
                .map(|registered| Arc::clone(&registered.entry.provider));
            let request = ContextRequest::new(
                conversation.clone(),
                keys.iter().map(|key| key.key.clone()).collect(),
                message.to_owned(),
                max_bytes,
                max_items,
            )
            .with_earlier_messages(earlier.to_vec());
            async move {
                let Some(provider) = provider else {
                    return Err(PLUGIN_OFF.to_owned());
                };
                match tokio::time::timeout(timeout, provider.gather(request)).await {
                    Ok(gathered) => gathered.map_err(|code| clean_code(&code)),
                    Err(_elapsed) => Err(TIMED_OUT.to_owned()),
                }
            }
        });
        let outcomes = join_all(gatherings).await;
        let mut numbers = 0_u32;
        groups
            .into_iter()
            .zip(outcomes)
            .map(|((provider, attachments), outcome)| {
                let (items, failure) = match outcome {
                    Ok(items) => (kept_items(items, share, &mut numbers), None),
                    Err(code) => (Vec::new(), Some(code)),
                };
                tracing::info!(
                    conversation = %conversation,
                    provider,
                    attachments = attachments.len(),
                    items = items.len(),
                    failure = failure.as_deref(),
                    "a context provider gathered for the owner's message"
                );
                let view = ContextView {
                    provider,
                    attachments,
                    items,
                    failure,
                };
                NewContext {
                    part_id: self.inner.ids.part(),
                    text: context_text(&view),
                    view,
                }
            })
            .collect()
    }
}

impl Runtime {
    /// Splits a message's mentions into those that attach what they name,
    /// whose providers are also context providers (ADR-0050), and those that
    /// a mention provider resolves.
    pub(crate) fn split_attaching(
        &self,
        mentions: Vec<MentionRef>,
    ) -> (Vec<MentionRef>, Vec<MentionRef>) {
        let snapshot = self.contributions();
        mentions.into_iter().partition(|mention| {
            snapshot
                .find::<ContextProviders>(&mention.provider)
                .is_some()
        })
    }

    /// The attachments that a message's attaching mentions name, each with
    /// the label that its provider gives it, and the mentions that the
    /// message stores for them. A key that its provider refuses refuses the
    /// message, which stays in the message box.
    pub(crate) async fn attach_mentioned(
        &self,
        mentions: &[MentionRef],
    ) -> Result<(Vec<Attachment>, Vec<NewMention>), ChatError> {
        if mentions.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let refs = mentions
            .iter()
            .map(|mention| AttachmentRef {
                provider: mention.provider.clone(),
                key: mention.key.clone(),
            })
            .collect();
        let attachments = self.describe_attachments(refs).await?;
        let stored = attachments
            .iter()
            .map(|attachment| NewMention {
                part_id: self.inner.ids.part(),
                text: texts::mention_attached(&attachment.label),
                view: MentionView {
                    provider: attachment.provider.clone(),
                    key: attachment.key.clone(),
                    label: attachment.label.clone(),
                    variant: ATTACHED_VARIANT.to_owned(),
                    summary: MentionSummary {
                        code: ATTACHED.to_owned(),
                        values: BTreeMap::new(),
                    },
                },
            })
            .collect();
        Ok((attachments, stored))
    }

    /// What a conversation's attachments hold, as their providers outline it
    /// for the model's instructions (ADR-0049, decision 3): each provider's
    /// outline, asked of all at once, or a line that says why one is
    /// missing. A provider that outlines nothing adds nothing.
    pub(crate) async fn outline_attachments(&self, attachments: &[Attachment]) -> String {
        let groups = by_provider(attachments);
        let snapshot = self.contributions();
        let outlines = groups.iter().map(|(provider_id, keys)| {
            let provider = snapshot
                .find::<ContextProviders>(provider_id)
                .map(|registered| Arc::clone(&registered.entry.provider));
            let keys: Vec<String> = keys.iter().map(|key| key.key.clone()).collect();
            async move {
                let Some(provider) = provider else {
                    return Err(PLUGIN_OFF.to_owned());
                };
                match tokio::time::timeout(OUTLINE_TIMEOUT, provider.outline(keys)).await {
                    Ok(outlined) => outlined.map_err(|code| clean_code(&code)),
                    Err(_elapsed) => Err(TIMED_OUT.to_owned()),
                }
            }
        });
        let outcomes = join_all(outlines).await;
        groups
            .iter()
            .zip(outcomes)
            .filter_map(|((provider, keys), outcome)| match outcome {
                Ok(text) => {
                    let text = text.trim();
                    (!text.is_empty()).then(|| text.to_owned())
                }
                Err(code) => {
                    tracing::info!(
                        provider,
                        code,
                        "a context provider could not outline its attachments"
                    );
                    let labels: Vec<&str> = keys.iter().map(|key| key.label.as_str()).collect();
                    Some(texts::outline_missing(&labels, &code))
                }
            })
            .collect::<Vec<String>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::points::context::ContextItem;
    use mannie_types::context::{
        AttachedKey, Attachment, ContextItemView, ContextLocation, ContextView, MAX_CONTEXT_ITEMS,
        PLUGIN_OFF, TIMED_OUT,
    };

    use super::{by_provider, clean_code, context_line, context_text, kept_items};

    fn item(key: &str, title: &str, location: Option<ContextLocation>, text: &str) -> ContextItem {
        ContextItem::new(key.to_owned(), title.to_owned(), location, text.to_owned())
    }

    fn view(items: Vec<ContextItemView>, failure: Option<&str>) -> ContextView {
        ContextView {
            provider: "notebooks".to_owned(),
            attachments: vec![
                AttachedKey {
                    key: "n1".to_owned(),
                    label: "Physics".to_owned(),
                },
                AttachedKey {
                    key: "n2".to_owned(),
                    label: "Optics".to_owned(),
                },
            ],
            items,
            failure: failure.map(str::to_owned),
        }
    }

    #[test]
    fn attachments_group_by_provider_in_the_order_of_their_first() {
        let attachment = |provider: &str, key: &str| Attachment {
            provider: provider.to_owned(),
            key: key.to_owned(),
            label: key.to_uppercase(),
        };

        let groups = by_provider(&[
            attachment("notebooks", "n1"),
            attachment("wiki", "w1"),
            attachment("notebooks", "n2"),
        ]);

        let shape: Vec<(&str, Vec<&str>)> = groups
            .iter()
            .map(|(provider, keys)| {
                (
                    provider.as_str(),
                    keys.iter().map(|key| key.key.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [("notebooks", vec!["n1", "n2"]), ("wiki", vec!["w1"])]
        );
    }

    #[test]
    fn items_are_numbered_on_and_kept_within_the_room_and_the_limits() {
        let mut numbers = 2;
        let many: Vec<ContextItem> = (0..30)
            .map(|index| item(&format!("s1:{index}"), "Doc", None, "abc"))
            .collect();

        let kept = kept_items(
            vec![
                item(
                    "s1:0",
                    " Optics.pdf ",
                    Some(ContextLocation::Page { number: 3 }),
                    "Light\u{7} bends.\n",
                ),
                item("", "No key", None, "Left out."),
                item("s1:1", "Blank text", None, " \n "),
                item(
                    "s1:2",
                    "",
                    Some(ContextLocation::Headings {
                        path: vec![" ".to_owned(), "Waves".to_owned()],
                    }),
                    "Grüße aus Köln",
                ),
            ],
            "Light bends.".len() + "Grü".len(),
            &mut numbers,
        );
        let mut counted = 0;
        let limited = kept_items(many, 10_000, &mut counted);

        assert_eq!(
            kept,
            [
                ContextItemView {
                    number: 3,
                    key: "s1:0".to_owned(),
                    title: "Optics.pdf".to_owned(),
                    location: Some(ContextLocation::Page { number: 3 }),
                    text: "Light bends.".to_owned(),
                },
                ContextItemView {
                    number: 4,
                    key: "s1:2".to_owned(),
                    title: "s1:2".to_owned(),
                    location: Some(ContextLocation::Headings {
                        path: vec!["Waves".to_owned()],
                    }),
                    text: "Grü".to_owned(),
                },
            ]
        );
        assert_eq!(numbers, 4);
        assert_eq!(limited.len(), MAX_CONTEXT_ITEMS);
    }

    #[test]
    fn a_part_reads_as_its_items_or_as_the_line_of_why_it_holds_none() {
        let items = vec![
            ContextItemView {
                number: 1,
                key: "s1:0".to_owned(),
                title: "Optics.pdf".to_owned(),
                location: Some(ContextLocation::Page { number: 3 }),
                text: "Light bends.".to_owned(),
            },
            ContextItemView {
                number: 2,
                key: "s2:0".to_owned(),
                title: "Notes.md".to_owned(),
                location: Some(ContextLocation::Headings {
                    path: vec!["Waves".to_owned(), "Energy".to_owned()],
                }),
                text: "Waves carry energy.".to_owned(),
            },
        ];

        assert_eq!(
            context_text(&view(items.clone(), None)),
            "[1] Optics.pdf, page 3 · cite: [Optics.pdf p. 3](cite:notebooks/s1:0)\nLight bends.\n\n\
             [2] Notes.md, Waves > Energy · cite: [Notes.md › Energy](cite:notebooks/s2:0)\nWaves carry energy."
        );
        assert_eq!(
            context_line(&view(items, None)),
            "[Excerpts [1] to [2] from the attached \"Physics\" and \"Optics\" went with this message and are not repeated.]"
        );
        assert_eq!(
            context_text(&view(Vec::new(), None)),
            "[The attached \"Physics\" and \"Optics\" hold nothing that applies to this message.]"
        );
        assert_eq!(
            context_text(&view(Vec::new(), Some(PLUGIN_OFF))),
            "[The attached \"Physics\" and \"Optics\" were not searched, because their plugin is off.]"
        );
        assert_eq!(
            context_text(&view(Vec::new(), Some(TIMED_OUT))),
            "[The attached \"Physics\" and \"Optics\" were not searched: the search took too long.]"
        );
        assert_eq!(
            context_line(&view(Vec::new(), Some("server_unreachable"))),
            "[Mannie could not search the attached \"Physics\" and \"Optics\": server_unreachable.]"
        );
    }

    #[test]
    fn a_provider_s_code_is_kept_only_when_it_looks_like_one() {
        assert_eq!(clean_code("server_unreachable"), "server_unreachable");
        assert_eq!(clean_code("notebooks.no_profile"), "notebooks.no_profile");
        for code in ["", "Server down", "a".repeat(65).as_str(), "line\nbreak"] {
            assert_eq!(clean_code(code), "provider_failed", "{code:?}");
        }
    }
}
