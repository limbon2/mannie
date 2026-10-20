//! Mentions (ADR-0032; docs/features/chat/mentions.md): the providers that
//! give the message box's "@" list its entries and the model what a mention
//! names, the registry of the core's providers, the check of the IDs of the
//! providers that plugins contribute through the SDK's `points::mentions`
//! (ADR-0034, decision 6), the adapter that calls them with the SDK's context
//! and the core's check of their reads (decision 3), and the runtime's side
//! of a message's mentions: the suggestions, the checks of the references,
//! and the budget that the mentions of one message share.

mod files;

use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, join_all};
use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint};
use mannie_plugin_sdk::host::{ContributionRefusal, ID_TAKEN};
use mannie_plugin_sdk::points::mentions::{
    MentionContent, MentionContext as PluginMentionContext,
    MentionProvider as PluginMentionProvider, Mentions, ProviderSuggestions, ReadCheck,
    ReadVerdict, RenderedMention,
};
use mannie_store::NewMention;
use mannie_tools::{
    Ceiling, Grants, MentionPathError, PolicyInputs, Verdict, decide, resolve_mention, result_limit,
};
use mannie_types::chat::ChatError;
use mannie_types::ids::{ConversationId, ProjectId};
use mannie_types::mentions::{
    MAX_MENTION_QUERY_CHARACTERS, MAX_SUGGESTIONS, MentionGroup, MentionPlace, MentionRef,
    MentionSuggestions, MentionView, SuggestMentions,
};
use mannie_types::tools::Access;
use tokio::runtime::Handle;

pub use files::{FILES, FileMentions};

use crate::Runtime;
use crate::budget::budget_bytes;
use crate::plugins::Snapshot;
use crate::policy::PolicyGrants;

/// Where the owner writes the message that mentions something, as a core
/// provider sees it: the conversation, its project and workspace, and what
/// the policy decides with there. A plugin's provider sees the SDK's context,
/// which checks its reads with this one.
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
                remote_rules: &self.grants.remote_rules,
                program_rules: &self.grants.program_rules,
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

/// A kind of mention of the core's (ADR-0032, decision 2), such as
/// [`FILES`], which decides with the policy itself. A plugin's provider
/// implements the SDK's trait instead, and the runtime calls it through an
/// adapter of this trait. Both kinds give the SDK's suggestions and content.
pub trait MentionProvider: fmt::Debug + Send + Sync {
    /// The provider's ID, such as [`FILES`] for the core's files and folders.
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

/// A plugin's provider (ADR-0032, decision 3) as the runtime calls the core's
/// providers. Each call gives it the SDK's view of the place, whose check of
/// reads is the core's policy (ADR-0034, decision 3).
#[derive(Debug)]
struct PluginMentions {
    provider: Arc<dyn PluginMentionProvider>,
}

impl MentionProvider for PluginMentions {
    fn id(&self) -> &str {
        self.provider.id()
    }

    fn offered(&self, context: &MentionContext) -> bool {
        self.provider.offered(&plugin_context(context))
    }

    fn suggest<'a>(
        &'a self,
        context: &'a MentionContext,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Result<ProviderSuggestions, String>> {
        async move {
            let context = plugin_context(context);
            self.provider.suggest(&context, query, limit).await
        }
        .boxed()
    }

    fn resolve<'a>(
        &'a self,
        context: &'a MentionContext,
        key: &'a str,
        max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>> {
        async move {
            let context = plugin_context(context);
            self.provider.resolve(&context, key, max_bytes).await
        }
        .boxed()
    }
}

/// The SDK's view of a place, for a plugin's provider.
fn plugin_context(context: &MentionContext) -> PluginMentionContext {
    PluginMentionContext::new(
        context.conversation.clone(),
        context.project.clone(),
        context.workspace.clone(),
        context.full_access,
        Arc::new(CoreReads::new(context.clone())),
    )
}

