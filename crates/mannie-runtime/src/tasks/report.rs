//! What the model reads of a task (docs/features/child-agents/tasks.md,
//! reports and the confirmation): the report within the result limit, with a
//! long report archived in the parent's conversation, and the result of a
//! task part that a crash or a stop ended before the model got one.

use mannie_store::{NewArchive, TaskEnding};
use mannie_types::ids::ArchiveId;
use mannie_types::run::RunFailure;
use mannie_types::tasks::{TaskDocument, TaskPartDocument, TaskRefusal, TaskState};
use mannie_types::tools::ArchiveRef;

use crate::texts;

/// The name of a report's archive.
const REPORT_ARCHIVE: &str = "report";

/// Who works on a task, as the task's result names it: the child, or what
/// the task's executor calls its target, such as "the worker" (ADR-0031), so
/// an executor's report reads as a child's.
pub(crate) fn doer_of(task: &TaskDocument) -> &str {
    task.executor
        .as_ref()
        .map_or(texts::THE_CHILD, |executor| executor.noun.as_str())
}

/// The result of a task's end, as the model reads it.
#[derive(Debug)]
pub(crate) struct TaskOutput {
    /// The result, within the result limit.
    pub(crate) output: String,
    /// The whole report, archived, when it did not fit.
    pub(crate) archive: Option<(NewArchive, ArchiveRef)>,
}

