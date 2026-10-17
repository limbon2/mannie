//! The master ends during a delegation, and no case starts a second task
//! (docs/features/workers/delegation.md, submitting a task; ADR-0027,
//! decision 7).
//!
//! Each test writes the master's store as a crash at one point leaves it:
//! after the parent's part recorded the task; after the task's rows; and
//! after a submission that the worker stored but whose answer was lost. It
//! then starts the master again, whose recovery and follower of the worker
//! decide what happens to the task.

#![cfg(test)]

use std::time::Duration;

use mannie_runtime::testing::reply;
use mannie_store::{
    Change, Durability, NewConversation, NewExecutorTask, NewPart, NewRun, Store, TaskOrigin,
};
use mannie_types::ids::{ConversationId, PartId, ProfileId, RunId, TaskId, TurnId};
use mannie_types::tasks::{
    ProfileSnapshot, TaskAssignment, TaskDocument, TaskExecutorRef, TaskPartDocument, TaskRefusal,
    TaskState, TaskView,
};
use mannie_types::tools::{ToolCallDocument, ToolCallStatus};
use mannie_types::transcript::PartKind;
use mannie_workers::testing::{DelegationHarness, LOCAL_WORKER_ID, LOCAL_WORKER_NAME};
use mannie_workers::texts::THE_WORKER;
use mannie_workers::{WORKER_AGENT, WORKERS_KIND};

fn new_id<T>(parse: fn(&str) -> Result<T, mannie_types::ids::InvalidId>) -> T {
    parse(&uuid::Uuid::now_v7().to_string()).unwrap()
}

/// What a crash left: the parent's conversation and the task it recorded.
struct Crashed {
    parent: ConversationId,
    task: TaskId,
    task_conversation: ConversationId,
}

fn the_worker() -> TaskExecutorRef {
    TaskExecutorRef {
        kind: WORKERS_KIND.to_owned(),
        target_id: LOCAL_WORKER_ID.to_owned(),
        target_name: LOCAL_WORKER_NAME.to_owned(),
        noun: THE_WORKER.to_owned(),
    }
}

/// Writes a parent whose reply runs and whose `delegate_task` part recorded
/// a task, as the run stores it before it stores the task's rows.
async fn crash_after_the_part(store: &Store, profile: &ProfileId) -> (Crashed, PartId) {
    let parent: ConversationId = new_id(ConversationId::parse);
    let turn: TurnId = new_id(TurnId::parse);
    let run: RunId = new_id(RunId::parse);
    let part: PartId = new_id(PartId::parse);
    let task: TaskId = new_id(TaskId::parse);
    let task_conversation: ConversationId = new_id(ConversationId::parse);
    let mut call = ToolCallDocument::receiving("call_1".to_owned(), "delegate_task".to_owned());
    let document = TaskDocument {
        task_id: task.clone(),
        number: 1,
        conversation_id: task_conversation.clone(),
        agent: WORKER_AGENT.to_owned(),
        title: "Build the backend".to_owned(),
        background: false,
        follow_up: false,
        state: TaskState::Accepted,
        instruction: "Build the backend.".to_owned(),
        activity: None,
        tool_calls: 0,
        approval: None,
        report: None,
        failure: None,
        started_at: None,
        ended_at: None,
        worker: None,
        executor: Some(the_worker()),
        detached: false,
    };
    store
        .apply(
            &parent,
            vec![
                Change::CreateConversation {
                    conversation: NewConversation {
                        id: parent.clone(),
                        title: "Build it".to_owned(),
                        profile_id: None,
                        project_id: None,
                        task: None,
                    },
                },
                Change::StartTurn {
                    turn_id: turn.clone(),
                    user_part_id: new_id(PartId::parse),
                    mentions: Vec::new(),
                    text: "Build it on the server".to_owned(),
                    run: NewRun {
                        id: run.clone(),
                        agent: "conversation".to_owned(),
                        profile_id: profile.clone(),
                        binding: format!("{profile}:1"),
                    },
                    task: None,
                },
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart {
                    part: NewPart {
                        id: part.clone(),
                        turn_id: turn,
                        run_id: run,
                        attempt: 1,
                        kind: PartKind::Task,
                        failure: None,
                        tool_call: Some(call.clone()),
                    },
                },
            ],
            Durability::Full,
        )
        .await
        .unwrap();
    call.status = ToolCallStatus::Running;
    store
        .apply(
            &parent,
            vec![Change::UpdateTaskPart {
                part_id: part.clone(),
                tool_call: call,
                task: Box::new(TaskPartDocument {
                    task: Some(document),
                    ..TaskPartDocument::default()
                }),
                archives: Vec::new(),
            }],
            Durability::Full,
        )
        .await
        .unwrap();
    (
        Crashed {
            parent,
            task,
            task_conversation,
        },
        part,
    )
}

