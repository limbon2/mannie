//! Mention providers (ADR-0032, decisions 2 and 3): a plugin's provider gives
//! the message box's "@" list its entries, and the model what a mention
//! names.
//!
//! A provider checks its reads through its context (ADR-0034, decision 3):
//! [`MentionContext::can_read`] asks the core whether the policy lets the
//! model read a path of the workspace without asking. The provider reads only
//! what the core allows, and gives the model a note for the rest, because a
//! mention never asks (ADR-0032, decision 8).
//!
//! The provider's ID follows the rule `Owned`, which
//! [`owned_by`](crate::ids::owned_by) checks: the plugin's ID, or the
//! plugin's ID, a dot and more (ADR-0034, decision 6).

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::BoxFuture;
use crate::contracts::{ConversationId, MentionEntry, MentionSummary, ProjectId};
use crate::contributions::{ExtensionPoint, IdRule};

/// The point of plugins' mention providers, whose entries are named by their
/// IDs.
#[derive(Debug)]
pub enum Mentions {}

impl ExtensionPoint for Mentions {
    const NAME: &'static str = "mentions";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Owned;
    type Entry = Arc<dyn MentionProvider>;

    fn id(entry: &Arc<dyn MentionProvider>) -> &str {
        entry.id()
    }
}

/// A kind of mention (ADR-0032, decision 2).
pub trait MentionProvider: Send + Sync + fmt::Debug + 'static {
    /// The provider's ID: the plugin's ID, or one that starts with it and a
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

/// Where the owner writes the message that mentions something, as a provider
/// sees it: the conversation, its project and workspace, whether it is in
/// full access, and the core's check of reads there.
///
/// The core builds a context for each call of a provider. Its verdicts
/// reflect the conversation's grants and full access at the moment of that
/// call, so a provider asks again for each read during the call, and keeps
/// neither the context nor a verdict after it.
#[derive(Clone)]
pub struct MentionContext {
    conversation: Option<ConversationId>,
    project: Option<ProjectId>,
    workspace: Option<PathBuf>,
    full_access: bool,
    reads: Arc<dyn ReadCheck>,
}

impl MentionContext {
    /// The context of a place. Only the core builds one, with its own check
    /// of reads.
    #[cfg(feature = "host")]
    pub fn new(
        conversation: Option<ConversationId>,
        project: Option<ProjectId>,
        workspace: Option<PathBuf>,
        full_access: bool,
        reads: Arc<dyn ReadCheck>,
    ) -> Self {
        Self {
            conversation,
            project,
            workspace,
            full_access,
            reads,
        }
    }

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

    /// Whether the conversation is in full access, which the owner switched
    /// on (ADR-0029). [`can_read`](Self::can_read) already counts it.
    pub fn full_access(&self) -> bool {
        self.full_access
    }

    /// Whether the policy lets the model read what `key` names without
    /// asking, as it would decide about the model's read in this conversation
    /// (ADR-0004, decision 2). The key is a plain path relative to the
    /// workspace, as a mention's key is: names only, and the core refuses a
    /// key with `.`, `..`, a root or a prefix such as `C:`. The core checks
    /// the key as untrusted input, resolves its links once and decides on the
    /// path that it leads to. The verdict covers the key exactly as given,
    /// relative to the workspace, so a provider reads that same relative path
    /// below the workspace and nothing derived from it, right after the
    /// verdict.
    pub fn can_read(&self, key: &str) -> BoxFuture<'static, ReadVerdict> {
        self.reads.can_read(key)
    }
}

impl fmt::Debug for MentionContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The read check is the core's. Printing it would put the core's
        // policy inputs, such as the conversation's grants, into a plugin's
        // logs.
        formatter
            .debug_struct("MentionContext")
            .field("conversation", &self.conversation)
            .field("project", &self.project)
            .field("workspace", &self.workspace)
            .field("full_access", &self.full_access)
            .finish_non_exhaustive()
    }
}

/// The core's side of [`MentionContext::can_read`], which decides with the
/// place's workspace, grants and full access. The runtime implements it, and
/// a plugin has no use for it, because only the core builds a context.
pub trait ReadCheck: Send + Sync + fmt::Debug {
    /// Whether the policy lets the model read what `key` names without
    /// asking.
    fn can_read(&self, key: &str) -> BoxFuture<'static, ReadVerdict>;
}

/// What the core decided about a provider's read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReadVerdict {
    /// The model may read it without asking, so the provider reads it.
    Allowed,
    /// The owner would be asked, so the provider leaves it out and gives the
    /// model a note.
    Ask,
    /// Nobody can allow the read: there is no workspace, the key is not a
    /// plain relative path, the key names a path outside the workspace,
    /// nothing exists at the path, the path cannot be resolved, the check
    /// failed, or the policy forbids the read. A link below the workspace
    /// that leads out of it is the policy's to decide, as for the model's
    /// read.
    Refused,
}

