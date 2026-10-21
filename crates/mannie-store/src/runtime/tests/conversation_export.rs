use super::*;

#[tokio::test]
async fn export_captures_complete_mentions_images_and_streaming_arguments() {
    let fixture = Fixture::new();
    let picture = image(&fixture, "../picture [é].png", b"stored-image");
    let expected_image = picture.data.as_slice().to_vec();
    let (conversation, _) = start_with_images(&fixture, vec![picture]).await;
    let latest = fixture
        .store
        .snapshot(&conversation, 1, 1)
        .await
        .unwrap()
        .unwrap();
    let turn = &latest.turns[0];
    let run = turn.run.as_ref().unwrap().id.clone();
    let call = fixture.tool_part(&turn.id, &run, 1, "long-call");
    let arguments = "é".repeat(40_000);
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun { run_id: run },
                Change::StartPart { part: call.clone() },
                Change::Delta {
                    part_id: call.id.clone(),
                    text: arguments.clone(),
                },
            ],
        )
        .await;

    let record = fixture
        .store
        .conversation_record(&conversation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.images[0].data, expected_image);
    assert_eq!(record.turns[0].parts.last().unwrap().text, arguments);
    assert_eq!(record.turns[0].parts.last().unwrap().state, "streaming");
    let mention = record.turns[0]
        .parts
        .iter()
        .find(|part| part.kind == "mention")
        .unwrap();
    assert_ne!(mention.text, "");
    let snapshot = fixture
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    let PartContent::ToolCall { call } = &snapshot.turns[0].parts.last().unwrap().content else {
        panic!("expected tool call");
    };
    assert!(call.arguments_cut);
    assert_eq!(record.through_seq, snapshot.through_seq);
    assert_eq!(record.captured_at, fixture.clock.now());
}

#[tokio::test]
async fn export_is_not_limited_to_the_latest_turns_and_excludes_model_only_parts() {
    let fixture = Fixture::new();
    let (conversation, turn, run, _) = fixture.start("first").await;
    fixture
        .apply(
            &conversation,
            vec![Change::StartRun {
                run_id: run.clone(),
            }],
        )
        .await;
    let mut note = fixture.text_part(&turn, &run, 1);
    note.kind = PartKind::ContinuationNote;
    fixture
        .apply(
            &conversation,
            vec![Change::AddPart {
                part: note,
                text: "model-only-marker".to_owned(),
            }],
        )
        .await;
    // Seed more than the UI's initial page while preserving foreign keys and
    // unique positions. Export must not inherit UI page limits.
    for position in 1..110 {
        let id = fixture.turn_id().to_string();
        let parent = conversation.to_string();
        let part = fixture.part_id().to_string();
        fixture.store.write(Durability::Full, move |transaction| {
            transaction.execute("INSERT INTO runtime_turns (id, conversation_id, position, created_at) VALUES (?1, ?2, ?3, 1)", rusqlite::params![id, parent, position])?;
            transaction.execute("INSERT INTO runtime_parts (id, conversation_id, turn_id, position, kind, audience, state, text, version, document, created_at) VALUES (?1, ?2, ?3, 0, 'user_text', 'both', 'complete', 'older turn', 1, '{}', 1)", rusqlite::params![part, parent, id])?;
            Ok(())
        }).await.unwrap();
    }
    let record = fixture
        .store
        .conversation_record(&conversation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.turns.len(), 110);
    assert_eq!(record.turns[0].parts[0].text, "first");
    assert!(
        !serde_json::to_string(&record)
            .unwrap()
            .contains("model-only-marker")
    );
    assert!(
        fixture
            .store
            .snapshot(&conversation, 50, 400)
            .await
            .unwrap()
            .unwrap()
            .has_earlier_turns
    );
}

