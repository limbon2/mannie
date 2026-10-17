//! Mentions (ADR-0032; docs/features/chat/mentions.md): the providers that
//! give the message box's "@" list its entries and the model what a mention
//! names, the registry of the core's providers, and the runtime's side of a
//! message's mentions: the suggestions, the checks of the references, and the
//! budget that the mentions of one message share.

mod files;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::future::{BoxFuture, join_all};
use mannie_store::NewMention;
use mannie_tools::{Ceiling, Grants, PolicyInputs, Verdict, decide, result_limit};
use mannie_types::chat::ChatError;
use mannie_types::ids::{ConversationId, ProjectId};
use mannie_types::mentions::{
    MAX_MENTION_QUERY_CHARACTERS, MAX_SUGGESTIONS, MentionEntry, MentionGroup, MentionPlace,
    MentionRef, MentionSuggestions, MentionSummary, MentionView, SuggestMentions,
};
use mannie_types::tools::Access;

pub use files::{FILES, FileMentions};

use crate::Runtime;
use crate::budget::budget_bytes;
use crate::policy::PolicyGrants;

/// Where the owner writes the message that mentions something, as a provider
/// sees it: the conversation, its project and workspace, and what the policy
/// decides with there.
#[derive(Debug, Clone)]
pub struct MentionContext {
    conversation: Option<ConversationId>,
    project: Option<ProjectId>,
    workspace: Option<PathBuf>,
    full_access: bool,
    grants: PolicyGrants,
    protected: Vec<PathBuf>,
    ceiling: Option<Ceiling>,
}

impl MentionContext {
    /// The conversation, or `None` for a message that starts one.
    pub fn conversation(&self) -> Option<&ConversationId> {
        self.conversation.as_ref()
    }

    /// The conversation's project, or the one that a new conversation starts
    /// in.
    pub fn project(&self) -> Option<&ProjectId> {
        self.project.as_ref()
    }

    /// The workspace, in canonical form: the project's folder, or `None` for a
    /// general conversation.
    pub fn workspace(&self) -> Option<&Path> {
        self.workspace.as_deref()
    }

    /// What the policy decides about reading what a mention names, as it would
    /// decide about the model's read in this conversation (ADR-0004, decision
    /// 2). A provider reads only what this allows, and gives the model a note
    /// for the rest (ADR-0032, decision 8).
    pub fn decide(&self, accesses: &[Access]) -> Verdict {
        decide(
            accesses,
            &PolicyInputs {
                workspace: self.workspace.as_deref(),
                allow_changes_in_workspace: self.grants.settings.allow_changes_in_workspace,
                protected: &self.protected,
                grants: Grants {
                    conversation: &self.grants.conversation,
                    project: &self.grants.project,
                    always: &self.grants.always,
                },
                rules: &self.grants.rules,
                project: self.project.as_ref(),
                // Content never counts as the owner's words, and a mention
                // moves no conversation.
                owner_request: None,
                ceiling: self.ceiling.as_ref(),
                host: None,
                full_access: self.full_access,
            },
        )
    }
}

/// What one provider suggests for a query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderSuggestions {
    /// The entries, best first.
    pub entries: Vec<MentionEntry>,
    /// Whether the provider holds more than it looked through.
    pub incomplete: bool,
}

/// A mention rendered within its share of its message's budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedMention {
    /// What the model reads.
    pub text: String,
    /// What the owner reads, such as the path.
    pub label: String,
    /// The provider's variant, which chooses the icon.
    pub variant: String,
    /// What the model got, for the owner.
    pub summary: MentionSummary,
}

/// What a mention gives: read once, when its message is sent, and rendered
/// within any number of bytes without reading again.
pub trait MentionContent: Send {
    /// What the model reads within `limit` bytes, and what the owner sees.
    fn render(&self, limit: usize) -> RenderedMention;
}

/// A kind of mention (ADR-0032, decision 2).
pub trait MentionProvider: fmt::Debug + Send + Sync {
    /// The provider's ID: [`FILES`] for the core's files and folders, and for
    /// a plugin's provider the plugin's ID, or one that starts with it and a
    /// dot.
    fn id(&self) -> &str;

    /// Whether the provider offers mentions in this place, such as only where
    /// there is a workspace.
    fn offered(&self, context: &MentionContext) -> bool;

    /// The entries that match what the owner typed after "@", best first and
    /// at most `limit` of them, or the provider's code for why it lists
    /// nothing.
    fn suggest<'a>(
        &'a self,
        context: &'a MentionContext,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Result<ProviderSuggestions, String>>;

    /// Reads what `key` names, keeping at most `max_bytes` of what the model
    /// reads. The key comes from the webview, so the provider checks it again
    /// (ADR-0004), and refuses a key that it does not accept with its code.
    fn resolve<'a>(
        &'a self,
        context: &'a MentionContext,
        key: &'a str,
        max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>>;
}

