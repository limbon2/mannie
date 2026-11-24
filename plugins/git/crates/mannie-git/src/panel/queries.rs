//! Read-only query handlers for the Git side panel: filtered topological
//! commit history, full refs/stashes/worktrees/remotes overview, stash
//! diffs, ref/worktree comparisons, file blame, repository tree search,
//! content grep, and untracked file diff synthesis.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use mannie_plugin_sdk::contracts::UtcMillis;

use crate::arguments::{pathspec, repository_path};
use crate::commands::{
    BlameRef, BlameRunView, BlameView, CompareMode, CompareRef, CompareView, FileDiff,
    FileTreeList, GitError, GrepQueryRef, GrepView, HISTORY_PAGE, HistoryQueryRef,
    HistoryQueryResult, HistoryScope, MAX_LISTED, MergeFilter, ProjectRef, RefsOverview,
    StashShowRef, StashView, TreeQueryRef,
};
use crate::describe::label_commits;
use crate::failure::GitFailure;
use crate::git::Git;
use crate::layout::is_inside;
use crate::outcome::head_commit;
use crate::parse::blame::{BlameRun, parse_blame};
use crate::parse::diff::{FileStat, Hunk, Line, LineKind, parse_hunks, parse_numstat, split_patch};
use crate::parse::grep::parse_grep;
use crate::parse::log::{Commit, LOG_FORMAT, REFLOG_FORMAT, parse_log, parse_reflog};
use crate::parse::refs::{REF_FORMAT, Ref, RefKind, parse_refs};
use crate::parse::remotes::{Remote, parse_remotes};
use crate::parse::worktrees::parse_worktrees;
use crate::tools::Reading;
use crate::tools::diff::PATCH_OPTIONS;
use crate::tools::show::FilePatch;

use super::{MAX_PATCHED_FILES, ROOM, project_reading};

/// Bytes inspected at the start of a file to classify it as binary, matching
/// git's own 8000-byte NUL probe.
const BINARY_PROBE_BYTES: usize = 8000;

/// Maximum commits returned in one compare ahead/behind list.
const MAX_COMPARE_COMMITS: usize = 200;

/// Maximum file paths returned by `repository_tree`.
const MAX_TREE_RESULTS: usize = 500;

/// Maximum grep matches returned by `repository_grep`.
const MAX_GREP_RESULTS: usize = 250;

/// Maximum bytes read from `git grep`.
const GREP_OUTPUT_LIMIT: u32 = 2 * 1024 * 1024;

/// Maximum lines returned in one blame view.
const MAX_BLAME_LINES: usize = 10_000;

/// The file of revisions that `git blame` skips when present in the repo root.
const IGNORE_REVS_FILE: &str = ".git-blame-ignore-revs";

/// Reads staged or unstaged `--numstat -z` line counts for `repository_overview`.
pub(crate) async fn diff_stats(reading: &Reading, staged: bool) -> Result<Vec<FileStat>, GitError> {
    let mut command = vec!["diff".to_owned()];
    if staged {
        command.push("--cached".to_owned());
    }
    command.extend(PATCH_OPTIONS.iter().map(|option| (*option).to_owned()));
    command.extend([
        "--ignore-submodules=dirty".to_owned(),
        "--numstat".to_owned(),
        "-z".to_owned(),
    ]);

    let ran = reading.git.run(reading.run(command)).await?;
    if !ran.succeeded() {
        return Ok(Vec::new());
    }

    Ok(parse_numstat(&ran.output)
        .into_iter()
        .take(MAX_LISTED)
        .collect())
}

/// Reads configured remotes with credentials redacted.
pub(crate) async fn read_remotes(reading: &Reading) -> Result<Vec<Remote>, GitError> {
    let ran = reading.git.output(reading.run(["remote", "-v"])).await?;
    Ok(parse_remotes(&ran.text()))
}

