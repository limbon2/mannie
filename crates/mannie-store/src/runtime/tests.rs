use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use mannie_types::clock::{Clock, ManualClock, UtcMillis};
use mannie_types::ids::{
    CheckpointId, ConversationId, PartId, ProfileId, ProjectId, RunId, TurnId,
};
use mannie_types::run::{FinishReason, RunFailure, RunOutcome, RunState};
use mannie_types::tools::ToolCallDocument;
use mannie_types::transcript::{
    ContextUsage, ConversationEvent, PartContent, PartKind, PartState, TurnView,
};

use super::{
    Applied, Change, EventsAfter, NewCheckpoint, NewConversation, NewPart, NewRun, ProviderItem,
    RuntimeStoreError,
};
use crate::{Durability, Store, StoreOptions};

/// 2026-10-13T12:00:00Z
const NOON: i64 = 1_791_892_800_000;

struct Fixture {
    directory: tempfile::TempDir,
    store: Store,
    clock: Arc<ManualClock>,
    next_id: AtomicU64,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::new(UtcMillis::from_millis(NOON)));
        let shared: Arc<dyn Clock> = Arc::<ManualClock>::clone(&clock);
        let options = StoreOptions::new(directory.path().join("backups"), shared);
        let store = Store::open(&directory.path().join("mannie.sqlite"), &options).unwrap();
        Self {
            directory,
            store,
            clock,
            next_id: AtomicU64::new(1),
        }
    }

    fn uuid(&self) -> String {
        let number = self.next_id.fetch_add(1, Ordering::SeqCst);
        format!("0199e0f2-7a1b-7c3d-8e4f-{number:012x}")
    }

    fn conversation_id(&self) -> ConversationId {
        ConversationId::parse(&self.uuid()).unwrap()
    }

    fn turn_id(&self) -> TurnId {
        TurnId::parse(&self.uuid()).unwrap()
    }

    fn run_id(&self) -> RunId {
        RunId::parse(&self.uuid()).unwrap()
    }

    fn part_id(&self) -> PartId {
        PartId::parse(&self.uuid()).unwrap()
    }

    fn profile_id() -> ProfileId {
        ProfileId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap()
    }

    fn new_run(&self) -> NewRun {
        let profile_id = Self::profile_id();
        NewRun {
            id: self.run_id(),
            agent: "conversation".to_owned(),
            binding: format!("{profile_id}:1"),
            profile_id,
        }
    }

    async fn apply(&self, conversation: &ConversationId, changes: Vec<Change>) -> Applied {
        self.store
            .apply(conversation, changes, Durability::Full)
            .await
            .unwrap()
    }

    /// Retries a turn with a new run, and returns the run and what the retry
    /// applied.
    async fn retry(&self, conversation: &ConversationId, turn: &TurnId) -> (RunId, Applied) {
        let run = self.new_run();
        let run_id = run.id.clone();
        let applied = self
            .apply(
                conversation,
                vec![Change::RetryTurn {
                    turn_id: turn.clone(),
                    run,
                }],
            )
            .await;
        (run_id, applied)
    }

    /// Creates a conversation with a first turn, and returns its IDs.
    async fn start(&self, text: &str) -> (ConversationId, TurnId, RunId, Applied) {
        let conversation = self.conversation_id();
        let turn = self.turn_id();
        let run = self.new_run();
        let run_id = run.id.clone();
        let applied = self
            .apply(
                &conversation,
                vec![
                    Change::CreateConversation {
                        conversation: NewConversation {
                            id: conversation.clone(),
                            title: "A title".to_owned(),
                            profile_id: None,
                            project_id: None,
                            task: None,
                        },
                    },
                    Change::StartTurn {
                        turn_id: turn.clone(),
                        user_part_id: self.part_id(),
                        text: text.to_owned(),
                        run,
                        task: None,
                    },
                ],
            )
            .await;
        (conversation, turn, run_id, applied)
    }

    /// Adds a project for a folder, and returns its ID.
    async fn project(&self, folder: &str) -> ProjectId {
        let id = ProjectId::parse(&self.uuid()).unwrap();
        self.store.add_project(&id, folder).await.unwrap();
        id
    }

    /// Creates a conversation in a project, and returns its ID.
    async fn conversation_in(&self, project: &ProjectId, title: &str) -> ConversationId {
        let conversation = self.conversation_id();
        self.apply(
            &conversation,
            vec![Change::CreateConversation {
                conversation: NewConversation {
                    id: conversation.clone(),
                    title: title.to_owned(),
                    profile_id: None,
                    project_id: Some(project.clone()),
                    task: None,
                },
            }],
        )
        .await;
        conversation
    }

    fn text_part(&self, turn: &TurnId, run: &RunId, attempt: u32) -> NewPart {
        NewPart {
            id: self.part_id(),
            turn_id: turn.clone(),
            run_id: run.clone(),
            attempt,
            kind: PartKind::Text,
            failure: None,
            tool_call: None,
        }
    }

    fn tool_part(&self, turn: &TurnId, run: &RunId, attempt: u32, call: &str) -> NewPart {
        NewPart {
            id: self.part_id(),
            turn_id: turn.clone(),
            run_id: run.clone(),
            attempt,
            kind: PartKind::ToolCall,
            failure: None,
            tool_call: Some(ToolCallDocument::receiving(
                call.to_owned(),
                "write_file".to_owned(),
            )),
        }
    }

    fn files_contain(&self, needle: &str) -> bool {
        let mut found = false;
        for entry in std::fs::read_dir(self.directory.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let bytes = std::fs::read(&path).unwrap();
                found |= bytes
                    .windows(needle.len())
                    .any(|window| window == needle.as_bytes());
            }
        }
        found
    }
}

/// The state that a subscriber builds from events: each turn's run state and
/// the visible parts with their state and text.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Reduced {
    turns: BTreeMap<u32, ReducedTurn>,
    title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReducedTurn {
    id: TurnId,
    run: Option<(RunId, RunState)>,
    parts: BTreeMap<u32, (PartId, Option<RunId>, PartState, String)>,
}

impl Reduced {
    fn from_turns(turns: &[TurnView]) -> Self {
        let mut reduced = Self::default();
        for turn in turns {
            reduced.insert_turn(turn);
        }
        reduced
    }