/// The core's check of a plugin provider's reads: the checks of a mentioned
/// path and the policy of a model's read in the place, as the `files`
/// provider applies them (ADR-0032, decision 8).
#[derive(Debug)]
struct CoreReads {
    context: Arc<MentionContext>,
    /// The runtime whose blocking threads resolve the keys, captured when
    /// the check is built, which happens on the runtime. A provider may poll
    /// the check on a thread of its own, where no runtime is current, and
    /// still gets a verdict. `None` only for a check built outside a
    /// runtime, which refuses every read.
    blocking: Option<Handle>,
}

impl CoreReads {
    fn new(context: MentionContext) -> Self {
        Self {
            context: Arc::new(context),
            blocking: Handle::try_current().ok(),
        }
    }
}

impl ReadCheck for CoreReads {
    fn can_read(&self, key: &str) -> BoxFuture<'static, ReadVerdict> {
        let Some(blocking) = self.blocking.clone() else {
            tracing::error!("a plugin provider's read was checked outside the runtime");
            return futures_util::future::ready(ReadVerdict::Refused).boxed();
        };
        let context = Arc::clone(&self.context);
        let key = key.to_owned();
        // Resolving the key reads the file system.
        checked_on(blocking, move || read_verdict(&context, &key))
    }
}

/// Runs a check of a read on the runtime's blocking threads, from the first
/// poll on. A check that fails to finish, such as one that panics, refuses
/// the read. Only a bug in the check reaches that failure, so the tests
/// reach it through this function with a check that panics.
fn checked_on(
    blocking: Handle,
    check: impl FnOnce() -> ReadVerdict + Send + 'static,
) -> BoxFuture<'static, ReadVerdict> {
    async move {
        blocking
            .spawn_blocking(check)
            .await
            .unwrap_or_else(|failure| {
                tracing::error!(error = %failure, "the check of a plugin provider's read failed");
                ReadVerdict::Refused
            })
    }
    .boxed()
}

/// Whether a key is a plain path relative to the workspace: names only, with
/// no `.`, `..`, root or prefix such as `C:`. The provider reads the key
/// below the workspace itself, and Unix resolves a `..` after a link from
/// the link's target, so `linked/../a.txt` would reach a file beside the
/// target while the core's lexical check decides on the workspace's
/// `a.txt`. A `.` inside a path, which [`Path::components`] drops, names the
/// same folder however the links resolve.
fn is_plain_relative(key: &str) -> bool {
    Path::new(key)
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
}

