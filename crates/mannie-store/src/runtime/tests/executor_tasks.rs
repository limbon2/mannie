//! Tasks that a plugin's executor runs, and tasks that another machine
//! assigns, in the store (ADR-0031; ADR-0028): an executor's task row, which
//! the executor's states change and which outlives its part, the outbox of
//! the deletions that wait for an executor, and an assigned task's row, whose
//! states take growing versions.

use mannie_types::ids::{ConversationId, PartId, RunId, TaskId};
use mannie_types::run::{RunOutcome, RunState};
use mannie_types::tasks::{
    TaskCallView, TaskDocument, TaskExecutorRef, TaskPartDocument, TaskState,
};
use mannie_types::tools::{ToolCallDocument, ToolCallStatus};
use mannie_types::transcript::{PartContent, PartKind};

use super::Fixture;
use crate::runtime::{
    Change, NewConversation, NewExecutorTask, NewPart, NewRun, NewTask, OutboxEntry,
    RuntimeStoreError, TaskEnding, TaskOrigin,
};

/// The kind of the fake executor of these tests.
const KIND: &str = "fake";

/// A parent whose run gave a task to the fake executor, and the task.
struct Given {
    parent: ConversationId,
    run: RunId,
    part: PartId,
    conversation: ConversationId,
    task: TaskId,
}

fn task_id(fixture: &Fixture) -> TaskId {
    TaskId::parse(&fixture.uuid()).unwrap()
}

fn target(last: u32) -> TaskExecutorRef {
    TaskExecutorRef {
        kind: KIND.to_owned(),
        target_id: format!("target-{last}"),
        target_name: format!("box-{last:02}"),
        noun: "the box".to_owned(),
    }
}

fn given_document(
    task: &TaskId,
    conversation: &ConversationId,
    executor: &TaskExecutorRef,
) -> TaskDocument {
    TaskDocument {
        task_id: task.clone(),
        number: 1,
        conversation_id: conversation.clone(),
        agent: "remote".to_owned(),
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
        worker: None,
        executor: Some(executor.clone()),
        detached: false,
    }
}

fn new_task(
    given: &Given,
    executor: &TaskExecutorRef,
    conversation: Option<NewConversation>,
) -> NewExecutorTask {
    NewExecutorTask {
        id: given.task.clone(),
        conversation_id: given.conversation.clone(),
        conversation,
        part_id: given.part.clone(),
        kind: executor.kind.clone(),
        target: executor.target_id.clone(),
        document: Some(r#"{"version":1,"queued":false}"#.to_owned()),
        follow_up: false,
        background: false,
    }
}

fn task_conversation(given: &Given, executor: &TaskExecutorRef) -> NewConversation {
    NewConversation {
        id: given.conversation.clone(),
        title: "Build the backend".to_owned(),
        profile_id: Some(Fixture::profile_id()),
        project_id: None,
        task: Some(TaskOrigin {
            parent_id: given.parent.clone(),
            number: 1,
            agent: "remote".to_owned(),
            executor: Some(executor.clone()),
        }),
    }
}

/// Starts a parent whose run opened a part of the fake executor's tool and
/// stored the task's ID in it, as the runtime does before it stores the
/// task's row.
async fn giving_parent(fixture: &Fixture, executor: &TaskExecutorRef) -> Given {
    let (parent, turn, run, _) = fixture.start("Build it on the box").await;
    let part = fixture.part_id();
    let conversation = fixture.conversation_id();
    let task = task_id(fixture);
    let mut call = ToolCallDocument::receiving("call_1".to_owned(), "give_task".to_owned());
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
                    task: Some(given_document(&task, &conversation, executor)),
                    ..TaskPartDocument::default()
                }),
                archives: Vec::new(),
            }],
        )
        .await;
    Given {
        parent,
        run,
        part,
        conversation,
        task,
    }
}

