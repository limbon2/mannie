//! What the git tools that change a repository share (plugins/git/docs/
//! approvals.md; ADR-0038, decisions 4 to 9).
//!
//! A change's preparation checks its arguments and plans it by reading the
//! repository, and returns its accesses: the write of the repository's top
//! with the repository's state, the writes of its git folders that lie
//! outside, the reads of the object folders that it borrows, and, when the
//! change does anything beyond the workspace, a run of git that lists it:
//! the hooks and the repository's own programs that it starts, the work
//! that it throws away and the published commits that it rewrites.
//!
//! The execution plans the change again, with the repository locked against
//! the plugin's other changes, and acts only when the plan gives the same
//! accesses that the owner or the policy allowed. Otherwise the repository
//! changed since, and nothing runs (`repository_changed`).

use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{AccessMode, ProgramEffect};
use mannie_plugin_sdk::points::tools::{
    PathAccess, ProgramAccess, Tool, ToolAccess, ToolCall, ToolExecution, ToolRefusal, ToolResult,
    ToolSpec,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use tokio::sync::OwnedMutexGuard;

use crate::arguments::{check_branch, is_object_id, repository_folder};
use crate::effects::{Change, configured_programs, hooks};
use crate::failure::GitFailure;
use crate::git::{GIT, Git, GitRun};
use crate::layout::{Layout, is_inside};
use crate::operation::Operation;
use crate::output::{Room, fitted};
use crate::panel::Changes;
use crate::parse::status::parse_status;
use crate::schema::parameters;
use crate::tools::{Answer, Reading, authorized_layout, comparable, read_arguments};

/// The longest that a change's run of git may last, in seconds: hooks may
/// run a project's checks.
const CHANGE_TIMEOUT_SECONDS: u32 = 600;

/// The room of a preparation's reads, which no model reads.
const PREPARATION_ROOM: usize = 64 * 1024;

/// What a change tool plans in its preparation, and plans again before it
/// acts.
#[derive(Debug, Clone)]
pub(crate) struct Plan<P> {
    /// What the change does, which names the hooks and the repository's
    /// programs that it may start.
    pub(crate) change: Change,
    /// The arguments of git's run that the owner decides on, after the ones
    /// that the manifest fixes.
    pub(crate) command: Vec<String>,
    /// The work that the change throws away and the published commits that
    /// it rewrites.
    pub(crate) effects: Vec<ProgramEffect>,
    /// The commits of the refs that the change works with besides HEAD,
    /// such as a branch that it merges, which the repository's state holds.
    pub(crate) tips: Vec<String>,
    /// What the tool keeps for its run.
    pub(crate) planned: P,
}

impl<P> Plan<P> {
    /// A plan of `change` that runs git with `command`, with nothing
    /// thrown away or rewritten.
    pub(crate) fn new(change: Change, command: Vec<String>, planned: P) -> Self {
        Self {
            change,
            command,
            effects: Vec::new(),
            tips: Vec::new(),
            planned,
        }
    }

    /// The same plan, which also throws away or rewrites `effect`.
    #[must_use]
    pub(crate) fn with_effect(mut self, effect: Option<ProgramEffect>) -> Self {
        self.effects.extend(effect);
        self
    }

    /// The same plan, whose state also holds `tip`.
    #[must_use]
    pub(crate) fn with_tip(mut self, tip: &str) -> Self {
        self.tips.push(tip.to_owned());
        self
    }
}

/// A git tool that changes a repository.
pub(crate) trait ChangeTool: fmt::Debug + Send + Sync + 'static {
    /// The tool's arguments, whose documentation the model reads.
    type Arguments: DeserializeOwned + JsonSchema + Send + Sync + 'static;

    /// What the tool's plan keeps for its run.
    type Planned: Send + Sync + 'static;

    /// The tool's name.
    const NAME: &'static str;

    /// What the model reads about the tool.
    const DESCRIPTION: &'static str;

    /// The `repository` argument.
    fn repository(arguments: &Self::Arguments) -> Option<&str>;

    /// Checks the arguments and plans the change. It only reads.
    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a Self::Arguments,
    ) -> BoxFuture<'a, Result<Plan<Self::Planned>, GitFailure>>;

    /// Makes the change that the owner or the policy allowed.
    fn act<'a>(
        acting: &'a Acting,
        arguments: &'a Self::Arguments,
        plan: Plan<Self::Planned>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>>;
}

