//! Tasks and task conversations in the store: creating a child with its task,
//! the task's life with its run, the parent's part that finishes from the
//! task, recovery, and deletion with the parent.

use mannie_types::ids::{ConversationId, PartId, RunId, TaskId, TurnId};
use mannie_types::run::{RunFailure, RunOutcome, RunState};
use mannie_types::tasks::{TaskDocument, TaskPartDocument, TaskRefusal, TaskState};
use mannie_types::tools::{ToolCallDocument, ToolCallStatus};
use mannie_types::transcript::{PartContent, PartKind, PartState};

use super::Fixture;
use crate::Durability;
use crate::runtime::{
    Change, NewConversation, NewPart, NewRun, NewTask, RuntimeStoreError, TaskEnding, TaskOrigin,
};

/// A parent conversation with a running run and an open task part.
struct Parent {
    conversation: ConversationId,
    turn: TurnId,
    run: RunId,
    part: PartId,
}

/// A child conversation with its task.
struct Child {
    conversation: ConversationId,
    run: RunId,
    task: TaskId,
}

impl Fixture {
    fn task_id(&self) -> TaskId {
        TaskId::parse(&self.uuid()).unwrap()
    }

    fn child_run(&self, agent: &str) -> NewRun {
        NewRun {
            agent: agent.to_owned(),
            ..self.new_run()
        }
    }

