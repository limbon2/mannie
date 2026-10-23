//! Delegating tasks to workers through the Workers plugin's executor
//! (ADR-0027, ADR-0031; plugins/workers/docs/delegation.md): a foreground
//! delegation returns the worker's report and a background one its number, a
//! task's lease is delivered once and revoked at its end, numbers are shared
//! with child agents, a follow-up continues on the same worker, the owner
//! answers a worker's approval in the parent's card, Stop cancels the
//! reply's worker tasks, a worker that cannot be reached is refused or let
//! go, quitting leaves the tasks running and a wait takes them up after a
//! restart, a worker that does not know a task ends it, a removed worker's
//! tasks end, the task's conversation is read through the worker, deleting
//! the parent deletes it there, `delegate_task` is offered only while
//! workers are paired, Workers reports its running tasks as its work, and
//! turning Workers off and on loses and repeats nothing
//! (plugins/workers/docs/plugin.md).
//!
//! The master and the worker run in one process, joined by an in-process
//! link. Each has a fake model whose scripts play in order.

#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use mannie_plugin_harness::provider::Step;
use mannie_plugin_harness::provider::{Content, ModelRequest, Role};
use mannie_plugin_harness::runtime::{
    Harness, HarnessOptions, Transcript, WorkerOptions, fast_config, reply, tool_calls,
};
use mannie_plugin_harness::runtime::{RuntimeConfig, texts};
use mannie_plugin_harness::store::StoredTask;
use mannie_plugin_harness::types::PartContent;
use mannie_plugin_harness::types::RunId;
use mannie_plugin_harness::types::RunState;
use mannie_plugin_harness::types::{Decision, UpdateToolsSettings};
use mannie_plugin_harness::types::{
    SendMessage, StartConversation, StopOutcome, SubscribeConversation,
};
use mannie_plugin_harness::types::{TaskRefusal, TaskView};
use mannie_plugin_sdk::contracts::CeilingMode;
use mannie_plugin_sdk::contracts::ChatError;
use mannie_plugin_sdk::contracts::ConversationId;
use mannie_plugin_sdk::contracts::ConversationUpdate;
use mannie_plugin_sdk::contracts::WorkItem;
use mannie_plugin_sdk::contracts::{ApprovalChoice, DecideApproval};
use mannie_plugin_sdk::contracts::{TaskExecutorRef, TaskState};
use mannie_workers::plugin::ACTIVE_TASKS;
use mannie_workers::texts as worker_texts;
use mannie_workers::{WORKER_AGENT, WORKERS_KIND};
use mannie_workers_testing::master::{DelegationHarness, LOCAL_WORKER_ID, LOCAL_WORKER_NAME};
use serde_json::json;

/// A step of the model that never ends, so a run stays busy until it stops.
fn hang() -> Vec<Step> {
    vec![Step::Hang]
}

fn delegate(instruction: &str, background: bool) -> String {
    json!({
        "activity": "Delegating",
        "worker": LOCAL_WORKER_NAME,
        "title": "Build the backend",
        "instruction": instruction,
        "background": background,
        "read_only": false,
    })
    .to_string()
}

fn follow_up(task: u32, instruction: &str) -> String {
    json!({ "activity": "Following up", "task": task, "instruction": instruction, "background": false })
        .to_string()
}

fn wait(tasks: &[u32]) -> String {
    json!({ "activity": "Waiting", "tasks": tasks }).to_string()
}

