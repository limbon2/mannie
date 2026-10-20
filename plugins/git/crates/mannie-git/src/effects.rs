//! What a change does beyond the workspace (plugins/git/docs/approvals.md;
//! ADR-0038, decision 5): the hooks that it starts, the programs that the
//! repository's own configuration names and it runs, the work that it
//! throws away and the published commits that it rewrites. The owner decides
//! on each, unless a grant, a rule or full access covers it.
//!
//! A change lists the hooks of every event that it may start, which a test
//! with a recording hook for every client hook checks against what git runs.
//! Listing a hook that does not run only asks once more; missing one would
//! run the repository's code without the owner's word (T-44).

use std::path::Path;

use mannie_plugin_sdk::contracts::{DiscardedWork, MAX_EFFECT_ITEMS, ProgramEffect};

use crate::config::Setting;
use crate::describe::CommitLabel;
use crate::failure::GitFailure;
use crate::layout::is_inside;
use crate::tools::Reading;
use crate::version::Version;

/// The events of the hooks that make a commit.
const COMMIT_EVENTS: [&str; 4] = [
    "pre-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
];

/// What a change does, which names the hooks and the programs of the
/// repository's configuration that it may start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    /// Stages files, which reads them through the clean filters.
    Stage,
    /// Takes files out of the index, keeping a snapshot first when it throws
    /// staged versions away.
    Unstage { snapshot: bool },
    /// Commits, after staging every tracked file when `all` says so.
    Commit { amend: bool, all: bool },
    /// Creates, renames or deletes a branch, or moves a ref.
    MoveRef,
    /// Writes the configuration only, such as a branch's upstream.
    Configure,
    /// Switches the work tree to another branch or commit.
    Switch,
    /// Restores files of the work tree or the index.
    Restore { snapshot: bool },
    /// Moves the current branch, `index` when the index changes too, and
    /// `work_tree` when the work tree does.
    Reset {
        index: bool,
        work_tree: bool,
        snapshot: bool,
    },
    /// Saves, applies or drops a stash entry.
    Stash { applies: bool },
    /// Merges, rebases, cherry-picks or reverts, or continues or skips such
    /// an operation that stopped.
    Integrate,
    /// Aborts an operation that stopped.
    Abort,
    /// Writes a resolved file to the index and the work tree.
    Resolve,
    /// Builds commits in memory and moves the current branch to them.
    EditHistory,
    /// Fetches from a remote, and merges or rebases the current branch onto
    /// what it fetched when `integrates` says so.
    Fetch { integrates: bool },
    /// Pushes to a remote.
    Push,
}