/// A parent that gave a task whose row is stored.
async fn given(fixture: &Fixture, executor: &TaskExecutorRef) -> Given {
    let given = giving_parent(fixture, executor).await;
    fixture
        .store
        .record_executor_task(new_task(
            &given,
            executor,
            Some(task_conversation(&given, executor)),
        ))
        .await
        .unwrap();
    given
}

/// The parent's task part as the UI shows it.
async fn task_call(fixture: &Fixture, given: &Given) -> TaskCallView {
    let snapshot = fixture
        .store
        .snapshot(&given.parent, 10, 100)
        .await
        .unwrap()
        .unwrap();
    snapshot
        .turns
        .iter()
        .flat_map(|turn| &turn.parts)
        .find(|part| part.id == given.part)
        .map(|part| match &part.content {
            PartContent::Task { call } => (**call).clone(),
            other => panic!("not a task part: {other:?}"),
        })
        .expect("the task part is shown")
}

async fn end_parent_run(fixture: &Fixture, given: &Given, state: RunState) {
    fixture
        .apply(
            &given.parent,
            vec![Change::EndRun {
                run_id: given.run.clone(),
                state,
                outcome: RunOutcome::default(),
                task: None,
            }],
        )
        .await;
}

fn ended(state: TaskState, report: Option<&str>) -> TaskEnding {
    TaskEnding {
        state,
        report: report.map(str::to_owned),
        failure: None,
    }
}