/// What a change works with while it acts: the repository as a reading
/// sees it, for the reads before and after the change.
#[derive(Debug)]
pub(crate) struct Acting {
    pub(crate) reading: Reading,
}

impl Acting {
    /// A run of the change in the repository's top. git starts the hooks and
    /// the repository's programs as in the owner's terminal, which the plan
    /// listed and the owner allowed.
    pub(crate) fn run<S: Into<String>>(&self, arguments: impl IntoIterator<Item = S>) -> GitRun {
        GitRun::new(&self.reading.layout.top, arguments).with_timeout(CHANGE_TIMEOUT_SECONDS)
    }

    /// Keeps the uncommitted changes of tracked files as a stash entry named
    /// "Mannie: before `operation`", and returns its commit, or `None` when
    /// there are none (plugins/git/docs/approvals.md, T-50).
    pub(crate) async fn snapshot(&self, operation: &str) -> Result<Option<String>, GitFailure> {
        let created = self
            .reading
            .git
            .output(self.run(["stash", "create"]))
            .await?;
        let commit = created.text().trim().to_owned();
        if !is_object_id(&commit) {
            return Ok(None);
        }
        let message = format!("Mannie: before {operation}");
        self.reading
            .git
            .output(self.run(["stash", "store", "-m", message.as_str(), commit.as_str()]))
            .await?;
        Ok(Some(commit))
    }
}

/// Which changed files a change counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Changed {
    /// Tracked files whose work tree differs from the index.
    Unstaged,
    /// Tracked files with any change that no commit holds.
    Any,
    /// Files whose index differs from both HEAD and the work tree, whose
    /// staged version unstaging throws away.
    PartlyStaged,
}

/// The tracked files under `pathspecs`, every file when there are none,
/// that have changes of the kind that `changed` names.
pub(crate) async fn changed_paths(
    reading: &Reading,
    pathspecs: &[String],
    changed: Changed,
) -> Result<Vec<String>, GitFailure> {
    let mut command: Vec<String> = [
        "status",
        "--porcelain=v2",
        "-z",
        "--untracked-files=no",
        "--ignore-submodules=dirty",
        "--",
    ]
    .map(str::to_owned)
    .to_vec();
    command.extend(pathspecs.iter().cloned());
    let ran = reading.git.output(reading.run(command)).await?;
    let status =
        parse_status(&ran.output).map_err(|error| GitFailure::failed(&error.to_string()))?;
    Ok(status
        .entries
        .into_iter()
        .filter(|entry| match changed {
            Changed::Unstaged => entry.is_unstaged(),
            Changed::Any => entry.is_staged() || entry.is_unstaged(),
            Changed::PartlyStaged => entry.is_staged() && entry.is_unstaged(),
        })
        .map(|entry| entry.path)
        .collect())
}

/// The current branch's short name, or `None` for a detached HEAD.
pub(crate) async fn current_branch(reading: &Reading) -> Result<Option<String>, GitFailure> {
    let ran = reading
        .git
        .run(reading.run(["symbolic-ref", "--quiet", "--short", "HEAD"]))
        .await?;
    Ok(ran.succeeded().then(|| ran.text().trim().to_owned()))
}

/// The commit of the local branch `name`, or `None` when it does not exist.
pub(crate) async fn branch_tip(
    reading: &Reading,
    name: &str,
) -> Result<Option<String>, GitFailure> {
    let ran = reading
        .git
        .run(reading.run([
            "rev-parse".to_owned(),
            "--verify".to_owned(),
            "--quiet".to_owned(),
            format!("refs/heads/{name}^{{commit}}"),
        ]))
        .await?;
    let oid = ran.text().trim().to_owned();
    Ok((ran.succeeded() && is_object_id(&oid)).then_some(oid))
}

/// Checks a branch's name as git checks a new branch's, which the argument
/// `argument` gave.
pub(crate) async fn check_branch_name(
    reading: &Reading,
    argument: &str,
    name: &str,
) -> Result<(), GitFailure> {
    check_branch(argument, name)?;
    let ran = reading
        .git
        .run(reading.run(["check-ref-format", "--branch", name]))
        .await?;
    if ran.succeeded() {
        Ok(())
    } else {
        Err(GitFailure::InvalidArgument {
            detail: format!("{argument} {name:?} is not a valid branch name."),
        })
    }
}