impl Change {
    /// The events of the hooks that the change may start.
    pub(crate) fn events(self) -> Vec<&'static str> {
        let mut events: Vec<&'static str> = match self {
            Self::Stage | Self::Unstage { .. } | Self::Resolve => vec!["post-index-change"],
            Self::Commit { amend, .. } => {
                let mut events = COMMIT_EVENTS.to_vec();
                events.extend(["post-index-change", "reference-transaction"]);
                if amend {
                    events.push("post-rewrite");
                }
                events
            }
            Self::MoveRef | Self::Fetch { integrates: false } => vec!["reference-transaction"],
            Self::Configure => Vec::new(),
            Self::Switch | Self::Abort => {
                vec![
                    "post-checkout",
                    "post-index-change",
                    "reference-transaction",
                ]
            }
            Self::Restore { .. } => vec!["post-checkout", "post-index-change"],
            Self::Reset { index, .. } => {
                let mut events = vec!["reference-transaction"];
                if index {
                    events.push("post-index-change");
                }
                events
            }
            Self::Stash { .. } | Self::EditHistory => {
                vec!["post-index-change", "reference-transaction"]
            }
            Self::Integrate | Self::Fetch { integrates: true } => {
                let mut events = COMMIT_EVENTS.to_vec();
                events.extend([
                    "pre-merge-commit",
                    "post-merge",
                    "pre-rebase",
                    "post-checkout",
                    "post-rewrite",
                    "post-index-change",
                    "reference-transaction",
                ]);
                events
            }
            Self::Push => vec!["pre-push", "reference-transaction"],
        };
        // A snapshot stores a stash entry, which moves `refs/stash`.
        if self.snapshots() && !events.contains(&"reference-transaction") {
            events.push("reference-transaction");
        }
        events
    }

    /// Whether the change keeps a snapshot of uncommitted changes first.
    fn snapshots(self) -> bool {
        matches!(
            self,
            Self::Unstage { snapshot: true }
                | Self::Restore { snapshot: true }
                | Self::Reset { snapshot: true, .. }
        )
    }

    /// Whether the change reads or writes files of the work tree, which runs
    /// the filters that their attributes name.
    fn runs_filters(self) -> bool {
        match self {
            Self::MoveRef | Self::Configure | Self::Push => false,
            Self::Unstage { snapshot } => snapshot,
            Self::Commit { all, .. } => all,
            Self::Reset {
                work_tree,
                snapshot,
                ..
            } => work_tree || snapshot,
            Self::Fetch { integrates } => integrates,
            Self::Stage
            | Self::Switch
            | Self::Restore { .. }
            | Self::Stash { .. }
            | Self::Integrate
            | Self::Abort
            | Self::Resolve
            | Self::EditHistory => true,
        }
    }

    /// Whether the change merges content, which runs the merge drivers that
    /// attributes name.
    fn merges(self) -> bool {
        matches!(
            self,
            Self::Integrate
                | Self::Stash { applies: true }
                | Self::EditHistory
                | Self::Fetch { integrates: true }
        )
    }

    /// Whether the change makes commits, which may sign them.
    fn commits(self) -> bool {
        matches!(
            self,
            Self::Commit { .. }
                | Self::Integrate
                | Self::EditHistory
                | Self::Fetch { integrates: true }
        )
    }

    /// Whether the change reaches a remote, which runs the programs that
    /// connect and authenticate.
    fn reaches_remote(self) -> bool {
        matches!(self, Self::Fetch { .. } | Self::Push)
    }
}

/// What a program that a setting names does, which decides when it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgramKind {
    Filter,
    MergeDriver,
    Signing,
    /// A program that connects to a remote or answers for credentials, such
    /// as `core.sshCommand`, a credential helper, or the program that a
    /// remote runs for a local transport.
    Transport,
}

/// The settings that name a program that reaching a remote runs: by key,
/// and by the name of a subsection's key.
const TRANSPORT_KEYS: [&str; 4] = [
    "core.sshcommand",
    "core.askpass",
    "core.gitproxy",
    "credential.helper",
];

/// The names of the keys of a remote's subsection that name programs.
const REMOTE_PROGRAMS: [&str; 3] = ["uploadpack", "receivepack", "vcs"];

/// The kind of program that a setting's key names, such as a filter for
/// `filter.lfs.smudge`, or `None` for a setting that names none.
fn program_kind(setting: &Setting) -> Option<ProgramKind> {
    if let Some((_driver, name)) = setting.subsection_of("filter") {
        return matches!(name, "clean" | "smudge" | "process").then_some(ProgramKind::Filter);
    }
    if let Some((_driver, name)) = setting.subsection_of("merge") {
        return (name == "driver").then_some(ProgramKind::MergeDriver);
    }
    if let Some((_remote, name)) = setting.subsection_of("remote") {
        return REMOTE_PROGRAMS
            .contains(&name)
            .then_some(ProgramKind::Transport);
    }
    if let Some((_url, name)) = setting.subsection_of("credential") {
        return (name == "helper").then_some(ProgramKind::Transport);
    }
    let key = setting.key.as_str();
    if TRANSPORT_KEYS.contains(&key) {
        return Some(ProgramKind::Transport);
    }
    let is_signing = key.starts_with("gpg.")
        && (key.ends_with(".program") || key.ends_with(".defaultkeycommand"));
    is_signing.then_some(ProgramKind::Signing)
}