/// The tool results of a request, by call ID.
fn result(request: &ModelRequest, call_id: &str) -> String {
    request
        .messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .flat_map(|message| &message.content)
        .find_map(|content| match content {
            Content::ToolResult {
                call_id: id,
                output,
            } if id == call_id => Some(output.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no result for {call_id}"))
}

/// The target of the test worker's tasks.
fn the_worker() -> TaskExecutorRef {
    TaskExecutorRef {
        kind: WORKERS_KIND.to_owned(),
        target_id: LOCAL_WORKER_ID.to_owned(),
        target_name: LOCAL_WORKER_NAME.to_owned(),
        noun: worker_texts::THE_WORKER.to_owned(),
    }
}

/// The refusal of a call of `delegate_task` with the executor's code.
fn refused(code: &str, worker: &str) -> TaskRefusal {
    TaskRefusal::Executor {
        kind: WORKERS_KIND.to_owned(),
        code: code.to_owned(),
        target: Some(worker.to_owned()),
    }
}

/// The executor's document of a task's row.
fn document_of(task: &StoredTask) -> serde_json::Value {
    let document = task
        .executor
        .as_ref()
        .and_then(|execution| execution.document.as_deref())
        .expect("a worker task has a document");
    serde_json::from_str(document).unwrap()
}

/// A configuration whose Stop gives the workers a short time to confirm.
fn quick_cancels() -> RuntimeConfig {
    RuntimeConfig {
        executor_cancel_timeout: Duration::from_millis(300),
        ..fast_config()
    }
}

async fn start(setup: &DelegationHarness, text: &str) -> (ConversationId, RunId) {
    let started = setup
        .master
        .runtime
        .start_conversation(StartConversation::new(text))
        .await
        .unwrap();
    (started.conversation_id, started.run_id)
}

/// The worker answers a task once and confirms it with `report`.
fn worker_reports(setup: &DelegationHarness, report: &str) {
    setup.worker.model.push_reply(reply("Working on it."));
    setup.worker.model.push_reply(reply(report));
}

/// Waits until the model of the worker got `calls` reply requests.
async fn until_worker_called(setup: &DelegationHarness, calls: usize) {
    let wait = async {
        while setup.worker.model.reply_requests().len() < calls {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(20), wait)
        .await
        .expect("the worker's model is called");
}

/// The conversations whose deletions wait for the test worker in the
/// core's outbox.
async fn waiting_deletions(setup: &DelegationHarness) -> Vec<ConversationId> {
    setup
        .master
        .store
        .executor_outbox(WORKERS_KIND)
        .await
        .unwrap()
        .into_iter()
        .filter(|entry| entry.target == LOCAL_WORKER_ID)
        .map(|entry| entry.conversation_id)
        .collect()
}

/// Waits until the worker deleted a task's conversation and the master
/// forgot the deletion.
async fn until_deleted_on_the_worker(setup: &DelegationHarness, conversation: &ConversationId) {
    let deleted = async {
        loop {
            let on_worker = setup
                .worker
                .store
                .conversation_summary(conversation)
                .await
                .unwrap();
            if on_worker.is_none() && waiting_deletions(setup).await.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(20), deleted)
        .await
        .expect("the worker deletes the task's conversation");
}

/// The tasks of the turn at `position` of the parent, once `done` holds.
async fn tasks_when(
    setup: &DelegationHarness,
    conversation: &ConversationId,
    position: u32,
    done: impl Fn(&[TaskView]) -> bool,
) -> Vec<TaskView> {
    let transcript = setup
        .master
        .wait_until(conversation, |transcript: &Transcript| {
            done(&transcript.tasks(position))
        })
        .await;
    transcript.tasks(position)
}

#[tokio::test]
async fn a_foreground_delegation_returns_the_workers_report() {
    let setup = DelegationHarness::start().await;
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build the backend and run its tests.", false),
    )]));
    setup.master.model.push_reply(reply("The worker built it."));
    worker_reports(&setup, "Report: the tests pass.");

    let (conversation, run) = start(&setup, "Build it on the server").await;
    let transcript = setup.master.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    assert_eq!(transcript.reply_text(0), "The worker built it.");
    let tasks = transcript.tasks(0);
    let task = &tasks[0];
    assert_eq!(task.number, 1);
    assert_eq!(task.executor.as_ref(), Some(&the_worker()));
    assert_eq!(task.agent, WORKER_AGENT);
    assert_eq!(task.state, TaskState::Reported);
    assert_eq!(task.report.as_deref(), Some("Report: the tests pass."));
    assert!(!task.detached);
    let requests = setup.master.model.reply_requests();
    assert_eq!(
        result(&requests[1], "call_1"),
        texts::task_reported(1, worker_texts::THE_WORKER, "Report: the tests pass.")
    );
    let worker_requests = setup.worker.model.reply_requests();
    assert_eq!(
        worker_requests[0]
            .messages
            .first()
            .map(|message| message.content.clone()),
        Some(vec![Content::Text(
            "Build the backend and run its tests.".to_owned()
        )])
    );
    assert_eq!(
        setup.worker.model.credentials(),
        [
            Some("sk-test-credential".to_owned()),
            Some("sk-test-credential".to_owned())
        ],
        "the worker's runs used the master's credential as a lease"
    );
    let stored = setup
        .master
        .store
        .task(&task.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.state, TaskState::Reported);
    let execution = stored.executor.clone().unwrap();
    assert_eq!(execution.kind, WORKERS_KIND);
    assert_eq!(execution.target, LOCAL_WORKER_ID);
    assert_eq!(
        document_of(&stored),
        json!({
            "version": 1,
            "profileId": setup.master.profile.id,
            "submission": "accepted",
            "cancelRequestedAt": null,
        })
    );
    assert_eq!(setup.worker.vault.references(), Vec::<String>::new());
}

#[tokio::test]
async fn a_tasks_lease_is_delivered_once_and_revoked_at_its_end() {
    let setup = DelegationHarness::start().await;
    // The follower of the worker sees the task wait for its lease before the
    // run that submitted the task gets the worker's answer, so both offer
    // the lease.
    setup.delegation.answer_when_waiting();
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build the backend and run its tests.", false),
    )]));
    setup.master.model.push_reply(reply("The worker built it."));
    worker_reports(&setup, "Report: the tests pass.");

    let (conversation, run) = start(&setup, "Build it on the server").await;
    let transcript = setup.master.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let tasks = transcript.tasks(0);
    assert_eq!(tasks[0].state, TaskState::Reported);
    assert_eq!(
        setup.delegation.delivered_leases(),
        [tasks[0].task_id.clone()],
        "the master delivered the task's lease once"
    );
    // The follower revokes the lease after it published the task's end,
    // which let the run go on.
    let revoked = async {
        while setup.delegation.revoked_leases().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(20), revoked)
        .await
        .expect("the master revokes the lease of the task that ended");
    assert_eq!(
        setup.delegation.revoked_leases(),
        [tasks[0].task_id.clone()]
    );
    assert_eq!(setup.worker.vault.references(), Vec::<String>::new());
}