/// Checks whether `path` is an untracked non-ignored file and, if so, builds
/// a synthetic all-added [`FileDiff`] bounded by [`ROOM`].
pub(crate) async fn untracked_file_diff(
    reading: &Reading,
    path: &str,
) -> Result<Option<FileDiff>, GitError> {
    let listed = reading
        .git
        .output(reading.run([
            "ls-files".to_owned(),
            "--others".to_owned(),
            "--exclude-standard".to_owned(),
            "-z".to_owned(),
            "--".to_owned(),
            pathspec(path),
        ]))
        .await?;

    let is_untracked = listed
        .output
        .split(|byte| *byte == 0)
        .any(|entry| String::from_utf8_lossy(entry) == path);
    if !is_untracked {
        return Ok(None);
    }

    let full_path = reading.layout.top.join(path);
    let Ok(metadata) = std::fs::symlink_metadata(&full_path) else {
        return Ok(None);
    };

    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(&full_path)
            .map(|dest| dest.display().to_string())
            .unwrap_or_default();
        return Ok(Some(synthetic_added_diff(path, target.as_bytes(), false)));
    }

    if !metadata.is_file() {
        return Ok(None);
    }

    // Refuse any path whose canonical target escapes the repository root.
    let Ok(canonical) = std::fs::canonicalize(&full_path) else {
        return Ok(None);
    };
    let Ok(canonical_top) = std::fs::canonicalize(&reading.layout.top) else {
        return Ok(None);
    };
    if !is_inside(&canonical, &canonical_top) {
        return Err(GitError::ReadNotAllowed);
    }

    let Ok(file) = std::fs::File::open(&canonical) else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    let max_read = u64::try_from(ROOM + 1).unwrap_or(u64::MAX);
    if file.take(max_read).read_to_end(&mut bytes).is_err() {
        return Ok(None);
    }

    let truncated = bytes.len() > ROOM;
    if truncated {
        bytes.truncate(ROOM);
    }

    Ok(Some(synthetic_added_diff(path, &bytes, truncated)))
}

/// Builds a synthetic single-hunk diff for an untracked file.
fn synthetic_added_diff(path: &str, bytes: &[u8], truncated: bool) -> FileDiff {
    let probe = &bytes[..bytes.len().min(BINARY_PROBE_BYTES)];
    if probe.contains(&0) {
        return FileDiff {
            stat: Some(FileStat {
                path: path.to_owned(),
                original: None,
                added: None,
                removed: None,
            }),
            hunks: Vec::new(),
            untracked: true,
            binary: true,
            truncated: false,
        };
    }

    let text = String::from_utf8_lossy(bytes);
    if text.is_empty() {
        return FileDiff {
            stat: Some(FileStat {
                path: path.to_owned(),
                original: None,
                added: Some(0),
                removed: Some(0),
            }),
            hunks: Vec::new(),
            untracked: true,
            binary: false,
            truncated,
        };
    }

    let trailing_newline = text.ends_with('\n');
    let raw_lines: Vec<&str> = text.split('\n').collect();
    let effective_lines = if trailing_newline {
        &raw_lines[..raw_lines.len().saturating_sub(1)]
    } else {
        &raw_lines[..]
    };

    let line_count = effective_lines.len();
    let mut lines = Vec::with_capacity(line_count);
    for (index, raw_line) in effective_lines.iter().enumerate() {
        let stripped = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let is_last = index + 1 == line_count;
        lines.push(Line {
            kind: LineKind::Added,
            old: None,
            new: Some(u32::try_from(index + 1).unwrap_or(u32::MAX)),
            text: stripped.to_owned(),
            no_newline: is_last && !trailing_newline && !truncated,
        });
    }

    let count_u32 = u32::try_from(line_count).unwrap_or(u32::MAX);
    let header = if count_u32 == 1 {
        "@@ -0,0 +1 @@".to_owned()
    } else {
        format!("@@ -0,0 +1,{count_u32} @@")
    };

    FileDiff {
        stat: Some(FileStat {
            path: path.to_owned(),
            original: None,
            added: Some(count_u32),
            removed: Some(0),
        }),
        hunks: vec![Hunk { header, lines }],
        untracked: true,
        binary: false,
        truncated,
    }
}