#[tokio::test]
async fn export_excludes_discarded_content_and_warns_about_missing_images() {
    let fixture = Fixture::new();
    let picture = image(&fixture, "image.png", b"original");
    let (conversation, _) = start_with_images(&fixture, vec![picture]).await;
    let snapshot = fixture
        .store
        .snapshot(&conversation, 1, 400)
        .await
        .unwrap()
        .unwrap();
    let turn = &snapshot.turns[0];
    let run = &turn.run.as_ref().unwrap().id;
    let part = fixture.text_part(&turn.id, run, 1);
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart { part: part.clone() },
                Change::Delta {
                    part_id: part.id,
                    text: "discarded-marker".to_owned(),
                },
                Change::DiscardAttempt {
                    run_id: run.clone(),
                    attempt: 1,
                },
            ],
        )
        .await;
    let id = conversation.to_string();
    fixture
        .store
        .write(Durability::Full, move |transaction| {
            transaction.execute(
                "DELETE FROM runtime_images WHERE conversation_id = ?1",
                [id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let record = fixture
        .store
        .conversation_record(&conversation)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !serde_json::to_string(&record)
            .unwrap()
            .contains("discarded-marker")
    );
    assert_eq!(record.warnings.len(), 1);
    assert_eq!(
        record.warnings[0].reason,
        mannie_types::export::ExportError::AttachmentMissing
    );
    assert!(
        fixture
            .store
            .conversation_record(&fixture.conversation_id())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn export_keeps_complete_archives_checkpoints_and_unknown_parts_without_provider_items() {
    let fixture = Fixture::new();
    let (conversation, turn, run, _) = fixture.start("record everything").await;
    let call = fixture.tool_part(&turn, &run, 1, "archive-call");
    let archive_id = mannie_types::ids::ArchiveId::parse(&fixture.uuid()).unwrap();
    let archive_text = format!("{}\r\nlast line\n", "é".repeat(100_000));
    let checkpoint_id = CheckpointId::parse(&fixture.uuid()).unwrap();
    let unknown_part = fixture.part_id();
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun { run_id: run },
                Change::StartPart { part: call.clone() },
                Change::Delta {
                    part_id: call.id.clone(),
                    text: "complete arguments".to_owned(),
                },
                Change::UpdatePart {
                    part_id: call.id.clone(),
                    tool_call: call.tool_call.unwrap(),
                    archives: vec![crate::NewArchive {
                        id: archive_id.clone(),
                        name: "stdout".to_owned(),
                        content: archive_text.clone(),
                    }],
                },
                Change::FinishPart {
                    part_id: call.id,
                    state: PartState::Complete,
                    tail: String::new(),
                    provider_item: Some(ProviderItem {
                        binding: "private".to_owned(),
                        item: r#"{"private":"provider-private-marker"}"#.to_owned(),
                    }),
                },
                Change::AddCheckpoint {
                    checkpoint: NewCheckpoint {
                        id: checkpoint_id.clone(),
                        covers_through_turn: 0,
                        content_hash: "hash".to_owned(),
                        summary: "saved checkpoint".to_owned(),
                        tokens_before: 1000,
                        tokens_after: 100,
                        trigger: "manual".to_owned(),
                    },
                },
            ],
        )
        .await;
    let parent = conversation.to_string();
    let turn_id = turn.to_string();
    fixture.store.write(Durability::Full, move |transaction| {
        transaction.execute("INSERT INTO runtime_parts (id, conversation_id, turn_id, position, kind, audience, state, text, version, document, created_at) VALUES (?1, ?2, ?3, 20, 'future_kind', 'owner', 'complete', 'future saved text', 1, '{}', 1)", rusqlite::params![unknown_part.to_string(), parent, turn_id])?;
        Ok(())
    }).await.unwrap();
    let record = fixture
        .store
        .conversation_record(&conversation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.archives[0].id, archive_id);
    assert_eq!(record.archives[0].content, archive_text);
    assert_eq!(record.checkpoints[0].view.id, checkpoint_id);
    assert_eq!(record.checkpoints[0].summary, "saved checkpoint");
    assert_eq!(record.turns[0].parts.last().unwrap().kind, "future_kind");
    assert_eq!(
        record.turns[0].parts.last().unwrap().text,
        "future saved text"
    );
    assert!(
        !serde_json::to_string(&record)
            .unwrap()
            .contains("provider-private-marker")
    );
}

#[tokio::test]
async fn export_keeps_tool_activity_and_run_endings_while_replacing_superseded_replies() {
    for state in [RunState::Failed, RunState::Stopped, RunState::Interrupted] {
        let fixture = Fixture::new();
        let (conversation, turn, first, _) = fixture.start("Retry this work").await;
        let call = fixture.tool_part(&turn, &first, 1, "retained-call");
        fixture
            .apply(
                &conversation,
                vec![
                    Change::StartRun {
                        run_id: first.clone(),
                    },
                    Change::StartPart { part: call.clone() },
                    Change::FinishPart {
                        part_id: call.id,
                        state: PartState::Complete,
                        tail: "complete retained arguments".to_owned(),
                        provider_item: None,
                    },
                    Change::EndRun {
                        run_id: first.clone(),
                        state,
                        outcome: RunOutcome::default(),
                        task: None,
                    },
                ],
            )
            .await;
        let record = fixture
            .store
            .conversation_record(&conversation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.turns[0].view.run.as_ref().unwrap().state, state);
        let (second, _) = fixture.retry(&conversation, &turn).await;
        let superseded = fixture.text_part(&turn, &second, 1);
        fixture
            .apply(
                &conversation,
                vec![
                    Change::StartRun {
                        run_id: second.clone(),
                    },
                    Change::AddPart {
                        part: superseded,
                        text: "superseded reply".to_owned(),
                    },
                    Change::EndRun {
                        run_id: second,
                        state,
                        outcome: RunOutcome::default(),
                        task: None,
                    },
                ],
            )
            .await;
        let (current, _) = fixture.retry(&conversation, &turn).await;
        let record = fixture
            .store
            .conversation_record(&conversation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.turns[0].view.run.as_ref().unwrap().id, current);
        assert_eq!(record.turns[0].retained_runs[0].id, first);
        assert_eq!(record.turns[0].retained_runs[0].state, state);
        assert_eq!(record.turns[0].parts.len(), 2);
        let retained = &record.turns[0].parts[1];
        assert_eq!(retained.run_id.as_ref(), Some(&first));
        assert_eq!(retained.text, "complete retained arguments");
        assert!(
            !serde_json::to_string(&record)
                .unwrap()
                .contains("superseded reply")
        );
    }
}
