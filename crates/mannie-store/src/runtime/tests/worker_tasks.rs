//! Tasks of workers in the store (docs/features/workers/delegation.md, data):
//! a delegated task's row on the master, which the worker's states change and
//! which outlives its part, and an assigned task's row on a worker, whose
//! states take growing versions.

use mannie_types::ids::{ConversationId, PartId, RunId, TaskId, WorkerId};
use mannie_types::run::{RunOutcome, RunState};
use mannie_types::tasks::{TaskCallView, TaskDocument, TaskPartDocument, TaskState, TaskWorker};
use mannie_types::tools::{ToolCallDocument, ToolCallStatus};
use mannie_types::transcript::{PartContent, PartKind};
use mannie_types::workers::WorkerDetails;

use super::Fixture;
use crate::NewPairedWorker;
use crate::runtime::{
    Change, NewConversation, NewDelegatedTask, NewPart, NewRun, NewTask, RuntimeStoreError,
    Submission, TaskEnding, TaskOrigin,
};

/// A parent whose run delegated a task, and the task.
struct Delegation {
    parent: ConversationId,
    run: RunId,
    part: PartId,
    conversation: ConversationId,
    task: TaskId,
}

fn task_id(fixture: &Fixture) -> TaskId {
    TaskId::parse(&fixture.uuid()).unwrap()
}

fn worker(last: u32) -> TaskWorker {
    TaskWorker {
        id: WorkerId::parse(&format!("0199e0f2-0000-7000-b000-{last:012}")).unwrap(),
        name: format!("build-{last:02}"),
    }
}

async fn pair(fixture: &Fixture, worker: &TaskWorker) {
    fixture
        .store
        .add_paired_worker(NewPairedWorker {
            id: worker.id.clone(),
            name: worker.name.clone(),
            chosen_name: None,
            address: format!("{}.lan:7445", worker.name),
            key_pin: format!("{:0>64}", worker.name),
            details: WorkerDetails::default(),
        })
        .await
        .unwrap();
}

fn delegated_document(
    task: &TaskId,
    conversation: &ConversationId,
    worker: &TaskWorker,
) -> TaskDocument {
    TaskDocument {
        task_id: task.clone(),
        number: 1,
        conversation_id: conversation.clone(),
        agent: "worker".to_owned(),
        title: "Build the backend".to_owned(),
        background: false,
        follow_up: false,
        state: TaskState::Accepted,
        instruction: "Build the backend and run its tests.".to_owned(),
        activity: None,
        tool_calls: 0,
        approval: None,
        report: None,
        failure: None,
        started_at: None,
        ended_at: None,
        worker: Some(worker.clone()),
        detached: false,
    }
}

fn new_delegated_task(
    delegation: &Delegation,
    worker: &TaskWorker,
    conversation: Option<NewConversation>,
) -> NewDelegatedTask {
    NewDelegatedTask {
        id: delegation.task.clone(),
        conversation_id: delegation.conversation.clone(),
        conversation,
        part_id: delegation.part.clone(),
        worker_id: worker.id.clone(),
        profile_id: Fixture::profile_id(),
        follow_up: false,
        background: false,
    }
}

fn worker_conversation(delegation: &Delegation, worker: &TaskWorker) -> NewConversation {
    NewConversation {
        id: delegation.conversation.clone(),
        title: "Build the backend".to_owned(),
        profile_id: Some(Fixture::profile_id()),
        project_id: None,
        task: Some(TaskOrigin {
            parent_id: delegation.parent.clone(),
            number: 1,
            agent: "worker".to_owned(),
            worker: Some(worker.clone()),
        }),
    }
}

