//! A worker's ceiling through the loop (plugins/workers/docs/
//! host-and-ceiling.md, the ceiling; decision D21): a read-only task refuses a
//! write and a command, an ask task asks for both, an autonomous task writes
//! inside an allowed folder without asking and asks outside it, a host rule
//! lets a command run, the worker's own folders ask, a host in full access
//! runs everything without asking (ADR-0029), a task narrows the mode, and a
//! choice that the request does not offer is refused.

#![cfg(test)]

use std::fs;
use std::path::Path;

use mannie_runtime::testing::{WorkerHarness, WorkerOptions, ceiling_mode_line, reply, tool_calls};
use mannie_types::ceiling::CeilingMode;
use mannie_types::ids::RuleId;
use mannie_types::tasks::TaskAssignment;
use mannie_types::tasks::{AssignedTaskState, TaskState};
use mannie_types::tools::{
    ApprovalChoice, ApprovalReason, CommandRule, DecideApproval, Decision, NotRunReason, Refusal,
    ToolCallStatus, ToolCallView, ToolsError,
};
use serde_json::json;

fn write(path: &Path) -> String {
    json!({ "activity": "Writing", "path": path.to_str().unwrap(), "content": "made\n" })
        .to_string()
}

fn read(path: &Path) -> String {
    json!({ "activity": "Reading", "path": path.to_str().unwrap() }).to_string()
}

/// A command that prints a word on every system.
fn echo() -> String {
    json!({ "activity": "Printing", "command": "echo ceiling", "timeout_seconds": 30 }).to_string()
}

fn worker_with(mode: CeilingMode) -> WorkerOptions {
    WorkerOptions {
        mode,
        ..WorkerOptions::default()
    }
}

/// Runs a task whose model makes `calls` in one step, then reports.
async fn run(worker: &WorkerHarness, calls: &[(&str, &str, &str)]) -> TaskAssignment {
    let assignment = worker.assignment("Do the work.");
    run_assignment(worker, assignment, calls).await
}

async fn run_assignment(
    worker: &WorkerHarness,
    assignment: TaskAssignment,
    calls: &[(&str, &str, &str)],
) -> TaskAssignment {
    worker.model.push_reply(tool_calls(calls));
    worker.model.push_reply(reply("Done."));
    worker.model.push_reply(reply("Report: done."));
    worker.assign(assignment.clone()).await;
    assignment
}

/// The tool calls of a task's first turn.
async fn calls_of(worker: &WorkerHarness, assignment: &TaskAssignment) -> Vec<ToolCallView> {
    worker
        .snapshot(&assignment.conversation_id)
        .await
        .tool_calls(0)
}

/// Waits until the task waits for the owner.
async fn awaiting(worker: &WorkerHarness, assignment: &TaskAssignment) -> AssignedTaskState {
    worker
        .wait_for_task(&assignment.task_id, |state| state.approval.is_some())
        .await
}

/// Answers the task's waiting call.
async fn decide(
    worker: &WorkerHarness,
    assignment: &TaskAssignment,
    state: &AssignedTaskState,
    choice: ApprovalChoice,
) -> Result<(), ToolsError> {
    let approval = state.approval.as_ref().unwrap();
    let request = approval.call.request.as_ref().unwrap();
    worker
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: assignment.conversation_id.clone(),
            part_id: approval.part_id.clone(),
            digest: request.digest.clone(),
            choice,
        })
        .await
}