    fn insert_turn(&mut self, turn: &TurnView) {
        let mut parts = BTreeMap::new();
        for part in &turn.parts {
            parts.insert(
                part.position,
                (
                    part.id.clone(),
                    part.run_id.clone(),
                    part.state,
                    text_of(&part.content),
                ),
            );
        }
        self.turns.insert(
            turn.position,
            ReducedTurn {
                id: turn.id.clone(),
                run: turn.run.as_ref().map(|run| (run.id.clone(), run.state)),
                parts,
            },
        );
    }

    fn apply(&mut self, event: &ConversationEvent) {
        match event {
            ConversationEvent::TurnStarted { turn } => self.insert_turn(turn),
            ConversationEvent::RunStarted { run, kept_run_ids } => {
                for turn in self.turns.values_mut() {
                    if turn.id == run.turn_id {
                        turn.run = Some((run.id.clone(), run.state));
                        // The owner's message is the turn's first part, and
                        // the runs that made tool calls stay.
                        turn.parts.retain(|position, (_, part_run, _, _)| {
                            let is_owners = *position == 0;
                            let is_kept = part_run
                                .as_ref()
                                .is_some_and(|part_run| kept_run_ids.contains(part_run));
                            is_owners || is_kept
                        });
                    }
                }
            }
            ConversationEvent::RunUpdated { run } => {
                for turn in self.turns.values_mut() {
                    let current = turn.run.as_ref().is_some_and(|(id, _)| *id == run.id);
                    if current {
                        turn.run = Some((run.id.clone(), run.state));
                    }
                }
            }
            ConversationEvent::PartStarted { part }
            | ConversationEvent::PartUpdated { part }
            | ConversationEvent::PartFinished { part } => {
                for turn in self.turns.values_mut() {
                    let belongs = part
                        .run_id
                        .as_ref()
                        .is_some_and(|run| turn.run.as_ref().is_some_and(|(id, _)| id == run));
                    if belongs {
                        if part.state == PartState::Discarded {
                            turn.parts.remove(&part.position);
                        } else {
                            turn.parts.insert(
                                part.position,
                                (
                                    part.id.clone(),
                                    part.run_id.clone(),
                                    part.state,
                                    text_of(&part.content),
                                ),
                            );
                        }
                    }
                }
            }
            ConversationEvent::PartDelta { part_id, text } => {
                for turn in self.turns.values_mut() {
                    for (id, _, _, existing) in turn.parts.values_mut() {
                        if id == part_id {
                            existing.push_str(text);
                        }
                    }
                }
            }
            ConversationEvent::AttemptDiscarded { part_ids, .. } => {
                for turn in self.turns.values_mut() {
                    turn.parts.retain(|_, (id, _, _, _)| !part_ids.contains(id));
                }
            }
            ConversationEvent::ConversationChanged { conversation } => {
                self.title = Some(conversation.title.clone());
            }
            _ => {}
        }
    }
}

fn text_of(content: &PartContent) -> String {
    match content {
        PartContent::UserText { text }
        | PartContent::Text { text }
        | PartContent::Reasoning { text } => text.clone(),
        PartContent::Error { failure } => failure.code().to_owned(),
        PartContent::ToolCall { call } => {
            format!("{} {:?} {}", call.tool, call.status, call.arguments)
        }
        PartContent::Task { call } => format!("{} {:?}", call.tool, call.status),
        PartContent::Unknown { name } => name.clone(),
    }
}

#[tokio::test]
async fn starting_a_conversation_stores_the_turn_and_announces_it() {
    let fixture = Fixture::new();

    let (conversation, turn, run, applied) = fixture.start("Hello there").await;

    assert_eq!(applied.last_seq, 1);
    let [(seq, ConversationEvent::TurnStarted { turn: started })] = applied.events.as_slice()
    else {
        panic!("{:?}", applied.events);
    };
    assert_eq!(*seq, 1);
    assert_eq!(started.id, turn);
    assert_eq!(started.position, 0);
    assert_eq!(
        started.run.as_ref().map(|run| (&run.id, run.state)),
        Some((&run, RunState::Queued))
    );
    assert_eq!(
        started
            .parts
            .iter()
            .map(|part| text_of(&part.content))
            .collect::<Vec<_>>(),
        ["Hello there"]
    );
    let summary = applied.summary.unwrap();
    assert!(summary.has_active_run);
    assert_eq!(summary.id, conversation);
    let listed = fixture.store.list_conversations().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title, "A title");
}

#[tokio::test]
async fn a_second_turn_is_refused_while_a_run_is_active_and_nothing_is_stored() {
    let fixture = Fixture::new();
    let (conversation, _, _, _) = fixture.start("First").await;

    let refused = fixture
        .store
        .apply(
            &conversation,
            vec![Change::StartTurn {
                turn_id: fixture.turn_id(),
                user_part_id: fixture.part_id(),
                text: "Second".to_owned(),
                run: fixture.new_run(),
                task: None,
            }],
            Durability::Full,
        )
        .await;

    assert!(
        matches!(refused, Err(RuntimeStoreError::RunActive)),
        "{refused:?}"
    );
    let snapshot = fixture
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.through_seq, 1);
    assert_eq!(snapshot.turns.len(), 1);
}

