//! What the git tools share (plugins/git/docs/tools.md, every tool;
//! ADR-0038, decisions 3 and 9).
//!
//! A tool that reads checks its arguments and finds the repository in its
//! preparation, which only reads, and returns the reads of the repository's
//! folders, which the core's policy decides as it decides `read_file`'s:
//! inside the workspace they run, outside they ask. Its execution finds the
//! repository again from the folder that the owner allowed and refuses to
//! read when the repository's folders no longer lie inside the allowed ones.
//! Every read passes `--no-optional-locks` and blanks the filters that the
//! repository's own configuration defines (T-44).
//!
//! A result's text fits the room that the core gives it, the rest archived
//! for `read_tool_output`, and its card's data travels beside it (ADR-0039).

pub(crate) mod apply;
pub(crate) mod blame;
pub(crate) mod branch;
pub(crate) mod commit;
pub(crate) mod conflicts;
pub(crate) mod diff;
pub(crate) mod edit_history;
pub(crate) mod fetch;
pub(crate) mod grep;
pub(crate) mod log;
pub(crate) mod merge;
pub(crate) mod operation;
pub(crate) mod push;
pub(crate) mod rebase;
pub(crate) mod refs;
pub(crate) mod reset;
pub(crate) mod resolve;
pub(crate) mod restore;
pub(crate) mod show;
pub(crate) mod stage;
pub(crate) mod stash;
pub(crate) mod status;
pub(crate) mod switch;

use std::fmt;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::points::tools::{
    MAX_TOOL_DATA_BYTES, Tool, ToolAccess, ToolCall, ToolExecution, ToolRefusal, ToolResult,
    ToolSpec,
};
use mannie_plugin_sdk::services::processes::ReadPlace;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::arguments::{check_revision, is_object_id, pathspecs, repository_folder};
use crate::config::{Setting, blank_repository_filters, read_settings};
use crate::failure::GitFailure;
use crate::git::{Git, GitRun};
use crate::layout::{Layout, is_inside};
use crate::output::{Room, fitted};
use crate::schema::parameters;

/// What a reading tool works with: git, the repository, the arguments that
/// blank the repository's own filters, and the room of the result.
#[derive(Debug, Clone)]
pub(crate) struct Reading {
    pub(crate) git: Git,
    pub(crate) layout: Layout,
    /// Every setting that applies in the repository, with its scope.
    pub(crate) settings: Vec<Setting>,
    blank_filters: Vec<String>,
    /// The bytes of text that the model reads of the result, which a tool
    /// that pages fills before it says how to read on.
    pub(crate) room: usize,
    /// The place whose policy checks each run's reads, for a mention: the
    /// repository's folders as a model's reads there. None for a tool's
    /// call, which the policy decided on already.
    checks: Option<ReadPlace>,
}

impl Reading {
    /// Reads the repository's settings and starts reading it. `checks`
    /// checks every run, the first one too.
    pub(crate) async fn open(
        git: &Git,
        layout: Layout,
        room: usize,
        checks: Option<ReadPlace>,
    ) -> Result<Self, GitFailure> {
        let settings = read_settings(git, &layout.top, checks.clone()).await?;
        Ok(Self {
            git: git.clone(),
            blank_filters: blank_repository_filters(&settings),
            settings,
            layout,
            room,
            checks,
        })
    }

    /// A read run in the repository's top: the blanked filters,
    /// `--no-optional-locks`, then `arguments`.
    pub(crate) fn run<S: Into<String>>(&self, arguments: impl IntoIterator<Item = S>) -> GitRun {
        let mut all = self.blank_filters.clone();
        all.push("--no-optional-locks".to_owned());
        all.extend(arguments.into_iter().map(Into::into));
        GitRun::new(&self.layout.top, all).checked_as(self.checks.clone())
    }

    /// The full hash of the commit that `revision` names, which the argument
    /// `argument` gave. A tag names its commit.
    pub(crate) async fn commit(
        &self,
        argument: &str,
        revision: &str,
    ) -> Result<String, GitFailure> {
        check_revision(argument, revision)?;
        let ran = self
            .git
            .run(self.run([
                "rev-parse".to_owned(),
                "--verify".to_owned(),
                "--quiet".to_owned(),
                format!("{revision}^{{commit}}"),
            ]))
            .await?;
        let oid = ran.text().trim().to_owned();
        if ran.succeeded() && is_object_id(&oid) {
            Ok(oid)
        } else {
            Err(GitFailure::UnknownRevision {
                revision: revision.to_owned(),
            })
        }
    }