/// The programs that the repository's own configuration names and that the
/// change may run: filters, merge drivers, signing programs, and the
/// programs that reach remotes. The owner's global and system configuration
/// is the owner's own, so its programs do not count.
pub(crate) fn configured_programs(settings: &[Setting], change: Change) -> Vec<ProgramEffect> {
    let mut programs: Vec<ProgramEffect> = Vec::new();
    for setting in settings
        .iter()
        .filter(|setting| setting.scope.is_repository())
    {
        let runs = match program_kind(setting) {
            Some(ProgramKind::Filter) => change.runs_filters(),
            Some(ProgramKind::MergeDriver) => change.merges(),
            Some(ProgramKind::Signing) => change.commits(),
            Some(ProgramKind::Transport) => change.reaches_remote(),
            None => false,
        };
        let names_a_program = setting
            .value
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        if runs && names_a_program {
            let effect = ProgramEffect::RunsFolderCode {
                name: setting.key.clone(),
                source: format!("the repository's {} configuration", setting.scope.name()),
            };
            if !programs.contains(&effect) {
                programs.push(effect);
            }
        }
    }
    programs
}

/// The hooks that the change may start: from `git hook list` on git 2.54
/// and later, which also names the hooks that configuration defines, or
/// from the hooks folder before.
pub(crate) async fn hooks(
    reading: &Reading,
    change: Change,
) -> Result<Vec<ProgramEffect>, GitFailure> {
    let events = change.events();
    if events.is_empty() {
        return Ok(Vec::new());
    }
    let folder = hooks_folder(reading).await?;
    let version = reading.git.version(&reading.layout.top).await?;
    let mut hooks = Vec::new();
    for event in events {
        if version >= Version::HOOK_LIST {
            hooks.extend(listed_hooks(reading, event, &folder).await?);
        } else if let Some(source) = folder_hook(reading, &folder, event) {
            hooks.push(ProgramEffect::RunsFolderCode {
                name: event.to_owned(),
                source,
            });
        }
    }
    Ok(hooks)
}

/// The folder of the repository's hooks, `core.hooksPath` or the shared git
/// folder's `hooks`, as an absolute path.
async fn hooks_folder(reading: &Reading) -> Result<std::path::PathBuf, GitFailure> {
    let ran = reading
        .git
        .output(reading.run(["rev-parse", "--path-format=absolute", "--git-path", "hooks"]))
        .await?;
    Ok(std::path::PathBuf::from(
        ran.text().trim_end_matches(['\r', '\n']),
    ))
}

/// Where a hook of the folder comes from, as the owner reads it: relative to
/// the repository's top when it lies inside, else absolute.
fn hook_source(reading: &Reading, path: &Path) -> String {
    let top = &reading.layout.top;
    if is_inside(path, top) {
        let relative = path.components().skip(top.components().count());
        let parts: Vec<String> = relative
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect();
        return parts.join("/");
    }
    path.display().to_string().replace('\\', "/")
}

/// The hook of `event` in the hooks folder, or `None`. On Windows, git also
/// runs `<event>.exe`.
fn folder_hook(reading: &Reading, folder: &Path, event: &str) -> Option<String> {
    let mut names = vec![event.to_owned()];
    if cfg!(windows) {
        names.push(format!("{event}.exe"));
    }
    names
        .into_iter()
        .map(|name| folder.join(name))
        .find(|path| is_runnable_hook(path))
        .map(|path| hook_source(reading, &path))
}

/// Whether git runs the file at `path` as a hook, as its `find_hook` decides:
/// a file, which on Unix must also be executable, since git skips a hook
/// without its executable bit there.
fn is_runnable_hook(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && is_executable(&metadata))
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    // git runs any file of the hooks folder on Windows.
    true
}

