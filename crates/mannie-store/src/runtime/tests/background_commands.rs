//! Background commands in the store (ADR-0041, docs/features/system-tools/
//! background-commands.md, data): a command's record beside the part of the
//! call that started it, the events of its start and its end, its archive,
//! the snapshot's list, numbers that are never reused, recovery and the
//! deletion with its conversation, the ends told to the model, and the note
//! that joins a request.

use mannie_types::clock::UtcMillis;
use mannie_types::ids::{ArchiveId, ConversationId, PartId};
use mannie_types::run::{RunOutcome, RunState};
use mannie_types::tools::{
    BackgroundCommandDocument, BackgroundEnding, BackgroundSource, CommandEndView, CommandNoteView,
    ProcessRecord,
};
use mannie_types::transcript::{ConversationEvent, PartContent, PartKind};

use super::{Fixture, NOON};
use crate::runtime::{Change, LOCAL_SOURCE, NewArchive};

fn running(text: &str) -> BackgroundCommandDocument {
    BackgroundCommandDocument {
        command: text.to_owned(),
        source: BackgroundSource::Local {
            shell: "/bin/sh".to_owned(),
            working_directory: "/work".to_owned(),
        },
        process: Some(ProcessRecord {
            pid: 4242,
            start_time: "17".to_owned(),
        }),
        started_at: UtcMillis::from_millis(NOON),
        ending: None,
        exit_code: None,
        signal: None,
        ended_at: None,
        lines: 0,
        bytes: 0,
        replaced: false,
        head_lines: 0,
        left_out: None,
        archive_id: None,
        read_up_to: 0,
        told: false,
    }
}

/// A conversation whose run made a call that starts a command, and the call's
/// part.
async fn with_call(fixture: &Fixture) -> (ConversationId, PartId) {
    let (conversation, turn, run, _) = fixture.start("Run the server").await;
    let part = fixture.tool_part(&turn, &run, 1, "call-1");
    let part_id = part.id.clone();
    fixture
        .apply(
            &conversation,
            vec![Change::StartRun { run_id: run }, Change::StartPart { part }],
        )
        .await;
    (conversation, part_id)
}

fn start(number: u32, part_id: &PartId, text: &str) -> Change {
    Change::StartBackgroundCommand {
        number,
        part_id: part_id.clone(),
        source: LOCAL_SOURCE.to_owned(),
        document: Box::new(running(text)),
    }
}