/// Filtered topological commit history or reflog for the Graph and Inspect tabs.
pub(crate) async fn history_query(
    git: Git,
    input: HistoryQueryRef,
) -> Result<HistoryQueryResult, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;
    let head_oid = head_commit(&reading).await?;
    let limit = input.limit.unwrap_or(HISTORY_PAGE);
    let page = usize::try_from(limit).unwrap_or(usize::MAX);

    if matches!(input.scope, HistoryScope::Reflog) {
        return history_query_reflog(&reading, head_oid, limit, page, input.skip).await;
    }

    let cleaned_path = input
        .path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(|path| repository_path(&reading.layout.top, "path", path))
        .transpose()?;

    let mut command = build_history_log_args(&input, limit, cleaned_path.is_some());
    if !append_history_scope(&reading, &input.scope, head_oid.as_deref(), &mut command).await? {
        return Ok(HistoryQueryResult {
            commits: Vec::new(),
            reflog: Vec::new(),
            head_oid: None,
            more: false,
        });
    }

    if let Some(path) = cleaned_path {
        command.push("--".to_owned());
        command.push(pathspec(&path));
    }

    let ran = reading.git.output(reading.run(command)).await?;
    let mut commits = parse_log(&ran.output).map_err(|_error| GitError::GitFailed)?;
    let more = commits.len() > page;
    commits.truncate(page);

    Ok(HistoryQueryResult {
        commits,
        reflog: Vec::new(),
        head_oid,
        more,
    })
}

async fn history_query_reflog(
    reading: &Reading,
    head_oid: Option<String>,
    limit: u32,
    page: usize,
    skip: u32,
) -> Result<HistoryQueryResult, GitError> {
    if head_oid.is_none() {
        return Ok(HistoryQueryResult {
            commits: Vec::new(),
            reflog: Vec::new(),
            head_oid: None,
            more: false,
        });
    }

    let ran = reading
        .git
        .output(reading.run([
            "log".to_owned(),
            "-z".to_owned(),
            "--walk-reflogs".to_owned(),
            REFLOG_FORMAT.to_owned(),
            format!("--max-count={}", limit + 1),
            format!("--skip={skip}"),
            "HEAD".to_owned(),
        ]))
        .await?;

    let mut entries = parse_reflog(&ran.output).map_err(|_error| GitError::GitFailed)?;
    let more = entries.len() > page;
    entries.truncate(page);

    Ok(HistoryQueryResult {
        commits: Vec::new(),
        reflog: entries,
        head_oid,
        more,
    })
}

fn build_history_log_args(input: &HistoryQueryRef, limit: u32, has_path: bool) -> Vec<String> {
    let mut command = vec![
        "log".to_owned(),
        "--no-ext-diff".to_owned(),
        "--no-textconv".to_owned(),
        "-z".to_owned(),
        "--topo-order".to_owned(),
        LOG_FORMAT.to_owned(),
        format!("--max-count={}", limit + 1),
        format!("--skip={}", input.skip),
    ];

    match input.merges {
        MergeFilter::All => {}
        MergeFilter::Only => command.push("--merges".to_owned()),
        MergeFilter::Exclude => command.push("--no-merges".to_owned()),
    }
    if input.first_parent {
        command.push("--first-parent".to_owned());
    }
    if let Some(query) = input
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
    {
        command.push("--regexp-ignore-case".to_owned());
        command.push("--fixed-strings".to_owned());
        command.push(format!("--grep={query}"));
    }
    if let Some(author) = input
        .author
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        command.push("--regexp-ignore-case".to_owned());
        command.push(format!("--author={author}"));
    }
    if let Some(pickaxe) = input
        .pickaxe
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        command.push(format!("-S{pickaxe}"));
    }
    if has_path {
        command.push("--follow".to_owned());
    }
    command
}

async fn append_history_scope(
    reading: &Reading,
    scope: &HistoryScope,
    head_oid: Option<&str>,
    command: &mut Vec<String>,
) -> Result<bool, GitError> {
    match scope {
        HistoryScope::CurrentBranch => {
            if head_oid.is_none() {
                return Ok(false);
            }
            command.push("HEAD".to_owned());
            Ok(true)
        }
        HistoryScope::AllBranches => {
            if head_oid.is_none() {
                let any_ref = reading
                    .git
                    .output(reading.run([
                        "for-each-ref",
                        "--count=1",
                        "--format=%(objectname)",
                        "refs/heads",
                        "refs/remotes",
                        "refs/tags",
                    ]))
                    .await?;
                if any_ref.text().trim().is_empty() {
                    return Ok(false);
                }
            }
            command.push("--branches".to_owned());
            command.push("--tags".to_owned());
            command.push("--remotes".to_owned());
            if head_oid.is_some() {
                command.push("HEAD".to_owned());
            }
            Ok(true)
        }
        HistoryScope::Ref { name } => {
            let oid = reading.commit("scope", name).await?;
            command.push(oid);
            Ok(true)
        }
        HistoryScope::Reflog => Ok(false),
    }
}

