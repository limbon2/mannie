use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use mannie_types::clock::{Clock, ManualClock, UtcMillis};
use mannie_types::ids::{CheckpointId, ConversationId, PartId, ProfileId, RunId, TurnId};
use mannie_types::run::{FinishReason, RunFailure, RunOutcome, RunState};
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
                        },
                    },
                    Change::StartTurn {
                        turn_id: turn.clone(),
                        user_part_id: self.part_id(),
                        text: text.to_owned(),
                        run,
                    },
                ],
            )
            .await;
        (conversation, turn, run_id, applied)
    }

    fn text_part(&self, turn: &TurnId, run: &RunId, attempt: u32) -> NewPart {
        NewPart {
            id: self.part_id(),
            turn_id: turn.clone(),
            run_id: run.clone(),
            attempt,
            kind: PartKind::Text,
            failure: None,
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
    parts: BTreeMap<u32, (PartId, PartState, String)>,
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
                (part.id.clone(), part.state, text_of(&part.content)),
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
            ConversationEvent::RunStarted { run } => {
                for turn in self.turns.values_mut() {
                    if turn.id == run.turn_id {
                        turn.run = Some((run.id.clone(), run.state));
                        // The owner's message is the turn's first part.
                        turn.parts.retain(|position, _| *position == 0);
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
            ConversationEvent::PartStarted { part } | ConversationEvent::PartFinished { part } => {
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
                                (part.id.clone(), part.state, text_of(&part.content)),
                            );
                        }
                    }
                }
            }
            ConversationEvent::PartDelta { part_id, text } => {
                for turn in self.turns.values_mut() {
                    for (id, _, existing) in turn.parts.values_mut() {
                        if id == part_id {
                            existing.push_str(text);
                        }
                    }
                }
            }
            ConversationEvent::AttemptDiscarded { part_ids, .. } => {
                for turn in self.turns.values_mut() {
                    turn.parts.retain(|_, (id, _, _)| !part_ids.contains(id));
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
                    },
                    Change::StartTurn {
                        turn_id: fixture.turn_id(),
                        user_part_id: fixture.part_id(),
                        text: format!("Turn {index}"),
                        run: next,
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