/// The core's mention providers, which a composition root registers
/// (ADR-0032, decision 3). The list offers their groups in this order, before
/// the providers of plugins.
#[derive(Debug, Clone, Default)]
pub struct MentionRegistry {
    providers: Vec<Arc<dyn MentionProvider>>,
}

/// A provider's ID is registered already.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a mention provider with the ID {id} is registered already")]
pub struct DuplicateMentionProvider {
    /// The ID.
    pub id: String,
}

impl MentionRegistry {
    /// A registry without providers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a provider. IDs are unique.
    pub fn register(
        &mut self,
        provider: Arc<dyn MentionProvider>,
    ) -> Result<(), DuplicateMentionProvider> {
        if self.contains(provider.id()) {
            return Err(DuplicateMentionProvider {
                id: provider.id().to_owned(),
            });
        }
        self.providers.push(provider);
        Ok(())
    }

    /// Whether a provider with this ID is registered.
    pub fn contains(&self, id: &str) -> bool {
        self.providers.iter().any(|provider| provider.id() == id)
    }

    /// The providers, in the order of registration.
    pub(crate) fn providers(&self) -> &[Arc<dyn MentionProvider>] {
        &self.providers
    }
}

impl Runtime {
    /// The suggestions of the "@" list: a group for each provider that is
    /// offered in the place, the core's first, then those of the plugins that
    /// are on. A provider that fails lists nothing, with its code, and the
    /// others still list theirs.
    pub async fn suggest_mentions(
        &self,
        input: SuggestMentions,
    ) -> Result<MentionSuggestions, ChatError> {
        let context = self.mention_context(&input.place).await?;
        let providers: Vec<Arc<dyn MentionProvider>> = self
            .mention_providers()
            .into_iter()
            .filter(|provider| provider.offered(&context))
            .collect();
        let matches_nothing = input.query.chars().count() > MAX_MENTION_QUERY_CHARACTERS;
        let context = &context;
        let query = input.query.as_str();
        let groups = join_all(providers.iter().map(|provider| async move {
            let suggested = if matches_nothing {
                Ok(ProviderSuggestions::default())
            } else {
                provider.suggest(context, query, MAX_SUGGESTIONS).await
            };
            match suggested {
                Ok(mut suggestions) => {
                    suggestions.entries.truncate(MAX_SUGGESTIONS);
                    MentionGroup {
                        provider: provider.id().to_owned(),
                        entries: suggestions.entries,
                        incomplete: suggestions.incomplete,
                        failure: None,
                    }
                }
                Err(code) => {
                    tracing::info!(
                        provider = provider.id(),
                        code,
                        "a mention provider listed nothing"
                    );
                    MentionGroup {
                        provider: provider.id().to_owned(),
                        entries: Vec::new(),
                        incomplete: false,
                        failure: Some(code),
                    }
                }
            }
        }))
        .await;
        Ok(MentionSuggestions { groups })
    }

    /// The place of a message as providers see it.
    async fn mention_context(&self, place: &MentionPlace) -> Result<MentionContext, ChatError> {
        match place {
            MentionPlace::Conversation { conversation_id } => {
                let summary = self
                    .inner
                    .store
                    .conversation_summary(conversation_id)
                    .await
                    .map_err(|error| {
                        tracing::error!(error = %error, error.code = error.code(), "a conversation could not be read for mentions");
                        ChatError::StorageFailed
                    })?
                    .ok_or(ChatError::ConversationNotFound)?;
                self.mention_context_for(
                    Some(conversation_id),
                    summary.project_id.as_ref(),
                    summary.workspace.as_deref(),
                    summary.full_access,
                )
                .await
            }
            MentionPlace::NewConversation { project_id } => {
                let folder = match project_id {
                    Some(project) => Some(self.mention_project_folder(project).await?),
                    None => None,
                };
                let full_access = self.full_access_for_new_conversations().await;
                self.mention_context_for(None, project_id.as_ref(), folder.as_deref(), full_access)
                    .await
            }
        }
    }

    /// The folder of the project that a new conversation starts in.
    pub(crate) async fn mention_project_folder(
        &self,
        project: &ProjectId,
    ) -> Result<String, ChatError> {
        if self.is_being_removed(project) {
            return Err(ChatError::ProjectNotFound);
        }
        let projects = self.inner.store.projects().await.map_err(|error| {
            tracing::error!(error = %error, error.code = error.code(), "the projects could not be read for mentions");
            ChatError::StorageFailed
        })?;
        projects
            .into_iter()
            .find(|candidate| candidate.id == *project)
            .map(|found| found.folder)
            .ok_or(ChatError::ProjectNotFound)
    }

    /// The place of a message, from what the caller read already.
    pub(crate) async fn mention_context_for(
        &self,
        conversation: Option<&ConversationId>,
        project: Option<&ProjectId>,
        workspace: Option<&str>,
        full_access: bool,
    ) -> Result<MentionContext, ChatError> {
        let grants = self
            .policy_grants(conversation, project)
            .await
            .map_err(|failure| {
                tracing::error!(error = %failure.error, "could not read {} for mentions", failure.what);
                ChatError::StorageFailed
            })?;
        Ok(MentionContext {
            conversation: conversation.cloned(),
            project: project.cloned(),
            workspace: workspace.map(PathBuf::from),
            full_access,
            grants,
            protected: self.inner.tools.protected.clone(),
            ceiling: self.inner.machine.ceiling.clone(),
        })
    }