/// Complete refs overview: local branches (current first, then newest),
/// remote branches (newest first), tags (newest first by creatordate),
/// stashes, remotes and worktrees.
pub(crate) async fn refs(git: Git, input: ProjectRef) -> Result<RefsOverview, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;

    let local_ran = reading
        .git
        .output(reading.run([
            "for-each-ref",
            "--sort=-HEAD",
            "--sort=-committerdate",
            REF_FORMAT,
            "refs/heads",
        ]))
        .await?;
    let branches: Vec<Ref> = parse_refs(&local_ran.text())
        .into_iter()
        .filter(|found| found.kind == RefKind::Branch)
        .take(MAX_LISTED)
        .collect();

    let remote_ran = reading
        .git
        .output(reading.run([
            "for-each-ref",
            "--sort=-committerdate",
            REF_FORMAT,
            "refs/remotes",
        ]))
        .await?;
    let remote_branches: Vec<Ref> = parse_refs(&remote_ran.text())
        .into_iter()
        .filter(|found| found.kind == RefKind::RemoteBranch)
        .take(MAX_LISTED)
        .collect();

    let tag_ran = reading
        .git
        .output(reading.run([
            "for-each-ref",
            "--sort=-creatordate",
            REF_FORMAT,
            "refs/tags",
        ]))
        .await?;
    let tags: Vec<Ref> = parse_refs(&tag_ran.text())
        .into_iter()
        .filter(|found| found.kind == RefKind::Tag)
        .take(MAX_LISTED)
        .collect();

    let stash_ran = reading
        .git
        .run(reading.run(["stash", "list", "-z", REFLOG_FORMAT]))
        .await?;
    let stashes = if stash_ran.succeeded() {
        parse_reflog(&stash_ran.output)
            .unwrap_or_default()
            .into_iter()
            .take(MAX_LISTED)
            .collect()
    } else {
        Vec::new()
    };

    let remotes = read_remotes(&reading).await?;

    let wt_ran = reading
        .git
        .output(reading.run(["worktree", "list", "--porcelain", "-z"]))
        .await?;
    let worktrees = parse_worktrees(&wt_ran.output)
        .into_iter()
        .take(MAX_LISTED)
        .collect();

    Ok(RefsOverview {
        branches,
        remote_branches,
        tags,
        stashes,
        remotes,
        worktrees,
    })
}

/// Shows a single stash entry with its changed files and patches.
pub(crate) async fn stash_show(git: Git, input: StashShowRef) -> Result<StashView, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;
    let list_ran = reading
        .git
        .output(reading.run(["stash", "list", "-z", REFLOG_FORMAT]))
        .await?;
    let entries = parse_reflog(&list_ran.output).map_err(|_error| GitError::GitFailed)?;
    let selector = format!("stash@{{{}}}", input.index);
    let entry = entries
        .into_iter()
        .find(|candidate| candidate.selector == selector)
        .ok_or(GitError::UnknownCommit)?;

    let mut base_args = vec![
        "stash".to_owned(),
        "show".to_owned(),
        "--include-untracked".to_owned(),
    ];
    base_args.extend(PATCH_OPTIONS.iter().map(|option| (*option).to_owned()));

    let numstat = reading
        .git
        .output(
            reading.run(
                [
                    base_args.clone(),
                    vec!["--numstat".to_owned(), "-z".to_owned(), selector.clone()],
                ]
                .concat(),
            ),
        )
        .await?;
    let files = parse_numstat(&numstat.output);

    let patches = if files.is_empty() {
        Vec::new()
    } else {
        let patch = reading
            .git
            .output(reading.run([base_args, vec!["--patch".to_owned(), selector]].concat()))
            .await?;
        let text = patch.text();
        let parts = split_patch(&text);
        files
            .iter()
            .zip(&parts)
            .take(MAX_PATCHED_FILES)
            .map(|(file, part)| FilePatch {
                path: file.path.clone(),
                hunks: parse_hunks(part),
            })
            .collect()
    };

    Ok(StashView {
        entry,
        files,
        patches,
    })
}