/// Starts a parent whose run opened a `delegate_task` part and stored the
/// task's ID in it, as the runtime does before it stores the task's row.
async fn delegating_parent(fixture: &Fixture, worker: &TaskWorker) -> Delegation {
    let (parent, turn, run, _) = fixture.start("Build it on the server").await;
    let part = fixture.part_id();
    let conversation = fixture.conversation_id();
    let task = task_id(fixture);
    let mut call = ToolCallDocument::receiving("call_1".to_owned(), "delegate_task".to_owned());
    fixture
        .apply(
            &parent,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart {
                    part: NewPart {
                        id: part.clone(),
                        turn_id: turn,
                        run_id: run.clone(),
                        attempt: 1,
                        kind: PartKind::Task,
                        failure: None,
                        tool_call: Some(call.clone()),
                    },
                },
            ],
        )
        .await;
    call.status = ToolCallStatus::Running;
    fixture
        .apply(
            &parent,
            vec![Change::UpdateTaskPart {
                part_id: part.clone(),
                tool_call: call,
                task: Box::new(TaskPartDocument {
                    task: Some(delegated_document(&task, &conversation, worker)),
                    ..TaskPartDocument::default()
                }),
                archives: Vec::new(),
            }],
        )
        .await;
    Delegation {
        parent,
        run,
        part,
        conversation,
        task,
    }
}

/// A parent that delegated a task whose row is stored.
async fn delegated(fixture: &Fixture, worker: &TaskWorker) -> Delegation {
    let delegation = delegating_parent(fixture, worker).await;
    fixture
        .store
        .record_delegated_task(new_delegated_task(
            &delegation,
            worker,
            Some(worker_conversation(&delegation, worker)),
        ))
        .await
        .unwrap();
    delegation
}

/// The parent's task part as the UI shows it.
async fn task_call(fixture: &Fixture, delegation: &Delegation) -> TaskCallView {
    let snapshot = fixture
        .store
        .snapshot(&delegation.parent, 10, 100)
        .await
        .unwrap()
        .unwrap();
    snapshot
        .turns
        .iter()
        .flat_map(|turn| &turn.parts)
        .find(|part| part.id == delegation.part)
        .map(|part| match &part.content {
            PartContent::Task { call } => (**call).clone(),
            other => panic!("not a task part: {other:?}"),
        })
        .expect("the task part is shown")
}

async fn end_parent_run(fixture: &Fixture, delegation: &Delegation, state: RunState) {
    fixture
        .apply(
            &delegation.parent,
            vec![Change::EndRun {
                run_id: delegation.run.clone(),
                state,
                outcome: RunOutcome::default(),
                task: None,
            }],
        )
        .await;
}