#[tokio::test]
async fn changes_to_a_missing_conversation_are_refused() {
    let fixture = Fixture::new();
    let missing = fixture.conversation_id();

    let refused = fixture
        .store
        .apply(
            &missing,
            vec![Change::Rename {
                title: "Renamed".to_owned(),
            }],
            Durability::Full,
        )
        .await;

    assert!(
        matches!(refused, Err(RuntimeStoreError::ConversationNotFound)),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_streaming_part_is_rebuilt_from_its_deltas_until_it_finishes() {
    let fixture = Fixture::new();
    let (conversation, turn, run, _) = fixture.start("Question").await;
    let part = fixture.text_part(&turn, &run, 1);
    let part_id = part.id.clone();
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart { part },
            ],
        )
        .await;
    for text in ["Hel", "lo, ", "world"] {
        fixture
            .store
            .apply(
                &conversation,
                vec![Change::Delta {
                    part_id: part_id.clone(),
                    text: text.to_owned(),
                }],
                Durability::Normal,
            )
            .await
            .unwrap();
    }

    let streaming = fixture
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    let streamed = &streaming.turns[0].parts[1];
    assert_eq!(streamed.state, PartState::Streaming);
    assert_eq!(text_of(&streamed.content), "Hello, world");

    let finished = fixture
        .apply(
            &conversation,
            vec![
                Change::FinishPart {
                    part_id: part_id.clone(),
                    state: PartState::Complete,
                    tail: "!".to_owned(),
                    provider_item: None,
                },
                Change::EndRun {
                    run_id: run.clone(),
                    state: RunState::Completed,
                    outcome: RunOutcome {
                        finish: Some(FinishReason::Stop),
                        ..RunOutcome::default()
                    },
                    task: None,
                },
            ],
        )
        .await;

    let Some((_, ConversationEvent::PartFinished { part })) = finished.events.first() else {
        panic!("{:?}", finished.events);
    };
    assert_eq!(text_of(&part.content), "Hello, world!");
    let summary = finished.summary.unwrap();
    assert!(!summary.has_active_run);
    let deltas_left = fixture
        .store
        .read(move |connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM runtime_events WHERE part_id = ?1 AND kind = 'part_delta'",
                [part_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(deltas_left, 0, "the finished part replaced its deltas");
}

#[tokio::test]
async fn replaying_from_any_cursor_reaches_the_state_of_a_fresh_snapshot() {
    let fixture = Fixture::new();
    let (conversation, turn, run, first) = fixture.start("Question").await;
    let mut live = first.events.clone();
    let reasoning = NewPart {
        kind: PartKind::Reasoning,
        ..fixture.text_part(&turn, &run, 1)
    };
    let text = fixture.text_part(&turn, &run, 1);
    let batches = vec![
        vec![Change::StartRun {
            run_id: run.clone(),
        }],
        vec![Change::StartPart {
            part: reasoning.clone(),
        }],
        vec![Change::Delta {
            part_id: reasoning.id.clone(),
            text: "Think".to_owned(),
        }],
        vec![Change::FinishPart {
            part_id: reasoning.id.clone(),
            state: PartState::Complete,
            tail: "ing".to_owned(),
            provider_item: None,
        }],
        vec![Change::StartPart { part: text.clone() }],
        vec![Change::Delta {
            part_id: text.id.clone(),
            text: "An".to_owned(),
        }],
        vec![Change::Delta {
            part_id: text.id.clone(),
            text: "swer".to_owned(),
        }],
        vec![Change::Rename {
            title: "Renamed".to_owned(),
        }],
    ];
    for changes in batches {
        live.extend(fixture.apply(&conversation, changes).await.events);
    }
    // The last part still streams; replay must rebuild its text, and a
    // second phase finishes it after more cursors were taken.
    let mut cursors_and_states = Vec::new();
    let mut state = Reduced::default();
    cursors_and_states.push((0_u32, state.clone()));
    for (seq, event) in &live {
        state.apply(event);
        cursors_and_states.push((*seq, state.clone()));
    }
    let finish = fixture
        .apply(
            &conversation,
            vec![
                Change::FinishPart {
                    part_id: text.id.clone(),
                    state: PartState::Complete,
                    tail: String::new(),
                    provider_item: None,
                },
                Change::EndRun {
                    run_id: run.clone(),
                    state: RunState::Completed,
                    outcome: RunOutcome::default(),
                    task: None,
                },
            ],
        )
        .await;
    live.extend(finish.events);

    let fresh = fixture
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    let expected = Reduced::from_turns(&fresh.turns);
    for (cursor, mut state_at_cursor) in cursors_and_states {
        let EventsAfter::Events { events, last_seq } = fixture
            .store
            .events_after(&conversation, cursor, 5_000)
            .await
            .unwrap()
        else {
            panic!("cursor {cursor} should replay");
        };
        let mut previous = cursor;
        for (seq, event) in &events {
            assert!(
                *seq > previous,
                "sequence numbers increase after cursor {cursor}"
            );
            previous = *seq;
            state_at_cursor.apply(event);
        }
        assert_eq!(last_seq, fresh.through_seq);
        assert_eq!(
            state_at_cursor.turns, expected.turns,
            "replay from cursor {cursor}"
        );
    }
}

#[tokio::test]
async fn a_cursor_ahead_or_far_behind_starts_from_a_snapshot() {
    let fixture = Fixture::new();
    let (conversation, _, _, _) = fixture.start("Question").await;

    let ahead = fixture
        .store
        .events_after(&conversation, 99, 5_000)
        .await
        .unwrap();
    let behind = fixture
        .store
        .events_after(&conversation, 0, 0)
        .await
        .unwrap();
    let missing = fixture
        .store
        .events_after(&fixture.conversation_id(), 0, 5_000)
        .await
        .unwrap();

    assert_eq!(ahead, EventsAfter::UseSnapshot);
    assert_eq!(behind, EventsAfter::UseSnapshot);
    assert_eq!(missing, EventsAfter::NotFound);
}

#[tokio::test]
async fn retry_replaces_the_reply_of_the_latest_turn_only_after_it_failed() {
    let fixture = Fixture::new();
    let (conversation, turn, run, _) = fixture.start("Question").await;
    let part = fixture.text_part(&turn, &run, 1);
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart { part: part.clone() },
                Change::Delta {
                    part_id: part.id.clone(),
                    text: "Partial".to_owned(),
                },
                Change::EndRun {
                    run_id: run.clone(),
                    state: RunState::Failed,
                    outcome: RunOutcome {
                        failure: Some(RunFailure::ServerError { status: 500 }),
                        ..RunOutcome::default()
                    },
                    task: None,
                },
            ],
        )
        .await;
    let failed = fixture
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.turns[0].parts[1].state, PartState::Incomplete);
    assert_eq!(text_of(&failed.turns[0].parts[1].content), "Partial");

    let retry = fixture.new_run();
    let retry_id = retry.id.clone();
    let applied = fixture
        .apply(
            &conversation,
            vec![Change::RetryTurn {
                turn_id: turn.clone(),
                run: retry,
            }],
        )
        .await;

    assert!(matches!(
        applied.events.as_slice(),
        [(_, ConversationEvent::RunStarted { .. })]
    ));
    let retried = fixture
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        retried.turns[0].run.as_ref().map(|run| &run.id),
        Some(&retry_id)
    );
    assert_eq!(
        retried.turns[0].parts.len(),
        1,
        "only the owner's message stays visible"
    );
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: retry_id.clone(),
                },
                Change::EndRun {
                    run_id: retry_id,
                    state: RunState::Completed,
                    outcome: RunOutcome::default(),
                    task: None,
                },
            ],
        )
        .await;
    let again = fixture
        .store
        .apply(
            &conversation,
            vec![Change::RetryTurn {
                turn_id: turn,
                run: fixture.new_run(),
            }],
            Durability::Full,
        )
        .await;
    assert!(
        matches!(again, Err(RuntimeStoreError::NothingToRetry)),
        "{again:?}"
    );
}