/// Compares two refs/commits or a ref against the work tree.
pub(crate) async fn compare(git: Git, input: CompareRef) -> Result<CompareView, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;
    let base_oid = reading.commit("base", &input.base).await?;

    let mut diff_base = vec!["diff".to_owned()];
    diff_base.extend(PATCH_OPTIONS.iter().map(|option| (*option).to_owned()));
    if input.ignore_whitespace {
        diff_base.push("-w".to_owned());
    }

    if input.head == "WORKTREE" {
        diff_base.push("--ignore-submodules=dirty".to_owned());
        let (files, patches) = read_diff_files_and_patches(&reading, &diff_base, &base_oid).await?;
        return Ok(CompareView {
            base: base_oid,
            head: None,
            base_label: input.base,
            head_label: input.head,
            ahead: Vec::new(),
            behind: Vec::new(),
            files,
            patches,
        });
    }

    let head_oid = reading.commit("head", &input.head).await?;
    let ahead = compare_log_range(&reading, &base_oid, &head_oid).await?;
    let behind = compare_log_range(&reading, &head_oid, &base_oid).await?;
    let range = match input.mode {
        CompareMode::ThreeDot => format!("{base_oid}...{head_oid}"),
        CompareMode::TwoDot => format!("{base_oid}..{head_oid}"),
    };
    let (files, patches) = read_diff_files_and_patches(&reading, &diff_base, &range).await?;

    Ok(CompareView {
        base: base_oid,
        head: Some(head_oid),
        base_label: input.base,
        head_label: input.head,
        ahead,
        behind,
        files,
        patches,
    })
}

async fn compare_log_range(
    reading: &Reading,
    from_oid: &str,
    to_oid: &str,
) -> Result<Vec<Commit>, GitError> {
    let ran = reading
        .git
        .output(reading.run([
            "log".to_owned(),
            "--no-ext-diff".to_owned(),
            "--no-textconv".to_owned(),
            "-z".to_owned(),
            LOG_FORMAT.to_owned(),
            format!("--max-count={MAX_COMPARE_COMMITS}"),
            format!("{from_oid}..{to_oid}"),
        ]))
        .await?;
    parse_log(&ran.output).map_err(|_error| GitError::GitFailed)
}

async fn read_diff_files_and_patches(
    reading: &Reading,
    diff_base: &[String],
    range_or_ref: &str,
) -> Result<(Vec<FileStat>, Vec<FilePatch>), GitError> {
    let numstat = reading
        .git
        .output(
            reading.run(
                [
                    diff_base.to_vec(),
                    vec![
                        "--numstat".to_owned(),
                        "-z".to_owned(),
                        range_or_ref.to_owned(),
                        "--".to_owned(),
                    ],
                ]
                .concat(),
            ),
        )
        .await?;
    let files = parse_numstat(&numstat.output);
    if files.is_empty() {
        return Ok((files, Vec::new()));
    }

    let patch = reading
        .git
        .output(
            reading.run(
                [
                    diff_base.to_vec(),
                    vec![
                        "--patch".to_owned(),
                        range_or_ref.to_owned(),
                        "--".to_owned(),
                    ],
                ]
                .concat(),
            ),
        )
        .await?;
    let text = patch.text();
    let parts = split_patch(&text);
    let patches = files
        .iter()
        .zip(&parts)
        .take(MAX_PATCHED_FILES)
        .map(|(file, part)| FilePatch {
            path: file.path.clone(),
            hunks: parse_hunks(part),
        })
        .collect();
    Ok((files, patches))
}

/// Per-line blame annotations for a file in the work tree or at a revision.
pub(crate) async fn blame(git: Git, input: BlameRef) -> Result<BlameView, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;
    let path = repository_path(&reading.layout.top, "path", &input.path)?;
    let revision = input
        .revision
        .as_deref()
        .map(str::trim)
        .filter(|rev| !rev.is_empty())
        .map(str::to_owned);

    let resolved_rev = match &revision {
        Some(rev) => Some(reading.commit("revision", rev).await?),
        None => None,
    };

    if is_binary_for_blame(&reading, &path, resolved_rev.as_deref()).await? {
        return Ok(BlameView {
            path,
            revision,
            runs: Vec::new(),
            min_timestamp: UtcMillis::EPOCH,
            max_timestamp: UtcMillis::EPOCH,
            total_lines: 0,
            binary: true,
            truncated: false,
        });
    }

    let command = build_blame_command(&reading, &input, &path, resolved_rev);
    let ran = reading.git.run(reading.run(command)).await?;
    if !ran.succeeded() {
        return Err(GitError::from(GitFailure::InvalidArgument {
            detail: ran.errors,
        }));
    }

    let (kept_runs, total_lines, truncated) = cap_blame_runs(parse_blame(&ran.text()), ran.cut);
    let (runs, min_timestamp, max_timestamp) = decorate_blame_runs(&reading, kept_runs).await?;

    Ok(BlameView {
        path,
        revision,
        runs,
        min_timestamp,
        max_timestamp,
        total_lines,
        binary: false,
        truncated,
    })
}