/// Starts an assigned task in a new conversation of a worker, as the worker
/// stores what the master assigned.
async fn assigned(fixture: &Fixture) -> (ConversationId, RunId, TaskId) {
    let conversation = fixture.conversation_id();
    let task = task_id(fixture);
    let run = NewRun {
        agent: "worker".to_owned(),
        ..fixture.new_run()
    };
    let run_id = run.id.clone();
    fixture
        .apply(
            &conversation,
            vec![
                Change::CreateConversation {
                    conversation: NewConversation {
                        id: conversation.clone(),
                        title: "Build the backend".to_owned(),
                        profile_id: Some(Fixture::profile_id()),
                        project_id: None,
                        task: None,
                    },
                },
                Change::StartTurn {
                    turn_id: fixture.turn_id(),
                    user_part_id: fixture.part_id(),
                    text: "Build the backend and run its tests.".to_owned(),
                    run,
                    task: Some(NewTask {
                        id: task.clone(),
                        part_id: None,
                        assignment: Some(r#"{"number":1}"#.to_owned()),
                        follow_up: false,
                        background: false,
                    }),
                },
            ],
        )
        .await;
    (conversation, run_id, task)
}

#[tokio::test]
async fn a_delegated_task_is_stored_once_with_its_conversation() {
    let fixture = Fixture::new();
    let build = worker(1);
    let delegation = delegating_parent(&fixture, &build).await;
    let new_task = new_delegated_task(
        &delegation,
        &build,
        Some(worker_conversation(&delegation, &build)),
    );

    let stored = fixture
        .store
        .record_delegated_task(new_task.clone())
        .await
        .unwrap();
    let again = fixture.store.record_delegated_task(new_task).await.unwrap();

    assert_eq!(again, stored, "storing a task again changes nothing");
    assert_eq!(stored.id, delegation.task);
    assert_eq!(stored.part_id.as_ref(), Some(&delegation.part));
    assert_eq!(stored.turn_id, None, "the task's turns live on the worker");
    assert_eq!(stored.worker_id.as_ref(), Some(&build.id));
    assert_eq!(stored.profile_id, Some(Fixture::profile_id()));
    assert_eq!(stored.submission, Some(Submission::Pending));
    assert_eq!(stored.state, TaskState::Accepted);
    assert_eq!(stored.remote_version, None);
    let summary = fixture
        .store
        .conversation_summary(&delegation.conversation)
        .await
        .unwrap()
        .unwrap();
    let origin = summary.task.unwrap();
    assert_eq!(origin.parent_id, delegation.parent);
    assert_eq!(origin.agent, "worker");
    assert_eq!(origin.worker.as_ref(), Some(&build));
    let child = fixture
        .store
        .child_by_number(&delegation.parent, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child.worker.as_ref(), Some(&build));
    assert_eq!(
        child.latest.map(|task| task.id),
        Some(delegation.task.clone())
    );
    let listed = fixture.store.list_conversations().await.unwrap();
    assert_eq!(
        listed.iter().map(|summary| &summary.id).collect::<Vec<_>>(),
        [&delegation.parent],
        "a worker task conversation is not listed"
    );

    assert!(
        fixture
            .store
            .set_task_submission(&delegation.task, Submission::Accepted)
            .await
            .unwrap()
    );
    let accepted = fixture.store.task(&delegation.task).await.unwrap().unwrap();
    assert_eq!(accepted.submission, Some(Submission::Accepted));
}

#[tokio::test]
async fn a_delegated_task_conversation_must_name_the_tasks_worker() {
    let fixture = Fixture::new();
    let build = worker(1);
    let delegation = delegating_parent(&fixture, &build).await;
    let mut without_worker = worker_conversation(&delegation, &build);
    if let Some(origin) = without_worker.task.as_mut() {
        origin.worker = None;
    }
    let mut other_worker = worker_conversation(&delegation, &build);
    if let Some(origin) = other_worker.task.as_mut() {
        origin.worker = Some(worker(2));
    }

    for conversation in [without_worker, other_worker] {
        let refused = fixture
            .store
            .record_delegated_task(new_delegated_task(&delegation, &build, Some(conversation)))
            .await;

        assert!(
            matches!(refused, Err(RuntimeStoreError::InvalidData { .. })),
            "{refused:?}"
        );
    }
    assert_eq!(fixture.store.task(&delegation.task).await.unwrap(), None);
}

#[tokio::test]
async fn the_workers_states_change_a_delegated_task_in_version_order() {
    let fixture = Fixture::new();
    let build = worker(1);
    let delegation = delegated(&fixture, &build).await;

    let running = fixture
        .store
        .apply_remote_task_state(&delegation.task, 3, TaskState::Running, None)
        .await
        .unwrap();
    let older = fixture
        .store
        .apply_remote_task_state(&delegation.task, 2, TaskState::Accepted, None)
        .await
        .unwrap();
    let ended = fixture
        .store
        .apply_remote_task_state(
            &delegation.task,
            7,
            TaskState::Reported,
            Some(TaskEnding {
                state: TaskState::Reported,
                report: Some("The tests pass.".to_owned()),
                failure: None,
            }),
        )
        .await
        .unwrap();
    let after_the_end = fixture
        .store
        .apply_remote_task_state(&delegation.task, 9, TaskState::Running, None)
        .await
        .unwrap();
    let mismatched = fixture
        .store
        .apply_remote_task_state(
            &delegation.task,
            10,
            TaskState::Failed,
            Some(TaskEnding {
                state: TaskState::Reported,
                report: None,
                failure: None,
            }),
        )
        .await;

    assert!(running);
    assert!(!older, "an older state never replaces a newer one");
    assert!(ended);
    assert!(!after_the_end, "an ended task keeps its first ending");
    assert!(matches!(
        mismatched,
        Err(RuntimeStoreError::InvalidData { .. })
    ));
    let task = fixture.store.task(&delegation.task).await.unwrap().unwrap();
    assert_eq!(task.state, TaskState::Reported);
    assert_eq!(task.remote_version, Some(7));
    assert_eq!(
        task.ending.and_then(|ending| ending.report).as_deref(),
        Some("The tests pass.")
    );
    assert!(task.ended_at.is_some());
    assert_eq!(
        fixture.store.active_delegated_tasks().await.unwrap(),
        Vec::new()
    );
}

#[tokio::test]
async fn a_delegated_task_that_still_runs_is_let_go_with_its_part() {
    for parent_state in [RunState::Stopped, RunState::Interrupted] {
        let fixture = Fixture::new();
        let build = worker(1);
        let delegation = delegated(&fixture, &build).await;
        fixture
            .store
            .apply_remote_task_state(&delegation.task, 4, TaskState::Running, None)
            .await
            .unwrap();
        let cancel_requested = fixture
            .store
            .request_task_cancel(&delegation.task)
            .await
            .unwrap();

        end_parent_run(&fixture, &delegation, parent_state).await;

        assert!(cancel_requested);
        let call = task_call(&fixture, &delegation).await;
        assert_eq!(call.status, ToolCallStatus::Completed, "{parent_state:?}");
        let task = call.task.unwrap();
        assert!(task.detached, "the part lets the task go");
        assert_eq!(task.worker.as_ref(), Some(&build));
        let stored = fixture.store.task(&delegation.task).await.unwrap().unwrap();
        assert_eq!(stored.state, TaskState::Running, "the task runs on");
        assert!(
            stored.cancel_requested_at.is_some(),
            "the cancel stays queued"
        );
        let active = fixture.store.active_delegated_tasks().await.unwrap();
        assert_eq!(active, std::slice::from_ref(&stored));

        // The worker's ending reaches the row later; the part never changes.
        fixture
            .store
            .apply_remote_task_state(
                &delegation.task,
                5,
                TaskState::Canceled,
                Some(TaskEnding {
                    state: TaskState::Canceled,
                    report: None,
                    failure: None,
                }),
            )
            .await
            .unwrap();
        let ended = fixture.store.task(&delegation.task).await.unwrap().unwrap();
        assert_eq!(ended.state, TaskState::Canceled);
        assert!(
            task_call(&fixture, &delegation)
                .await
                .task
                .unwrap()
                .detached
        );
        assert!(
            !fixture
                .store
                .request_task_cancel(&delegation.task)
                .await
                .unwrap(),
            "an ended task takes no cancel"
        );
    }
}

#[tokio::test]
async fn a_follow_up_waits_until_the_previous_task_ended() {
    let fixture = Fixture::new();
    let build = worker(1);
    let first = delegated(&fixture, &build).await;
    let follow_up = NewDelegatedTask {
        id: task_id(&fixture),
        conversation_id: first.conversation.clone(),
        conversation: None,
        part_id: first.part.clone(),
        worker_id: build.id.clone(),
        profile_id: Fixture::profile_id(),
        follow_up: true,
        background: false,
    };

    let refused = fixture.store.record_delegated_task(follow_up.clone()).await;
    fixture
        .store
        .apply_remote_task_state(
            &first.task,
            2,
            TaskState::Reported,
            Some(TaskEnding {
                state: TaskState::Reported,
                report: Some("Built.".to_owned()),
                failure: None,
            }),
        )
        .await
        .unwrap();
    let stored = fixture
        .store
        .record_delegated_task(follow_up)
        .await
        .unwrap();

    assert!(matches!(refused, Err(RuntimeStoreError::TaskActive)));
    assert!(stored.follow_up);
    assert_eq!(stored.conversation_id, first.conversation);
    let child = fixture
        .store
        .child_by_number(&first.parent, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child.latest.map(|task| task.id), Some(stored.id));
}

#[tokio::test]
async fn assigned_tasks_take_growing_state_versions() {
    let fixture = Fixture::new();
    let (conversation, run, first) = assigned(&fixture).await;
    let accepted = fixture.store.task(&first).await.unwrap().unwrap();
    fixture
        .apply(
            &conversation,
            vec![Change::StartRun {
                run_id: run.clone(),
            }],
        )
        .await;
    fixture
        .apply(
            &conversation,
            vec![Change::EndRun {
                run_id: run,
                state: RunState::Completed,
                outcome: RunOutcome::default(),
                task: Some(TaskEnding {
                    state: TaskState::Reported,
                    report: Some("The tests pass.".to_owned()),
                    failure: None,
                }),
            }],
        )
        .await;
    let (_, _, second) = assigned(&fixture).await;

    assert_eq!(accepted.state_version, 1);
    assert_eq!(accepted.part_id, None, "the master's part is on the master");
    assert_eq!(accepted.assignment.as_deref(), Some(r#"{"number":1}"#));
    let first = fixture.store.task(&first).await.unwrap().unwrap();
    let second = fixture.store.task(&second).await.unwrap().unwrap();
    assert_eq!((first.state, first.state_version), (TaskState::Reported, 3));
    assert_eq!(
        (second.state, second.state_version),
        (TaskState::Accepted, 4)
    );
    assert_eq!(
        fixture
            .store
            .latest_task(&conversation)
            .await
            .unwrap()
            .map(|task| task.id),
        Some(first.id)
    );
}

#[tokio::test]
async fn a_task_needs_the_part_or_the_assignment_that_started_it() {
    let fixture = Fixture::new();
    let conversation = fixture.conversation_id();

    let refused = fixture
        .store
        .apply(
            &conversation,
            vec![
                Change::CreateConversation {
                    conversation: NewConversation {
                        id: conversation.clone(),
                        title: "Nobody asked".to_owned(),
                        profile_id: Some(Fixture::profile_id()),
                        project_id: None,
                        task: None,
                    },
                },
                Change::StartTurn {
                    turn_id: fixture.turn_id(),
                    user_part_id: fixture.part_id(),
                    text: "Nobody asked".to_owned(),
                    run: fixture.new_run(),
                    task: Some(NewTask {
                        id: task_id(&fixture),
                        part_id: None,
                        assignment: None,
                        follow_up: false,
                        background: false,
                    }),
                },
            ],
            crate::Durability::Full,
        )
        .await;

    assert!(
        matches!(refused, Err(RuntimeStoreError::InvalidData { .. })),
        "{refused:?}"
    );
    assert_eq!(
        fixture
            .store
            .conversation_summary(&conversation)
            .await
            .unwrap(),
        None,
        "nothing of the batch was stored"
    );
}

#[tokio::test]
async fn deleting_a_parent_queues_the_deletions_of_its_worker_task_conversations() {
    let fixture = Fixture::new();
    let build = worker(1);
    let unpaired = worker(2);
    pair(&fixture, &build).await;
    let first = delegated(&fixture, &build).await;
    let second = delegated(&fixture, &build).await;
    let elsewhere = delegated(&fixture, &unpaired).await;

    for delegation in [&first, &second, &elsewhere] {
        assert!(
            fixture
                .store
                .delete_conversation(&delegation.parent)
                .await
                .unwrap()
        );
    }

    assert_eq!(
        fixture
            .store
            .conversation_summary(&first.conversation)
            .await
            .unwrap(),
        None,
        "the master's copy goes with the parent"
    );
    assert_eq!(fixture.store.task(&first.task).await.unwrap(), None);
    assert_eq!(
        fixture.store.worker_deletions(&build.id).await.unwrap(),
        [first.conversation.clone(), second.conversation.clone()]
    );
    assert_eq!(
        fixture.store.worker_deletions(&unpaired.id).await.unwrap(),
        Vec::new(),
        "nothing waits for a worker that is not paired"
    );

    fixture
        .store
        .drop_worker_deletions(&build.id, std::slice::from_ref(&first.conversation))
        .await
        .unwrap();
    assert_eq!(
        fixture.store.worker_deletions(&build.id).await.unwrap(),
        std::slice::from_ref(&second.conversation)
    );
    assert!(fixture.store.remove_paired_worker(&build.id).await.unwrap());
    assert_eq!(
        fixture.store.worker_deletions(&build.id).await.unwrap(),
        Vec::new(),
        "removing a worker drops what waits for it"
    );
}