/// Writes the task's rows after its part, with the executor's document, as
/// the run stores them before the executor submits the task.
async fn crash_after_the_rows(store: &Store, profile: &ProfileId) -> Crashed {
    let (crashed, part) = crash_after_the_part(store, profile).await;
    let document = serde_json::json!({
        "version": 1,
        "profileId": profile,
        "submission": "pending",
        "cancelRequestedAt": null,
    });
    store
        .record_executor_task(NewExecutorTask {
            id: crashed.task.clone(),
            conversation_id: crashed.task_conversation.clone(),
            conversation: Some(NewConversation {
                id: crashed.task_conversation.clone(),
                title: "Build the backend".to_owned(),
                profile_id: Some(profile.clone()),
                project_id: None,
                task: Some(TaskOrigin {
                    parent_id: crashed.parent.clone(),
                    number: 1,
                    agent: WORKER_AGENT.to_owned(),
                    executor: Some(the_worker()),
                }),
            }),
            part_id: part,
            kind: WORKERS_KIND.to_owned(),
            target: LOCAL_WORKER_ID.to_owned(),
            document: Some(document.to_string()),
            follow_up: false,
            background: false,
        })
        .await
        .unwrap();
    crashed
}

/// The master's task of a crash, once `done` holds for its row.
async fn row_when(
    setup: &DelegationHarness,
    task: &TaskId,
    done: impl Fn(TaskState) -> bool,
) -> TaskState {
    let wait = async {
        loop {
            let row = setup.master.store.task(task).await.unwrap().unwrap();
            if done(row.state) {
                return row.state;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(20), wait)
        .await
        .expect("the task's row reaches the state")
}

/// The parent's task part after the restart.
async fn part_after(
    setup: &DelegationHarness,
    parent: &ConversationId,
) -> (Option<TaskView>, Option<TaskRefusal>) {
    let transcript = setup.master.snapshot(parent).await;
    let call = transcript.task_calls(0).into_iter().next().unwrap();
    (call.task, call.refusal)
}

#[tokio::test]
async fn a_crash_after_the_part_leaves_it_not_started_and_nothing_runs() {
    let setup = DelegationHarness::start().await;
    let profile = setup.master.profile.id.clone();
    setup.master.runtime.shutdown().await;
    let (crashed, _part) = crash_after_the_part(&setup.master.store, &profile).await;

    let setup = setup.restart_master().await;

    let (_task, refusal) = part_after(&setup, &crashed.parent).await;
    assert_eq!(refusal, Some(TaskRefusal::NotStarted));
    assert_eq!(setup.master.store.task(&crashed.task).await.unwrap(), None);
    assert_eq!(
        setup
            .worker
            .runtime
            .assigned_task(&crashed.task)
            .await
            .unwrap(),
        None
    );
    assert_eq!(setup.worker.model.reply_requests().len(), 0);
}

#[tokio::test]
async fn a_crash_after_the_rows_lets_the_task_go_and_a_task_the_worker_never_got_ends() {
    let setup = DelegationHarness::start().await;
    let profile = setup.master.profile.id.clone();
    setup.master.runtime.shutdown().await;
    let crashed = crash_after_the_rows(&setup.master.store, &profile).await;

    let setup = setup.restart_master().await;
    let ended = row_when(&setup, &crashed.task, |state| !state.is_active()).await;

    let (task, _refusal) = part_after(&setup, &crashed.parent).await;
    assert!(
        task.is_some_and(|task| task.detached),
        "the part lets the task go"
    );
    assert_eq!(ended, TaskState::Interrupted, "the worker never got it");
    assert_eq!(
        setup
            .worker
            .runtime
            .assigned_task(&crashed.task)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        setup.worker.model.reply_requests().len(),
        0,
        "nothing submits it again"
    );
}

#[tokio::test]
async fn a_crash_after_a_lost_answer_follows_the_task_the_worker_stored_and_runs_it_once() {
    let setup = DelegationHarness::start().await;
    let profile = setup.master.profile.id.clone();
    setup.worker.model.push_reply(reply("Built."));
    setup.worker.model.push_reply(reply("Report: built once."));
    setup.master.runtime.shutdown().await;
    let crashed = crash_after_the_rows(&setup.master.store, &profile).await;
    // The worker stored the task; the master never heard back.
    setup
        .worker
        .runtime
        .assign_task(TaskAssignment {
            task_id: crashed.task.clone(),
            conversation_id: crashed.task_conversation.clone(),
            follow_up: false,
            title: "Build the backend".to_owned(),
            instruction: "Build the backend.".to_owned(),
            read_only: false,
            profile: ProfileSnapshot {
                id: profile.clone(),
                config_revision: 1,
                interface: mannie_types::provider::ApiInterface::ChatCompletions,
                base_url: "http://127.0.0.1:9/v1".to_owned(),
                model: "test-model".to_owned(),
                context_window: 128_000,
                output_limit: 16_384,
                reasoning_effort: mannie_types::provider::ReasoningEffort::ServerDefault,
                server: None,
                use_tools: true,
                has_credential: true,
            },
        })
        .await
        .unwrap();

    let setup = setup.restart_master().await;
    let ended = row_when(&setup, &crashed.task, |state| !state.is_active()).await;

    assert_eq!(ended, TaskState::Reported);
    let on_worker = setup
        .worker
        .runtime
        .assigned_task(&crashed.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(on_worker.report.as_deref(), Some("Report: built once."));
    assert_eq!(
        setup.worker.model.reply_requests().len(),
        2,
        "the task ran once, with its confirmation"
    );
    assert_eq!(
        setup
            .worker
            .model
            .credentials()
            .first()
            .cloned()
            .flatten()
            .as_deref(),
        Some("sk-test-credential"),
        "the follower delivered the lease that the task waited for"
    );
}
