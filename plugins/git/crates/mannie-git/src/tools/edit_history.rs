//! `git_edit_history` (plugins/git/docs/adr/0002-edit-history-in-memory.md):
//! rewrites the commits of the current branch after a base, in a new order,
//! each picked, reworded, squashed, fixed up or dropped, all or nothing.
//!
//! Each commit whose parent changed is cherry-picked in memory with `git
//! merge-tree --write-tree --merge-base`, and written with `git
//! commit-tree`, which keeps its author and date and signs as the owner's
//! configuration says. A commit whose parent did not change is kept as it
//! is. When a step conflicts, the branch does not move, and only objects
//! that nothing names were written. When every step succeeded, `git reset
//! --keep` moves the branch, which refuses to overwrite local changes.
//!
//! The run that the owner allows is `git reset --keep`: the commit that it
//! moves to is built after the approval, so the approval cannot name it.

use std::fmt::Write as _;

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::arguments::is_object_id;
use crate::changes::{Acting, ChangeTool, Plan, current_branch};
use crate::describe::{CommitLabel, label_commits};
use crate::effects::{Change, discarded_commits, published, unkept_commits};
use crate::failure::GitFailure;
use crate::outcome::{head_commit, new_commits, refusal};
use crate::parse::status::Entry;
use crate::texts;
use crate::tools::commit::{check_identity, check_message};
use crate::tools::merge::no_operation;
use crate::tools::status::read_status;
use crate::tools::{Answer, Reading};

/// The tool's name.
pub(crate) const GIT_EDIT_HISTORY: &str = "git_edit_history";

/// The most commits that one edit rewrites.
const MAX_STEPS: usize = 200;

/// The format of a commit as an edit reads it: its hash, its parents, its
/// tree, its author's name, email and raw date, and its whole message.
const ORIGINAL_FORMAT: &str = "--format=%H%x1f%P%x1f%T%x1f%an%x1f%ae%x1f%ad%x1f%B";

/// What a step does with its commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StepAction {
    Pick,
    Reword,
    Squash,
    Fixup,
    Drop,
}

/// A step of an edit.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistoryStep {
    /// A commit after base, such as 1a2b3c4.
    commit: String,
    /// What to do with it.
    action: StepAction,
    /// For reword: the new message. For squash: the message of the commit
    /// that it is folded into, for both.
    message: Option<String>,
}

/// The arguments of `git_edit_history`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditHistoryArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The commit after which the current branch's commits are rewritten,
    /// such as main or HEAD~5.
    base: String,
    /// Every commit after base up to the branch's tip, once each, oldest
    /// first, in the new order.
    steps: Vec<HistoryStep>,
}

/// A commit of the branch as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Original {
    oid: String,
    parent: String,
    tree: String,
    author_name: String,
    author_email: String,
    author_date: String,
    message: String,
}

/// A step with its commit read.
#[derive(Debug, Clone)]
pub(crate) struct Planned {
    action: StepAction,
    commit: Original,
    message: Option<String>,
}

/// What an edit keeps for its run.
#[derive(Debug, Clone)]
pub(crate) struct Editing {
    branch: String,
    base: String,
    head: String,
    steps: Vec<Planned>,
    sign: bool,
}

/// `git_edit_history`.
#[derive(Debug)]
pub(crate) struct GitEditHistory;

impl ChangeTool for GitEditHistory {
    type Arguments = EditHistoryArguments;
    type Planned = Editing;

    const NAME: &'static str = GIT_EDIT_HISTORY;
    const DESCRIPTION: &'static str = texts::EDIT_HISTORY_DESCRIPTION;

    fn repository(arguments: &EditHistoryArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a EditHistoryArguments,
    ) -> BoxFuture<'a, Result<Plan<Editing>, GitFailure>> {
        Box::pin(plan(reading, arguments))
    }

    fn act<'a>(
        acting: &'a Acting,
        _arguments: &'a EditHistoryArguments,
        plan: Plan<Editing>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(act(acting, plan))
    }
}

fn invalid(detail: String) -> GitFailure {
    GitFailure::InvalidArgument { detail }
}