/// Returns the canonical path of a regular file inside `top` when
/// `relative_path` is a non-symlink regular file whose canonical path stays
/// within `top`. Returns `Ok(None)` for missing paths, symlinks, directories,
/// FIFOs, or special files without opening them, and `Err(GitError::ReadNotAllowed)`
/// if a symlinked parent directory escapes the repository root.
pub(super) fn safe_worktree_regular_file(
    top: &Path,
    relative_path: &str,
) -> Result<Option<PathBuf>, GitError> {
    let full = top.join(relative_path);
    let Ok(metadata) = std::fs::symlink_metadata(&full) else {
        return Ok(None);
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(None);
    }
    let Ok(canonical) = std::fs::canonicalize(&full) else {
        return Ok(None);
    };
    let Ok(canonical_top) = std::fs::canonicalize(top) else {
        return Ok(None);
    };
    if !is_inside(&canonical, &canonical_top) {
        return Err(GitError::ReadNotAllowed);
    }
    Ok(Some(canonical))
}

async fn is_binary_for_blame(
    reading: &Reading,
    path: &str,
    resolved_rev: Option<&str>,
) -> Result<bool, GitError> {
    if let Some(oid) = resolved_rev {
        let blob = reading
            .git
            .run(
                reading
                    .run(["cat-file", "blob", &format!("{oid}:{path}")])
                    .with_output_limit(u32::try_from(BINARY_PROBE_BYTES).unwrap_or(8000)),
            )
            .await?;
        Ok(blob.output.contains(&0))
    } else {
        // Symlinks are blamed by Git as their link-target text (non-binary),
        // and non-regular files such as FIFOs must never be opened.
        let Some(canonical) = safe_worktree_regular_file(&reading.layout.top, path)? else {
            return Ok(false);
        };
        Ok(std::fs::File::open(&canonical)
            .ok()
            .and_then(|file| {
                let mut buf = Vec::with_capacity(BINARY_PROBE_BYTES);
                file.take(u64::try_from(BINARY_PROBE_BYTES).unwrap_or(8000))
                    .read_to_end(&mut buf)
                    .ok()
                    .map(|_count| buf.contains(&0))
            })
            .unwrap_or(false))
    }
}

fn build_blame_command(
    reading: &Reading,
    input: &BlameRef,
    path: &str,
    resolved_rev: Option<String>,
) -> Vec<String> {
    let mut command = vec![
        "blame".to_owned(),
        "--porcelain".to_owned(),
        "--no-textconv".to_owned(),
    ];
    if input.ignore_whitespace {
        command.push("-w".to_owned());
    }
    if input.follow_moves {
        command.push("-M".to_owned());
        command.push("-C".to_owned());
    }
    if safe_worktree_regular_file(&reading.layout.top, IGNORE_REVS_FILE)
        .ok()
        .flatten()
        .is_some()
    {
        command.push(format!("--ignore-revs-file={IGNORE_REVS_FILE}"));
    }
    if let Some(oid) = resolved_rev {
        command.push(oid);
    }
    command.push("--".to_owned());
    command.push(path.to_owned());
    command
}

fn cap_blame_runs(parsed: Vec<BlameRun>, already_cut: bool) -> (Vec<BlameRun>, u32, bool) {
    let total_lines_usize: usize = parsed.iter().map(|run| run.lines.len()).sum();
    let mut kept_runs = Vec::new();
    let mut kept_lines = 0usize;
    let mut truncated = already_cut;

    for mut run in parsed {
        if kept_lines >= MAX_BLAME_LINES {
            truncated = true;
            break;
        }
        let room = MAX_BLAME_LINES - kept_lines;
        if run.lines.len() > room {
            run.lines.truncate(room);
            truncated = true;
        }
        kept_lines += run.lines.len();
        kept_runs.push(run);
    }

    (
        kept_runs,
        u32::try_from(total_lines_usize).unwrap_or(u32::MAX),
        truncated,
    )
}

