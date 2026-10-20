//! What results say of commits and of the operation in progress
//! (plugins/git/docs/conflicts-and-history.md, operations that can stop): a
//! commit by its short hash and subject, and a merge, rebase, cherry-pick or
//! revert that stopped by its step and the commits that it works with, named
//! "current" and "incoming" whatever git calls ours and theirs.

use std::fmt::Write as _;

use serde::Serialize;

use crate::arguments::is_object_id;
use crate::failure::GitFailure;
use crate::operation::{Operation, OperationKind};
use crate::tools::Reading;

/// A commit as a result names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitLabel {
    pub(crate) oid: String,
    pub(crate) short: String,
    pub(crate) subject: String,
}

impl CommitLabel {
    /// The label in a sentence, such as `1a2b3c4 Fix the parser`.
    pub(crate) fn text(&self) -> String {
        if self.subject.is_empty() {
            self.short.clone()
        } else {
            format!("{} {}", self.short, self.subject)
        }
    }

    /// The label of a commit that git could not describe: its hash's start.
    fn bare(oid: &str) -> Self {
        Self {
            oid: oid.to_owned(),
            short: oid.chars().take(7).collect(),
            subject: String::new(),
        }
    }
}

/// The commits that one run labels, which keeps its command line short on
/// every system.
const LABELED_AT_ONCE: usize = 200;

/// The labels of commits named by their full hashes, in their order, each
/// once. A commit that git cannot read, such as one that a state file names
/// after it was pruned, keeps the start of its hash.
pub(crate) async fn label_commits(
    reading: &Reading,
    oids: &[String],
) -> Result<Vec<CommitLabel>, GitFailure> {
    let mut unique: Vec<&String> = Vec::new();
    for oid in oids.iter().filter(|oid| is_object_id(oid)) {
        if !unique.contains(&oid) {
            unique.push(oid);
        }
    }
    let mut labels = Vec::with_capacity(unique.len());
    for chunk in unique.chunks(LABELED_AT_ONCE) {
        let read = read_labels(reading, chunk).await?;
        labels.extend(chunk.iter().map(|oid| {
            read.iter()
                .find(|label| &label.oid == *oid)
                .cloned()
                .unwrap_or_else(|| CommitLabel::bare(oid))
        }));
    }
    Ok(labels)
}

/// The labels that one run of git reads, or none when it fails.
async fn read_labels(reading: &Reading, oids: &[&String]) -> Result<Vec<CommitLabel>, GitFailure> {
    let mut arguments = vec![
        "log".to_owned(),
        "--no-walk=unsorted".to_owned(),
        "-z".to_owned(),
        "--format=%H%x1f%h%x1f%s".to_owned(),
    ];
    arguments.extend(oids.iter().map(|oid| (*oid).clone()));
    let ran = reading.git.run(reading.run(arguments)).await?;
    if !ran.succeeded() {
        return Ok(Vec::new());
    }
    Ok(ran
        .text()
        .split('\0')
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').splitn(3, '\u{1f}');
            Some(CommitLabel {
                oid: fields.next()?.to_owned(),
                short: fields.next()?.to_owned(),
                subject: fields.next()?.to_owned(),
            })
        })
        .collect())
}

/// The step of a rebase that stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Step {
    pub(crate) number: u32,
    pub(crate) total: u32,
}

/// The operation in progress, as a result describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperationView {
    pub(crate) kind: OperationKind,
    pub(crate) step: Option<Step>,
    /// The branch that a rebase rewrites, such as `feature`.
    pub(crate) branch: Option<String>,
    /// The commit that the operation applies now.
    pub(crate) incoming: Option<CommitLabel>,
    /// The commit that a rebase replays onto.
    pub(crate) onto: Option<CommitLabel>,
}

impl OperationView {
    /// The operation in a sentence, such as `A rebase of feature onto
    /// 9f8e7d6 Start stopped at step 3 of 7, applying 1a2b3c4 Fix the
    /// parser.`
    pub(crate) fn text(&self) -> String {
        let incoming = self.incoming.as_ref().map(CommitLabel::text);
        match self.kind {
            OperationKind::Rebase => {
                let mut text = "A rebase".to_owned();
                if let Some(branch) = &self.branch {
                    let _written = write!(text, " of {branch}");
                }
                if let Some(onto) = &self.onto {
                    let _written = write!(text, " onto {}", onto.text());
                }
                match self.step {
                    Some(step) => {
                        let _written =
                            write!(text, " stopped at step {} of {}", step.number, step.total);
                    }
                    None => text.push_str(" stopped"),
                }
                if let Some(incoming) = incoming {
                    let _written = write!(text, ", applying {incoming}");
                }
                text.push('.');
                text
            }
            OperationKind::Merge | OperationKind::CherryPick | OperationKind::Revert => {
                let name = self.kind.name();
                match incoming {
                    Some(incoming) => format!("A {name} of {incoming} is in progress."),
                    None => format!("A {name} is in progress."),
                }
            }
        }
    }
}

/// The operation in progress in the repository, with its commits labeled,
/// or `None`.
pub(crate) async fn operation_view(reading: &Reading) -> Result<Option<OperationView>, GitFailure> {
    let Some(operation) = Operation::read(&reading.layout.git_dir) else {
        return Ok(None);
    };
    let Operation {
        kind,
        step,
        incoming,
        branch,
        onto,
    } = operation;
    let named: Vec<String> = [incoming.clone(), onto.clone()]
        .into_iter()
        .flatten()
        .collect();
    let labels = label_commits(reading, &named).await?;
    let label_of = |oid: Option<String>| {
        let oid = oid?;
        labels.iter().find(|label| label.oid == oid).cloned()
    };
    Ok(Some(OperationView {
        kind,
        step: step.map(|(number, total)| Step { number, total }),
        branch: branch.map(|branch| {
            branch
                .strip_prefix("refs/heads/")
                .map_or(branch.clone(), str::to_owned)
        }),
        incoming: label_of(incoming),
        onto: label_of(onto),
    }))
}

#[cfg(test)]
mod tests {
    use super::{CommitLabel, OperationView, Step};
    use crate::operation::OperationKind;

    fn label(short: &str, subject: &str) -> CommitLabel {
        CommitLabel {
            oid: format!("{short}{}", "0".repeat(40 - short.len())),
            short: short.to_owned(),
            subject: subject.to_owned(),
        }
    }

    #[test]
    fn an_operation_says_what_it_applies_and_where_it_stopped() {
        let rebase = OperationView {
            kind: OperationKind::Rebase,
            step: Some(Step {
                number: 3,
                total: 7,
            }),
            branch: Some("feature".to_owned()),
            incoming: Some(label("1a2b3c4", "Fix the parser")),
            onto: Some(label("9f8e7d6", "Start")),
        };
        let pick = OperationView {
            kind: OperationKind::CherryPick,
            step: None,
            branch: None,
            incoming: Some(label("1a2b3c4", "Fix the parser")),
            onto: None,
        };

        assert_eq!(
            rebase.text(),
            "A rebase of feature onto 9f8e7d6 Start stopped at step 3 of 7, applying 1a2b3c4 Fix the parser."
        );
        assert_eq!(
            pick.text(),
            "A cherry-pick of 1a2b3c4 Fix the parser is in progress."
        );
        assert_eq!(CommitLabel::bare(&"a".repeat(40)).text(), "aaaaaaa");
    }
}