#[tokio::test]
async fn a_read_only_host_refuses_writes_and_commands_and_lets_reads_run() {
    let worker = WorkerHarness::with_options(worker_with(CeilingMode::ReadOnly)).await;
    let inside = worker.allowed.join("notes.txt");
    fs::write(&inside, "notes\n").unwrap();
    let assignment = run(
        &worker,
        &[
            ("call_1", "read_file", &read(&inside)),
            (
                "call_2",
                "write_file",
                &write(&worker.allowed.join("new.txt")),
            ),
            ("call_3", "run_command", &echo()),
        ],
    )
    .await;

    let ended = worker.wait_for_end(&assignment.task_id).await;

    assert_eq!(ended.state, TaskState::Reported);
    let calls = calls_of(&worker, &assignment).await;
    assert_eq!(calls[0].status, ToolCallStatus::Completed);
    for refused in &calls[1..] {
        assert_eq!(refused.status, ToolCallStatus::NotRun, "{}", refused.tool);
        assert_eq!(
            refused.not_run,
            Some(NotRunReason::Refused {
                refusal: Refusal::ReadOnlyHost
            })
        );
    }
    assert!(!worker.allowed.join("new.txt").exists());
    let offered: Vec<String> = worker.model.reply_requests()[0]
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    assert!(!offered.contains(&"write_file".to_owned()), "{offered:?}");
    assert!(!offered.contains(&"run_command".to_owned()), "{offered:?}");
    assert!(offered.contains(&"read_file".to_owned()), "{offered:?}");
}

#[tokio::test]
async fn an_ask_host_asks_for_a_change_inside_the_allowed_folders_and_offers_only_once() {
    let worker = WorkerHarness::with_options(worker_with(CeilingMode::Ask)).await;
    let target = worker.allowed.join("asked.txt");
    let assignment = run(&worker, &[("call_1", "write_file", &write(&target))]).await;

    let waiting = awaiting(&worker, &assignment).await;
    let request = waiting
        .approval
        .as_ref()
        .and_then(|approval| approval.call.request.clone())
        .unwrap();
    let forged = decide(&worker, &assignment, &waiting, ApprovalChoice::AllowAlways).await;
    let for_conversation = decide(
        &worker,
        &assignment,
        &waiting,
        ApprovalChoice::AllowForConversation,
    )
    .await;
    decide(&worker, &assignment, &waiting, ApprovalChoice::AllowOnce)
        .await
        .unwrap();
    let ended = worker.wait_for_end(&assignment.task_id).await;

    assert_eq!(request.reasons, [ApprovalReason::HostAsks]);
    assert_eq!(request.host.as_deref(), Some("build-01"));
    assert_eq!(request.grants, Vec::new());
    assert_eq!(request.rule, None);
    assert_eq!(forged, Err(ToolsError::GrantNotPossible));
    assert_eq!(for_conversation, Err(ToolsError::GrantNotPossible));
    assert_eq!(ended.state, TaskState::Reported);
    assert_eq!(fs::read_to_string(&target).unwrap(), "made\n");
}

#[tokio::test]
async fn a_denied_command_does_not_run() {
    let worker = WorkerHarness::with_options(worker_with(CeilingMode::Ask)).await;
    let assignment = worker.assignment("Do the work.");
    worker
        .model
        .push_reply(tool_calls(&[("call_1", "run_command", &echo())]));
    worker
        .model
        .push_reply(reply("The owner denied the command."));
    worker
        .model
        .push_reply(reply("Report: the owner denied it."));
    worker.assign(assignment.clone()).await;

    let waiting = awaiting(&worker, &assignment).await;
    decide(&worker, &assignment, &waiting, ApprovalChoice::Deny)
        .await
        .unwrap();
    let ended = worker.wait_for_end(&assignment.task_id).await;

    assert_eq!(ended.state, TaskState::Reported);
    let calls = calls_of(&worker, &assignment).await;
    assert_eq!(calls[0].status, ToolCallStatus::Denied);
    assert_eq!(calls[0].decision, Some(Decision::Denied));
}

#[tokio::test]
async fn an_autonomous_host_writes_inside_its_folders_and_asks_outside_them() {
    let worker = WorkerHarness::with_options(worker_with(CeilingMode::Autonomous)).await;
    let inside = worker.allowed.join("inside.txt");
    let outside = worker.outside.join("outside.txt");
    let assignment = run(
        &worker,
        &[
            ("call_1", "write_file", &write(&inside)),
            ("call_2", "write_file", &write(&outside)),
        ],
    )
    .await;

    let waiting = awaiting(&worker, &assignment).await;
    let request = waiting
        .approval
        .as_ref()
        .and_then(|approval| approval.call.request.clone())
        .unwrap();
    decide(&worker, &assignment, &waiting, ApprovalChoice::AllowOnce)
        .await
        .unwrap();
    worker.wait_for_end(&assignment.task_id).await;

    let calls = calls_of(&worker, &assignment).await;
    assert_eq!(calls[0].decision, Some(Decision::AllowedFolder));
    assert_eq!(request.reasons, [ApprovalReason::OutsideAllowedFolders]);
    assert_eq!(calls[1].decision, Some(Decision::AllowedOnce));
    assert!(inside.exists() && outside.exists());
}