#[tokio::test]
async fn a_background_delegation_shares_the_numbers_and_reports_through_a_wait() {
    let setup = DelegationHarness::start().await;
    let explore = json!({
        "activity": "Exploring",
        "title": "Find the config",
        "agent": "explore",
        "instruction": "CHILD-1: find the config.",
        "background": true,
    })
    .to_string();
    setup.master.model.push_reply(tool_calls(&[
        ("call_1", "start_task", &explore),
        (
            "call_2",
            "delegate_task",
            &delegate("Build the backend.", true),
        ),
    ]));
    setup
        .master
        .model
        .push_reply_for("CHILD-1", reply("The config is in app.toml."));
    setup
        .master
        .model
        .push_reply_for("CHILD-1", reply("Report: app.toml."));
    setup
        .master
        .model
        .push_reply(tool_calls(&[("call_3", "wait_for_tasks", &wait(&[2]))]));
    setup
        .master
        .model
        .push_reply(tool_calls(&[("call_4", "wait_for_tasks", &wait(&[]))]));
    setup.master.model.push_reply(reply("Both done."));
    worker_reports(&setup, "Report: built in the background.");

    let (conversation, run) = start(&setup, "Do both").await;
    let transcript = setup.master.wait_for_run(&conversation, &run).await;

    let tasks = transcript.tasks(0);
    assert_eq!(
        tasks
            .iter()
            .map(|task| (task.number, task.executor.is_some()))
            .collect::<Vec<_>>(),
        [(1, false), (2, true)]
    );
    let requests = setup.master.model.reply_requests();
    let parent: Vec<&ModelRequest> = requests
        .iter()
        .filter(|request| {
            !request
                .instructions
                .contains(texts::CHILD_BASE_INSTRUCTIONS)
        })
        .collect();
    assert_eq!(
        result(parent[1], "call_2"),
        texts::task_started_in_background_on(2, worker_texts::THE_WORKER, LOCAL_WORKER_NAME)
    );
    assert_eq!(
        result(parent[2], "call_3"),
        texts::task_reported(
            2,
            worker_texts::THE_WORKER,
            "Report: built in the background."
        )
    );
    assert_eq!(transcript.reply_text(0), "Both done.");
}

#[tokio::test]
async fn a_follow_up_continues_the_tasks_conversation_on_the_same_worker() {
    let setup = DelegationHarness::start().await;
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build the backend.", false),
    )]));
    setup.master.model.push_reply(tool_calls(&[(
        "call_2",
        "follow_up_task",
        &follow_up(1, "Now deploy it."),
    )]));
    setup.master.model.push_reply(reply("Built and deployed."));
    worker_reports(&setup, "Report: built.");
    worker_reports(&setup, "Report: deployed.");

    let (conversation, run) = start(&setup, "Build and deploy").await;
    let transcript = setup.master.wait_for_run(&conversation, &run).await;

    let tasks = transcript.tasks(0);
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[1].number, 1);
    assert!(tasks[1].follow_up);
    assert_eq!(tasks[1].conversation_id, tasks[0].conversation_id);
    assert_eq!(tasks[1].report.as_deref(), Some("Report: deployed."));
    let worker_requests = setup.worker.model.reply_requests();
    let continued = worker_requests.last().unwrap();
    let first = continued.messages.first().unwrap();
    assert_eq!(
        first.content,
        vec![Content::Text("Build the backend.".to_owned())]
    );
}

#[tokio::test]
async fn the_owner_answers_a_workers_approval_in_the_parents_card() {
    let setup = DelegationHarness::with_options(
        HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        },
        WorkerOptions {
            mode: CeilingMode::Ask,
            ..WorkerOptions::default()
        },
        "127.0.0.1:7445",
    )
    .await;
    let target = setup.worker.allowed.join("built.txt");
    let write =
        json!({ "activity": "Writing", "path": target.to_str().unwrap(), "content": "built\n" })
            .to_string();
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Write the build marker.", false),
    )]));
    setup
        .master
        .model
        .push_reply(reply("Written on the worker."));
    setup
        .worker
        .model
        .push_reply(tool_calls(&[("call_w", "write_file", &write)]));
    worker_reports(&setup, "Report: written.");

    let (conversation, run) = start(&setup, "Write it").await;
    let tasks = tasks_when(&setup, &conversation, 0, |tasks| {
        tasks.first().is_some_and(|task| task.approval.is_some())
    })
    .await;
    let approval = tasks[0].approval.clone().unwrap();
    let request = approval.call.request.clone().unwrap();
    let forged = setup
        .master
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: tasks[0].conversation_id.clone(),
            part_id: approval.part_id.clone(),
            digest: request.digest.clone(),
            choice: ApprovalChoice::AllowAlways,
        })
        .await;
    setup
        .master
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: tasks[0].conversation_id.clone(),
            part_id: approval.part_id.clone(),
            digest: request.digest.clone(),
            choice: ApprovalChoice::AllowOnce,
        })
        .await
        .unwrap();
    let transcript = setup.master.wait_for_run(&conversation, &run).await;

    assert_eq!(request.host.as_deref(), Some("build-01"));
    assert_eq!(
        forged,
        Err(mannie_plugin_sdk::contracts::ToolsError::GrantNotPossible)
    );
    assert_eq!(transcript.tasks(0)[0].state, TaskState::Reported);
    assert_eq!(fs::read_to_string(&target).unwrap(), "built\n");
}