/// Resolves a key below the workspace once, following its links as a read
/// does, and decides on the access to the path that it leads to, as the
/// `files` provider does. A key that is not a plain relative path, or that
/// names nothing below the workspace, is refused before anything is read,
/// so the provider reads nothing for it.
fn read_verdict(context: &MentionContext, key: &str) -> ReadVerdict {
    let Some(workspace) = context.workspace() else {
        return ReadVerdict::Refused;
    };
    if !is_plain_relative(key) {
        return ReadVerdict::Refused;
    }
    let target = match resolve_mention(workspace, key) {
        Ok(target) => target,
        Err(MentionPathError::OutsideWorkspace | MentionPathError::NotFound) => {
            return ReadVerdict::Refused;
        }
        Err(MentionPathError::Unreadable { kind }) => {
            tracing::debug!(%kind, "a path that a plugin's provider checked could not be resolved");
            return ReadVerdict::Refused;
        }
    };
    match context.decide(std::slice::from_ref(&target.access)) {
        Verdict::Unrestricted | Verdict::Allowed(_) => ReadVerdict::Allowed,
        Verdict::Ask(_) => ReadVerdict::Ask,
        Verdict::Refused(_) => ReadVerdict::Refused,
    }
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
    /// Refuses a plugin's mention providers whose IDs a provider of the core
    /// or of another plugin has. The coordinator of the SDK made sure that
    /// each ID is the plugin's (the rule `Owned`), and the core's providers
    /// have IDs such as `files`, which a plugin of that name could own.
    pub(crate) fn check_mention_providers(
        &self,
        snapshot: &Snapshot,
        contributions: &Contributions,
    ) -> Result<(), ContributionRefusal> {
        for provider in contributions.entries::<Mentions>() {
            let id = provider.id();
            if self.inner.mentions.contains(id) || snapshot.find::<Mentions>(id).is_some() {
                return Err(ContributionRefusal::new(
                    Mentions::NAME,
                    ID_TAKEN,
                    format!("a mention provider with the ID {id} is registered already"),
                ));
            }
        }
        Ok(())
    }

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
    pub(crate) async fn mention_context(
        &self,
        place: &MentionPlace,
    ) -> Result<MentionContext, ChatError> {
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

    /// The core's providers, then those of the plugins that are on, each
    /// behind the adapter that gives it the SDK's context.
    pub(crate) fn mention_providers(&self) -> Vec<Arc<dyn MentionProvider>> {
        let mut providers: Vec<Arc<dyn MentionProvider>> = self.inner.mentions.providers().to_vec();
        for registered in self.contributions().items::<Mentions>() {
            providers.push(Arc::new(PluginMentions {
                provider: Arc::clone(&registered.entry),
            }));
        }
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
    use std::fs;
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::pin::pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    use std::thread::{self, Thread};

    use futures_util::FutureExt;
    use mannie_plugin_sdk::points::mentions::ReadVerdict;
    use mannie_tools::{Ceiling, MentionPathError, resolve_mention};
    use mannie_types::ceiling::CeilingMode;
    use tempfile::TempDir;
    use tokio::runtime::Handle;

    use super::{MentionContext, checked_on, cut_to, fair_shares, plugin_context, read_verdict};
    use crate::policy::PolicyGrants;

    /// A workspace in canonical form, in a folder that the test removes.
    fn workspace() -> (TempDir, PathBuf) {
        let folder = tempfile::tempdir().unwrap();
        let root = PathBuf::from(mannie_tools::canonical_folder(folder.path()).unwrap());
        (folder, root)
    }

    /// The place of a message in the workspace `root`, without grants or a
    /// ceiling, where the model reads the workspace without asking.
    fn in_workspace(root: &Path) -> MentionContext {
        MentionContext {
            conversation: None,
            project: None,
            workspace: Some(root.to_path_buf()),
            full_access: false,
            grants: PolicyGrants::default(),
            protected: Vec::new(),
            ceiling: None,
        }
    }

    /// A key below the workspace whose path cannot be resolved: a loop of
    /// links.
    #[cfg(unix)]
    fn unresolvable(root: &Path) -> &'static str {
        std::os::unix::fs::symlink(root.join("second"), root.join("first")).unwrap();
        std::os::unix::fs::symlink(root.join("first"), root.join("second")).unwrap();
        "first"
    }

    /// A key below the workspace whose path cannot be resolved: a name that
    /// Windows does not accept, which Mannie's check of paths lets through.
    #[cfg(windows)]
    fn unresolvable(_root: &Path) -> &'static str {
        "a<b.txt"
    }

    /// Wakes the thread that polls a future.
    struct Unpark(Thread);

    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    /// Polls a future to its end on the calling thread, as a provider that
    /// runs a thread of its own may, without a Tokio runtime.
    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(Unpark(thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut future = pin!(future);
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
            thread::park();
        }
    }

    #[test]
    fn a_read_check_allows_a_ceiling_s_folders_asks_outside_them_and_refuses_what_is_not_below_the_workspace()
     {
        let (_folder, root) = workspace();
        fs::create_dir_all(root.join("open")).unwrap();
        fs::write(root.join("open").join("a.txt"), "a").unwrap();
        fs::write(root.join("b.txt"), "b").unwrap();
        let context = MentionContext {
            ceiling: Some(Ceiling {
                mode: CeilingMode::ReadOnly,
                allowed_folders: vec![root.join("open")],
                rules: Vec::new(),
            }),
            ..in_workspace(&root)
        };
        let general = MentionContext {
            workspace: None,
            ceiling: None,
            ..context.clone()
        };

        assert_eq!(read_verdict(&context, "open/a.txt"), ReadVerdict::Allowed);
        assert_eq!(read_verdict(&context, "b.txt"), ReadVerdict::Ask);
        assert_eq!(read_verdict(&context, "missing.txt"), ReadVerdict::Refused);
        assert_eq!(read_verdict(&context, "../b.txt"), ReadVerdict::Refused);
        assert_eq!(read_verdict(&general, "b.txt"), ReadVerdict::Refused);
    }

    #[test]
    fn a_read_check_refuses_a_key_that_is_not_a_plain_relative_path() {
        let (_folder, root) = workspace();
        fs::create_dir_all(root.join("open")).unwrap();
        fs::write(root.join("b.txt"), "b").unwrap();
        let context = in_workspace(&root);
        let absolute = root.join("b.txt");
        let mut refused = vec![
            ".",
            "./b.txt",
            "open/../b.txt",
            "..",
            "/b.txt",
            absolute.to_str().unwrap(),
        ];
        if cfg!(windows) {
            refused.extend([r"open\..\b.txt", r".\b.txt", r"\b.txt", "C:b.txt"]);
        }

        // Each of them names the workspace or its b.txt lexically.
        assert_eq!(read_verdict(&context, "b.txt"), ReadVerdict::Allowed);
        assert_eq!(read_verdict(&context, "open"), ReadVerdict::Allowed);
        for key in refused {
            assert_eq!(read_verdict(&context, key), ReadVerdict::Refused, "{key}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_key_that_climbs_out_of_a_linked_folder_is_refused() {
        let (_folder, root) = workspace();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir_all(outside.path().join("target")).unwrap();
        fs::write(outside.path().join("target").join("b.txt"), "b").unwrap();
        fs::write(outside.path().join("a.txt"), "outside").unwrap();
        fs::write(root.join("a.txt"), "inside").unwrap();
        std::os::unix::fs::symlink(outside.path().join("target"), root.join("linked")).unwrap();
        let context = in_workspace(&root);

        // The key names the workspace's a.txt lexically, but a provider's
        // read of it climbs out of the link's target.
        assert_eq!(
            fs::read_to_string(root.join("linked/../a.txt")).unwrap(),
            "outside"
        );
        assert_eq!(read_verdict(&context, "a.txt"), ReadVerdict::Allowed);
        assert_eq!(
            read_verdict(&context, "linked/../a.txt"),
            ReadVerdict::Refused
        );
        // A link that leads out of the workspace is the policy's to decide,
        // as for the model's read.
        assert_eq!(read_verdict(&context, "linked/b.txt"), ReadVerdict::Ask);
    }

    #[test]
    fn a_read_check_refuses_a_path_that_cannot_be_resolved() {
        let (_folder, root) = workspace();
        let key = unresolvable(&root);
        let context = in_workspace(&root);

        assert!(
            matches!(
                resolve_mention(&root, key),
                Err(MentionPathError::Unreadable { .. })
            ),
            "{:?}",
            resolve_mention(&root, key)
        );
        assert_eq!(read_verdict(&context, key), ReadVerdict::Refused);
    }

    #[test]
    fn a_read_check_answers_a_provider_that_polls_it_on_a_thread_without_a_runtime() {
        let (_folder, root) = workspace();
        fs::write(root.join("a.txt"), "a").unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = {
            let _entered = runtime.enter();
            plugin_context(&in_workspace(&root))
        };

        let verdicts = thread::spawn(move || {
            assert!(Handle::try_current().is_err(), "the thread has a runtime");
            [
                block_on(context.can_read("a.txt")),
                block_on(context.can_read("missing.txt")),
            ]
        })
        .join()
        .unwrap();

        assert_eq!(verdicts, [ReadVerdict::Allowed, ReadVerdict::Refused]);
    }

    #[test]
    fn a_read_check_built_outside_a_runtime_refuses_every_read() {
        let (_folder, root) = workspace();
        fs::write(root.join("a.txt"), "a").unwrap();

        let context = plugin_context(&in_workspace(&root));

        assert_eq!(
            context.can_read("a.txt").now_or_never(),
            Some(ReadVerdict::Refused)
        );
    }

    #[tokio::test]
    async fn a_check_of_a_read_that_fails_to_finish_refuses_the_read() {
        let verdict = checked_on(Handle::current(), || panic!("the check failed")).await;

        assert_eq!(verdict, ReadVerdict::Refused);
    }

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