/// The commits after `base` up to HEAD, oldest first, as an edit reads them.
async fn originals(reading: &Reading, base: &str) -> Result<Vec<Original>, GitFailure> {
    let ran = reading
        .git
        .output(reading.run([
            "log".to_owned(),
            "-z".to_owned(),
            "--reverse".to_owned(),
            "--topo-order".to_owned(),
            "--date=raw".to_owned(),
            ORIGINAL_FORMAT.to_owned(),
            format!("{base}..HEAD"),
        ]))
        .await?;
    let text = ran.text();
    let mut originals = Vec::new();
    for record in text.split('\0').filter(|record| !record.trim().is_empty()) {
        let fields: Vec<&str> = record
            .trim_start_matches('\n')
            .splitn(7, '\u{1f}')
            .collect();
        let [
            oid,
            parents,
            tree,
            author_name,
            author_email,
            author_date,
            message,
        ] = fields.as_slice()
        else {
            return Err(GitFailure::failed(
                "git listed a commit in a form that this version cannot read",
            ));
        };
        let parents: Vec<&str> = parents.split_whitespace().collect();
        let [parent] = parents.as_slice() else {
            return Err(invalid(format!(
                "{oid} merges branches. A history edit rewrites a range without merges; use git_rebase instead."
            )));
        };
        originals.push(Original {
            oid: (*oid).to_owned(),
            parent: (*parent).to_owned(),
            tree: (*tree).to_owned(),
            author_name: (*author_name).to_owned(),
            author_email: (*author_email).to_owned(),
            author_date: (*author_date).to_owned(),
            message: message.trim_end_matches(['\n', '\r']).to_owned(),
        });
    }
    Ok(originals)
}

/// Whether the owner's configuration signs commits.
fn signs_commits(reading: &Reading) -> bool {
    reading
        .settings
        .iter()
        .rev()
        .find(|setting| setting.key == "commit.gpgsign")
        .and_then(|setting| setting.value.as_deref())
        .is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "true" | "yes" | "on" | "1"
            )
        })
}

async fn plan(
    reading: &Reading,
    arguments: &EditHistoryArguments,
) -> Result<Plan<Editing>, GitFailure> {
    no_operation(reading)?;
    check_identity(reading).await?;
    let branch = current_branch(reading).await?.ok_or_else(|| {
        invalid("HEAD is detached. Switch to the branch to edit first.".to_owned())
    })?;
    let head = head_commit(reading)
        .await?
        .ok_or_else(|| invalid("The branch has no commits.".to_owned()))?;
    let base = reading.commit("base", &arguments.base).await?;
    let is_ancestor = reading
        .git
        .run(reading.run(["merge-base", "--is-ancestor", base.as_str(), head.as_str()]))
        .await?;
    if !is_ancestor.succeeded() {
        return Err(invalid(format!(
            "{} is not an ancestor of HEAD.",
            arguments.base
        )));
    }
    let originals = originals(reading, &base).await?;
    if arguments.steps.len() > MAX_STEPS || arguments.steps.len() != originals.len() {
        return Err(invalid(format!(
            "steps must name every one of the {} commits after base, once each.",
            originals.len()
        )));
    }
    let staged = read_status(reading)
        .await?
        .entries
        .iter()
        .any(Entry::is_staged);
    if staged {
        return Err(invalid(
            "The index holds staged changes. Commit them, or take them out with git_stage, first."
                .to_owned(),
        ));
    }
    let steps = planned_steps(reading, &arguments.steps, &originals).await?;
    // The commits before the first step that changes anything stay as they
    // are.
    let kept = steps
        .iter()
        .zip(&originals)
        .take_while(|(step, original)| {
            step.action == StepAction::Pick && step.commit.oid == original.oid
        })
        .count();
    if kept == originals.len() {
        return Err(invalid(
            "The steps keep every commit as it is, so nothing changes.".to_owned(),
        ));
    }
    let first_rewritten = originals
        .get(kept)
        .map_or_else(|| base.clone(), |original| original.parent.clone());
    let rewritten = published(reading, &format!("{first_rewritten}..HEAD")).await?;
    let dropped: Vec<String> = steps
        .iter()
        .filter(|step| step.action == StepAction::Drop)
        .map(|step| step.commit.oid.clone())
        .collect();
    let unkept = unkept_commits(reading, &head, Some(&branch), Some(&base)).await?;
    let lost: Vec<String> = dropped
        .into_iter()
        .filter(|oid| unkept.contains(oid))
        .collect();
    let lost = label_commits(reading, &lost).await?;
    let editing = Editing {
        branch,
        base: base.clone(),
        head,
        steps,
        sign: signs_commits(reading),
    };
    let command = vec!["reset".to_owned(), "--keep".to_owned()];
    Ok(Plan::new(Change::EditHistory, command, editing)
        .with_tip(&base)
        .with_effect(rewritten)
        .with_effect(discarded_commits(&lost)))
}