/// The locks that run one change at a time in a repository, by its shared
/// git folder, so the plugin's own changes never race each other.
#[derive(Debug, Clone, Default)]
pub(crate) struct RepositoryLocks {
    locks: Arc<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>>,
}

impl RepositoryLocks {
    /// Waits for the repository whose shared git folder is `common_dir`.
    pub(crate) async fn lock(&self, common_dir: &Path) -> OwnedMutexGuard<()> {
        let key = comparable(common_dir);
        let lock = {
            let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
            Arc::clone(locks.entry(key).or_default())
        };
        lock.lock_owned().await
    }
}

/// The SDK's tool of a change tool.
pub(crate) struct ChangingTool<T> {
    git: Git,
    locks: RepositoryLocks,
    changes: Changes,
    kind: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for ChangingTool<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChangingTool")
            .finish_non_exhaustive()
    }
}

impl<T: ChangeTool> ChangingTool<T> {
    pub(crate) fn new(git: Git, locks: RepositoryLocks, changes: Changes) -> Self {
        Self {
            git,
            locks,
            changes,
            kind: PhantomData,
        }
    }
}

impl<T: ChangeTool> Tool for ChangingTool<T> {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(T::NAME, T::DESCRIPTION, parameters::<T::Arguments>())
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let git = self.git.clone();
        Box::pin(async move {
            prepare::<T>(&git, &call)
                .await
                .map_err(|failure| failure.refusal())
        })
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let git = self.git.clone();
        let locks = self.locks.clone();
        let changes = self.changes.clone();
        Box::pin(async move {
            let executed = execute::<T>(&git, &locks, &execution).await;
            // A change that failed may still have moved something, such as
            // an operation that stopped; the panels read again either way.
            changes.notify();
            let answer = match executed {
                Ok(answer) => answer,
                Err(failure) => return failure.result(),
            };
            let room = Room {
                limit: usize::try_from(execution.result_limit).unwrap_or(usize::MAX),
                archive_ids: &execution.archive_ids,
            };
            let result = fitted(answer.text, room);
            match answer.data {
                Some(data) => result.with_data(data),
                None => result,
            }
        })
    }
}

/// The accesses of a change: the repository's folders and, when the change
/// does anything beyond the workspace, the run of git that lists it.
async fn prepare<T: ChangeTool>(git: &Git, call: &ToolCall) -> Result<Vec<ToolAccess>, GitFailure> {
    let arguments: T::Arguments = read_arguments(&call.arguments)?;
    let folder = repository_folder(call.workspace.as_deref(), T::repository(&arguments))?;
    let layout = Layout::discover(git, &folder, None).await?;
    let reading = Reading::open(git, layout, PREPARATION_ROOM, None).await?;
    let plan = T::plan(&reading, &arguments).await?;
    accesses(&reading, &plan).await
}

/// Plans the change again with the repository locked, and makes it when
/// nothing moved since the owner or the policy allowed it.
async fn execute<T: ChangeTool>(
    git: &Git,
    locks: &RepositoryLocks,
    execution: &ToolExecution,
) -> Result<Answer, GitFailure> {
    let arguments: T::Arguments = read_arguments(&execution.arguments)?;
    let layout = authorized_layout(git, &execution.authorized).await?;
    let _locked = locks.lock(&layout.common_dir).await;
    let room = usize::try_from(execution.result_limit).unwrap_or(usize::MAX);
    let reading = Reading::open(git, layout, room, None).await?;
    let plan = T::plan(&reading, &arguments).await?;
    let planned = accesses(&reading, &plan).await?;
    if !same_accesses(&planned, &execution.authorized) {
        tracing::info!(
            tool = T::NAME,
            "a git change found the repository changed since it was allowed"
        );
        return Err(GitFailure::RepositoryChanged);
    }
    T::act(&Acting { reading }, &arguments, plan).await
}