    /// A revision or a range of two of them, such as `main...feature`, with
    /// each side's commit named by its full hash; a side left empty is HEAD.
    pub(crate) async fn range(&self, argument: &str, text: &str) -> Result<String, GitFailure> {
        check_revision(argument, text)?;
        for separator in ["...", ".."] {
            if let Some((left, right)) = text.split_once(separator) {
                let left = self.commit(argument, or_head(left)).await?;
                let right = self.commit(argument, or_head(right)).await?;
                return Ok(format!("{left}{separator}{right}"));
            }
        }
        self.commit(argument, text).await
    }
}

/// `HEAD` for a side of a range that is left empty.
fn or_head(side: &str) -> &str {
    if side.is_empty() { "HEAD" } else { side }
}

/// What a tool answers: the text that the model reads, and its card's data.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Answer {
    pub(crate) text: String,
    pub(crate) data: Option<serde_json::Value>,
}

/// A git tool that only reads.
pub(crate) trait ReadTool: fmt::Debug + Send + Sync + 'static {
    /// The tool's arguments, whose documentation the model reads.
    type Arguments: DeserializeOwned + JsonSchema + Send + 'static;

    /// The tool's name.
    const NAME: &'static str;

    /// What the model reads about the tool.
    const DESCRIPTION: &'static str;

    /// The `repository` argument.
    fn repository(arguments: &Self::Arguments) -> Option<&str>;

    /// Checks the arguments beyond their shape, once the repository is
    /// found, before the call is decided on.
    fn check(arguments: &Self::Arguments, layout: &Layout) -> Result<(), GitFailure>;

    /// Answers a call that the core authorized.
    fn answer(
        reading: Reading,
        arguments: Self::Arguments,
    ) -> BoxFuture<'static, Result<Answer, GitFailure>>;
}

/// The SDK's tool of a reading tool.
pub(crate) struct ReadingTool<T> {
    git: Git,
    kind: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for ReadingTool<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadingTool")
            .finish_non_exhaustive()
    }
}

impl<T: ReadTool> ReadingTool<T> {
    pub(crate) fn new(git: Git) -> Self {
        Self {
            git,
            kind: PhantomData,
        }
    }
}

/// Reads a call's arguments.
pub(crate) fn read_arguments<A: DeserializeOwned>(text: &str) -> Result<A, GitFailure> {
    // A model that calls a tool without arguments may send nothing.
    let text = if text.trim().is_empty() || text.trim() == "null" {
        "{}"
    } else {
        text
    };
    serde_json::from_str(text).map_err(|error| GitFailure::InvalidArgument {
        detail: format!("The arguments are not valid: {error}."),
    })
}