/// A master whose new conversations start in full access, and a worker in
/// `worker` options whose task writes `built.txt` in its allowed folder, which
/// its ask mode asks for.
async fn full_access_delegation(worker: WorkerOptions) -> (DelegationHarness, PathBuf) {
    let setup = DelegationHarness::with_options(
        HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        },
        worker,
        "127.0.0.1:7445",
    )
    .await;
    setup
        .master
        .runtime
        .update_tools_settings(UpdateToolsSettings {
            allow_changes_in_workspace: false,
            command_environment: Vec::new(),
            full_access_for_new_conversations: true,
            wake_on_command_end: true,
        })
        .await
        .unwrap();
    let target = setup.worker.allowed.join("built.txt");
    let write =
        json!({ "activity": "Writing", "path": target.to_str().unwrap(), "content": "built\n" })
            .to_string();
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Write the build marker.", false),
    )]));
    setup
        .master
        .model
        .push_reply(reply("Written on the worker."));
    setup
        .worker
        .model
        .push_reply(tool_calls(&[("call_w", "write_file", &write)]));
    worker_reports(&setup, "Report: written.");
    (setup, target)
}

/// The decisions of the tool calls of a task's conversation, as the worker
/// journaled them.
async fn worker_decisions(setup: &DelegationHarness, task: &TaskView) -> Vec<Option<Decision>> {
    setup
        .worker
        .store
        .model_turns(&task.conversation_id, None)
        .await
        .unwrap()
        .iter()
        .flat_map(|turn| &turn.parts)
        .filter_map(|part| part.tool_call.as_ref())
        .map(|call| call.decision)
        .collect()
}

#[tokio::test]
async fn a_conversation_in_full_access_answers_its_worker_tasks_requests() {
    let (setup, target) = full_access_delegation(WorkerOptions {
        mode: CeilingMode::Ask,
        ..WorkerOptions::default()
    })
    .await;

    let (conversation, run) = start(&setup, "Write it").await;
    let transcript = setup.master.wait_for_run(&conversation, &run).await;

    let task = transcript.tasks(0)[0].clone();
    assert_eq!(task.state, TaskState::Reported);
    assert_eq!(fs::read_to_string(&target).unwrap(), "built\n");
    assert_eq!(
        worker_decisions(&setup, &task).await,
        [Some(Decision::FullAccess)]
    );
}

#[tokio::test]
async fn a_host_that_refuses_the_masters_full_access_asks_the_owner() {
    let (setup, target) = full_access_delegation(WorkerOptions {
        mode: CeilingMode::Ask,
        accepts_master_full_access: false,
        ..WorkerOptions::default()
    })
    .await;

    let (conversation, run) = start(&setup, "Write it").await;
    let tasks = tasks_when(&setup, &conversation, 0, |tasks| {
        tasks.first().is_some_and(|task| task.approval.is_some())
    })
    .await;
    assert!(!target.exists(), "the request waits for the owner");
    let approval = tasks[0].approval.clone().unwrap();
    setup
        .master
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: tasks[0].conversation_id.clone(),
            part_id: approval.part_id.clone(),
            digest: approval.call.request.clone().unwrap().digest,
            choice: ApprovalChoice::AllowOnce,
        })
        .await
        .unwrap();
    let transcript = setup.master.wait_for_run(&conversation, &run).await;

    let task = transcript.tasks(0)[0].clone();
    assert_eq!(task.state, TaskState::Reported);
    assert_eq!(
        worker_decisions(&setup, &task).await,
        [Some(Decision::AllowedOnce)]
    );
}

#[tokio::test]
async fn stop_cancels_the_replys_worker_tasks() {
    let setup = DelegationHarness::with_options(
        HarnessOptions {
            use_tools: true,
            // This online confirmation uses the normal cancellation grace;
            // the offline tests exercise the short deadline and letting go.
            config: fast_config(),
            ..HarnessOptions::default()
        },
        WorkerOptions::default(),
        "127.0.0.1:7445",
    )
    .await;
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build forever.", false),
    )]));
    setup.worker.model.push_reply(hang());

    let (conversation, run) = start(&setup, "Build").await;
    tasks_when(&setup, &conversation, 0, |tasks| {
        tasks
            .first()
            .is_some_and(|task| task.state == TaskState::Running)
    })
    .await;
    until_worker_called(&setup, 1).await;
    let stopped = setup.master.runtime.stop_reply(&conversation).await;
    let transcript = setup.master.wait_for_run(&conversation, &run).await;

    assert_eq!(stopped, StopOutcome::Stopped);
    assert_eq!(transcript.run_state(&run), Some(RunState::Stopped));
    let task = &transcript.tasks(0)[0];
    assert_eq!(task.state, TaskState::Canceled);
    assert!(!task.detached);
    let on_worker = setup
        .worker
        .runtime
        .assigned_task(&task.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(on_worker.state, TaskState::Canceled);
}