/// The hooks that `git hook list` names for `event`: each hook of the
/// configuration with its scope, and the hooks folder's, which it lists
/// without a scope.
async fn listed_hooks(
    reading: &Reading,
    event: &str,
    folder: &Path,
) -> Result<Vec<ProgramEffect>, GitFailure> {
    let ran = reading
        .git
        .run(reading.run(["hook", "list", "-z", "--show-scope", event]))
        .await?;
    // git exits with 1 when no hook runs for the event.
    if !ran.succeeded() {
        return Ok(Vec::new());
    }
    let text = ran.text();
    let mut hooks = Vec::new();
    for entry in text.split('\0').filter(|entry| !entry.trim().is_empty()) {
        let source = match entry.split_once('\t') {
            Some((scope, name)) => format!("hook.{name} in the {scope} configuration"),
            None => folder_hook(reading, folder, event)
                .unwrap_or_else(|| hook_source(reading, &folder.join(event))),
        };
        hooks.push(ProgramEffect::RunsFolderCode {
            name: event.to_owned(),
            source,
        });
    }
    Ok(hooks)
}

/// The URLs that a remote reaches, after `insteadOf` and, for a push,
/// `pushInsteadOf`, without credentials (T-47, T-49), as the endpoints of
/// the run.
pub(crate) async fn remote_endpoints(
    reading: &Reading,
    remote: &str,
    push: bool,
) -> Result<Vec<ProgramEffect>, GitFailure> {
    let mut command = vec!["remote", "get-url", "--all"];
    if push {
        command.push("--push");
    }
    command.push(remote);
    let ran = reading.git.run(reading.run(command)).await?;
    if !ran.succeeded() {
        return Err(GitFailure::InvalidArgument {
            detail: format!(
                "The repository has no remote named {remote}. git_refs lists its remotes."
            ),
        });
    }
    let mut endpoints: Vec<ProgramEffect> = Vec::new();
    for url in ran.text().lines().filter(|url| !url.trim().is_empty()) {
        let effect = ProgramEffect::Network {
            endpoint: crate::redact::redact(url.trim()),
        };
        if !endpoints.contains(&effect) {
            endpoints.push(effect);
        }
    }
    Ok(endpoints)
}

/// Uncommitted changes that a run throws away: how many files, and the
/// first of them, with whether a snapshot keeps them first.
pub(crate) fn discarded_changes(paths: &[String], snapshot: bool) -> Option<ProgramEffect> {
    (!paths.is_empty()).then(|| ProgramEffect::DiscardsWork {
        work: DiscardedWork::UncommittedChanges,
        count: u32::try_from(paths.len()).unwrap_or(u32::MAX),
        items: paths.iter().take(MAX_EFFECT_ITEMS).cloned().collect(),
        snapshot,
    })
}

/// Commits that a run leaves on no branch or tag.
pub(crate) fn discarded_commits(commits: &[CommitLabel]) -> Option<ProgramEffect> {
    (!commits.is_empty()).then(|| ProgramEffect::DiscardsWork {
        work: DiscardedWork::Commits,
        count: u32::try_from(commits.len()).unwrap_or(u32::MAX),
        items: commits
            .iter()
            .take(MAX_EFFECT_ITEMS)
            .map(CommitLabel::text)
            .collect(),
        snapshot: false,
    })
}

/// The commits of `tip` that no other branch, tag or remote-tracking branch
/// holds, besides `except`, a branch's short name that the run moves or
/// deletes, and besides `kept`, a commit that the run keeps.
pub(crate) async fn unkept_commits(
    reading: &Reading,
    tip: &str,
    except: Option<&str>,
    kept: Option<&str>,
) -> Result<Vec<String>, GitFailure> {
    let mut command = vec!["rev-list".to_owned(), tip.to_owned(), "--not".to_owned()];
    if let Some(kept) = kept {
        command.push(kept.to_owned());
    }
    if let Some(except) = except {
        // A pattern of --exclude applies to the next --branches, without
        // its refs/heads/.
        command.push(format!("--exclude={except}"));
    }
    command.extend([
        "--branches".to_owned(),
        "--tags".to_owned(),
        "--remotes".to_owned(),
    ]);
    let ran = reading.git.output(reading.run(command)).await?;
    Ok(ran.text().lines().map(str::to_owned).collect())
}