impl<T: ReadTool> Tool for ReadingTool<T> {
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
        Box::pin(async move {
            let answer = match execute::<T>(&git, &execution).await {
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

/// The reads of a call: the repository's folders.
async fn prepare<T: ReadTool>(git: &Git, call: &ToolCall) -> Result<Vec<ToolAccess>, GitFailure> {
    let arguments: T::Arguments = read_arguments(&call.arguments)?;
    let folder = repository_folder(call.workspace.as_deref(), T::repository(&arguments))?;
    let layout = Layout::discover(git, &folder, None).await?;
    T::check(&arguments, &layout)?;
    Ok(layout.read_accesses())
}

/// Answers an authorized call.
async fn execute<T: ReadTool>(git: &Git, execution: &ToolExecution) -> Result<Answer, GitFailure> {
    let arguments: T::Arguments = read_arguments(&execution.arguments)?;
    let layout = authorized_layout(git, &execution.authorized).await?;
    T::check(&arguments, &layout)?;
    let room = usize::try_from(execution.result_limit).unwrap_or(usize::MAX);
    let reading = Reading::open(git, layout, room, None).await?;
    T::answer(reading, arguments).await
}

/// The repository that the authorized reads name, found again from the
/// first of them, its top, whose folders must all still lie inside them.
pub(crate) async fn authorized_layout(
    git: &Git,
    authorized: &[ToolAccess],
) -> Result<Layout, GitFailure> {
    let folders: Vec<PathBuf> = authorized
        .iter()
        .filter_map(|access| match access {
            ToolAccess::Path(path) => Some(path.path.clone()),
            ToolAccess::RemoteCommand(_) | ToolAccess::Program(_) => None,
        })
        .collect();
    let top = folders.first().ok_or(GitFailure::RepositoryChanged)?;
    let layout = Layout::discover(git, top, None).await?;
    let allowed: Vec<PathBuf> = folders.iter().map(|folder| comparable(folder)).collect();
    let stays_inside = layout.read_folders().iter().all(|folder| {
        let folder = comparable(folder);
        allowed.iter().any(|allowed| is_inside(&folder, allowed))
    });
    if stays_inside {
        Ok(layout)
    } else {
        Err(GitFailure::RepositoryChanged)
    }
}

/// A path in the form of the core's canonical paths, for comparing them:
/// its links resolved, without the verbatim prefix that Windows adds.
pub(crate) fn comparable(path: &Path) -> PathBuf {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_error| path.to_path_buf());
    let text = resolved.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => resolved.clone(),
    }
}

/// The pathspecs of a call's paths, or of the whole repository with `all`:
/// exactly one of them.
pub(crate) fn pathspecs_or_all(
    top: &Path,
    paths: Option<&[String]>,
    all: bool,
) -> Result<Vec<String>, GitFailure> {
    let paths = paths.unwrap_or_default();
    let refused = |detail: &str| GitFailure::InvalidArgument {
        detail: detail.to_owned(),
    };
    match (paths.is_empty(), all) {
        // The run's folder is the repository's top.
        (true, true) => Ok(vec![".".to_owned()]),
        (false, false) => pathspecs(top, "paths", paths),
        (true, false) => Err(refused("Pass paths, or all for every change.")),
        (false, true) => Err(refused("Pass paths or all, not both.")),
    }
}

/// A count of things in words, such as `1 file` or `3 files`.
pub(crate) fn count(number: usize, one: &str, many: &str) -> String {
    if number == 1 {
        format!("1 {one}")
    } else {
        format!("{number} {many}")
    }
}

/// The most characters of one line of a file, a match or a blame in a
/// result, as the core's `read_file` keeps them.
pub(crate) const MAX_LINE_CHARS: usize = 2000;

/// The bytes of a result's room that a tool that pages keeps for its note
/// of how to read on.
pub(crate) const NOTE_ROOM: usize = 400;

/// A line cut to `MAX_LINE_CHARS` characters, with a note of what it left
/// out.
pub(crate) fn cut_line(line: &str) -> String {
    let total = line.chars().count();
    if total <= MAX_LINE_CHARS {
        return line.to_owned();
    }
    let kept: String = line.chars().take(MAX_LINE_CHARS).collect();
    format!("{kept} [{} more characters]", total - MAX_LINE_CHARS)
}

/// The data of a card within the core's limit: what `build` makes of the
/// first `items` of its lists, halved until it fits, or `None` when even
/// none fit.
pub(crate) fn fitting_data<D: Serialize>(
    items: usize,
    build: impl Fn(usize) -> D,
) -> Option<serde_json::Value> {
    let mut kept = items;
    loop {
        let data = serde_json::to_value(build(kept)).ok()?;
        let size = serde_json::to_string(&data).map_or(usize::MAX, |text| text.len());
        if size <= MAX_TOOL_DATA_BYTES {
            return Some(data);
        }
        if kept == 0 {
            return None;
        }
        kept /= 2;
    }
}

/// What the tools' tests share: a call of a tool in a repository that the
/// test builds, prepared and executed as the core runs an allowed call.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Arc;

    use mannie_git_testing::{HostGit, TestRepository};
    use mannie_plugin_sdk::contracts::ConversationId;
    use mannie_plugin_sdk::points::tools::{Tool, ToolCall, ToolExecution, ToolResult};

    use super::{ReadTool, ReadingTool};
    use crate::git::Git;

    /// The room of a result unless a test names another: the core's
    /// largest.
    pub(crate) const ROOM: u32 = 64 * 1024;

    fn conversation() -> ConversationId {
        ConversationId::parse("0199e0f2-0000-7000-8000-000000000002").unwrap()
    }

    /// Calls `T` in the repository's workspace with `arguments`, with the
    /// accesses that its preparation returned allowed. A refusal comes back
    /// as a result with its code, as the core records it.
    pub(crate) async fn call<T: ReadTool>(
        repository: &TestRepository,
        arguments: serde_json::Value,
    ) -> ToolResult {
        call_with::<T>(&Arc::new(HostGit::new()), repository, arguments, ROOM).await
    }

    /// Calls `T` through `host`, in a result of `room` bytes.
    pub(crate) async fn call_with<T: ReadTool>(
        host: &Arc<HostGit>,
        repository: &TestRepository,
        arguments: serde_json::Value,
        room: u32,
    ) -> ToolResult {
        let tool = ReadingTool::<T>::new(Git::new(Arc::<HostGit>::clone(host)));
        let workspace = Some(repository.path().to_path_buf());
        let text = arguments.to_string();
        let prepared =
            ToolCall::new(text.clone(), conversation()).with_workspace(workspace.clone());
        let accesses = match tool.prepare(prepared).await {
            Ok(accesses) => accesses,
            Err(refusal) => {
                let result = ToolResult::new(refusal.detail);
                return match refusal.code {
                    Some(code) => result.with_failure(code),
                    None => result,
                };
            }
        };
        let execution = ToolExecution::new(text, conversation())
            .with_workspace(workspace)
            .with_authorized(accesses)
            .with_result_limit(room);
        tool.execute(execution).await
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use mannie_git_testing::{HostGit, TestRepository};
    use mannie_plugin_sdk::points::tools::ToolResult;
    use serde_json::json;

    use super::testing::{ROOM, call_with};
    use super::{comparable, count, read_arguments};
    use crate::failure::GitFailure;
    use crate::tools::blame::GitBlame;
    use crate::tools::diff::GitDiff;
    use crate::tools::grep::GitGrep;
    use crate::tools::log::GitLog;
    use crate::tools::show::GitShow;
    use crate::tools::status::GitStatus;

    #[tokio::test]
    async fn reads_start_none_of_the_programs_that_the_repository_s_configuration_names() {
        let repository = TestRepository::new();
        let marker = repository.sibling("markers").join("ran");
        repository.write(".gitattributes", "*.txt filter=planted diff=planted\n");
        repository.write("a.txt", "one\n");
        repository.commit("Start");
        for (key, then) in [
            ("filter.planted.clean", "cat"),
            ("filter.planted.smudge", "cat"),
            ("diff.planted.textconv", "cat"),
            ("diff.external", "true"),
            ("core.fsmonitor", "true"),
            ("core.pager", "cat"),
        ] {
            let command = TestRepository::marking_command(&marker, key, then);
            repository.git(&["config", key, &command]);
        }
        repository.write("a.txt", "one and two\n");
        // The planted programs do run under git's own reads.
        repository.git(&["status", "--porcelain"]);
        assert!(marker.exists(), "the planted filter runs under plain git");
        std::fs::remove_file(&marker).unwrap();

        let host = Arc::new(HostGit::new());
        let ran_nothing = |tool: &str, result: &ToolResult| {
            assert_eq!(result.failure, None, "{tool}: {}", result.output);
            let ran = std::fs::read_to_string(&marker).unwrap_or_default();
            let runs: Vec<Vec<String>> = host.runs().into_iter().map(|run| run.arguments).collect();
            assert!(
                ran.is_empty(),
                "{tool} ran planted programs: {ran}\nruns: {runs:#?}"
            );
        };
        let status = call_with::<GitStatus>(&host, &repository, json!({}), ROOM).await;
        ran_nothing("git_status", &status);
        let unstaged = call_with::<GitDiff>(&host, &repository, json!({}), ROOM).await;
        ran_nothing("git_diff", &unstaged);
        let from_head =
            call_with::<GitDiff>(&host, &repository, json!({ "from": "HEAD" }), ROOM).await;
        ran_nothing("git_diff from HEAD", &from_head);
        let show = call_with::<GitShow>(&host, &repository, json!({}), ROOM).await;
        ran_nothing("git_show", &show);
        let blame =
            call_with::<GitBlame>(&host, &repository, json!({ "path": "a.txt" }), ROOM).await;
        ran_nothing("git_blame", &blame);
        let lines = json!({ "lines": { "path": "a.txt", "start": 1, "end": 1 } });
        ran_nothing(
            "git_log",
            &call_with::<GitLog>(&host, &repository, lines, ROOM).await,
        );
        let grep =
            call_with::<GitGrep>(&host, &repository, json!({ "pattern": "two" }), ROOM).await;
        ran_nothing("git_grep", &grep);
        assert!(
            unstaged.output.contains("+one and two\n"),
            "{}",
            unstaged.output
        );
    }

    #[derive(Debug, serde::Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Arguments {
        repository: Option<String>,
    }

    #[test]
    fn arguments_may_be_missing_and_unknown_ones_are_refused() {
        assert_eq!(
            read_arguments::<Arguments>("").unwrap(),
            Arguments { repository: None }
        );
        assert_eq!(
            read_arguments::<Arguments>("null").unwrap(),
            Arguments { repository: None }
        );
        assert!(matches!(
            read_arguments::<Arguments>(r#"{"folder":"x"}"#),
            Err(GitFailure::InvalidArgument { .. })
        ));
    }

    #[test]
    fn a_comparable_path_has_no_verbatim_prefix() {
        let folder = mannie_git_testing::TempFolder::new();

        let compared = comparable(folder.path());

        assert!(!compared.to_string_lossy().starts_with(r"\\?\"));
        assert!(compared.is_absolute());
        assert_eq!(
            comparable(Path::new("/no/such/folder/x")),
            Path::new("/no/such/folder/x")
        );
    }

    #[test]
    fn counts_use_the_singular_for_one() {
        assert_eq!(count(1, "file", "files"), "1 file");
        assert_eq!(count(0, "file", "files"), "0 files");
    }
}