#[tokio::test]
async fn a_discarded_attempt_disappears_and_leaves_no_deltas() {
    let fixture = Fixture::new();
    let (conversation, turn, run, _) = fixture.start("Question").await;
    let part = fixture.text_part(&turn, &run, 1);
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart { part: part.clone() },
                Change::Delta {
                    part_id: part.id.clone(),
                    text: "Lost".to_owned(),
                },
            ],
        )
        .await;

    let applied = fixture
        .apply(
            &conversation,
            vec![Change::DiscardAttempt {
                run_id: run,
                attempt: 1,
            }],
        )
        .await;

    let [(_, ConversationEvent::AttemptDiscarded { part_ids, .. })] = applied.events.as_slice()
    else {
        panic!("{:?}", applied.events);
    };
    assert_eq!(part_ids, &[part.id]);
    let snapshot = fixture
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.turns[0].parts.len(), 1);
}

#[tokio::test]
async fn recovery_interrupts_active_runs_and_keeps_their_stored_text() {
    let fixture = Fixture::new();
    let (conversation, turn, run, _) = fixture.start("Question").await;
    let part = fixture.text_part(&turn, &run, 1);
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart { part: part.clone() },
                Change::Delta {
                    part_id: part.id.clone(),
                    text: "Half an".to_owned(),
                },
            ],
        )
        .await;

    let recovered = fixture.store.recover_interrupted_runs().await.unwrap();

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].run_id, run);
    let snapshot = fixture
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    let turn_view = &snapshot.turns[0];
    assert_eq!(
        turn_view.run.as_ref().map(|run| run.state),
        Some(RunState::Interrupted)
    );
    assert_eq!(turn_view.parts[1].state, PartState::Interrupted);
    assert_eq!(text_of(&turn_view.parts[1].content), "Half an");
    assert_eq!(
        fixture.store.recover_interrupted_runs().await.unwrap(),
        Vec::new()
    );
    assert_eq!(fixture.store.active_run_count().await.unwrap(), 0);
}

#[tokio::test]
async fn the_model_reads_its_parts_and_provider_items_but_not_errors_or_discarded_parts() {
    let fixture = Fixture::new();
    let (conversation, turn, run, _) = fixture.start("Question").await;
    let reasoning = NewPart {
        kind: PartKind::Reasoning,
        ..fixture.text_part(&turn, &run, 1)
    };
    let note = NewPart {
        kind: PartKind::ContinuationNote,
        ..fixture.text_part(&turn, &run, 1)
    };
    let error = NewPart {
        kind: PartKind::Error,
        failure: Some(RunFailure::BrokenStream),
        ..fixture.text_part(&turn, &run, 2)
    };
    let item = ProviderItem {
        binding: "binding".to_owned(),
        item: r#"{"type":"reasoning"}"#.to_owned(),
    };
    let applied = fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart {
                    part: reasoning.clone(),
                },
                Change::FinishPart {
                    part_id: reasoning.id.clone(),
                    state: PartState::Complete,
                    tail: "thought".to_owned(),
                    provider_item: Some(item.clone()),
                },
                Change::AddPart {
                    part: note,
                    text: "Continue.".to_owned(),
                },
                Change::AddPart {
                    part: error,
                    text: String::new(),
                },
            ],
        )
        .await;
    assert!(
        applied.events.iter().all(|(_, event)| !matches!(
            event,
            ConversationEvent::PartFinished { part } if text_of(&part.content) == "Continue."
        )),
        "the continuation note never reaches subscribers"
    );

    let turns = fixture
        .store
        .model_turns(&conversation, None)
        .await
        .unwrap();

    let kinds: Vec<_> = turns[0].parts.iter().map(|part| part.kind).collect();
    assert_eq!(
        kinds,
        [
            Some(PartKind::UserText),
            Some(PartKind::Reasoning),
            Some(PartKind::ContinuationNote)
        ]
    );
    assert_eq!(turns[0].parts[1].provider_item.as_ref(), Some(&item));
    assert_eq!(turns[0].run_state, Some(RunState::Running));
    assert_eq!(
        fixture
            .store
            .model_turns(&conversation, Some(0))
            .await
            .unwrap(),
        Vec::new()
    );
}

#[tokio::test]
async fn snapshots_and_earlier_turns_page_through_long_conversations() {
    let fixture = Fixture::new();
    let (conversation, _, first_run, _) = fixture.start("Turn 0").await;
    let mut run = first_run;
    for index in 1..=5 {
        let next = fixture.new_run();
        let next_id = next.id.clone();
        fixture
            .apply(
                &conversation,
                vec![
                    Change::StartRun {
                        run_id: run.clone(),
                    },
                    Change::EndRun {
                        run_id: run.clone(),
                        state: RunState::Completed,
                        outcome: RunOutcome::default(),
                        task: None,
                    },
                    Change::StartTurn {
                        turn_id: fixture.turn_id(),
                        user_part_id: fixture.part_id(),
                        text: format!("Turn {index}"),
                        run: next,
                        task: None,
                    },
                ],
            )
            .await;
        run = next_id;
    }

    let snapshot = fixture
        .store
        .snapshot(&conversation, 2, 400)
        .await
        .unwrap()
        .unwrap();
    let small = fixture
        .store
        .snapshot(&conversation, 50, 3)
        .await
        .unwrap()
        .unwrap();
    let (earlier, more) = fixture
        .store
        .earlier_turns(&conversation, 4, 3)
        .await
        .unwrap();
    let (first, none_before) = fixture
        .store
        .earlier_turns(&conversation, 1, 3)
        .await
        .unwrap();

    let positions = |turns: &[TurnView]| turns.iter().map(|turn| turn.position).collect::<Vec<_>>();
    assert_eq!(positions(&snapshot.turns), [4, 5]);
    assert!(snapshot.has_earlier_turns);
    assert_eq!(positions(&small.turns), [3, 4, 5]);
    assert!(small.has_earlier_turns);
    assert_eq!(positions(&earlier), [1, 2, 3]);
    assert!(more);
    assert_eq!(positions(&first), [0]);
    assert!(!none_before);
}

