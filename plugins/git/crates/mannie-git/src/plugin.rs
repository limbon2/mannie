//! The git plugin's definition (ADR-0038; plugins/git/docs/README.md): its
//! start and stop and its contributions. Its manifest, `plugins/git/
//! plugin.toml`, gives the service `processes`, which runs the installed
//! git. It is off by default, like every plugin (D28).
//!
//! A start contributes the tools that read, offered to every agent where
//! the conversation's workspace is in a repository; the tools that change
//! a repository, offered to the owner's conversations and the general child
//! agent there; the section that tells those calls how to use them; the
//! mention provider of commits, branches and changes; and the handlers of
//! the side panel's commands. Turning the plugin off removes them, and the
//! host waits for the calls that run.

use std::sync::Arc;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::StartContext;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::Interface;
use mannie_plugin_sdk::plugin::{Instance, Plugin, Started};
use mannie_plugin_sdk::points::mentions::{MentionProvider, Mentions};
use mannie_plugin_sdk::points::sections::{Section, SectionBody, Sections};
use mannie_plugin_sdk::points::tools::{AgentSelector, PluginTool, ToolOffering, Tools};
use mannie_plugin_sdk::services::processes::Processes;

use crate::changes::{ChangeTool, ChangingTool, RepositoryLocks};
use crate::commands::interface;
use crate::git::Git;
use crate::mentions::GitMentions;
use crate::panel::{Changes, handlers};
use crate::section::{GitSection, SECTION_ID, SECTION_POSITION};
use crate::tools::apply::{GitCherryPick, GitRevert};
use crate::tools::blame::GitBlame;
use crate::tools::branch::GitBranch;
use crate::tools::commit::GitCommit;
use crate::tools::conflicts::GitConflicts;
use crate::tools::diff::GitDiff;
use crate::tools::edit_history::GitEditHistory;
use crate::tools::fetch::GitFetch;
use crate::tools::grep::GitGrep;
use crate::tools::log::GitLog;
use crate::tools::merge::GitMerge;
use crate::tools::operation::GitOperation;
use crate::tools::push::GitPush;
use crate::tools::rebase::GitRebase;
use crate::tools::refs::GitRefs;
use crate::tools::reset::GitReset;
use crate::tools::resolve::GitResolve;
use crate::tools::restore::GitRestore;
use crate::tools::show::GitShow;
use crate::tools::stage::GitStage;
use crate::tools::stash::GitStash;
use crate::tools::status::GitStatus;
use crate::tools::switch::GitSwitch;
use crate::tools::{ReadTool, ReadingTool};

/// The plugin's ID.
pub const GIT: &str = "git";

/// The entry whose presence in the workspace, or a folder above it, offers
/// the tools: a repository's git folder, or the `.git` file of a linked
/// worktree or a submodule.
const REPOSITORY_MARKER: &str = ".git";

/// The agents that are offered the tools that change a repository: the
/// owner's conversations and the general child agent. The explore agent
/// only reads (D37).
const CHANGING_AGENTS: [&str; 2] = ["conversation", "general"];

/// The git plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitPlugin;

impl GitPlugin {
    /// The plugin, off.
    pub fn new() -> Self {
        Self
    }

    fn start_now(context: &StartContext) -> Result<Started, StartFailure> {
        let git = Git::new(context.service::<Processes>()?);
        let locks = RepositoryLocks::default();
        let changes = Changes::new();
        let section = Section::new(
            SECTION_ID,
            SECTION_POSITION,
            SectionBody::Source(Arc::new(GitSection)),
        );
        let mentions: Arc<dyn MentionProvider> = Arc::new(GitMentions::new(git.clone()));
        let changing = Changing {
            git: &git,
            locks: &locks,
            changes: &changes,
        };
        let contributions = Contributions::new()
            .add::<Tools>(reading_tool::<GitStatus>(&git))
            .add::<Tools>(reading_tool::<GitLog>(&git))
            .add::<Tools>(reading_tool::<GitShow>(&git))
            .add::<Tools>(reading_tool::<GitDiff>(&git))
            .add::<Tools>(reading_tool::<GitBlame>(&git))
            .add::<Tools>(reading_tool::<GitGrep>(&git))
            .add::<Tools>(reading_tool::<GitRefs>(&git))
            .add::<Tools>(reading_tool::<GitConflicts>(&git))
            .add::<Tools>(changing.tool::<GitStage>())
            .add::<Tools>(changing.tool::<GitCommit>())
            .add::<Tools>(changing.tool::<GitBranch>())
            .add::<Tools>(changing.tool::<GitSwitch>())
            .add::<Tools>(changing.tool::<GitRestore>())
            .add::<Tools>(changing.tool::<GitReset>())
            .add::<Tools>(changing.tool::<GitStash>())
            .add::<Tools>(changing.tool::<GitMerge>())
            .add::<Tools>(changing.tool::<GitRebase>())
            .add::<Tools>(changing.tool::<GitCherryPick>())
            .add::<Tools>(changing.tool::<GitRevert>())
            .add::<Tools>(changing.tool::<GitResolve>())
            .add::<Tools>(changing.tool::<GitOperation>())
            .add::<Tools>(changing.tool::<GitEditHistory>())
            .add::<Tools>(changing.tool::<GitFetch>())
            .add::<Tools>(changing.tool::<GitPush>())
            .add::<Sections>(section)
            .add::<Mentions>(mentions);
        let handlers = handlers(&git, &changes);
        tracing::info!("the git plugin started");
        Ok(Started::new(GitInstance)
            .with_contributions(contributions)
            .with_handlers(handlers))
    }
}

/// A tool that reads, offered to every agent, the explore agent too, where
/// the workspace is in a repository (ADR-0038, decision 3).
fn reading_tool<T: ReadTool>(git: &Git) -> PluginTool {
    PluginTool::new(
        Arc::new(ReadingTool::<T>::new(git.clone())),
        AgentSelector::Every,
    )
    .with_file_reads()
    .offered_where(ToolOffering::WhereWorkspaceHolds(
        REPOSITORY_MARKER.to_owned(),
    ))
}

/// What the tools that change a repository share: git, and the locks that
/// run one change at a time in a repository.
struct Changing<'a> {
    git: &'a Git,
    locks: &'a RepositoryLocks,
    changes: &'a Changes,
}

impl Changing<'_> {
    /// A tool that changes a repository, offered where the workspace is in
    /// one; each call is an operation that the policy decides on.
    fn tool<T: ChangeTool>(&self) -> PluginTool {
        let agents = CHANGING_AGENTS
            .iter()
            .map(|agent| (*agent).to_owned())
            .collect();
        let tool =
            ChangingTool::<T>::new(self.git.clone(), self.locks.clone(), self.changes.clone());
        PluginTool::new(Arc::new(tool), AgentSelector::Agents(agents))
            .with_side_effects()
            .offered_where(ToolOffering::WhereWorkspaceHolds(
                REPOSITORY_MARKER.to_owned(),
            ))
    }
}

impl Plugin for GitPlugin {
    fn interface(&self) -> Interface {
        interface()
    }

    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        Box::pin(std::future::ready(Self::start_now(&context)))
    }
}

/// The plugin while it is on. Its git runs belong to the tools' calls, which
/// the host ends or waits for before it stops the plugin.
#[derive(Debug)]
struct GitInstance;

impl Instance for GitInstance {
    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        tracing::info!("the git plugin stopped");
        Box::pin(std::future::ready(()))
    }
}