#[tokio::test]
async fn a_host_rule_lets_a_command_run_in_autonomous_mode() {
    let worker = WorkerHarness::with_options(WorkerOptions {
        mode: CeilingMode::Autonomous,
        rules: vec![CommandRule {
            id: RuleId::parse("0199e0f2-0000-7000-8000-000000000071").unwrap(),
            words: vec!["echo".to_owned()],
            folder: None,
            created_at: mannie_types::clock::UtcMillis::EPOCH,
            background: false,
        }],
        ..WorkerOptions::default()
    })
    .await;
    let assignment = run(&worker, &[("call_1", "run_command", &echo())]).await;

    let ended = worker.wait_for_end(&assignment.task_id).await;

    assert_eq!(ended.state, TaskState::Reported);
    let calls = calls_of(&worker, &assignment).await;
    assert_eq!(calls[0].status, ToolCallStatus::Completed, "{:?}", calls[0]);
    assert_eq!(calls[0].decision, Some(Decision::HostRule));
}

#[tokio::test]
async fn the_workers_own_folders_ask_even_in_autonomous_mode() {
    let worker = WorkerHarness::with_options(worker_with(CeilingMode::Autonomous)).await;
    let data = worker.data_folder().join("planted.txt");
    let assignment = run(&worker, &[("call_1", "write_file", &write(&data))]).await;

    let waiting = awaiting(&worker, &assignment).await;
    let request = waiting
        .approval
        .as_ref()
        .and_then(|approval| approval.call.request.clone())
        .unwrap();
    decide(&worker, &assignment, &waiting, ApprovalChoice::Deny)
        .await
        .unwrap();
    worker.wait_for_end(&assignment.task_id).await;

    assert_eq!(request.reasons, [ApprovalReason::MannieFolder]);
    assert!(!data.exists());
}

#[tokio::test]
async fn a_host_in_full_access_runs_everything_without_asking() {
    let worker = WorkerHarness::with_options(worker_with(CeilingMode::FullAccess)).await;
    let outside = worker.outside.join("outside.txt");
    let data = worker.data_folder().join("planted.txt");
    let assignment = run(
        &worker,
        &[
            ("call_1", "write_file", &write(&outside)),
            ("call_2", "write_file", &write(&data)),
            ("call_3", "run_command", &echo()),
        ],
    )
    .await;

    let ended = worker.wait_for_end(&assignment.task_id).await;

    assert_eq!(ended.state, TaskState::Reported);
    let calls = calls_of(&worker, &assignment).await;
    let decisions: Vec<_> = calls.iter().map(|call| call.decision).collect();
    assert_eq!(decisions, [Some(Decision::HostFullAccess); 3], "{calls:?}");
    assert!(outside.exists() && data.exists());
}

#[tokio::test]
async fn a_task_narrows_the_ceiling_to_reading_and_never_widens_it() {
    let worker = WorkerHarness::with_options(worker_with(CeilingMode::Autonomous)).await;
    let mut narrowed = worker.assignment("Only look.");
    narrowed.read_only = true;
    let target = worker.allowed.join("never.txt");

    let assignment = run_assignment(
        &worker,
        narrowed,
        &[("call_1", "write_file", &write(&target))],
    )
    .await;
    worker.wait_for_end(&assignment.task_id).await;

    let calls = calls_of(&worker, &assignment).await;
    assert_eq!(
        calls[0].not_run,
        Some(NotRunReason::Refused {
            refusal: Refusal::ReadOnlyHost
        })
    );
    assert!(!target.exists());
    let instructions = &worker.model.reply_requests()[0].instructions;
    assert!(
        instructions.contains(&ceiling_mode_line(CeilingMode::ReadOnly)),
        "{instructions}"
    );
}