/// Starts an assigned task in a new conversation, as a machine stores what
/// another machine assigned.
async fn assigned(fixture: &Fixture) -> (ConversationId, RunId, TaskId) {
    let conversation = fixture.conversation_id();
    let task = task_id(fixture);
    let run = NewRun {
        agent: "assigned".to_owned(),
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
                    mentions: Vec::new(),
                    images: Vec::new(),
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
async fn an_executors_task_is_stored_once_with_its_conversation() {
    let fixture = Fixture::new();
    let executor = target(1);
    let given = giving_parent(&fixture, &executor).await;
    let new_task = new_task(
        &given,
        &executor,
        Some(task_conversation(&given, &executor)),
    );

    let stored = fixture
        .store
        .record_executor_task(new_task.clone())
        .await
        .unwrap();
    let again = fixture.store.record_executor_task(new_task).await.unwrap();

    assert_eq!(again, stored, "storing a task again changes nothing");
    assert_eq!(stored.id, given.task);
    assert_eq!(stored.part_id.as_ref(), Some(&given.part));
    assert_eq!(
        stored.turn_id, None,
        "the task's turns live with its target"
    );
    assert_eq!(stored.state, TaskState::Accepted);
    let execution = stored.executor.clone().unwrap();
    assert_eq!(execution.kind, KIND);
    assert_eq!(execution.target, "target-1");
    assert_eq!(execution.version, 0);
    assert_eq!(
        execution.document.as_deref(),
        Some(r#"{"version":1,"queued":false}"#)
    );
    let summary = fixture
        .store
        .conversation_summary(&given.conversation)
        .await
        .unwrap()
        .unwrap();
    let origin = summary.task.unwrap();
    assert_eq!(origin.parent_id, given.parent);
    assert_eq!(origin.agent, "remote");
    assert_eq!(origin.executor.as_ref(), Some(&executor));
    assert_eq!(origin.worker, None);
    let child = fixture
        .store
        .child_by_number(&given.parent, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child.executor.as_ref(), Some(&executor));
    assert_eq!(child.latest.map(|task| task.id), Some(given.task.clone()));
    let listed = fixture.store.list_conversations().await.unwrap();
    assert_eq!(
        listed.iter().map(|summary| &summary.id).collect::<Vec<_>>(),
        [&given.parent],
        "an executor's task conversation is not listed"
    );

    assert!(
        fixture
            .store
            .set_executor_document(
                &given.task,
                KIND,
                Some(r#"{"version":1,"queued":true}"#.to_owned())
            )
            .await
            .unwrap()
    );
    assert!(
        !fixture
            .store
            .set_executor_document(&given.task, "other", None)
            .await
            .unwrap(),
        "another executor's document does not change it"
    );
    let changed = fixture.store.task(&given.task).await.unwrap().unwrap();
    assert_eq!(
        changed
            .executor
            .and_then(|execution| execution.document)
            .as_deref(),
        Some(r#"{"version":1,"queued":true}"#)
    );
}

#[tokio::test]
async fn an_executors_task_conversation_must_name_its_executor_and_target() {
    let fixture = Fixture::new();
    let executor = target(1);
    let given = giving_parent(&fixture, &executor).await;
    let mut without_executor = task_conversation(&given, &executor);
    if let Some(origin) = without_executor.task.as_mut() {
        origin.executor = None;
    }
    let mut other_target = task_conversation(&given, &executor);
    if let Some(origin) = other_target.task.as_mut() {
        origin.executor = Some(target(2));
    }

    for conversation in [without_executor, other_target] {
        let refused = fixture
            .store
            .record_executor_task(new_task(&given, &executor, Some(conversation)))
            .await;

        assert!(
            matches!(refused, Err(RuntimeStoreError::InvalidData { .. })),
            "{refused:?}"
        );
    }
    assert_eq!(fixture.store.task(&given.task).await.unwrap(), None);
}

#[tokio::test]
async fn the_executors_states_change_its_task_in_version_order() {
    let fixture = Fixture::new();
    let executor = target(1);
    let given = given(&fixture, &executor).await;
    let store = &fixture.store;

    let running = store
        .apply_executor_task_state(&given.task, KIND, 3, TaskState::Running, None)
        .await
        .unwrap();
    let older = store
        .apply_executor_task_state(&given.task, KIND, 2, TaskState::Accepted, None)
        .await
        .unwrap();
    let other_kind = store
        .apply_executor_task_state(&given.task, "other", 4, TaskState::Running, None)
        .await
        .unwrap();
    let finished = store
        .apply_executor_task_state(
            &given.task,
            KIND,
            7,
            TaskState::Reported,
            Some(ended(TaskState::Reported, Some("The tests pass."))),
        )
        .await
        .unwrap();
    let after_the_end = store
        .apply_executor_task_state(&given.task, KIND, 9, TaskState::Running, None)
        .await
        .unwrap();
    let mismatched = store
        .apply_executor_task_state(
            &given.task,
            KIND,
            10,
            TaskState::Failed,
            Some(ended(TaskState::Reported, None)),
        )
        .await;

    assert!(running);
    assert!(!older, "an older state never replaces a newer one");
    assert!(!other_kind, "another executor's state changes nothing");
    assert!(finished);
    assert!(!after_the_end, "an ended task keeps its first ending");
    assert!(matches!(
        mismatched,
        Err(RuntimeStoreError::InvalidData { .. })
    ));
    let task = store.task(&given.task).await.unwrap().unwrap();
    assert_eq!(task.state, TaskState::Reported);
    assert_eq!(task.executor.map(|execution| execution.version), Some(7));
    assert_eq!(
        task.ending.and_then(|ending| ending.report).as_deref(),
        Some("The tests pass.")
    );
    assert!(task.ended_at.is_some());
    assert_eq!(store.active_executor_tasks(KIND).await.unwrap(), Vec::new());
}

#[tokio::test]
async fn an_executors_task_that_still_runs_is_let_go_with_its_part() {
    for parent_state in [RunState::Stopped, RunState::Interrupted] {
        let fixture = Fixture::new();
        let executor = target(1);
        let given = given(&fixture, &executor).await;
        let store = &fixture.store;
        store
            .apply_executor_task_state(&given.task, KIND, 4, TaskState::Running, None)
            .await
            .unwrap();

        end_parent_run(&fixture, &given, parent_state).await;

        let call = task_call(&fixture, &given).await;
        assert_eq!(call.status, ToolCallStatus::Completed, "{parent_state:?}");
        let task = call.task.unwrap();
        assert!(task.detached, "the part lets the task go");
        assert_eq!(task.executor.as_ref(), Some(&executor));
        let stored = store.task(&given.task).await.unwrap().unwrap();
        assert_eq!(stored.state, TaskState::Running, "the task runs on");
        let active = store.active_executor_tasks(KIND).await.unwrap();
        assert_eq!(active, std::slice::from_ref(&stored));
        assert_eq!(
            store.active_executor_tasks("other").await.unwrap(),
            Vec::new()
        );

        // The executor's ending reaches the row later; the part never changes.
        store
            .apply_executor_task_state(
                &given.task,
                KIND,
                5,
                TaskState::Canceled,
                Some(ended(TaskState::Canceled, None)),
            )
            .await
            .unwrap();
        let finished = store.task(&given.task).await.unwrap().unwrap();
        assert_eq!(finished.state, TaskState::Canceled);
        assert!(task_call(&fixture, &given).await.task.unwrap().detached);
    }
}

#[tokio::test]
async fn a_follow_up_waits_until_the_previous_task_ended() {
    let fixture = Fixture::new();
    let executor = target(1);
    let first = given(&fixture, &executor).await;
    let follow_up = NewExecutorTask {
        id: task_id(&fixture),
        conversation_id: first.conversation.clone(),
        conversation: None,
        part_id: first.part.clone(),
        kind: KIND.to_owned(),
        target: executor.target_id.clone(),
        document: None,
        follow_up: true,
        background: false,
    };

    let refused = fixture.store.record_executor_task(follow_up.clone()).await;
    fixture
        .store
        .apply_executor_task_state(
            &first.task,
            KIND,
            2,
            TaskState::Reported,
            Some(ended(TaskState::Reported, Some("Built."))),
        )
        .await
        .unwrap();
    let stored = fixture.store.record_executor_task(follow_up).await.unwrap();

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
                task: Some(ended(TaskState::Reported, Some("The tests pass."))),
            }],
        )
        .await;
    let (_, _, second) = assigned(&fixture).await;

    assert_eq!(accepted.state_version, 1);
    assert_eq!(accepted.part_id, None, "the assigning side keeps the part");
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
                    mentions: Vec::new(),
                    images: Vec::new(),
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
async fn deleting_a_parent_queues_the_deletions_of_its_executors_task_conversations() {
    let fixture = Fixture::new();
    let first_box = target(1);
    let second_box = target(2);
    let first = given(&fixture, &first_box).await;
    let second = given(&fixture, &first_box).await;
    let elsewhere = given(&fixture, &second_box).await;

    for given in [&first, &second, &elsewhere] {
        assert!(
            fixture
                .store
                .delete_conversation(&given.parent)
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
        "this machine's copy goes with the parent"
    );
    assert_eq!(fixture.store.task(&first.task).await.unwrap(), None);
    let outbox = fixture.store.executor_outbox(KIND).await.unwrap();
    let entry = |target: &TaskExecutorRef, given: &Given| OutboxEntry {
        target: target.target_id.clone(),
        conversation_id: given.conversation.clone(),
    };
    let mut expected = vec![
        entry(&first_box, &first),
        entry(&first_box, &second),
        entry(&second_box, &elsewhere),
    ];
    let mut queued = outbox.clone();
    expected.sort_by(|a, b| a.conversation_id.as_str().cmp(b.conversation_id.as_str()));
    queued.sort_by(|a, b| a.conversation_id.as_str().cmp(b.conversation_id.as_str()));
    assert_eq!(queued, expected);
    assert_eq!(
        fixture.store.executor_outbox("other").await.unwrap(),
        Vec::new()
    );

    fixture
        .store
        .drop_executor_outbox(KIND, &[entry(&first_box, &first)])
        .await
        .unwrap();
    let left = fixture.store.executor_outbox(KIND).await.unwrap();
    assert_eq!(left.len(), 2);
    assert!(!left.contains(&entry(&first_box, &first)));
}