#[tokio::test]
async fn workers_reports_its_running_tasks_as_work_until_they_end() {
    let setup = DelegationHarness::with_options(
        HarnessOptions {
            use_tools: true,
            config: quick_cancels(),
            ..HarnessOptions::default()
        },
        WorkerOptions::default(),
        "127.0.0.1:7445",
    )
    .await;
    let executor = setup.executor.clone().unwrap();
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build forever.", false),
    )]));
    setup.worker.model.push_reply(hang());

    let idle = executor.work().await;
    let (conversation, run) = start(&setup, "Build").await;
    tasks_when(&setup, &conversation, 0, |tasks| {
        tasks
            .first()
            .is_some_and(|task| task.state == TaskState::Running)
    })
    .await;
    until_worker_called(&setup, 1).await;
    let busy = executor.work().await;
    setup.master.runtime.stop_reply(&conversation).await;
    setup.master.wait_for_run(&conversation, &run).await;
    let ended = executor.work().await;

    assert_eq!(idle, Vec::new());
    assert_eq!(busy, vec![WorkItem::new(ACTIVE_TASKS, 1)]);
    assert_eq!(ended, Vec::new());
}

#[tokio::test]
async fn stop_lets_go_of_a_task_whose_worker_is_offline_and_the_cancel_waits_for_it() {
    let setup = DelegationHarness::with_options(
        HarnessOptions {
            use_tools: true,
            config: quick_cancels(),
            ..HarnessOptions::default()
        },
        WorkerOptions::default(),
        "127.0.0.1:7445",
    )
    .await;
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build forever.", false),
    )]));
    setup.worker.model.push_reply(hang());
    let (conversation, run) = start(&setup, "Build").await;
    let tasks = tasks_when(&setup, &conversation, 0, |tasks| {
        tasks
            .first()
            .is_some_and(|task| task.state == TaskState::Running)
    })
    .await;
    until_worker_called(&setup, 1).await;
    let task = tasks[0].task_id.clone();

    setup.delegation.set_online(false);
    setup.master.runtime.stop_reply(&conversation).await;
    let transcript = setup.master.wait_for_run(&conversation, &run).await;
    let let_go = transcript.tasks(0)[0].clone();
    let queued = setup.master.store.task(&task).await.unwrap().unwrap();
    setup.delegation.set_online(true);
    let ended = setup.worker.wait_for_end(&task).await;
    let master_row = async {
        loop {
            let row = setup.master.store.task(&task).await.unwrap().unwrap();
            if !row.state.is_active() {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let master_row = tokio::time::timeout(Duration::from_secs(20), master_row)
        .await
        .expect("the master learns the end");

    assert!(let_go.detached, "the part lets the task go");
    assert!(
        document_of(&queued)["cancelRequestedAt"].is_i64(),
        "the cancel waits"
    );
    assert!(queued.state.is_active());
    assert_eq!(
        ended.state,
        TaskState::Canceled,
        "the worker got the cancel"
    );
    assert_eq!(master_row.state, TaskState::Canceled);
}

#[tokio::test]
async fn quitting_leaves_a_worker_task_running_and_a_wait_takes_it_up_after_a_restart() {
    let setup = DelegationHarness::start().await;
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build slowly.", false),
    )]));
    setup.worker.model.push_reply(vec![
        Step::Delay(Duration::from_millis(400)),
        Step::Event(mannie_plugin_harness::provider::ProviderEvent::TextDelta(
            "Built.".to_owned(),
        )),
        Step::Event(mannie_plugin_harness::provider::ProviderEvent::Finished(
            mannie_plugin_harness::types::FinishReason::Stop,
        )),
    ]);
    setup
        .worker
        .model
        .push_reply(reply("Report: built after the restart."));
    let (conversation, _run) = start(&setup, "Build").await;
    tasks_when(&setup, &conversation, 0, |tasks| {
        tasks
            .first()
            .is_some_and(|task| task.state == TaskState::Running)
    })
    .await;

    let setup = setup.restart_master().await;
    let snapshot = setup.master.snapshot(&conversation).await;
    let let_go = snapshot.tasks(0)[0].clone();
    setup
        .master
        .model
        .push_reply(tool_calls(&[("call_2", "wait_for_tasks", &wait(&[1]))]));
    setup.master.model.push_reply(reply("It finished."));
    let reply_started = setup
        .master
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "Is it done?"))
        .await
        .unwrap();
    let transcript = setup
        .master
        .wait_for_run(&conversation, &reply_started.run_id)
        .await;

    assert!(let_go.detached);
    assert_eq!(transcript.reply_text(1), "It finished.");
    let requests = setup.master.model.reply_requests();
    let last = requests.last().unwrap();
    assert_eq!(
        result(last, "call_1"),
        texts::task_let_go(1, worker_texts::THE_WORKER, LOCAL_WORKER_NAME),
        "the first reply's part says that the task was let go"
    );
    assert_eq!(
        result(last, "call_2"),
        texts::task_reported(
            1,
            worker_texts::THE_WORKER,
            "Report: built after the restart."
        )
    );
}