#[tokio::test]
async fn checkpoints_and_context_usage_reach_the_snapshot() {
    let fixture = Fixture::new();
    let (conversation, _, _, _) = fixture.start("Question").await;
    let checkpoint_id = CheckpointId::parse(&fixture.uuid()).unwrap();
    let usage = ContextUsage {
        used_tokens: 1_200,
        budget_tokens: 100_000,
        estimated: false,
    };

    let applied = fixture
        .apply(
            &conversation,
            vec![
                Change::AddCheckpoint {
                    checkpoint: NewCheckpoint {
                        id: checkpoint_id.clone(),
                        covers_through_turn: 0,
                        content_hash: "hash".to_owned(),
                        summary: "The owner asked a question.".to_owned(),
                        tokens_before: 9_000,
                        tokens_after: 1_000,
                        trigger: "manual".to_owned(),
                    },
                },
                Change::UpdateContext {
                    ledger: r#"{"anchor":null}"#.to_owned(),
                    usage,
                },
            ],
        )
        .await;

    assert_eq!(applied.events.len(), 2);
    let snapshot = fixture
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.context, Some(usage));
    assert_eq!(
        snapshot.checkpoint.map(|view| view.id),
        Some(checkpoint_id.clone())
    );
    let checkpoint = fixture
        .store
        .checkpoint(&conversation, &checkpoint_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint.summary, "The owner asked a question.");
    assert_eq!(checkpoint.view.tokens_before, 9_000);
    assert_eq!(
        fixture
            .store
            .checkpoints(&conversation)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fixture
            .store
            .context_ledger(&conversation)
            .await
            .unwrap()
            .as_deref(),
        Some(r#"{"anchor":null}"#)
    );
}

#[tokio::test]
async fn renaming_and_choosing_a_profile_change_the_summary() {
    let fixture = Fixture::new();
    let (conversation, _, _, _) = fixture.start("Question").await;
    fixture.clock.advance(std::time::Duration::from_secs(60));

    let renamed = fixture
        .apply(
            &conversation,
            vec![
                Change::Rename {
                    title: "Plans".to_owned(),
                },
                Change::SetProfile {
                    profile_id: Some(Fixture::profile_id()),
                },
            ],
        )
        .await;

    let summary = renamed.summary.unwrap();
    assert_eq!(summary.title, "Plans");
    assert_eq!(summary.profile_id, Some(Fixture::profile_id()));
    assert_eq!(
        summary.updated_at.as_millis(),
        NOON,
        "renaming does not reorder the list"
    );
}

#[tokio::test]
async fn a_deleted_conversation_leaves_no_text_in_the_database_or_its_log() {
    let fixture = Fixture::new();
    let planted = "planted-conversation-text-7f3a91";
    let (conversation, _, _, _) = fixture.start(planted).await;
    assert!(fixture.files_contain(planted), "the text was stored");

    let deleted = fixture
        .store
        .delete_conversation(&conversation)
        .await
        .unwrap();

    assert!(deleted);
    assert!(
        !fixture
            .store
            .delete_conversation(&conversation)
            .await
            .unwrap()
    );
    assert_eq!(
        fixture.store.list_conversations().await.unwrap(),
        Vec::new()
    );
    assert!(
        !fixture.files_contain(planted),
        "the deleted text is gone from every file"
    );
}

/// A tool call's document as the runtime moves it through its statuses.
fn with_status(
    document: &ToolCallDocument,
    status: mannie_types::tools::ToolCallStatus,
) -> ToolCallDocument {
    ToolCallDocument {
        status,
        ..document.clone()
    }
}

const WRITE_ARGUMENTS: &str = r#"{"path":"a.txt","content":"hello"}"#;

/// The changes that move a write's tool call from its start to its end, with
/// an archived output, and the document it ends with.
fn tool_call_lifecycle(
    run: &RunId,
    call: &NewPart,
    archive_id: &mannie_types::ids::ArchiveId,
) -> (Vec<Vec<Change>>, ToolCallDocument) {
    use mannie_types::tools::{ArchiveRef, ToolCallStatus, ToolOutcome, ToolResult};

    let part_id = call.id.clone();
    let receiving = call.tool_call.clone().unwrap();
    let completed = ToolCallDocument {
        status: ToolCallStatus::Completed,
        result: Some(ToolResult {
            outcome: ToolOutcome::Written {
                path: "/work/a.txt".to_owned(),
                created: true,
                bytes: 5,
            },
            output: "Created /work/a.txt (5 bytes).".to_owned(),
            archives: vec![ArchiveRef {
                id: archive_id.clone(),
                name: "stdout".to_owned(),
                bytes: 17,
                lines: 2,
            }],
        }),
        ..receiving.clone()
    };
    let batches = vec![
        vec![Change::StartRun {
            run_id: run.clone(),
        }],
        vec![Change::StartPart { part: call.clone() }],
        vec![
            Change::Delta {
                part_id: part_id.clone(),
                text: WRITE_ARGUMENTS.to_owned(),
            },
            Change::UpdatePart {
                part_id: part_id.clone(),
                tool_call: with_status(&receiving, ToolCallStatus::Pending),
                archives: Vec::new(),
            },
        ],
        vec![Change::UpdatePart {
            part_id: part_id.clone(),
            tool_call: with_status(&receiving, ToolCallStatus::Executing),
            archives: Vec::new(),
        }],
        vec![Change::UpdatePart {
            part_id: part_id.clone(),
            tool_call: completed.clone(),
            archives: vec![crate::NewArchive {
                id: archive_id.clone(),
                name: "stdout".to_owned(),
                content: "line one\nline two".to_owned(),
            }],
        }],
        vec![Change::FinishPart {
            part_id,
            state: PartState::Complete,
            tail: String::new(),
            provider_item: None,
        }],
    ];
    (batches, completed)
}