fn changed(events: &[(u32, ConversationEvent)]) -> Vec<(u32, Option<BackgroundEnding>)> {
    events
        .iter()
        .filter_map(|(_seq, event)| match event {
            ConversationEvent::BackgroundCommandChanged { command } => {
                Some((command.number, command.ending))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_start_and_an_end_are_events_and_the_end_archives_the_output_under_the_call() {
    let fixture = Fixture::new();
    let (conversation, part_id) = with_call(&fixture).await;

    let started = fixture
        .apply(&conversation, vec![start(1, &part_id, "npm run dev")])
        .await;
    let mut ended = fixture
        .store
        .background_command(&conversation, 1)
        .await
        .unwrap()
        .unwrap()
        .document;
    let archive_id = ArchiveId::parse(&fixture.uuid()).unwrap();
    ended.ending = Some(BackgroundEnding::Exited);
    ended.exit_code = Some(0);
    ended.lines = 2;
    ended.archive_id = Some(archive_id.clone());
    let finished = fixture
        .apply(
            &conversation,
            vec![Change::UpdateBackgroundCommand {
                number: 1,
                document: Box::new(ended),
                archive: Some(NewArchive {
                    id: archive_id.clone(),
                    name: "output".to_owned(),
                    content: "ready\nbye\n".to_owned(),
                }),
                announce: true,
            }],
        )
        .await;

    assert_eq!(changed(&started.events), [(1, None)]);
    assert_eq!(
        changed(&finished.events),
        [(1, Some(BackgroundEnding::Exited))]
    );
    let archive = fixture
        .store
        .archive(&conversation, &archive_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(archive.name, "output");
    assert_eq!(archive.lines, 2);
    let snapshot = fixture
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.background_commands.len(), 1);
    let view = &snapshot.background_commands[0];
    assert_eq!(view.part_id, part_id);
    assert_eq!(view.ending, Some(BackgroundEnding::Exited));
    assert_eq!(view.archive_id, Some(archive_id));
}

#[tokio::test]
async fn a_change_that_is_not_announced_publishes_nothing() {
    let fixture = Fixture::new();
    let (conversation, part_id) = with_call(&fixture).await;
    fixture
        .apply(&conversation, vec![start(1, &part_id, "journalctl -f")])
        .await;
    let mut read = running("journalctl -f");
    read.read_up_to = 40;

    let applied = fixture
        .apply(
            &conversation,
            vec![Change::UpdateBackgroundCommand {
                number: 1,
                document: Box::new(read),
                archive: None,
                announce: false,
            }],
        )
        .await;

    assert_eq!(changed(&applied.events), Vec::new());
    let stored = fixture
        .store
        .background_command(&conversation, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.document.read_up_to, 40);
}

#[tokio::test]
async fn numbers_follow_the_highest_that_a_conversation_ever_had() {
    let fixture = Fixture::new();
    let (conversation, part_id) = with_call(&fixture).await;

    assert_eq!(
        fixture
            .store
            .next_background_number(&conversation)
            .await
            .unwrap(),
        1
    );
    fixture
        .apply(
            &conversation,
            vec![start(1, &part_id, "one"), start(2, &part_id, "two")],
        )
        .await;

    assert_eq!(
        fixture
            .store
            .next_background_number(&conversation)
            .await
            .unwrap(),
        3
    );
    let numbers: Vec<u32> = fixture
        .store
        .background_commands(&conversation)
        .await
        .unwrap()
        .iter()
        .map(|command| command.number)
        .collect();
    assert_eq!(numbers, [1, 2]);
}

#[tokio::test]
async fn a_number_that_the_conversation_has_is_refused() {
    let fixture = Fixture::new();
    let (conversation, part_id) = with_call(&fixture).await;
    fixture
        .apply(&conversation, vec![start(1, &part_id, "one")])
        .await;

    let again = fixture
        .store
        .apply(
            &conversation,
            vec![start(1, &part_id, "again")],
            crate::Durability::Full,
        )
        .await;

    again.unwrap_err();
}

#[tokio::test]
async fn recovery_interrupts_the_commands_that_ran_and_names_their_processes() {
    let fixture = Fixture::new();
    let (conversation, part_id) = with_call(&fixture).await;
    let mut ended = running("cargo test");
    ended.ending = Some(BackgroundEnding::Exited);
    fixture
        .apply(
            &conversation,
            vec![
                start(1, &part_id, "npm run dev"),
                Change::StartBackgroundCommand {
                    number: 2,
                    part_id: part_id.clone(),
                    source: LOCAL_SOURCE.to_owned(),
                    document: Box::new(ended),
                },
            ],
        )
        .await;

    let recovered = fixture.store.recover_background_commands().await.unwrap();
    let again = fixture.store.recover_background_commands().await.unwrap();

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].number, 1);
    assert_eq!(recovered[0].process.as_ref().unwrap().pid, 4242);
    assert_eq!(again, Vec::new());
    let interrupted = fixture
        .store
        .background_command(&conversation, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        interrupted.document.ending,
        Some(BackgroundEnding::Interrupted)
    );
    assert!(!interrupted.document.told);
    assert!(interrupted.document.ended_at.is_some());
}

#[tokio::test]
async fn a_deleted_conversation_takes_its_commands_with_it() {
    let fixture = Fixture::new();
    let (conversation, part_id) = with_call(&fixture).await;
    fixture
        .apply(
            &conversation,
            vec![start(1, &part_id, "tail -f planted-text-7f3a")],
        )
        .await;

    assert!(
        fixture
            .store
            .delete_conversation(&conversation)
            .await
            .unwrap()
    );

    assert_eq!(
        fixture
            .store
            .background_commands(&conversation)
            .await
            .unwrap(),
        Vec::new()
    );
    assert_eq!(
        fixture.store.recover_background_commands().await.unwrap(),
        Vec::new()
    );
}

#[tokio::test]
async fn an_end_is_told_once_and_telling_changes_nothing_that_the_view_shows() {
    let fixture = Fixture::new();
    let (conversation, part_id) = with_call(&fixture).await;
    let mut ended = running("cargo test");
    ended.ending = Some(BackgroundEnding::Exited);
    fixture
        .apply(
            &conversation,
            vec![Change::StartBackgroundCommand {
                number: 1,
                part_id,
                source: LOCAL_SOURCE.to_owned(),
                document: Box::new(ended),
            }],
        )
        .await;

    let told = fixture
        .apply(
            &conversation,
            vec![Change::TellBackgroundEnds { numbers: vec![1] }],
        )
        .await;
    let again = fixture
        .apply(
            &conversation,
            vec![Change::TellBackgroundEnds { numbers: vec![1] }],
        )
        .await;

    assert_eq!(changed(&told.events), Vec::new());
    assert_eq!(changed(&again.events), Vec::new());
    let stored = fixture
        .store
        .background_command(&conversation, 1)
        .await
        .unwrap()
        .unwrap();
    assert!(stored.document.told);
}

#[tokio::test]
async fn telling_a_command_that_the_conversation_never_had_fails_the_batch() {
    let fixture = Fixture::new();
    let (conversation, _part_id) = with_call(&fixture).await;

    let told = fixture
        .store
        .apply(
            &conversation,
            vec![Change::TellBackgroundEnds { numbers: vec![4] }],
            crate::Durability::Full,
        )
        .await;

    told.unwrap_err();
}

#[tokio::test]
async fn a_request_note_follows_the_request_and_only_the_model_reads_it() {
    let fixture = Fixture::new();
    let (conversation, turn, _run, _applied) = fixture.start("Is the server up?").await;

    let applied = fixture
        .apply(
            &conversation,
            vec![Change::AddRequestNote {
                turn_id: turn.clone(),
                part_id: fixture.part_id(),
                text: "Background commands of this conversation:".to_owned(),
            }],
        )
        .await;

    assert_eq!(applied.events, Vec::new());
    let turns = fixture
        .store
        .model_turns(&conversation, None)
        .await
        .unwrap();
    let parts: Vec<_> = turns[0]
        .parts
        .iter()
        .map(|part| (part.kind, part.run_id.clone(), part.text.as_str()))
        .collect();
    assert_eq!(
        parts,
        [
            (Some(PartKind::UserText), None, "Is the server up?"),
            (
                Some(PartKind::ContinuationNote),
                None,
                "Background commands of this conversation:"
            ),
        ]
    );
    let snapshot = fixture
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.turns[0].parts.len(), 1);
}

#[tokio::test]
async fn a_request_note_needs_a_turn_of_its_own_conversation() {
    let fixture = Fixture::new();
    let (_first, turn, _run, _applied) = fixture.start("First").await;
    let (second, _turn, _run, _applied) = fixture.start("Second").await;

    let added = fixture
        .store
        .apply(
            &second,
            vec![Change::AddRequestNote {
                turn_id: turn,
                part_id: fixture.part_id(),
                text: "A note".to_owned(),
            }],
            crate::Durability::Full,
        )
        .await;

    added.unwrap_err();
}

fn note_of(number: u32) -> CommandNoteView {
    CommandNoteView {
        ends: vec![CommandEndView {
            number,
            command: "cargo test".to_owned(),
            ending: BackgroundEnding::Exited,
            exit_code: Some(0),
            signal: None,
            unread: 12,
        }],
    }
}

#[tokio::test]
async fn a_command_turn_stands_on_the_note_and_the_owner_and_the_model_see_it() {
    let fixture = Fixture::new();
    let (conversation, turn, run, _applied) = fixture.start("Run the tests").await;
    fixture
        .apply(
            &conversation,
            vec![Change::EndRun {
                run_id: run,
                state: RunState::Completed,
                outcome: RunOutcome::default(),
                task: None,
            }],
        )
        .await;
    let woken = fixture.turn_id();

    let applied = fixture
        .apply(
            &conversation,
            vec![Change::StartCommandTurn {
                turn_id: woken.clone(),
                part_id: fixture.part_id(),
                text: "Background commands of this conversation:".to_owned(),
                note: note_of(1),
                run: fixture.new_run(),
            }],
        )
        .await;

    let started = applied.events.iter().find_map(|(_seq, event)| match event {
        ConversationEvent::TurnStarted { turn } => Some(turn.clone()),
        _ => None,
    });
    let started = started.expect("the turn is announced");
    assert_eq!(started.id, woken);
    assert_ne!(started.id, turn);
    assert_eq!(
        started.parts[0].content,
        PartContent::CommandNote { note: note_of(1) }
    );
    let turns = fixture
        .store
        .model_turns(&conversation, None)
        .await
        .unwrap();
    let request = &turns[1].parts[0];
    assert_eq!(request.kind, Some(PartKind::CommandNote));
    assert_eq!(request.text, "Background commands of this conversation:");
    assert_eq!(fixture.store.turn_request(&woken).await.unwrap(), None);
}

#[tokio::test]
async fn a_command_turn_waits_for_the_active_run() {
    let fixture = Fixture::new();
    let (conversation, _turn, _run, _applied) = fixture.start("Run the tests").await;

    let refused = fixture
        .store
        .apply(
            &conversation,
            vec![Change::StartCommandTurn {
                turn_id: fixture.turn_id(),
                part_id: fixture.part_id(),
                text: "A note".to_owned(),
                note: note_of(1),
                run: fixture.new_run(),
            }],
            crate::Durability::Full,
        )
        .await;

    assert!(matches!(refused, Err(crate::RuntimeStoreError::RunActive)));
}