    /// Starts a parent conversation whose run is running and has opened the
    /// part of a `start_task` call.
    async fn parent(&self, text: &str) -> Parent {
        let (conversation, turn, run, _) = self.start(text).await;
        let part = self.part_id();
        self.apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart {
                    part: NewPart {
                        id: part.clone(),
                        turn_id: turn.clone(),
                        run_id: run.clone(),
                        attempt: 1,
                        kind: PartKind::Task,
                        failure: None,
                        tool_call: Some(ToolCallDocument::receiving(
                            "call_1".to_owned(),
                            "start_task".to_owned(),
                        )),
                    },
                },
            ],
        )
        .await;
        Parent {
            conversation,
            turn,
            run,
            part,
        }
    }

    /// The parent's task part records a task, as the runtime stores it before
    /// the task exists.
    async fn record_task(&self, parent: &Parent, task: &TaskId, child: &ConversationId) {
        let mut call = ToolCallDocument::receiving("call_1".to_owned(), "start_task".to_owned());
        call.status = ToolCallStatus::Running;
        let section = TaskPartDocument {
            task: Some(TaskDocument {
                task_id: task.clone(),
                number: 1,
                conversation_id: child.clone(),
                agent: "explore".to_owned(),
                title: "Find the sessions".to_owned(),
                background: false,
                follow_up: false,
                state: TaskState::Accepted,
                instruction: "Find where sessions expire.".to_owned(),
                activity: None,
                tool_calls: 0,
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
        self.apply(
            &parent.conversation,
            vec![Change::UpdateTaskPart {
                part_id: parent.part.clone(),
                tool_call: call,
                task: Box::new(section),
                archives: Vec::new(),
            }],
        )
        .await;
    }

    /// Creates the child's conversation with its first task, for the parent's
    /// part.
    async fn child(&self, parent: &Parent, instruction: &str) -> Child {
        let conversation = self.conversation_id();
        let task = self.task_id();
        self.record_task(parent, &task, &conversation).await;
        let run = self.child_run("explore");
        let run_id = run.id.clone();
        let applied = self
            .apply(
                &conversation,
                vec![
                    Change::CreateConversation {
                        conversation: NewConversation {
                            id: conversation.clone(),
                            title: "Find the sessions".to_owned(),
                            profile_id: Some(Self::profile_id()),
                            project_id: None,
                            task: Some(TaskOrigin {
                                parent_id: parent.conversation.clone(),
                                number: 1,
                                agent: "explore".to_owned(),
                                executor: None,
                            }),
                        },
                    },
                    Change::StartTurn {
                        turn_id: self.turn_id(),
                        user_part_id: self.part_id(),
                        text: instruction.to_owned(),
                        run,
                        task: Some(NewTask {
                            id: task.clone(),
                            part_id: Some(parent.part.clone()),
                            assignment: None,
                            follow_up: false,
                            background: false,
                        }),
                    },
                ],
            )
            .await;
        assert_eq!(applied.summary, None, "a task conversation is not listed");
        Child {
            conversation,
            run: run_id,
            task,
        }
    }

    async fn end_run(
        &self,
        conversation: &ConversationId,
        run: &RunId,
        state: RunState,
        task: Option<TaskEnding>,
    ) {
        self.apply(
            conversation,
            vec![Change::EndRun {
                run_id: run.clone(),
                state,
                outcome: RunOutcome::default(),
                task,
            }],
        )
        .await;
    }

    /// The parent's task part as the UI shows it.
    async fn task_view(&self, parent: &Parent) -> (PartState, mannie_types::tasks::TaskCallView) {
        let snapshot = self
            .store
            .snapshot(&parent.conversation, 10, 100)
            .await
            .unwrap()
            .unwrap();
        snapshot
            .turns
            .iter()
            .flat_map(|turn| &turn.parts)
            .find(|part| part.id == parent.part)
            .map(|part| match &part.content {
                PartContent::Task { call } => (part.state, (**call).clone()),
                other => panic!("not a task part: {other:?}"),
            })
            .expect("the task part is shown")
    }
}

#[tokio::test]
async fn a_task_conversation_is_created_with_its_task_and_stays_out_of_the_list() {
    let fixture = Fixture::new();
    let parent = fixture.parent("Split the work").await;

    let child = fixture.child(&parent, "Find where sessions expire.").await;

    let listed = fixture.store.list_conversations().await.unwrap();
    assert_eq!(
        listed.iter().map(|summary| &summary.id).collect::<Vec<_>>(),
        [&parent.conversation]
    );
    assert_eq!(listed[0].task, None);
    let summary = fixture
        .store
        .conversation_summary(&child.conversation)
        .await
        .unwrap()
        .unwrap();
    let origin = summary.task.unwrap();
    assert_eq!(origin.parent_id, parent.conversation);
    assert_eq!(origin.number, 1);
    assert_eq!(origin.agent, "explore");
    assert_eq!(summary.project_id, None);
    let found = fixture
        .store
        .child_by_number(&parent.conversation, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.conversation_id, child.conversation);
    assert_eq!(found.title, "Find the sessions");
    let latest = found.latest.unwrap();
    assert_eq!(latest.id, child.task);
    assert_eq!(latest.part_id.as_ref(), Some(&parent.part));
    assert_eq!(latest.executor, None);
    assert_eq!(latest.assignment, None);
    assert_eq!(
        latest.state_version, 0,
        "only assigned tasks take state versions"
    );
    assert_eq!(found.executor, None);
    assert_eq!(latest.state, TaskState::Accepted);
    assert_eq!(
        fixture
            .store
            .child_by_number(&parent.conversation, 2)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        fixture
            .store
            .next_task_number(&parent.conversation)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        fixture
            .store
            .task_conversations(&parent.conversation)
            .await
            .unwrap(),
        std::slice::from_ref(&child.conversation)
    );
}

#[tokio::test]
async fn a_task_runs_with_its_run_and_ends_with_its_report() {
    let fixture = Fixture::new();
    let parent = fixture.parent("Split the work").await;
    let child = fixture.child(&parent, "Find where sessions expire.").await;

    fixture
        .apply(
            &child.conversation,
            vec![Change::StartRun {
                run_id: child.run.clone(),
            }],
        )
        .await;
    let running = fixture.store.task(&child.task).await.unwrap().unwrap();
    fixture
        .end_run(
            &child.conversation,
            &child.run,
            RunState::Completed,
            Some(TaskEnding {
                state: TaskState::Reported,
                report: Some("Sessions expire in session.rs:88.".to_owned()),
                failure: None,
            }),
        )
        .await;

    assert_eq!(running.state, TaskState::Running);
    let ended = fixture.store.task(&child.task).await.unwrap().unwrap();
    assert_eq!(ended.state, TaskState::Reported);
    assert_eq!(
        ended.ending.unwrap().report.as_deref(),
        Some("Sessions expire in session.rs:88.")
    );
    assert!(ended.ended_at.is_some());
    assert_eq!(
        fixture
            .store
            .task_of_part(&parent.part)
            .await
            .unwrap()
            .map(|task| task.id),
        Some(child.task)
    );
}

#[tokio::test]
async fn a_run_that_ends_without_a_task_ending_ends_its_task_as_its_state_implies() {
    for (state, expected) in [
        (RunState::Stopped, TaskState::Canceled),
        (RunState::Interrupted, TaskState::Interrupted),
        (RunState::Failed, TaskState::Failed),
    ] {
        let fixture = Fixture::new();
        let parent = fixture.parent("Split the work").await;
        let child = fixture.child(&parent, "Find where sessions expire.").await;
        fixture
            .apply(
                &child.conversation,
                vec![Change::StartRun {
                    run_id: child.run.clone(),
                }],
            )
            .await;

        fixture
            .apply(
                &child.conversation,
                vec![Change::EndRun {
                    run_id: child.run.clone(),
                    state,
                    outcome: RunOutcome {
                        failure: (state == RunState::Failed).then_some(RunFailure::QuotaExceeded),
                        ..RunOutcome::default()
                    },
                    task: None,
                }],
            )
            .await;

        let task = fixture.store.task(&child.task).await.unwrap().unwrap();
        assert_eq!(task.state, expected, "{state:?}");
        let failure = task.ending.unwrap().failure;
        assert_eq!(
            failure,
            (state == RunState::Failed).then_some(RunFailure::QuotaExceeded)
        );
    }
}

#[tokio::test]
async fn only_a_task_part_takes_task_updates_and_its_arguments_never_stream_to_the_ui() {
    let fixture = Fixture::new();
    let parent = fixture.parent("Split the work").await;
    let tool_part = fixture.tool_part(&parent.turn, &parent.run, 1, "call_2");
    let tool_part_id = tool_part.id.clone();
    fixture
        .apply(
            &parent.conversation,
            vec![Change::StartPart { part: tool_part }],
        )
        .await;

    let refused = fixture
        .store
        .apply(
            &parent.conversation,
            vec![Change::UpdateTaskPart {
                part_id: tool_part_id.clone(),
                tool_call: ToolCallDocument::receiving(
                    "call_2".to_owned(),
                    "write_file".to_owned(),
                ),
                task: Box::default(),
                archives: Vec::new(),
            }],
            Durability::Full,
        )
        .await;
    let delta = fixture
        .apply(
            &parent.conversation,
            vec![Change::Delta {
                part_id: parent.part.clone(),
                text: r#"{"instruction":"planted-instruction"}"#.to_owned(),
            }],
        )
        .await;

    assert!(matches!(
        refused,
        Err(RuntimeStoreError::NotATaskPart { part }) if part == tool_part_id
    ));
    assert_eq!(delta.events, Vec::new(), "subscribers get no arguments");
    let (state, call) = fixture.task_view(&parent).await;
    assert_eq!(state, PartState::Streaming);
    assert_eq!(call.tool, "start_task");
    assert_eq!(call.task, None);
}

#[tokio::test]
async fn an_open_task_part_finishes_from_a_task_that_ended_first() {
    let fixture = Fixture::new();
    let parent = fixture.parent("Split the work").await;
    let child = fixture.child(&parent, "Find where sessions expire.").await;
    fixture
        .apply(
            &child.conversation,
            vec![Change::StartRun {
                run_id: child.run.clone(),
            }],
        )
        .await;
    fixture
        .end_run(
            &child.conversation,
            &child.run,
            RunState::Completed,
            Some(TaskEnding {
                state: TaskState::Reported,
                report: Some("Found it.".to_owned()),
                failure: None,
            }),
        )
        .await;

    // The parent's run ends before it stored the report, as when Mannie
    // quits between the two.
    fixture
        .end_run(
            &parent.conversation,
            &parent.run,
            RunState::Interrupted,
            None,
        )
        .await;

    let (state, call) = fixture.task_view(&parent).await;
    assert_eq!(state, PartState::Interrupted);
    assert_eq!(call.status, ToolCallStatus::Completed);
    let task = call.task.unwrap();
    assert_eq!(task.state, TaskState::Reported);
    assert_eq!(task.report.as_deref(), Some("Found it."));
}

#[tokio::test]
async fn a_task_that_still_runs_ends_with_its_parents_run() {
    for (parent_state, expected) in [
        (RunState::Stopped, TaskState::Canceled),
        (RunState::Interrupted, TaskState::Interrupted),
    ] {
        let fixture = Fixture::new();
        let parent = fixture.parent("Split the work").await;
        let child = fixture.child(&parent, "Find where sessions expire.").await;

        fixture
            .end_run(&parent.conversation, &parent.run, parent_state, None)
            .await;

        let (_, call) = fixture.task_view(&parent).await;
        assert_eq!(call.status, ToolCallStatus::Canceled, "{parent_state:?}");
        assert_eq!(call.task.unwrap().state, expected);
        let task = fixture.store.task(&child.task).await.unwrap().unwrap();
        assert_eq!(task.state, expected);
        // The child's own end, which comes later, keeps the first ending.
        fixture
            .end_run(
                &child.conversation,
                &child.run,
                RunState::Completed,
                Some(TaskEnding {
                    state: TaskState::Reported,
                    report: Some("Too late.".to_owned()),
                    failure: None,
                }),
            )
            .await;
        let kept = fixture.store.task(&child.task).await.unwrap().unwrap();
        assert_eq!(kept.state, expected);
    }
}

#[tokio::test]
async fn a_task_part_whose_task_was_never_stored_did_not_start() {
    let fixture = Fixture::new();
    let parent = fixture.parent("Split the work").await;
    let never_created = fixture.conversation_id();
    let task = fixture.task_id();
    fixture.record_task(&parent, &task, &never_created).await;

    fixture
        .end_run(
            &parent.conversation,
            &parent.run,
            RunState::Interrupted,
            None,
        )
        .await;

    let (_, call) = fixture.task_view(&parent).await;
    assert_eq!(call.refusal, Some(TaskRefusal::NotStarted));
    assert_eq!(call.status, ToolCallStatus::Canceled);
}

#[tokio::test]
async fn recovery_ends_task_conversations_before_their_parents() {
    let fixture = Fixture::new();
    let parent = fixture.parent("Split the work").await;
    let child = fixture.child(&parent, "Find where sessions expire.").await;
    fixture
        .apply(
            &child.conversation,
            vec![Change::StartRun {
                run_id: child.run.clone(),
            }],
        )
        .await;

    let recovered = fixture.store.recover_interrupted_runs().await.unwrap();

    let order: Vec<_> = recovered
        .iter()
        .map(|run| run.conversation_id.clone())
        .collect();
    assert_eq!(
        order,
        [child.conversation.clone(), parent.conversation.clone()]
    );
    let task = fixture.store.task(&child.task).await.unwrap().unwrap();
    assert_eq!(task.state, TaskState::Interrupted);
    let (_, call) = fixture.task_view(&parent).await;
    assert_eq!(call.task.unwrap().state, TaskState::Interrupted);
}

#[tokio::test]
async fn deleting_a_parent_deletes_its_task_conversations_and_their_text() {
    let fixture = Fixture::new();
    let parent = fixture.parent("Split the work").await;
    let planted = "planted-child-instruction-4d1c";
    let child = fixture.child(&parent, planted).await;
    assert!(fixture.files_contain(planted), "the instruction was stored");

    let children = fixture
        .store
        .task_conversations(&parent.conversation)
        .await
        .unwrap();
    let deleted = fixture
        .store
        .delete_conversation(&parent.conversation)
        .await
        .unwrap();

    assert!(deleted);
    assert_eq!(children, std::slice::from_ref(&child.conversation));
    assert_eq!(
        fixture
            .store
            .conversation_summary(&child.conversation)
            .await
            .unwrap(),
        None
    );
    assert_eq!(fixture.store.task(&child.task).await.unwrap(), None);
    assert!(
        !fixture.files_contain(planted),
        "the child's text is gone from every file"
    );
}

#[tokio::test]
async fn removing_a_project_reports_the_task_conversations_it_deleted() {
    let fixture = Fixture::new();
    let project = fixture.project("/work/app").await;
    let conversation = fixture.conversation_in(&project, "In the project").await;
    let turn = fixture.turn_id();
    let run = fixture.new_run();
    let run_id = run.id.clone();
    let part = fixture.part_id();
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartTurn {
                    turn_id: turn.clone(),
                    user_part_id: fixture.part_id(),
                    text: "Split the work".to_owned(),
                    run,
                    task: None,
                },
                Change::StartRun {
                    run_id: run_id.clone(),
                },
                Change::StartPart {
                    part: NewPart {
                        id: part.clone(),
                        turn_id: turn.clone(),
                        run_id: run_id.clone(),
                        attempt: 1,
                        kind: PartKind::Task,
                        failure: None,
                        tool_call: Some(ToolCallDocument::receiving(
                            "call_1".to_owned(),
                            "start_task".to_owned(),
                        )),
                    },
                },
            ],
        )
        .await;
    let parent = Parent {
        conversation: conversation.clone(),
        turn,
        run: run_id.clone(),
        part,
    };
    let child = fixture.child(&parent, "Find where sessions expire.").await;
    fixture
        .end_run(&conversation, &run_id, RunState::Stopped, None)
        .await;
    fixture
        .end_run(&child.conversation, &child.run, RunState::Stopped, None)
        .await;

    let deletion = fixture
        .store
        .delete_project(&project)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(deletion.conversations, [conversation]);
    assert_eq!(deletion.task_conversations, [child.conversation]);
    assert!(deletion.project_deleted);
}