/// `text` cut to at most `max_bytes` bytes, at a character boundary.
fn cut(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The result of a task that ended, within `limit` bytes. A report that does
/// not fit is cut, and the whole of it goes to an archive whose ID
/// `archive_id` makes, where `read_tool_output` reads it. `doer` names who
/// worked on the task, as [`doer_of`] does.
pub(crate) fn render_report(
    number: u32,
    ending: &TaskEnding,
    doer: &str,
    limit: usize,
    archive_id: impl FnOnce() -> ArchiveId,
) -> TaskOutput {
    let report = ending
        .report
        .as_deref()
        .map(str::trim)
        .filter(|report| !report.is_empty());
    let output = match (ending.state, report) {
        (TaskState::Reported, Some(report)) => {
            let whole = texts::task_reported(number, report);
            if whole.len() <= limit {
                whole
            } else {
                let id = archive_id();
                let note = texts::report_cut(id.as_str());
                let framing = texts::task_reported(number, "").len() + note.len() + 2;
                let head = cut(report, limit.saturating_sub(framing));
                let archive = NewArchive {
                    id: id.clone(),
                    name: REPORT_ARCHIVE.to_owned(),
                    content: report.to_owned(),
                };
                let reference = ArchiveRef {
                    id,
                    name: REPORT_ARCHIVE.to_owned(),
                    bytes: u32::try_from(report.len()).unwrap_or(u32::MAX),
                    lines: u32::try_from(report.lines().count()).unwrap_or(u32::MAX),
                };
                return TaskOutput {
                    output: format!("{}\n\n{note}", texts::task_reported(number, head)),
                    archive: Some((archive, reference)),
                };
            }
        }
        (TaskState::Reported, None) => texts::task_without_report(number),
        (state, _) => ended_text(number, state, ending.failure.as_ref(), doer),
    };
    TaskOutput {
        output,
        archive: None,
    }
}

/// The text of a task that ended without a report.
fn ended_text(number: u32, state: TaskState, failure: Option<&RunFailure>, doer: &str) -> String {
    match state {
        TaskState::Failed => {
            let cause = failure.unwrap_or(&RunFailure::Internal).to_string();
            texts::task_failed(number, &cause, doer)
        }
        TaskState::Canceled => texts::task_stopped(number, doer),
        // A task that still runs when the model reads its part did not end
        // before its parent's run: Mannie's end interrupted both.
        TaskState::Interrupted | TaskState::Accepted | TaskState::Running => {
            texts::task_interrupted(number, doer)
        }
        TaskState::Reported => texts::task_without_report(number),
    }
}

/// The text of a call that assigned no task. An executor's refusal keeps the
/// text that the model read in the part's output, which [`task_result`]
/// reads first; so do the refusals of workers that 0.2.0 stored, since M5's
/// migration wrote theirs (ADR-0031).
fn refusal_text(refusal: &TaskRefusal) -> String {
    match refusal {
        TaskRefusal::UnknownTask { number } => texts::no_such_task(*number),
        TaskRefusal::StillRunning { number } => texts::task_still_running(*number),
        TaskRefusal::BudgetUsed => texts::TASK_BUDGET_USED.to_owned(),
        TaskRefusal::NotStarted => texts::TASK_NOT_STARTED.to_owned(),
        TaskRefusal::WorkerOffline { worker }
        | TaskRefusal::WorkerUnknown { worker }
        | TaskRefusal::WorkerIncompatible { worker }
        | TaskRefusal::ProfileNotReachable { worker } => texts::target_refused(worker),
        TaskRefusal::Executor { target, .. } => {
            texts::target_refused(target.as_deref().unwrap_or_default())
        }
    }
}

/// The text of a task from its part's document, as after a crash that ended
/// the parent's run before the model got the task's result. A part that let
/// an executor's task go says so: the task may still run on its target.
fn task_text(task: &TaskDocument) -> String {
    if let Some(executor) = task.executor.as_ref().filter(|_executor| task.detached) {
        return texts::task_let_go(task.number, &executor.noun, &executor.target_name);
    }
    let doer = doer_of(task);
    let ending = TaskEnding {
        state: task.state,
        report: task.report.clone(),
        failure: task.failure.clone(),
    };
    match task.state {
        TaskState::Reported => {
            render_report(task.number, &ending, doer, usize::MAX, || {
                unreachable!("a report without a limit is never archived")
            })
            .output
        }
        state => ended_text(task.number, state, task.failure.as_ref(), doer),
    }
}

/// What the model reads as the result of a task part: the result it was
/// given; or why the call assigned no task; or else what the task's state
/// says, for a part that ended before it got a result. `None` for a part
/// without any of them, such as a call that a stop canceled before it ran.
pub(crate) fn task_result(section: &TaskPartDocument) -> Option<String> {
    if let Some(output) = &section.output {
        return Some(output.clone());
    }
    if let Some(refusal) = &section.refusal {
        return Some(refusal_text(refusal));
    }
    section.task.as_ref().map(task_text)
}

/// Whether a task part's result holds reports: a wait's, or a start's or a
/// follow-up's that the model waited for.
pub(crate) fn returned_reports(section: &TaskPartDocument) -> bool {
    let waited = !section.awaited.is_empty();
    let foreground = section.task.as_ref().is_some_and(|task| !task.background);
    section.refusal.is_none() && (waited || foreground)
}

/// The numbers of the tasks whose reports a task part's result holds.
pub(crate) fn reported_numbers(section: &TaskPartDocument) -> Vec<u32> {
    if section.awaited.is_empty() {
        section.task.iter().map(|task| task.number).collect()
    } else {
        section.awaited.iter().map(|task| task.number).collect()
    }
}

#[cfg(test)]
mod tests {
    use mannie_store::TaskEnding;
    use mannie_types::ids::{ArchiveId, ConversationId, TaskId};
    use mannie_types::run::RunFailure;
    use mannie_types::tasks::{TaskDocument, TaskPartDocument, TaskRefusal, TaskState};

    use super::{render_report, task_result};
    use crate::texts;

    fn archive_id() -> ArchiveId {
        ArchiveId::parse("0199e0f2-0000-7000-a000-000000000009").unwrap()
    }

    fn reported(report: &str) -> TaskEnding {
        TaskEnding {
            state: TaskState::Reported,
            report: Some(report.to_owned()),
            failure: None,
        }
    }

    #[test]
    fn a_report_that_fits_is_the_whole_result() {
        let output = render_report(
            2,
            &reported("Found it in session.rs."),
            texts::THE_CHILD,
            1_000,
            || panic!("nothing is archived"),
        );

        assert_eq!(output.output, "Task 2 reported:\n\nFound it in session.rs.");
        assert!(output.archive.is_none());
    }

    #[test]
    fn a_long_report_is_cut_to_the_limit_and_archived_whole() {
        let report = "é".repeat(5_000);

        let output = render_report(1, &reported(&report), texts::THE_CHILD, 2_000, archive_id);

        assert!(output.output.len() <= 2_000, "{}", output.output.len());
        assert!(output.output.starts_with("Task 1 reported:\n\né"));
        assert!(
            output
                .output
                .ends_with(&texts::report_cut(archive_id().as_str()))
        );
        let (archive, reference) = output.archive.unwrap();
        assert_eq!(archive.content, report);
        assert_eq!(reference.bytes, 10_000);
        assert_eq!(reference.name, "report");
    }

    #[test]
    fn ends_without_a_report_say_what_happened() {
        let failed = TaskEnding {
            state: TaskState::Failed,
            report: None,
            failure: Some(RunFailure::QuotaExceeded),
        };
        let blank = reported("   ");

        assert_eq!(
            render_report(3, &failed, "the worker", 1_000, archive_id).output,
            texts::task_failed(3, "the account's quota is used up", "the worker")
        );
        assert_eq!(
            render_report(3, &blank, texts::THE_CHILD, 1_000, archive_id).output,
            texts::task_without_report(3)
        );
    }

    #[test]
    fn a_part_reads_as_its_result_its_refusal_or_its_tasks_state() {
        let given = TaskPartDocument {
            output: Some("Task 1 started in the background.".to_owned()),
            ..TaskPartDocument::default()
        };
        let refused = TaskPartDocument {
            refusal: Some(TaskRefusal::StillRunning { number: 4 }),
            ..TaskPartDocument::default()
        };
        let interrupted = TaskPartDocument {
            task: Some(TaskDocument {
                task_id: TaskId::parse("0199e0f2-0000-7000-a000-000000000001").unwrap(),
                number: 5,
                conversation_id: ConversationId::parse("0199e0f2-0000-7000-a000-000000000002")
                    .unwrap(),
                agent: "general".to_owned(),
                title: "Fix it".to_owned(),
                background: false,
                follow_up: false,
                state: TaskState::Interrupted,
                instruction: String::new(),
                activity: None,
                tool_calls: 2,
                approval: None,
                report: None,
                failure: None,
                started_at: None,
                ended_at: None,
                worker: None,
                executor: None,
                detached: false,
            }),
            ..TaskPartDocument::default()
        };

        assert_eq!(
            task_result(&given).as_deref(),
            Some("Task 1 started in the background.")
        );
        assert_eq!(task_result(&refused), Some(texts::task_still_running(4)));
        assert_eq!(
            task_result(&interrupted),
            Some(texts::task_interrupted(5, texts::THE_CHILD))
        );
        assert_eq!(task_result(&TaskPartDocument::default()), None);
    }
}