/// The commits of `range` that remote-tracking branches hold, which a run
/// rewrites, with the branches that hold them. `range` is what `git
/// rev-list` reads, such as `<base>..HEAD`, or `<commit>^!` for one commit.
pub(crate) async fn published(
    reading: &Reading,
    range: &str,
) -> Result<Option<ProgramEffect>, GitFailure> {
    let listed = |extra: &[&str]| {
        let mut command = vec!["rev-list".to_owned(), range.to_owned()];
        command.extend(extra.iter().map(|part| (*part).to_owned()));
        reading.run(command)
    };
    let all = reading.git.output(listed(&[])).await?.text();
    let unpublished = reading
        .git
        .output(listed(&["--not", "--remotes"]))
        .await?
        .text();
    let unpublished: Vec<&str> = unpublished.lines().collect();
    let held: Vec<&str> = all
        .lines()
        .filter(|commit| !unpublished.contains(commit))
        .collect();
    if held.is_empty() {
        return Ok(None);
    }
    let mut command = vec![
        "for-each-ref".to_owned(),
        "--format=%(refname:short)".to_owned(),
    ];
    // A ref that holds any of them counts.
    command.extend(
        held.iter()
            .take(MAX_EFFECT_ITEMS)
            .map(|commit| format!("--contains={commit}")),
    );
    command.push("refs/remotes".to_owned());
    let places = reading.git.output(reading.run(command)).await?;
    Ok(Some(ProgramEffect::RewritesPublished {
        count: u32::try_from(held.len()).unwrap_or(u32::MAX),
        places: places
            .text()
            .lines()
            .take(MAX_EFFECT_ITEMS)
            .map(str::to_owned)
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::ProgramEffect;

    use super::{Change, configured_programs};
    use crate::config::parse_settings;

    #[test]
    fn every_change_that_moves_a_ref_or_writes_the_index_lists_their_hooks() {
        let commit = Change::Commit {
            amend: true,
            all: false,
        };

        assert!(commit.events().contains(&"post-rewrite"));
        assert!(commit.events().contains(&"pre-commit"));
        assert_eq!(Change::MoveRef.events(), ["reference-transaction"]);
        assert_eq!(Change::Configure.events(), Vec::<&str>::new());
        assert!(
            Change::Restore { snapshot: true }
                .events()
                .contains(&"reference-transaction"),
            "a snapshot stores a stash entry"
        );
        assert!(
            !Change::Restore { snapshot: false }
                .events()
                .contains(&"reference-transaction")
        );
        assert!(Change::Integrate.events().contains(&"pre-rebase"));
    }

    #[test]
    fn only_the_repository_s_own_programs_count_and_only_where_the_change_runs_them() {
        let settings = parse_settings(
            b"global\0filter.lfs.smudge\ngit-lfs smudge\0local\0filter.planted.clean\nrun %f\0\
local\0merge.ours.driver\ntrue\0local\0gpg.program\nsign-it\0local\0filter.blank.clean\n\0\
worktree\0gpg.ssh.defaultkeycommand\nkey-it\0local\0core.editor\nvim\0",
        );

        let stage = configured_programs(&settings, Change::Stage);
        let commit = configured_programs(
            &settings,
            Change::Commit {
                amend: false,
                all: false,
            },
        );
        let merge = configured_programs(&settings, Change::Integrate);

        assert_eq!(
            stage,
            [ProgramEffect::RunsFolderCode {
                name: "filter.planted.clean".to_owned(),
                source: "the repository's local configuration".to_owned(),
            }]
        );
        let names = |effects: &[ProgramEffect]| -> Vec<String> {
            effects
                .iter()
                .map(|effect| match effect {
                    ProgramEffect::RunsFolderCode { name, .. } => name.clone(),
                    other => format!("{other:?}"),
                })
                .collect()
        };
        assert_eq!(names(&commit), ["gpg.program", "gpg.ssh.defaultkeycommand"]);
        assert_eq!(
            names(&merge),
            [
                "filter.planted.clean",
                "merge.ours.driver",
                "gpg.program",
                "gpg.ssh.defaultkeycommand"
            ]
        );
    }
}