#[tokio::test]
async fn a_tool_call_part_changes_until_it_finishes_and_replays_like_a_snapshot() {
    let fixture = Fixture::new();
    let (conversation, turn, run, first) = fixture.start("Write a file").await;
    let mut live = first.events.clone();
    let call = fixture.tool_part(&turn, &run, 1, "call_1");
    let archive_id = mannie_types::ids::ArchiveId::parse(&fixture.uuid()).unwrap();
    let (batches, completed) = tool_call_lifecycle(&run, &call, &archive_id);
    for changes in batches {
        live.extend(fixture.apply(&conversation, changes).await.events);
    }

    let deltas = live
        .iter()
        .filter(|(_, event)| matches!(event, ConversationEvent::PartDelta { .. }))
        .count();
    assert_eq!(deltas, 0, "a tool call's arguments arrive with its updates");
    let updated: Vec<_> = live
        .iter()
        .filter_map(|(_, event)| match event {
            ConversationEvent::PartUpdated { part } => Some(text_of(&part.content)),
            _ => None,
        })
        .collect();
    assert_eq!(
        updated,
        [
            format!("write_file Pending {WRITE_ARGUMENTS}"),
            format!("write_file Executing {WRITE_ARGUMENTS}"),
            format!("write_file Completed {WRITE_ARGUMENTS}"),
        ]
    );
    let mut reduced = Reduced::default();
    for (_, event) in &live {
        reduced.apply(event);
    }
    let snapshot = fixture
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reduced.turns, Reduced::from_turns(&snapshot.turns).turns);
    let EventsAfter::Events { events, .. } = fixture
        .store
        .events_after(&conversation, 0, 1_000)
        .await
        .unwrap()
    else {
        panic!("the events are replayed");
    };
    let mut replayed = Reduced::default();
    for (_, event) in &events {
        replayed.apply(event);
    }
    assert_eq!(replayed.turns, reduced.turns);
    let archive = fixture
        .store
        .archive(&conversation, &archive_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(archive.content, "line one\nline two");
    assert_eq!(archive.lines, 2);
    let turns = fixture
        .store
        .model_turns(&conversation, None)
        .await
        .unwrap();
    let tool_part = &turns[0].parts[1];
    assert_eq!(tool_part.text, WRITE_ARGUMENTS);
    assert_eq!(tool_part.tool_call.as_ref(), Some(&completed));
}

#[tokio::test]
async fn only_open_tool_call_parts_take_updates() {
    let fixture = Fixture::new();
    let (conversation, turn, run, _) = fixture.start("Question").await;
    let text = fixture.text_part(&turn, &run, 1);
    let call = fixture.tool_part(&turn, &run, 1, "call_1");
    let document = call.tool_call.clone().unwrap();
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart { part: text.clone() },
                Change::StartPart { part: call.clone() },
                Change::FinishPart {
                    part_id: call.id.clone(),
                    state: PartState::Complete,
                    tail: String::new(),
                    provider_item: None,
                },
            ],
        )
        .await;

    let on_text = fixture
        .store
        .apply(
            &conversation,
            vec![Change::UpdatePart {
                part_id: text.id.clone(),
                tool_call: document.clone(),
                archives: Vec::new(),
            }],
            Durability::Full,
        )
        .await;
    let on_finished = fixture
        .store
        .apply(
            &conversation,
            vec![Change::UpdatePart {
                part_id: call.id.clone(),
                tool_call: document,
                archives: Vec::new(),
            }],
            Durability::Full,
        )
        .await;

    assert!(matches!(
        on_text,
        Err(RuntimeStoreError::NotAToolCall { .. })
    ));
    assert!(matches!(
        on_finished,
        Err(RuntimeStoreError::PartNotStreaming { .. })
    ));
}

#[tokio::test]
async fn open_tool_calls_end_as_uncertain_only_when_they_were_executing() {
    use mannie_types::tools::{ProcessRecord, ToolCallStatus};

    let fixture = Fixture::new();
    let (conversation, turn, run, _) = fixture.start("Run two things").await;
    let waiting = fixture.tool_part(&turn, &run, 1, "call_1");
    let executing = fixture.tool_part(&turn, &run, 1, "call_2");
    let process = ProcessRecord {
        pid: 4242,
        start_time: "1234567".to_owned(),
    };
    let running_command = ToolCallDocument {
        status: ToolCallStatus::Executing,
        tool: "run_command".to_owned(),
        process: Some(process.clone()),
        ..executing.tool_call.clone().unwrap()
    };
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart {
                    part: waiting.clone(),
                },
                Change::UpdatePart {
                    part_id: waiting.id.clone(),
                    tool_call: with_status(
                        waiting.tool_call.as_ref().unwrap(),
                        ToolCallStatus::AwaitingApproval,
                    ),
                    archives: Vec::new(),
                },
                Change::StartPart {
                    part: executing.clone(),
                },
                Change::UpdatePart {
                    part_id: executing.id.clone(),
                    tool_call: running_command,
                    archives: Vec::new(),
                },
            ],
        )
        .await;

    let recovered = fixture.store.recover_interrupted_runs().await.unwrap();

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].uncertain.len(), 1);
    assert_eq!(recovered[0].uncertain[0].part_id, executing.id);
    assert_eq!(recovered[0].uncertain[0].process.as_ref(), Some(&process));
    let turns = fixture
        .store
        .model_turns(&conversation, None)
        .await
        .unwrap();
    let statuses: Vec<_> = turns[0]
        .parts
        .iter()
        .filter_map(|part| part.tool_call.as_ref().map(|call| call.status))
        .collect();
    assert_eq!(
        statuses,
        [ToolCallStatus::Canceled, ToolCallStatus::Uncertain]
    );
    assert!(
        turns[0].parts[1..]
            .iter()
            .all(|part| part.state == PartState::Interrupted)
    );
}

#[tokio::test]
async fn a_retry_keeps_a_reply_that_made_tool_calls_and_replaces_one_that_made_none() {
    let fixture = Fixture::new();
    let (conversation, turn, first_run, _) = fixture.start("Do the work").await;
    let call = fixture.tool_part(&turn, &first_run, 1, "call_1");
    let failure = RunOutcome {
        failure: Some(RunFailure::ServerError { status: 500 }),
        ..RunOutcome::default()
    };
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: first_run.clone(),
                },
                Change::StartPart { part: call.clone() },
                Change::FinishPart {
                    part_id: call.id.clone(),
                    state: PartState::Complete,
                    tail: String::new(),
                    provider_item: None,
                },
                Change::EndRun {
                    run_id: first_run.clone(),
                    state: RunState::Failed,
                    outcome: failure.clone(),
                    task: None,
                },
            ],
        )
        .await;
    let (second_run, retried) = fixture.retry(&conversation, &turn).await;
    let text = fixture.text_part(&turn, &second_run, 1);
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: second_run.clone(),
                },
                Change::AddPart {
                    part: text,
                    text: "No tools this time".to_owned(),
                },
                Change::EndRun {
                    run_id: second_run.clone(),
                    state: RunState::Failed,
                    outcome: failure,
                    task: None,
                },
            ],
        )
        .await;
    let (third_run, retried_again) = fixture.retry(&conversation, &turn).await;

    let kept_of = |applied: &Applied| match applied.events.as_slice() {
        [(_, ConversationEvent::RunStarted { kept_run_ids, .. })] => kept_run_ids.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(kept_of(&retried), vec![first_run.clone()]);
    assert_eq!(kept_of(&retried_again), vec![first_run.clone()]);
    let snapshot = fixture
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    let shown: Vec<_> = snapshot.turns[0]
        .parts
        .iter()
        .map(|part| part.run_id.clone())
        .collect();
    assert_eq!(shown, vec![None, Some(first_run.clone())]);
    let turns = fixture
        .store
        .model_turns(&conversation, None)
        .await
        .unwrap();
    assert_eq!(turns[0].run_id.as_ref(), Some(&third_run));
    assert_eq!(
        turns[0]
            .kept_runs
            .iter()
            .map(|kept| (kept.run_id.clone(), kept.state))
            .collect::<Vec<_>>(),
        vec![(first_run, RunState::Failed)]
    );
    assert_eq!(turns[0].parts.len(), 2);
}