#[tokio::test]
async fn a_run_with_only_task_parts_stays_when_its_turn_is_retried() {
    let fixture = Fixture::new();
    let parent = fixture.parent("Split the work").await;
    fixture
        .end_run(&parent.conversation, &parent.run, RunState::Failed, None)
        .await;

    let (_, retried) = fixture.retry(&parent.conversation, &parent.turn).await;

    let kept = retried.events.iter().any(|(_, event)| {
        matches!(
            event,
            mannie_types::transcript::ConversationEvent::RunStarted { kept_run_ids, .. }
                if kept_run_ids.contains(&parent.run)
        )
    });
    assert!(kept, "{:?}", retried.events);
}

#[tokio::test]
async fn the_model_reads_an_open_task_part_and_counts_it_as_an_earlier_call() {
    let fixture = Fixture::new();
    let parent = fixture.parent("Split the work").await;
    let child = fixture.child(&parent, "Find where sessions expire.").await;
    let later = fixture.tool_part(&parent.turn, &parent.run, 2, "call_2");
    let later_id = later.id.clone();
    fixture
        .apply(
            &parent.conversation,
            vec![Change::StartPart { part: later }],
        )
        .await;

    let turns = fixture
        .store
        .model_turns(&parent.conversation, None)
        .await
        .unwrap();
    let before = fixture.store.tool_calls_before(&later_id).await.unwrap();

    let part = turns[0]
        .parts
        .iter()
        .find(|part| part.id == parent.part)
        .expect("the open task part reaches the model");
    assert_eq!(part.kind, Some(PartKind::Task));
    assert_eq!(part.state, PartState::Streaming);
    let task = part.task.as_ref().and_then(|section| section.task.as_ref());
    assert_eq!(task.map(|task| &task.task_id), Some(&child.task));
    assert_eq!(before, 1);
}