/// The steps with their commits read, each checked: it names a commit after
/// the base once, a squash or fixup has a kept commit before it to fold
/// into, and a message comes exactly with a reword or a squash.
async fn planned_steps(
    reading: &Reading,
    steps: &[HistoryStep],
    originals: &[Original],
) -> Result<Vec<Planned>, GitFailure> {
    let mut planned: Vec<Planned> = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        let number = index + 1;
        let oid = reading.commit("steps.commit", &step.commit).await?;
        let commit = originals
            .iter()
            .find(|original| original.oid == oid)
            .cloned()
            .ok_or_else(|| invalid(format!("{} is not a commit after base.", step.commit)))?;
        if planned.iter().any(|earlier| earlier.commit.oid == oid) {
            return Err(invalid(format!("{} appears twice in steps.", step.commit)));
        }
        let folds = matches!(step.action, StepAction::Squash | StepAction::Fixup);
        let has_kept_before = planned
            .iter()
            .any(|earlier| earlier.action != StepAction::Drop);
        if folds && !has_kept_before {
            return Err(invalid(format!(
                "Step {number} folds into the commit before it, and no commit is kept before it."
            )));
        }
        match (step.action, &step.message) {
            (StepAction::Reword | StepAction::Squash, Some(message)) => {
                check_message("steps.message", message)?;
            }
            (StepAction::Reword | StepAction::Squash, None) => {
                return Err(invalid(format!("Step {number} needs a message.")));
            }
            (StepAction::Pick | StepAction::Fixup | StepAction::Drop, Some(_message)) => {
                return Err(invalid(format!(
                    "Step {number} keeps or drops its message, so it takes none."
                )));
            }
            (StepAction::Pick | StepAction::Fixup | StepAction::Drop, None) => {}
        }
        planned.push(Planned {
            action: step.action,
            commit,
            message: step.message.clone(),
        });
    }
    Ok(planned)
}

/// A commit being built: its tree, the original whose author it keeps, its
/// message, and the original commit itself while nothing changed it.
#[derive(Debug, Clone)]
struct Building {
    tree: String,
    author: Original,
    message: String,
    unchanged: Option<String>,
}

/// Why a step could not be built: its number and the files that conflict.
#[derive(Debug)]
struct Conflicted {
    step: usize,
    files: Vec<String>,
}

/// The tree of `commit` applied onto `onto`, against `parent`, or the files
/// that conflict.
async fn applied_tree(
    acting: &Acting,
    parent: &str,
    onto: &str,
    commit: &str,
) -> Result<Result<String, Vec<String>>, GitFailure> {
    let ran = acting
        .reading
        .git
        .run(acting.run([
            "merge-tree".to_owned(),
            "--write-tree".to_owned(),
            "--name-only".to_owned(),
            "--no-messages".to_owned(),
            format!("--merge-base={parent}"),
            onto.to_owned(),
            commit.to_owned(),
        ]))
        .await?;
    let text = ran.text();
    let mut lines = text.lines();
    let tree = lines.next().unwrap_or_default().trim().to_owned();
    match ran.exit_code {
        Some(0) if is_object_id(&tree) => Ok(Ok(tree)),
        Some(1) => Ok(Err(lines
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())),
        _other => Err(refusal("history edit", &ran)),
    }
}

/// Writes a commit of `building` on `parent`, or keeps the original when
/// nothing changed it.
async fn written(
    acting: &Acting,
    building: &Building,
    parent: &str,
    sign: bool,
) -> Result<String, GitFailure> {
    if let Some(unchanged) = &building.unchanged
        && building.author.parent == parent
    {
        return Ok(unchanged.clone());
    }
    let sign = if sign { "-S" } else { "--no-gpg-sign" };
    let run = acting
        .run([
            "commit-tree".to_owned(),
            building.tree.clone(),
            "-p".to_owned(),
            parent.to_owned(),
            sign.to_owned(),
            "-F".to_owned(),
            "-".to_owned(),
        ])
        .with_input(building.message.clone().into_bytes())
        .with_variable("GIT_AUTHOR_NAME", &building.author.author_name)
        .with_variable("GIT_AUTHOR_EMAIL", &building.author.author_email)
        .with_variable("GIT_AUTHOR_DATE", &building.author.author_date);
    let ran = acting.reading.git.output(run).await?;
    let commit = ran.text().trim().to_owned();
    if is_object_id(&commit) {
        Ok(commit)
    } else {
        Err(GitFailure::failed("git commit-tree printed no commit"))
    }
}