#[tokio::test]
async fn a_conversation_in_a_project_works_in_its_folder_and_shows_a_waiting_approval() {
    use mannie_types::run::RunActivity;

    let fixture = Fixture::new();
    let project = fixture.project("/work").await;
    let conversation = fixture.conversation_id();
    let turn = fixture.turn_id();
    let run = fixture.new_run();
    let run_id = run.id.clone();
    let created = fixture
        .apply(
            &conversation,
            vec![
                Change::CreateConversation {
                    conversation: NewConversation {
                        id: conversation.clone(),
                        title: "Work".to_owned(),
                        profile_id: None,
                        project_id: Some(project.clone()),
                        task: None,
                    },
                },
                Change::StartTurn {
                    turn_id: turn,
                    user_part_id: fixture.part_id(),
                    text: "Hello".to_owned(),
                    run,
                    task: None,
                },
                Change::StartRun {
                    run_id: run_id.clone(),
                },
            ],
        )
        .await;
    let waiting = fixture
        .apply(
            &conversation,
            vec![Change::SetActivity {
                run_id: run_id.clone(),
                activity: RunActivity::AwaitingApproval {
                    part_id: fixture.part_id(),
                },
            }],
        )
        .await;

    let summary = created.summary.unwrap();
    assert_eq!(summary.workspace.as_deref(), Some("/work"));
    assert_eq!(summary.project_id.as_ref(), Some(&project));
    let listed = fixture.store.list_conversations().await.unwrap();
    assert!(listed[0].awaiting_approval);
    assert!(waiting.summary.unwrap().awaiting_approval);
}

#[tokio::test]
async fn a_general_conversation_moves_into_a_project_once() {
    let fixture = Fixture::new();
    let first = fixture.project("/work/first").await;
    let second = fixture.project("/work/second").await;
    let (conversation, _turn, _run, started) = fixture.start("Hello").await;
    let missing = ProjectId::parse(&fixture.uuid()).unwrap();

    let moved = fixture
        .apply(
            &conversation,
            vec![Change::SetProject {
                project_id: first.clone(),
            }],
        )
        .await;
    let again = fixture
        .store
        .apply(
            &conversation,
            vec![Change::SetProject { project_id: second }],
            Durability::Full,
        )
        .await;
    let (general, _turn, _run, _started) = fixture.start("Elsewhere").await;
    let nowhere = fixture
        .store
        .apply(
            &general,
            vec![Change::SetProject {
                project_id: missing,
            }],
            Durability::Full,
        )
        .await;

    let before = started.summary.unwrap();
    assert_eq!(before.project_id, None);
    assert_eq!(before.workspace, None);
    let Some((
        _,
        ConversationEvent::ConversationChanged {
            conversation: summary,
        },
    )) = moved.events.first()
    else {
        panic!("{:?}", moved.events);
    };
    assert_eq!(summary.project_id.as_ref(), Some(&first));
    assert_eq!(summary.workspace.as_deref(), Some("/work/first"));
    assert_eq!(moved.summary.as_ref(), Some(summary));
    assert!(
        matches!(again, Err(RuntimeStoreError::AlreadyInProject)),
        "{again:?}"
    );
    assert!(
        matches!(nowhere, Err(RuntimeStoreError::ProjectNotFound)),
        "{nowhere:?}"
    );
    let stored = fixture
        .store
        .conversation_summary(&conversation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.project_id.as_ref(), Some(&first));
    assert_eq!(stored.workspace.as_deref(), Some("/work/first"));
}

#[tokio::test]
async fn a_conversation_cannot_start_in_a_project_that_does_not_exist() {
    let fixture = Fixture::new();
    let conversation = fixture.conversation_id();

    let refused = fixture
        .store
        .apply(
            &conversation,
            vec![Change::CreateConversation {
                conversation: NewConversation {
                    id: conversation.clone(),
                    title: "Nowhere".to_owned(),
                    profile_id: None,
                    project_id: Some(ProjectId::parse(&fixture.uuid()).unwrap()),
                    task: None,
                },
            }],
            Durability::Full,
        )
        .await;

    assert!(
        matches!(refused, Err(RuntimeStoreError::ProjectNotFound)),
        "{refused:?}"
    );
    assert_eq!(fixture.store.list_conversations().await.unwrap(), []);
}

#[tokio::test]
async fn a_folder_is_one_project_whose_name_is_the_folders_until_renamed() {
    let fixture = Fixture::new();
    let id = ProjectId::parse(&fixture.uuid()).unwrap();
    let other_id = ProjectId::parse(&fixture.uuid()).unwrap();

    let added = fixture
        .store
        .add_project(&id, "/work/mannie")
        .await
        .unwrap();
    let again = fixture
        .store
        .add_project(&other_id, "/work/mannie")
        .await
        .unwrap();
    let renamed = fixture
        .store
        .rename_project(&id, "Mannie app")
        .await
        .unwrap()
        .unwrap();
    let missing = fixture
        .store
        .rename_project(&other_id, "Nothing")
        .await
        .unwrap();

    assert!(added.created);
    assert_eq!(added.project.id, id);
    assert_eq!(added.project.name, "mannie");
    assert_eq!(added.project.folder, "/work/mannie");
    assert_eq!(added.project.created_at, UtcMillis::from_millis(NOON));
    assert!(!again.created);
    assert_eq!(again.project, added.project);
    assert_eq!(renamed.name, "Mannie app");
    assert_eq!(renamed.folder, "/work/mannie");
    assert_eq!(missing, None);
    assert_eq!(
        fixture.store.projects().await.unwrap(),
        vec![renamed.clone()]
    );
    assert_eq!(fixture.store.project(&id).await.unwrap(), Some(renamed));
    assert_eq!(fixture.store.project(&other_id).await.unwrap(), None);
}