    /// Reads a message's mentions, which [`check_mentions`] checked, and
    /// divides the budget among them: the tool result limit of a profile whose
    /// context budget is `budget` tokens (ADR-0032, decisions 5 and 6).
    ///
    /// [`check_mentions`]: mannie_types::mentions::check_mentions
    pub(crate) async fn resolve_mentions(
        &self,
        context: &MentionContext,
        mentions: &[MentionRef],
        budget: u32,
    ) -> Result<Vec<NewMention>, ChatError> {
        if mentions.is_empty() {
            return Ok(Vec::new());
        }
        let total = result_limit(budget_bytes(budget));
        let providers = self.mention_providers();
        let mut contents = Vec::with_capacity(mentions.len());
        for mention in mentions {
            let provider = providers
                .iter()
                .find(|provider| provider.id() == mention.provider)
                .filter(|provider| provider.offered(context))
                .ok_or_else(|| ChatError::MentionUnavailable {
                    provider: mention.provider.clone(),
                })?;
            let resolved = provider
                .resolve(context, &mention.key, total)
                .await
                .map_err(|code| {
                    tracing::info!(provider = provider.id(), code, "a mention was refused");
                    ChatError::MentionRefused {
                        provider: mention.provider.clone(),
                        reason: code,
                    }
                })?;
            contents.push(resolved);
        }
        let whole: Vec<RenderedMention> = contents
            .iter()
            .map(|content| content.render(total))
            .collect();
        let needs: Vec<usize> = whole.iter().map(|rendered| rendered.text.len()).collect();
        let shares = fair_shares(&needs, total);
        let mut stored = Vec::with_capacity(mentions.len());
        for (((mention, content), whole), share) in
            mentions.iter().zip(&contents).zip(whole).zip(shares)
        {
            let mut rendered = if whole.text.len() <= share {
                whole
            } else {
                content.render(share)
            };
            if rendered.text.len() > share {
                tracing::warn!(
                    provider = mention.provider,
                    "a mention provider rendered more than its share, which was cut"
                );
                cut_to(&mut rendered.text, share);
            }
            stored.push(NewMention {
                part_id: self.inner.ids.part(),
                text: rendered.text,
                view: MentionView {
                    provider: mention.provider.clone(),
                    key: mention.key.clone(),
                    label: rendered.label,
                    variant: rendered.variant,
                    summary: rendered.summary,
                },
            });
        }
        Ok(stored)
    }

    /// The core's providers, then those of the plugins that are on.
    pub(crate) fn mention_providers(&self) -> Vec<Arc<dyn MentionProvider>> {
        let mut providers: Vec<Arc<dyn MentionProvider>> = self.inner.mentions.providers().to_vec();
        providers.extend(self.plugin_mention_providers());
        providers
    }
}

/// Divides `total` bytes among items that need `needs` bytes each, max-min
/// fairly: an item that needs less than an equal share of what is left gets
/// what it needs, and the others divide the rest equally.
fn fair_shares(needs: &[usize], total: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..needs.len()).collect();
    order.sort_by_key(|index| needs[*index]);
    let mut shares = vec![0; needs.len()];
    let mut left = total;
    for (position, index) in order.into_iter().enumerate() {
        let equal = left / (needs.len() - position);
        let share = needs[index].min(equal);
        shares[index] = share;
        left -= share;
    }
    shares
}

/// Cuts a text to at most `limit` bytes, at a character.
fn cut_to(text: &mut String, limit: usize) {
    let mut end = limit.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

#[cfg(test)]
mod tests {
    use super::{cut_to, fair_shares};

    #[test]
    fn small_mentions_get_what_they_need_and_large_ones_share_the_rest() {
        assert_eq!(
            fair_shares(&[100, 50_000, 30_000], 60_000),
            [100, 29_950, 29_950]
        );
        assert_eq!(fair_shares(&[10, 20], 1_000), [10, 20]);
        assert_eq!(fair_shares(&[900, 900, 900], 900), [300, 300, 300]);
        assert_eq!(fair_shares(&[], 900), Vec::<usize>::new());
        let shares = fair_shares(&[5_000, 70_000, 1_000, 70_000], 65_536);
        assert_eq!(shares[2], 1_000);
        assert_eq!(shares[0], 5_000);
        assert!(shares.iter().sum::<usize>() <= 65_536);
        assert_eq!(shares[1], shares[3]);
    }

    #[test]
    fn texts_are_cut_at_a_character() {
        let mut text = "aé".to_owned();
        cut_to(&mut text, 2);
        assert_eq!(text, "a");
        let mut short = "abc".to_owned();
        cut_to(&mut short, 10);
        assert_eq!(short, "abc");
    }
}