/// What one provider suggests for a query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ProviderSuggestions {
    /// The entries, best first.
    pub entries: Vec<MentionEntry>,
    /// Whether the provider holds more than it looked through.
    pub incomplete: bool,
}

impl ProviderSuggestions {
    /// The entries, best first, and whether the provider holds more than it
    /// looked through.
    pub fn new(entries: Vec<MentionEntry>, incomplete: bool) -> Self {
        Self {
            entries,
            incomplete,
        }
    }
}

/// A mention rendered within its share of its message's budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
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

impl RenderedMention {
    /// A rendered mention. The three texts are easy to swap, so callers pass
    /// named locals.
    pub fn new(text: String, label: String, variant: String, summary: MentionSummary) -> Self {
        Self {
            text,
            label,
            variant,
            summary,
        }
    }
}

/// What a mention gives: read once, when its message is sent, and rendered
/// within any number of bytes without reading again.
pub trait MentionContent: Send {
    /// What the model reads within `limit` bytes, and what the owner sees.
    fn render(&self, limit: usize) -> RenderedMention;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{ProviderSuggestions, RenderedMention};
    use crate::contracts::{MentionEntry, MentionSummary};

    #[test]
    fn the_constructors_keep_each_argument_in_its_field() {
        let entry = MentionEntry {
            key: "release".to_owned(),
            label: "Release notes".to_owned(),
            token: "@release".to_owned(),
            variant: "note".to_owned(),
            warning: None,
            expands: false,
        };
        let text = "The notes.".to_owned();
        let label = "Release notes".to_owned();
        let variant = "note".to_owned();
        let summary = MentionSummary {
            code: "lines".to_owned(),
            values: BTreeMap::from([("total".to_owned(), 3)]),
        };

        let suggestions = ProviderSuggestions::new(vec![entry.clone()], true);
        let rendered = RenderedMention::new(text, label, variant, summary.clone());

        assert_eq!(suggestions.entries, [entry]);
        assert!(suggestions.incomplete);
        assert_eq!(rendered.text, "The notes.");
        assert_eq!(rendered.label, "Release notes");
        assert_eq!(rendered.variant, "note");
        assert_eq!(rendered.summary, summary);
    }

    #[cfg(feature = "host")]
    mod host {
        use std::path::{Path, PathBuf};
        use std::sync::Arc;

        use futures_util::FutureExt;

        use crate::BoxFuture;
        use crate::contracts::{ConversationId, ProjectId};
        use crate::points::mentions::{MentionContext, ReadCheck, ReadVerdict};

        /// A check of reads that allows `notes.txt`, asks about `.env` and
        /// refuses every other key.
        #[derive(Debug)]
        struct ByKey;

        impl ReadCheck for ByKey {
            fn can_read(&self, key: &str) -> BoxFuture<'static, ReadVerdict> {
                let verdict = match key {
                    "notes.txt" => ReadVerdict::Allowed,
                    ".env" => ReadVerdict::Ask,
                    _ => ReadVerdict::Refused,
                };
                futures_util::future::ready(verdict).boxed()
            }
        }

        #[test]
        fn a_context_names_its_place_and_answers_with_the_core_s_read_check() {
            let conversation =
                ConversationId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap();
            let project = ProjectId::parse("0199e0f2-0000-7000-8000-000000000002").unwrap();

            let context = MentionContext::new(
                Some(conversation.clone()),
                Some(project.clone()),
                Some(PathBuf::from("/work/app")),
                true,
                Arc::new(ByKey),
            );
            let general = MentionContext::new(None, None, None, false, Arc::new(ByKey));

            assert_eq!(context.conversation(), Some(&conversation));
            assert_eq!(context.project(), Some(&project));
            assert_eq!(context.workspace(), Some(Path::new("/work/app")));
            assert!(context.full_access());
            assert_eq!(
                (
                    general.conversation(),
                    general.project(),
                    general.workspace()
                ),
                (None, None, None)
            );
            assert!(!general.full_access());
            for (key, verdict) in [
                ("notes.txt", ReadVerdict::Allowed),
                (".env", ReadVerdict::Ask),
                ("../outside.txt", ReadVerdict::Refused),
            ] {
                assert_eq!(context.can_read(key).now_or_never(), Some(verdict), "{key}");
            }
        }

        #[test]
        fn a_context_s_debug_output_leaves_out_the_core_s_read_check() {
            let context = MentionContext::new(
                None,
                None,
                Some(PathBuf::from("/work/app")),
                false,
                Arc::new(ByKey),
            );

            let printed = format!("{context:?}");

            assert!(printed.contains("app"), "{printed}");
            assert!(!printed.contains("ByKey"), "{printed}");
        }
    }
}