/// Builds the new commits after the base, or names the step that
/// conflicts.
async fn build(
    acting: &Acting,
    editing: &Editing,
) -> Result<Result<String, Conflicted>, GitFailure> {
    let mut tip = editing.base.clone();
    let mut building: Option<Building> = None;
    for (index, step) in editing.steps.iter().enumerate() {
        let commit = &step.commit;
        match step.action {
            StepAction::Drop => {}
            StepAction::Pick | StepAction::Reword => {
                if let Some(done) = building.take() {
                    tip = written(acting, &done, &tip, editing.sign).await?;
                }
                // A commit whose parent stays keeps its tree.
                let applied = if commit.parent == tip {
                    Ok(commit.tree.clone())
                } else {
                    applied_tree(acting, &commit.parent, &tip, &commit.oid).await?
                };
                let tree = match applied {
                    Ok(tree) => tree,
                    Err(files) => {
                        return Ok(Err(Conflicted {
                            step: index + 1,
                            files,
                        }));
                    }
                };
                let unchanged = (step.action == StepAction::Pick).then(|| commit.oid.clone());
                building = Some(Building {
                    tree,
                    author: commit.clone(),
                    message: step
                        .message
                        .clone()
                        .unwrap_or_else(|| commit.message.clone()),
                    unchanged,
                });
            }
            StepAction::Squash | StepAction::Fixup => {
                let Some(mut folded) = building.take() else {
                    return Err(GitFailure::failed("a step folds into no commit"));
                };
                // merge-tree applies commits onto commits, so the commit
                // being built is written first; nothing names it.
                let so_far = written(acting, &folded, &tip, false).await?;
                let tree = match applied_tree(acting, &commit.parent, &so_far, &commit.oid).await? {
                    Ok(tree) => tree,
                    Err(files) => {
                        return Ok(Err(Conflicted {
                            step: index + 1,
                            files,
                        }));
                    }
                };
                folded.tree = tree;
                folded.unchanged = None;
                if let Some(message) = &step.message {
                    folded.message.clone_from(message);
                }
                building = Some(folded);
            }
        }
    }
    if let Some(done) = building {
        tip = written(acting, &done, &tip, editing.sign).await?;
    }
    Ok(Ok(tip))
}

async fn act(acting: &Acting, plan: Plan<Editing>) -> Result<Answer, GitFailure> {
    let editing = &plan.planned;
    let tip = match build(acting, editing).await? {
        Ok(tip) => tip,
        Err(conflicted) => {
            let step = &editing.steps[conflicted.step - 1];
            let label =
                label_commits(&acting.reading, std::slice::from_ref(&step.commit.oid)).await?;
            let named = label
                .first()
                .map_or_else(|| step.commit.oid.clone(), CommitLabel::text);
            return Err(GitFailure::refused(
                "history edit",
                &format!(
                    "step {} ({named}) conflicts in {}. Nothing changed. Rebase instead with git_rebase, which stops at the conflict for git_resolve.",
                    conflicted.step,
                    conflicted.files.join(", ")
                ),
            ));
        }
    };
    let mut command = plan.command;
    command.push(tip.clone());
    let ran = acting.reading.git.run(acting.run(command)).await?;
    if !ran.succeeded() {
        return Err(refusal("history edit", &ran));
    }
    let commits = new_commits(&acting.reading, Some(&editing.base)).await?;
    let mut text = format!(
        "Rewrote the commits of {} after {}; it was at {} and is now at {}.\n",
        editing.branch,
        editing.base.chars().take(10).collect::<String>(),
        editing.head.chars().take(10).collect::<String>(),
        tip.chars().take(10).collect::<String>()
    );
    let _written = writeln!(
        text,
        "Its commits after the base, newest first ({}):",
        commits.len()
    );
    for commit in &commits {
        let _written = writeln!(text, "  {}", commit.text());
    }
    Ok(Answer { text, data: None })
}