#[tokio::test]
async fn a_task_that_the_worker_does_not_know_after_a_restart_ends_as_interrupted() {
    let setup = DelegationHarness::start().await;
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build forever.", false),
    )]));
    setup.worker.model.push_reply(hang());
    let (conversation, _run) = start(&setup, "Build").await;
    let tasks = tasks_when(&setup, &conversation, 0, |tasks| {
        tasks
            .first()
            .is_some_and(|task| task.state == TaskState::Running)
    })
    .await;
    let task = tasks[0].task_id.clone();
    setup.delegation.set_online(false);
    let setup = setup.restart_master().await;

    // The worker forgets the task, as one whose data was reset would.
    setup
        .worker
        .runtime
        .delete_assigned_conversations(std::slice::from_ref(&tasks[0].conversation_id))
        .await
        .unwrap();
    setup.delegation.set_online(true);
    let ended = async {
        loop {
            let row = setup.master.store.task(&task).await.unwrap().unwrap();
            if !row.state.is_active() {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let row = tokio::time::timeout(Duration::from_secs(20), ended)
        .await
        .expect("the master compares its tasks with the worker's");

    assert_eq!(row.state, TaskState::Interrupted);
}

#[tokio::test]
async fn removing_a_worker_ends_its_tasks_on_the_master() {
    let setup = DelegationHarness::start().await;
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build forever.", true),
    )]));
    setup
        .master
        .model
        .push_reply(tool_calls(&[("call_2", "wait_for_tasks", &wait(&[1]))]));
    setup.master.model.push_reply(reply("The worker is gone."));
    setup.worker.model.push_reply(hang());
    let (conversation, run) = start(&setup, "Build").await;
    tasks_when(&setup, &conversation, 0, |tasks| {
        tasks
            .first()
            .is_some_and(|task| task.state == TaskState::Running)
    })
    .await;

    setup.delegation.remove();
    let transcript = setup.master.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.tasks(0)[0].state, TaskState::Interrupted);
    assert_eq!(transcript.reply_text(0), "The worker is gone.");
}

#[tokio::test]
async fn a_worker_that_cannot_take_a_task_is_refused_before_anything_runs() {
    let setup = DelegationHarness::with_options(
        HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        },
        WorkerOptions::default(),
        "build-01.lan:7445",
    )
    .await;
    let unknown = json!({
        "activity": "Delegating",
        "worker": "nobody",
        "title": "T",
        "instruction": "Do it.",
        "background": false,
        "read_only": false,
    })
    .to_string();
    setup.master.model.push_reply(tool_calls(&[
        ("call_1", "delegate_task", &delegate("Build it.", false)),
        ("call_2", "delegate_task", &unknown),
    ]));
    setup.master.model.push_reply(reply("Neither ran."));

    let (conversation, run) = start(&setup, "Build").await;
    let transcript = setup.master.wait_for_run(&conversation, &run).await;

    let calls = transcript.task_calls(0);
    assert_eq!(
        calls[0].refusal,
        Some(refused("profile_not_reachable", LOCAL_WORKER_NAME)),
        "the profile's loopback address names the worker's own host there"
    );
    assert_eq!(calls[1].refusal, Some(refused("worker_unknown", "nobody")));
    let requests = setup.master.model.reply_requests();
    assert_eq!(
        result(&requests[1], "call_1"),
        worker_texts::profile_not_reachable(LOCAL_WORKER_NAME)
    );
    assert_eq!(
        result(&requests[1], "call_2"),
        worker_texts::no_such_worker("nobody")
    );
    assert_eq!(setup.worker.model.reply_requests().len(), 0);
}

#[tokio::test]
async fn an_offline_worker_is_refused() {
    let setup = DelegationHarness::start().await;
    setup.delegation.set_online(false);
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build it.", false),
    )]));
    setup
        .master
        .model
        .push_reply(reply("The worker is offline."));

    let (conversation, run) = start(&setup, "Build").await;
    let transcript = setup.master.wait_for_run(&conversation, &run).await;

    assert_eq!(
        transcript.task_calls(0)[0].refusal,
        Some(refused("worker_offline", LOCAL_WORKER_NAME))
    );
    assert_eq!(
        result(&setup.master.model.reply_requests()[1], "call_1"),
        worker_texts::worker_offline(LOCAL_WORKER_NAME)
    );
}

#[tokio::test]
async fn a_submission_whose_answer_was_lost_runs_the_task_once() {
    let setup = DelegationHarness::start().await;
    setup.delegation.lose_submit_answers(1);
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build it once.", false),
    )]));
    setup.master.model.push_reply(reply("Built once."));
    worker_reports(&setup, "Report: built once.");

    let (conversation, run) = start(&setup, "Build").await;
    let transcript = setup.master.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.tasks(0)[0].state, TaskState::Reported);
    assert_eq!(
        setup.worker.model.reply_requests().len(),
        2,
        "one task ran, with one confirmation"
    );
}