async fn decorate_blame_runs(
    reading: &Reading,
    kept_runs: Vec<BlameRun>,
) -> Result<(Vec<BlameRunView>, UtcMillis, UtcMillis), GitError> {
    let oids: Vec<String> = kept_runs
        .iter()
        .filter(|run| !run.commit.uncommitted)
        .map(|run| run.commit.oid.clone())
        .collect();
    let labels = label_commits(reading, &oids).await?;

    let mut min_ts: Option<i64> = None;
    let mut max_ts: Option<i64> = None;
    let mut runs = Vec::with_capacity(kept_runs.len());

    for run in kept_runs {
        if !run.commit.uncommitted && run.commit.timestamp != UtcMillis::EPOCH {
            let ms = run.commit.timestamp.as_millis();
            min_ts = Some(min_ts.map_or(ms, |current| current.min(ms)));
            max_ts = Some(max_ts.map_or(ms, |current| current.max(ms)));
        }
        let short = if run.commit.uncommitted {
            "0000000".to_owned()
        } else {
            labels
                .iter()
                .find(|label| label.oid == run.commit.oid)
                .map_or_else(
                    || run.commit.oid.chars().take(7).collect(),
                    |label| label.short.clone(),
                )
        };
        runs.push(BlameRunView {
            short,
            commit: run.commit,
            lines: run.lines,
        });
    }

    Ok((
        runs,
        UtcMillis::from_millis(min_ts.unwrap_or(0)),
        UtcMillis::from_millis(max_ts.unwrap_or(0)),
    ))
}

/// Tracked file paths in the repository, optionally filtered by a query.
pub(crate) async fn tree(git: Git, input: TreeQueryRef) -> Result<FileTreeList, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;
    let revision = input
        .revision
        .as_deref()
        .map(str::trim)
        .filter(|rev| !rev.is_empty());

    let output = match revision {
        Some(rev) => {
            let oid = reading.commit("revision", rev).await?;
            reading
                .git
                .output(reading.run(["ls-tree", "-r", "--name-only", "-z", &oid]))
                .await?
        }
        None => reading.git.output(reading.run(["ls-files", "-z"])).await?,
    };

    let needle = input
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_lowercase);

    let all: Vec<String> = output
        .output
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .filter(|path| match &needle {
            Some(needle) => path.to_lowercase().contains(needle),
            None => true,
        })
        .collect();

    let total = u32::try_from(all.len()).unwrap_or(u32::MAX);
    let files = all.into_iter().take(MAX_TREE_RESULTS).collect();

    Ok(FileTreeList { files, total })
}

/// Content search across the work tree or a specific revision (`git grep`).
pub(crate) async fn grep(git: Git, input: GrepQueryRef) -> Result<GrepView, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;
    let mut command = vec![
        "grep".to_owned(),
        "--no-textconv".to_owned(),
        "-z".to_owned(),
        "-n".to_owned(),
        "-I".to_owned(),
        "--no-color".to_owned(),
        "--full-name".to_owned(),
    ];
    command.push(if input.fixed_strings { "-F" } else { "-E" }.to_owned());
    if !input.case_sensitive {
        command.push("-i".to_owned());
    }
    command.push("-e".to_owned());
    command.push(input.pattern.clone());

    let revision = input
        .revision
        .as_deref()
        .map(str::trim)
        .filter(|rev| !rev.is_empty());
    let at_revision = if let Some(rev) = revision {
        let oid = reading.commit("revision", rev).await?;
        command.push(oid);
        true
    } else {
        false
    };

    command.push("--".to_owned());
    if let Some(path) = input
        .path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let clean = repository_path(&reading.layout.top, "path", path)?;
        command.push(pathspec(&clean));
    }

    let ran = reading
        .git
        .run(reading.run(command).with_output_limit(GREP_OUTPUT_LIMIT))
        .await?;
    let found_nothing = ran.exit_code == Some(1) && ran.output.is_empty();
    if !ran.succeeded() && !found_nothing && !ran.cut {
        return Err(GitError::from(ran.failure(&reading.layout.top)));
    }

    let mut matches = parse_grep(&ran.text(), at_revision);
    if ran.cut {
        matches.pop();
    }
    let truncated = matches.len() > MAX_GREP_RESULTS || ran.cut;
    matches.truncate(MAX_GREP_RESULTS);

    Ok(GrepView { matches, truncated })
}