/// The accesses of a plan.
pub(crate) async fn accesses<P>(
    reading: &Reading,
    plan: &Plan<P>,
) -> Result<Vec<ToolAccess>, GitFailure> {
    let state = repository_state(reading, &plan.tips).await?;
    let layout = &reading.layout;
    let mut accesses = vec![ToolAccess::Path(
        PathAccess::new(&layout.top, AccessMode::Write).with_state(state.clone()),
    )];
    for folder in layout.read_folders().into_iter().skip(1) {
        // The repository's git folders change too; the object folders that
        // it borrows only give objects.
        let is_git_folder =
            is_inside(&layout.git_dir, &folder) || is_inside(&layout.common_dir, &folder);
        let mode = if is_git_folder {
            AccessMode::Write
        } else {
            AccessMode::Read
        };
        accesses.push(ToolAccess::Path(PathAccess::new(folder, mode)));
    }
    let mut effects = hooks(reading, plan.change).await?;
    effects.extend(configured_programs(&reading.settings, plan.change));
    effects.extend(plan.effects.iter().cloned());
    if !effects.is_empty() {
        let mut run = ProgramAccess::new(GIT, &layout.top, plan.command.clone()).with_state(state);
        for effect in effects {
            run = run.with_effect(effect);
        }
        accesses.push(ToolAccess::Program(run));
    }
    Ok(accesses)
}

/// What the plugin reads of the repository's state, which the execution
/// reads again: HEAD, its branch, the index's entries, the operation in
/// progress and the commits of the other refs that the change works with.
/// The index counts by its entries and not by its file's time, which the
/// owner's editor refreshes on its own.
async fn repository_state(reading: &Reading, tips: &[String]) -> Result<String, GitFailure> {
    let head = reading
        .git
        .run(reading.run(["rev-parse", "--verify", "--quiet", "HEAD"]))
        .await?;
    let head = if head.succeeded() {
        head.text().trim().to_owned()
    } else {
        "unborn".to_owned()
    };
    let branch = reading
        .git
        .run(reading.run(["symbolic-ref", "--quiet", "HEAD"]))
        .await?;
    let branch = if branch.succeeded() {
        branch.text().trim().to_owned()
    } else {
        "detached".to_owned()
    };
    let index = reading
        .git
        .output(
            reading
                .run(["ls-files", "--stage", "-z"])
                .with_output_limit(16 * 1024 * 1024),
        )
        .await?;
    let operation =
        Operation::read(&reading.layout.git_dir).map_or("none", |operation| operation.kind.name());
    Ok(format!(
        "HEAD {head}; branch {branch}; index {:016x}; operation {operation}; tips {}",
        fnv1a(&index.output),
        tips.join(" ")
    ))
}

/// The 64-bit FNV-1a hash of `bytes`, which names the index's entries in the
/// state without keeping them.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

/// An access with its paths in the form of the core's canonical paths.
fn normalized(access: &ToolAccess) -> ToolAccess {
    match access {
        ToolAccess::Path(path) => {
            let mut normal = PathAccess::new(comparable(&path.path), path.mode);
            if let Some(state) = &path.state {
                normal = normal.with_state(state.clone());
            }
            ToolAccess::Path(normal)
        }
        ToolAccess::Program(run) => {
            let mut normal = ProgramAccess::new(
                run.name.clone(),
                comparable(&run.working_directory),
                run.arguments.clone(),
            );
            for effect in &run.effects {
                normal = normal.with_effect(effect.clone());
            }
            if let Some(state) = &run.state {
                normal = normal.with_state(state.clone());
            }
            ToolAccess::Program(normal)
        }
        ToolAccess::RemoteCommand(_) | ToolAccess::ExternalTool(_) => access.clone(),
    }
}

/// Whether the accesses that a plan gives now are the ones that were
/// allowed.
fn same_accesses(planned: &[ToolAccess], authorized: &[ToolAccess]) -> bool {
    let planned: Vec<ToolAccess> = planned.iter().map(normalized).collect();
    let authorized: Vec<ToolAccess> = authorized.iter().map(normalized).collect();
    planned.len() == authorized.len() && planned.iter().all(|access| authorized.contains(access))
}

#[cfg(test)]
mod coverage;
#[cfg(test)]
mod scenarios;