#[tokio::test]
async fn the_tasks_conversation_is_read_through_its_worker() {
    let setup = DelegationHarness::start().await;
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build the backend.", false),
    )]));
    setup.master.model.push_reply(reply("Built."));
    worker_reports(&setup, "Report: built.");
    let (conversation, run) = start(&setup, "Build").await;
    let transcript = setup.master.wait_for_run(&conversation, &run).await;
    let task = transcript.tasks(0)[0].clone();

    let mut subscription = setup
        .master
        .runtime
        .subscribe(SubscribeConversation {
            conversation_id: task.conversation_id.clone(),
            after_seq: None,
        })
        .await
        .unwrap();
    let first = subscription.next().await.unwrap();

    let ConversationUpdate::Snapshot { snapshot } = first else {
        panic!("a snapshot comes first: {first:?}");
    };
    let origin = snapshot.conversation.task.clone().unwrap();
    assert_eq!(origin.parent_id, conversation);
    assert_eq!(origin.number, 1);
    assert_eq!(origin.executor.as_ref(), Some(&the_worker()));
    let user = snapshot.turns[0]
        .parts
        .first()
        .map(|part| part.content.clone());
    assert_eq!(
        user,
        Some(PartContent::UserText {
            text: "Build the backend.".to_owned()
        })
    );
    setup.delegation.set_online(false);
    let after_the_link = subscription.next().await;
    assert!(
        matches!(after_the_link, Some(ConversationUpdate::Ended { .. })),
        "{after_the_link:?}"
    );
    let unreachable = setup
        .master
        .runtime
        .subscribe(SubscribeConversation {
            conversation_id: task.conversation_id.clone(),
            after_seq: None,
        })
        .await;
    assert!(
        matches!(
            unreachable,
            Err(ChatError::TaskTargetUnreachable { ref kind, ref target })
                if kind == WORKERS_KIND && target == LOCAL_WORKER_NAME
        ),
        "{unreachable:?}"
    );
}

#[tokio::test]
async fn deleting_the_parent_deletes_the_tasks_conversation_on_the_worker() {
    let setup = DelegationHarness::start().await;
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build the backend.", false),
    )]));
    setup.master.model.push_reply(reply("Built."));
    worker_reports(&setup, "Report: built.");
    let (conversation, run) = start(&setup, "Build").await;
    let transcript = setup.master.wait_for_run(&conversation, &run).await;
    let task = transcript.tasks(0)[0].clone();

    setup
        .master
        .runtime
        .delete_conversation(&conversation)
        .await
        .unwrap();
    let queued = waiting_deletions(&setup).await;
    // The worker deletes the conversation, and the master forgets the
    // deletion once the worker confirmed it.
    until_deleted_on_the_worker(&setup, &task.conversation_id).await;

    assert!(
        queued.is_empty() || queued == [task.conversation_id.clone()],
        "{queued:?}"
    );
    assert_eq!(
        setup
            .master
            .store
            .conversation_summary(&task.conversation_id)
            .await
            .unwrap(),
        None,
        "the master's row went with the parent"
    );
}

/// Waits until a worker task's row on the master shows `state`.
async fn until_master_row(
    setup: &DelegationHarness,
    task: &mannie_plugin_sdk::contracts::TaskId,
    state: TaskState,
) {
    let reached = async {
        loop {
            let row = setup.master.store.task(task).await.unwrap().unwrap();
            if row.state == state {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(20), reached)
        .await
        .expect("the master's row reaches the state");
}

/// Sends the owner's next message in a conversation and waits for the reply.
async fn send(setup: &DelegationHarness, conversation: &ConversationId, text: &str) -> Transcript {
    let started = setup
        .master
        .runtime
        .send_message(SendMessage::new(conversation.clone(), text))
        .await
        .unwrap();
    setup
        .master
        .wait_for_run(conversation, &started.run_id)
        .await
}

#[tokio::test]
async fn turning_workers_off_lets_a_running_task_go_and_turning_it_on_reads_its_report() {
    let mut setup = DelegationHarness::start().await;
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build slowly.", false),
    )]));
    setup.master.model.push_reply(reply("Workers went off."));
    setup.worker.model.push_reply(vec![
        Step::Delay(Duration::from_millis(500)),
        Step::Event(mannie_plugin_harness::provider::ProviderEvent::TextDelta(
            "Built.".to_owned(),
        )),
        Step::Event(mannie_plugin_harness::provider::ProviderEvent::Finished(
            mannie_plugin_harness::types::FinishReason::Stop,
        )),
    ]);
    setup
        .worker
        .model
        .push_reply(reply("Report: built while Workers was off."));
    let (conversation, run) = start(&setup, "Build").await;
    let tasks = tasks_when(&setup, &conversation, 0, |tasks| {
        tasks
            .first()
            .is_some_and(|task| task.state == TaskState::Running)
    })
    .await;
    let task = tasks[0].task_id.clone();

    setup.turn_off();
    let transcript = setup.master.wait_for_run(&conversation, &run).await;
    let on_worker = setup.worker.wait_for_end(&task).await;
    let row = setup.master.store.task(&task).await.unwrap().unwrap();
    // A wait while Workers is off does not run.
    setup
        .master
        .model
        .push_reply(tool_calls(&[("call_2", "wait_for_tasks", &wait(&[1]))]));
    setup.master.model.push_reply(reply("Workers is off."));
    send(&setup, &conversation, "Is it done?").await;
    let while_off = setup.master.model.reply_requests().last().cloned().unwrap();
    // Turning Workers on catches up, and a wait reads the report.
    setup.turn_on();
    setup
        .master
        .model
        .push_reply(tool_calls(&[("call_3", "wait_for_tasks", &wait(&[1]))]));
    setup.master.model.push_reply(reply("It finished."));
    send(&setup, &conversation, "And now?").await;
    let after = setup.master.model.reply_requests().last().cloned().unwrap();

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    assert!(transcript.tasks(0)[0].detached);
    let requests = setup.master.model.reply_requests();
    assert_eq!(
        result(&requests[1], "call_1"),
        texts::task_let_go(1, worker_texts::THE_WORKER, LOCAL_WORKER_NAME)
    );
    assert_eq!(on_worker.state, TaskState::Reported, "nothing was canceled");
    assert!(row.state.is_active(), "nobody followed the task while off");
    assert_eq!(
        result(&while_off, "call_2"),
        texts::task_plugin_off(1, WORKERS_KIND)
    );
    assert_eq!(
        result(&after, "call_3"),
        texts::task_reported(
            1,
            worker_texts::THE_WORKER,
            "Report: built while Workers was off."
        )
    );
    until_master_row(&setup, &task, TaskState::Reported).await;
}