#[tokio::test]
async fn deleting_a_project_deletes_its_conversations_first_and_only_its_own() {
    let fixture = Fixture::new();
    let project = fixture.project("/work").await;
    let kept_project = fixture.project("/kept").await;
    let in_project = fixture.conversation_in(&project, "In the project").await;
    let also_in_project = fixture.conversation_in(&project, "Also in it").await;
    let elsewhere = fixture.conversation_in(&kept_project, "Elsewhere").await;
    let (general, _turn, _run, _started) = fixture.start("General").await;

    let deletion = fixture
        .store
        .delete_project(&project)
        .await
        .unwrap()
        .unwrap();
    let again = fixture.store.delete_project(&project).await.unwrap();

    assert_eq!(deletion.conversations, vec![in_project, also_in_project]);
    assert!(deletion.project_deleted);
    assert_eq!(again, None);
    let remaining: Vec<ConversationId> = fixture
        .store
        .list_conversations()
        .await
        .unwrap()
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    assert_eq!(remaining.len(), 2);
    assert!(remaining.contains(&elsewhere) && remaining.contains(&general));
    let projects = fixture.store.projects().await.unwrap();
    assert_eq!(
        projects
            .iter()
            .map(|project| &project.id)
            .collect::<Vec<_>>(),
        [&kept_project]
    );
    assert!(!fixture.files_contain("In the project"));
}

#[tokio::test]
async fn the_list_reads_its_conversations_and_projects_together() {
    let fixture = Fixture::new();
    let project = fixture.project("/work").await;
    let conversation = fixture.conversation_in(&project, "In the project").await;

    let list = fixture.store.conversation_list().await.unwrap();

    assert_eq!(
        list.conversations
            .iter()
            .map(|summary| &summary.id)
            .collect::<Vec<_>>(),
        [&conversation]
    );
    assert_eq!(
        list.projects
            .iter()
            .map(|project| &project.id)
            .collect::<Vec<_>>(),
        [&project]
    );
}

#[tokio::test]
async fn a_turn_knows_its_request_and_the_tool_calls_before_a_part() {
    let fixture = Fixture::new();
    let (conversation, turn, run, _started) = fixture.start("Work in mannie").await;
    let first = fixture.tool_part(&turn, &run, 1, "call-1");
    let second = fixture.tool_part(&turn, &run, 1, "call-2");
    let (first_id, second_id) = (first.id.clone(), second.id.clone());
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart { part: first },
                Change::StartPart { part: second },
            ],
        )
        .await;

    assert_eq!(
        fixture.store.turn_request(&turn).await.unwrap().as_deref(),
        Some("Work in mannie")
    );
    assert_eq!(fixture.store.tool_calls_before(&first_id).await.unwrap(), 0);
    assert_eq!(
        fixture.store.tool_calls_before(&second_id).await.unwrap(),
        1
    );
    assert_eq!(
        fixture
            .store
            .turn_request(&fixture.turn_id())
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn the_summary_changes_when_a_reply_starts_or_stops_waiting_for_the_owner() {
    use mannie_types::run::RunActivity;

    let fixture = Fixture::new();
    let (conversation, _turn, run_id, _started) = fixture.start("Hello").await;
    fixture
        .apply(
            &conversation,
            vec![Change::StartRun {
                run_id: run_id.clone(),
            }],
        )
        .await;
    let activity = |activity: RunActivity| {
        vec![Change::SetActivity {
            run_id: run_id.clone(),
            activity,
        }]
    };

    let streaming_text = fixture
        .apply(&conversation, activity(RunActivity::Writing))
        .await;
    let asking_the_owner = fixture
        .apply(
            &conversation,
            activity(RunActivity::AwaitingApproval {
                part_id: fixture.part_id(),
            }),
        )
        .await;
    let tool_started = fixture
        .apply(
            &conversation,
            activity(RunActivity::RunningTool {
                part_id: fixture.part_id(),
                started_at: UtcMillis::from_millis(1),
            }),
        )
        .await;
    let next_call = fixture
        .apply(&conversation, activity(RunActivity::Preparing))
        .await;

    assert_eq!(streaming_text.summary, None);
    assert!(asking_the_owner.summary.unwrap().awaiting_approval);
    assert!(!tool_started.summary.unwrap().awaiting_approval);
    assert_eq!(next_call.summary, None);
}

#[tokio::test]
async fn a_deleted_conversation_leaves_no_archive_or_grant_behind() {
    use mannie_types::ids::{ArchiveId, GrantId};
    use mannie_types::tools::{AccessMode, GrantScope};

    let fixture = Fixture::new();
    let archived = "planted-archived-output-5c21e8";
    let granted = "planted-granted-folder-91ad04";
    let (conversation, turn, run, _) = fixture.start("Run it").await;
    let call = fixture.tool_part(&turn, &run, 1, "call_1");
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart { part: call.clone() },
                Change::UpdatePart {
                    part_id: call.id.clone(),
                    tool_call: call.tool_call.clone().unwrap(),
                    archives: vec![crate::NewArchive {
                        id: ArchiveId::parse(&fixture.uuid()).unwrap(),
                        name: "stdout".to_owned(),
                        content: archived.to_owned(),
                    }],
                },
            ],
        )
        .await;
    fixture
        .store
        .add_grants(
            &conversation,
            vec![(
                GrantId::parse(&fixture.uuid()).unwrap(),
                GrantScope::Path {
                    mode: AccessMode::Write,
                    folder: format!("/work/{granted}"),
                },
            )],
        )
        .await
        .unwrap();
    assert!(fixture.files_contain(archived));
    assert!(fixture.files_contain(granted));

    fixture
        .store
        .delete_conversation(&conversation)
        .await
        .unwrap();

    assert!(!fixture.files_contain(archived), "the archive is gone");
    assert!(!fixture.files_contain(granted), "the grant is gone");
}

/// The tests of tasks and task conversations (docs/features/child-agents/
/// tasks.md, data).
mod tasks;
mod worker_tasks;