/// What the change tools' tests share: a call prepared and executed as the
/// core runs an allowed call.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Arc;

    use mannie_git_testing::{HostGit, TestRepository};
    use mannie_plugin_sdk::contracts::{ConversationId, ProgramEffect};
    use mannie_plugin_sdk::points::tools::{Tool, ToolAccess, ToolCall, ToolExecution, ToolResult};

    use super::{ChangeTool, ChangingTool, RepositoryLocks};
    use crate::git::Git;
    use crate::panel::Changes;

    /// A call that was prepared: its accesses, or its refusal as a result.
    pub(crate) struct Prepared<T: ChangeTool> {
        tool: ChangingTool<T>,
        arguments: String,
        workspace: Option<std::path::PathBuf>,
        pub(crate) accesses: Result<Vec<ToolAccess>, ToolResult>,
    }

    fn conversation() -> ConversationId {
        ConversationId::parse("0199e0f2-0000-7000-8000-000000000004").unwrap()
    }

    impl<T: ChangeTool> Prepared<T> {
        /// The effects of the call's run of git, none without one.
        pub(crate) fn effects(&self) -> Vec<ProgramEffect> {
            self.accesses
                .iter()
                .flatten()
                .filter_map(|access| match access {
                    ToolAccess::Program(run) => Some(run.effects.clone()),
                    ToolAccess::Path(_)
                    | ToolAccess::RemoteCommand(_)
                    | ToolAccess::ExternalTool(_) => None,
                })
                .flatten()
                .collect()
        }

        /// The names of the hooks and programs that the call lists.
        pub(crate) fn folder_code(&self) -> Vec<String> {
            self.effects()
                .into_iter()
                .filter_map(|effect| match effect {
                    ProgramEffect::RunsFolderCode { name, .. } => Some(name),
                    _other => None,
                })
                .collect()
        }

        /// Executes the call with its accesses allowed.
        pub(crate) async fn execute(self) -> ToolResult {
            let accesses = match self.accesses {
                Ok(accesses) => accesses,
                Err(refusal) => return refusal,
            };
            let execution = ToolExecution::new(self.arguments, conversation())
                .with_workspace(self.workspace)
                .with_authorized(accesses)
                .with_result_limit(64 * 1024);
            self.tool.execute(execution).await
        }
    }

    /// Prepares a call of `T` in the repository's workspace.
    pub(crate) async fn prepare<T: ChangeTool>(
        repository: &TestRepository,
        arguments: serde_json::Value,
    ) -> Prepared<T> {
        let tool = ChangingTool::<T>::new(
            Git::new(Arc::new(HostGit::new())),
            RepositoryLocks::default(),
            Changes::new(),
        );
        let workspace = Some(repository.path().to_path_buf());
        let arguments = arguments.to_string();
        let call =
            ToolCall::new(arguments.clone(), conversation()).with_workspace(workspace.clone());
        let accesses = tool.prepare(call).await.map_err(|refusal| {
            let result = ToolResult::new(refusal.detail);
            match refusal.code {
                Some(code) => result.with_failure(code),
                None => result,
            }
        });
        Prepared {
            tool,
            arguments,
            workspace,
            accesses,
        }
    }

    /// Prepares and executes a call of `T`, as the core runs an allowed call.
    pub(crate) async fn change<T: ChangeTool>(
        repository: &TestRepository,
        arguments: serde_json::Value,
    ) -> ToolResult {
        prepare::<T>(repository, arguments).await.execute().await
    }
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::{AccessMode, ProgramEffect};
    use mannie_plugin_sdk::points::tools::{PathAccess, ProgramAccess, ToolAccess};

    use super::{fnv1a, same_accesses};

    #[test]
    fn the_hash_of_the_index_is_fnv1a() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(fnv1a(b"ab"), fnv1a(b"ba"));
    }

    #[test]
    fn accesses_match_only_with_the_same_state_and_effects() {
        let folder = mannie_git_testing::TempFolder::new();
        let write = |state: &str| {
            ToolAccess::Path(PathAccess::new(folder.path(), AccessMode::Write).with_state(state))
        };
        let run = |source: &str| {
            ToolAccess::Program(
                ProgramAccess::new("git", folder.path(), vec!["commit".to_owned()]).with_effect(
                    ProgramEffect::RunsFolderCode {
                        name: "pre-commit".to_owned(),
                        source: source.to_owned(),
                    },
                ),
            )
        };

        assert!(same_accesses(
            &[write("HEAD a"), run(".git/hooks/pre-commit")],
            &[run(".git/hooks/pre-commit"), write("HEAD a")]
        ));
        assert!(!same_accesses(&[write("HEAD b")], &[write("HEAD a")]));
        assert!(!same_accesses(
            &[write("HEAD a"), run("hook.lint")],
            &[write("HEAD a"), run(".git/hooks/pre-commit")]
        ));
        assert!(!same_accesses(
            &[write("HEAD a"), run(".git/hooks/pre-commit")],
            &[write("HEAD a")]
        ));
    }
}