#[tokio::test]
async fn deleting_a_conversation_while_workers_is_off_reaches_the_worker_once_it_is_on() {
    let mut setup = DelegationHarness::start().await;
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build the backend.", false),
    )]));
    setup.master.model.push_reply(reply("Built."));
    worker_reports(&setup, "Report: built.");
    let (conversation, run) = start(&setup, "Build").await;
    let transcript = setup.master.wait_for_run(&conversation, &run).await;
    let task = transcript.tasks(0)[0].clone();

    setup.turn_off();
    setup
        .master
        .runtime
        .delete_conversation(&conversation)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let waiting = waiting_deletions(&setup).await;
    let kept = setup
        .worker
        .store
        .conversation_summary(&task.conversation_id)
        .await
        .unwrap();
    setup.turn_on();

    assert_eq!(waiting, std::slice::from_ref(&task.conversation_id));
    assert!(kept.is_some(), "the worker keeps it while Workers is off");
    until_deleted_on_the_worker(&setup, &task.conversation_id).await;
}

#[tokio::test]
async fn a_submission_that_turning_workers_off_cut_short_is_let_go_and_checked_on_return() {
    let mut setup = DelegationHarness::start().await;
    setup.delegation.go_offline_at_next_submit();
    setup.master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegate("Build it.", false),
    )]));
    setup.master.model.push_reply(reply("It may run there."));
    let (conversation, run) = start(&setup, "Build").await;
    let parts = tasks_when(&setup, &conversation, 0, |tasks| !tasks.is_empty()).await;
    let task = parts[0].task_id.clone();

    setup.turn_off();
    let transcript = setup.master.wait_for_run(&conversation, &run).await;
    let row = setup.master.store.task(&task).await.unwrap().unwrap();
    setup.delegation.set_online(true);
    setup.turn_on();

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let requests = setup.master.model.reply_requests();
    assert_eq!(
        result(&requests[1], "call_1"),
        texts::task_let_go(1, worker_texts::THE_WORKER, LOCAL_WORKER_NAME)
    );
    assert!(row.state.is_active(), "the worker may have stored the task");
    // The worker never stored it, so the master ends it once it can ask.
    until_master_row(&setup, &task, TaskState::Interrupted).await;
    assert_eq!(setup.worker.model.reply_requests().len(), 0);
}

#[tokio::test]
async fn delegate_task_is_offered_only_while_workers_are_paired_as_its_reviewed_copy() {
    let without = Harness::with_options(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await;
    without.model.push_reply(reply("Hello."));
    let started = without
        .runtime
        .start_conversation(StartConversation::new("Hi"))
        .await
        .unwrap();
    without
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let setup = DelegationHarness::start().await;
    setup.master.model.push_reply(reply("Hello."));
    let (conversation, run) = start(&setup, "Hi").await;
    setup.master.wait_for_run(&conversation, &run).await;

    let names = |request: &ModelRequest| -> Vec<String> {
        request.tools.iter().map(|tool| tool.name.clone()).collect()
    };
    let without_workers = names(&without.model.reply_requests()[0]);
    let request = &setup.master.model.reply_requests()[0];
    assert!(!without_workers.contains(&"delegate_task".to_owned()));
    assert!(
        !without.model.reply_requests()[0]
            .instructions
            .contains(worker_texts::WORKERS_HEADING)
    );
    assert!(request.instructions.contains(worker_texts::WORKERS_HEADING));
    assert!(request.instructions.contains("- build-01 (online)"));
    let definition = request
        .tools
        .iter()
        .find(|tool| tool.name == "delegate_task")
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })
        })
        .expect("delegate_task is offered");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("delegate_task_definition.json");
    if std::env::var_os("MANNIE_UPDATE_EXPECTED").is_some() {
        let text = serde_json::to_string_pretty(&definition).unwrap();
        fs::write(&fixture, format!("{text}\n")).unwrap();
        return;
    }
    let expected: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&fixture).unwrap()).unwrap();
    assert_eq!(
        definition,
        expected,
        "the definition of delegate_task changed; review it and update {}",
        fixture.display()
    );
}
