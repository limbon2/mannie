//! The run actor, branch by branch, against a fake model
//! (docs/features/chat/conversations.md, runs; stop, retry and recovery).

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use mannie_provider::testing::Step;
use mannie_provider::{
    Content, EventStream, ModelRequest, Provider, ProviderError, ProviderEvent, Role, Target,
};
use mannie_runtime::testing::{
    FakeModel, Harness, HarnessOptions, MemoryVault, failure, fast_config, open_store, reply,
    reply_with_usage, server_error, start_runtime,
};
use mannie_runtime::{RuntimeConfig, texts};
use mannie_types::chat::{
    ChatError, SendMessage, StartConversation, StartedConversation, StopOutcome, UpdateChatSettings,
};
use mannie_types::clock::UtcMillis;
use mannie_types::provider::{
    ApiInterface, CreateProviderProfile, CredentialChange, CredentialStoreProblem, ProfileDraft,
    ReasoningEffort, ServerKind,
};
use mannie_types::run::{FinishReason, RunActivity, RunFailure, RunState};
use mannie_types::secret::Secret;
use mannie_types::transcript::{ConversationEvent, PartContent, PartState};

async fn start(harness: &Harness, text: &str) -> StartedConversation {
    harness
        .runtime
        .start_conversation(StartConversation {
            mentions: Vec::new(),
            images: Vec::new(),
            text: text.to_owned(),
            profile_id: None,
            project_id: None,
        })
        .await
        .unwrap()
}

fn text_part(text: &str) -> PartContent {
    PartContent::Text {
        text: text.to_owned(),
    }
}

fn failure_of(
    transcript: &mannie_runtime::testing::Transcript,
    position: u32,
) -> Option<RunFailure> {
    transcript
        .parts(position)
        .iter()
        .find_map(|part| match &part.content {
            PartContent::Error { failure } => Some(failure.clone()),
            _ => None,
        })
}

#[tokio::test]
async fn a_reply_streams_into_a_completed_turn() {
    let harness = Harness::start().await;
    harness.model.push_reply(vec![
        Step::Event(ProviderEvent::TextDelta("Hi ".to_owned())),
        Step::Delay(Duration::from_millis(15)),
        Step::Event(ProviderEvent::TextDelta("there".to_owned())),
        Step::Event(ProviderEvent::Finished(FinishReason::Stop)),
    ]);

    let started = start(&harness, "Hello").await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let run = transcript.run(&started.run_id).unwrap();
    assert_eq!(run.state, RunState::Completed);
    assert_eq!(
        run.outcome.as_ref().and_then(|outcome| outcome.finish),
        Some(FinishReason::Stop)
    );
    let parts = transcript.parts(0);
    assert_eq!(parts.len(), 2, "{parts:#?}");
    assert_eq!(
        parts[0].content,
        PartContent::UserText {
            text: "Hello".to_owned()
        }
    );
    assert_eq!(parts[1].content, text_part("Hi there"));
    assert_eq!(parts[1].state, PartState::Complete);
    let requests = harness.model.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].messages,
        [mannie_provider::Message::text(Role::User, "Hello")]
    );
    assert!(
        requests[0]
            .instructions
            .starts_with(texts::BASE_INSTRUCTIONS)
    );
    assert_eq!(
        harness.model.credentials(),
        [Some("sk-test-credential".to_owned())]
    );
    assert_eq!(
        transcript.content(),
        harness.snapshot(&started.conversation_id).await.content()
    );
}

#[tokio::test]
async fn the_owners_instructions_reach_the_system_prompt() {
    let harness = Harness::start().await;
    harness
        .runtime
        .update_chat_settings(UpdateChatSettings {
            instructions: "Answer in German.".to_owned(),
        })
        .await
        .unwrap();
    harness.model.push_reply(reply("Hallo"));

    let started = start(&harness, "Hello").await;
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let instructions = &harness.model.requests()[0].instructions;
    assert!(
        instructions.contains(texts::OWNER_INSTRUCTIONS_HEADING),
        "{instructions}"
    );
    assert!(
        instructions.ends_with("Answer in German."),
        "{instructions}"
    );
}

#[tokio::test]
async fn reasoning_items_go_back_only_to_the_binding_that_wrote_them() {
    let harness = Harness::with_options(HarnessOptions {
        interface: ApiInterface::Responses,
        ..HarnessOptions::default()
    })
    .await;
    let item = r#"{"type":"reasoning","encrypted_content":"opaque"}"#;
    harness.model.push_reply(vec![
        Step::Event(ProviderEvent::ReasoningDelta("Thinking.".to_owned())),
        Step::Event(ProviderEvent::ProviderItem(item.to_owned())),
        Step::Event(ProviderEvent::TextDelta("Answer".to_owned())),
        Step::Event(ProviderEvent::Finished(FinishReason::Stop)),
    ]);
    harness.model.push_reply(reply("Second"));
    harness.model.push_reply(reply("Third"));

    let first = start(&harness, "Question").await;
    let transcript = harness
        .wait_for_run(&first.conversation_id, &first.run_id)
        .await;
    let second = harness
        .runtime
        .send_message(SendMessage {
            mentions: Vec::new(),
            images: Vec::new(),
            conversation_id: first.conversation_id.clone(),
            text: "Again".to_owned(),
        })
        .await
        .unwrap();
    harness
        .wait_for_run(&first.conversation_id, &second.run_id)
        .await;
    let mut draft = draft(ApiInterface::Responses);
    draft.model = "another-model".to_owned();
    harness
        .runtime
        .update_profile(&harness.profile.id, draft, CredentialChange::Keep)
        .await
        .unwrap();
    let third = harness
        .runtime
        .send_message(SendMessage {
            mentions: Vec::new(),
            images: Vec::new(),
            conversation_id: first.conversation_id.clone(),
            text: "Once more".to_owned(),
        })
        .await
        .unwrap();
    harness
        .wait_for_run(&first.conversation_id, &third.run_id)
        .await;

    let parts = transcript.parts(0);
    assert_eq!(
        parts[1].content,
        PartContent::Reasoning {
            text: "Thinking.".to_owned()
        }
    );
    assert_eq!(parts[2].content, text_part("Answer"));
    let requests = harness.model.requests();
    assert_eq!(
        requests[1].messages[1].content,
        [
            Content::ProviderItem(item.to_owned()),
            Content::Text("Answer".to_owned())
        ]
    );
    assert_eq!(
        requests[2].messages[1].content,
        [Content::Text("Answer".to_owned())],
        "the profile's configuration changed, so the item stays behind"
    );
}

fn draft(interface: ApiInterface) -> ProfileDraft {
    ProfileDraft {
        name: "Test".to_owned(),
        interface,
        base_url: "http://127.0.0.1:9/v1".to_owned(),
        model: "test-model".to_owned(),
        context_window: 128_000,
        output_limit: 16_384,
        reasoning_effort: ReasoningEffort::ServerDefault,
        use_tools: false,
    }
}

#[tokio::test]
async fn an_empty_step_gets_continuation_notes_up_to_the_limit() {
    let harness = Harness::start().await;
    let empty = vec![Step::Event(ProviderEvent::Finished(FinishReason::Stop))];
    harness.model.push_reply(empty.clone());
    harness.model.push_reply(empty.clone());
    harness.model.push_reply(reply("Finally"));
    harness.model.push_reply(empty.clone());
    harness.model.push_reply(empty.clone());
    harness.model.push_reply(empty);

    let first = start(&harness, "Question").await;
    let transcript = harness
        .wait_for_run(&first.conversation_id, &first.run_id)
        .await;
    let second = harness
        .runtime
        .send_message(SendMessage {
            mentions: Vec::new(),
            images: Vec::new(),
            conversation_id: first.conversation_id.clone(),
            text: "And?".to_owned(),
        })
        .await
        .unwrap();
    let after_second = harness
        .wait_for_run(&first.conversation_id, &second.run_id)
        .await;

    let requests = harness.model.requests();
    assert_eq!(requests.len(), 6, "three calls per turn");
    let note = mannie_provider::Message::text(Role::User, texts::CONTINUATION_NOTE);
    assert_eq!(requests[1].messages.last(), Some(&note));
    assert_eq!(
        requests[2]
            .messages
            .iter()
            .filter(|message| **message == note)
            .count(),
        2
    );
    assert_eq!(transcript.reply_text(0), "Finally");
    assert_eq!(
        transcript.parts(0).len(),
        2,
        "continuation notes are not shown"
    );
    assert_eq!(
        after_second.run_state(&second.run_id),
        Some(RunState::Completed)
    );
    assert_eq!(after_second.reply_text(1), "", "the limit ends the reply");
}

#[tokio::test]
async fn transient_failures_are_retried_and_the_failed_text_is_discarded() {
    let harness = Harness::start().await;
    harness.model.push_reply(vec![
        Step::Event(ProviderEvent::TextDelta("Partial".to_owned())),
        Step::Delay(Duration::from_millis(30)),
        Step::Error(server_error()),
    ]);
    harness.model.push_reply(reply("Complete answer"));

    let started = start(&harness, "Question").await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    assert_eq!(
        transcript.run_state(&started.run_id),
        Some(RunState::Completed)
    );
    assert_eq!(transcript.reply_text(0), "Complete answer");
    assert_eq!(transcript.parts(0).len(), 2, "{:#?}", transcript.parts(0));
    let retrying = transcript.events.iter().find_map(|event| match event {
        ConversationEvent::RunUpdated { run } => match &run.activity {
            Some(RunActivity::Retrying {
                attempt,
                max_attempts,
                cause,
                ..
            }) => Some((*attempt, *max_attempts, cause.clone())),
            _ => None,
        },
        _ => None,
    });
    assert_eq!(
        retrying,
        Some((2, 6, RunFailure::ServerError { status: 503 }))
    );
    assert!(
        transcript
            .events
            .iter()
            .any(|event| matches!(event, ConversationEvent::AttemptDiscarded { part_ids, .. } if part_ids.len() == 1)),
        "the partial text was discarded"
    );
}

#[tokio::test]
async fn retries_end_after_the_last_attempt_with_the_cause() {
    let harness = Harness::start().await;
    for _ in 0..6 {
        harness.model.push_reply(failure(server_error()));
    }

    let started = start(&harness, "Question").await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    assert_eq!(
        transcript.run_state(&started.run_id),
        Some(RunState::Failed)
    );
    assert_eq!(
        failure_of(&transcript, 0),
        Some(RunFailure::ServerError { status: 503 })
    );
    assert_eq!(harness.model.requests().len(), 6);
}

#[tokio::test]
async fn a_long_retry_after_ends_the_reply_with_the_time_of_the_next_attempt() {
    let harness = Harness::start().await;
    let retry_at = UtcMillis::from_millis(1_791_893_400_000);
    harness.model.push_reply(failure(ProviderError::Transient {
        cause: RunFailure::RateLimited {
            retry_at: Some(retry_at),
        },
        retry_after: Some(Duration::from_mins(11)),
    }));

    let started = start(&harness, "Question").await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    assert_eq!(
        failure_of(&transcript, 0),
        Some(RunFailure::RateLimited {
            retry_at: Some(retry_at)
        })
    );
    assert_eq!(harness.model.requests().len(), 1);
}

#[tokio::test]
async fn a_permanent_failure_keeps_the_partial_text_as_incomplete() {
    let harness = Harness::start().await;
    harness.model.push_reply(vec![
        Step::Event(ProviderEvent::TextDelta("Half".to_owned())),
        Step::Delay(Duration::from_millis(20)),
        Step::Error(ProviderError::Permanent(RunFailure::ContentFiltered)),
    ]);

    let started = start(&harness, "Question").await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let parts = transcript.parts(0);
    assert_eq!(parts[1].content, text_part("Half"));
    assert_eq!(parts[1].state, PartState::Incomplete);
    assert_eq!(
        failure_of(&transcript, 0),
        Some(RunFailure::ContentFiltered)
    );
    let run = transcript.run(&started.run_id).unwrap();
    assert_eq!(run.state, RunState::Failed);
    assert_eq!(
        run.outcome
            .as_ref()
            .and_then(|outcome| outcome.failure.clone()),
        Some(RunFailure::ContentFiltered)
    );
}

#[tokio::test]
async fn stop_keeps_the_streamed_text_and_ends_the_run_within_a_second() {
    let harness = Harness::start().await;
    harness.model.push_reply(vec![
        Step::Event(ProviderEvent::TextDelta("Some text".to_owned())),
        Step::Hang,
    ]);

    let started = start(&harness, "Question").await;
    harness
        .wait_until(&started.conversation_id, |transcript| {
            transcript.reply_text(0) == "Some text"
        })
        .await;
    let stopping = Instant::now();
    let outcome = harness.runtime.stop_reply(&started.conversation_id).await;
    let took = stopping.elapsed();
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let again = harness.runtime.stop_reply(&started.conversation_id).await;

    assert_eq!(outcome, StopOutcome::Stopped);
    assert!(took < Duration::from_secs(1), "stopping took {took:?}");
    assert_eq!(
        transcript.run_state(&started.run_id),
        Some(RunState::Stopped)
    );
    let parts = transcript.parts(0);
    assert_eq!(parts[1].content, text_part("Some text"));
    assert_eq!(parts[1].state, PartState::Stopped);
    assert_eq!(again, StopOutcome::NotRunning);
}

#[tokio::test]
async fn stop_ends_a_run_that_waits_to_retry() {
    let harness = Harness::start().await;
    harness.model.push_reply(failure(ProviderError::Transient {
        cause: RunFailure::RateLimited { retry_at: None },
        retry_after: Some(Duration::from_secs(60)),
    }));

    let started = start(&harness, "Question").await;
    harness
        .wait_until(&started.conversation_id, |transcript| {
            transcript
                .run(&started.run_id)
                .is_some_and(|run| matches!(run.activity, Some(RunActivity::Retrying { .. })))
        })
        .await;
    let outcome = harness.runtime.stop_reply(&started.conversation_id).await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    assert_eq!(outcome, StopOutcome::Stopped);
    assert_eq!(
        transcript.run_state(&started.run_id),
        Some(RunState::Stopped)
    );
    assert_eq!(harness.model.requests().len(), 1);
}

#[tokio::test]
async fn retry_replaces_a_failed_reply_and_only_the_latest_turn_can_retry() {
    let harness = Harness::start().await;
    harness.model.push_reply(failure(ProviderError::Permanent(
        RunFailure::InvalidCredential,
    )));
    harness.model.push_reply(reply("Second try"));

    let started = start(&harness, "Question").await;
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let retried = harness
        .runtime
        .retry_reply(&started.conversation_id)
        .await
        .unwrap();
    let transcript = harness
        .wait_for_run(&started.conversation_id, &retried.run_id)
        .await;
    let completed_again = harness.runtime.retry_reply(&started.conversation_id).await;

    assert_eq!(retried.turn_id, started.turn_id);
    assert_eq!(transcript.reply_text(0), "Second try");
    assert_eq!(
        failure_of(&transcript, 0),
        None,
        "the failed attempt is replaced"
    );
    assert_eq!(
        harness.model.requests()[1].messages,
        [mannie_provider::Message::text(Role::User, "Question")]
    );
    assert_eq!(completed_again.unwrap_err(), ChatError::NothingToRetry);
}

#[tokio::test]
async fn a_conversation_has_one_active_run() {
    let harness = Harness::start().await;
    harness.model.push_reply(vec![Step::Hang]);

    let started = start(&harness, "Question").await;
    let second = harness
        .runtime
        .send_message(SendMessage {
            mentions: Vec::new(),
            images: Vec::new(),
            conversation_id: started.conversation_id.clone(),
            text: "Another".to_owned(),
        })
        .await;
    harness.runtime.stop_reply(&started.conversation_id).await;

    assert_eq!(second.unwrap_err(), ChatError::RunActive);
}

#[tokio::test]
async fn a_context_overflow_compacts_once_and_a_second_one_fails() {
    let harness = Harness::start().await;
    harness.model.push_reply(reply("One"));
    harness.model.push_reply(reply("Two"));
    harness
        .model
        .push_reply(failure(ProviderError::ContextOverflow));
    harness.model.push_reply(reply("Three"));
    harness
        .model
        .push_reply(failure(ProviderError::ContextOverflow));
    harness
        .model
        .push_reply(failure(ProviderError::ContextOverflow));

    let started = start(&harness, "First").await;
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let mut runs = Vec::new();
    for text in ["Second", "Third", "Fourth"] {
        let sent = harness
            .runtime
            .send_message(SendMessage {
                mentions: Vec::new(),
                images: Vec::new(),
                conversation_id: started.conversation_id.clone(),
                text: text.to_owned(),
            })
            .await
            .unwrap();
        harness
            .wait_for_run(&started.conversation_id, &sent.run_id)
            .await;
        runs.push(sent.run_id);
    }
    let transcript = harness.snapshot(&started.conversation_id).await;

    assert_eq!(transcript.run_state(&runs[1]), Some(RunState::Completed));
    assert_eq!(transcript.reply_text(2), "Three");
    let summaries = harness.model.summary_requests();
    assert_eq!(summaries.len(), 2, "one compaction per overflow");
    let retried = &harness.model.reply_requests()[3];
    assert!(
        retried.instructions.contains(texts::CHECKPOINT_HEADING),
        "{}",
        retried.instructions
    );
    assert!(retried.instructions.ends_with("Summary 1."));
    assert_eq!(
        retried.messages[0],
        mannie_provider::Message::text(Role::User, "Third")
    );
    assert_eq!(transcript.run_state(&runs[2]), Some(RunState::Failed));
    assert_eq!(
        failure_of(&transcript, 3),
        Some(RunFailure::ContextOverflow)
    );
    assert_eq!(
        transcript.parts(0).len(),
        2,
        "the stored transcript keeps every turn"
    );
}

#[tokio::test]
async fn an_overflow_with_nothing_to_compact_fails_as_too_large() {
    let harness = Harness::start().await;
    harness
        .model
        .push_reply(failure(ProviderError::ContextOverflow));

    let started = start(&harness, "A very long first message").await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    assert_eq!(
        failure_of(&transcript, 0),
        Some(RunFailure::RequestTooLarge)
    );
    assert_eq!(harness.model.summary_requests(), Vec::new());
}

#[tokio::test]
async fn shutdown_interrupts_active_runs_and_refuses_new_ones() {
    let harness = Harness::start().await;
    harness.model.push_reply(vec![
        Step::Event(ProviderEvent::TextDelta("Streaming".to_owned())),
        Step::Hang,
    ]);

    let started = start(&harness, "Question").await;
    harness
        .wait_until(&started.conversation_id, |transcript| {
            transcript.reply_text(0) == "Streaming"
        })
        .await;
    harness.runtime.shutdown().await;
    let refused = harness
        .runtime
        .send_message(SendMessage {
            mentions: Vec::new(),
            images: Vec::new(),
            conversation_id: started.conversation_id.clone(),
            text: "More".to_owned(),
        })
        .await;
    let transcript = harness.snapshot(&started.conversation_id).await;

    assert_eq!(refused.unwrap_err(), ChatError::ShuttingDown);
    assert_eq!(
        transcript.run_state(&started.run_id),
        Some(RunState::Interrupted)
    );
    let parts = transcript.parts(0);
    assert_eq!(parts[1].content, text_part("Streaming"));
    assert_eq!(parts[1].state, PartState::Interrupted);
}

#[tokio::test]
async fn runs_beyond_the_slot_limit_wait_their_turn() {
    let harness = Harness::with_options(HarnessOptions {
        config: RuntimeConfig {
            max_concurrent_calls: 1,
            ..fast_config()
        },
        ..HarnessOptions::default()
    })
    .await;
    harness.model.push_reply(vec![Step::Hang]);
    harness.model.push_reply(reply("Second"));

    let first = start(&harness, "First").await;
    harness
        .wait_until(&first.conversation_id, |transcript| {
            transcript
                .run(&first.run_id)
                .is_some_and(|run| run.state == RunState::Running)
        })
        .await;
    let second = start(&harness, "Second").await;
    let waiting = harness
        .wait_until(&second.conversation_id, |transcript| {
            transcript
                .run(&second.run_id)
                .is_some_and(|run| run.activity == Some(RunActivity::WaitingForSlot))
        })
        .await;
    harness.runtime.stop_reply(&first.conversation_id).await;
    let transcript = harness
        .wait_for_run(&second.conversation_id, &second.run_id)
        .await;

    assert_eq!(waiting.run_state(&second.run_id), Some(RunState::Queued));
    assert_eq!(
        transcript.run_state(&second.run_id),
        Some(RunState::Completed)
    );
}

#[tokio::test]
async fn a_credential_that_cannot_be_read_fails_the_run_with_the_cause() {
    let harness = Harness::start().await;
    let reference = harness.vault.references().remove(0);

    harness
        .vault
        .fail_reads(Some(CredentialStoreProblem::AccessDenied));
    let locked = start(&harness, "First").await;
    let locked_transcript = harness
        .wait_for_run(&locked.conversation_id, &locked.run_id)
        .await;
    harness.vault.fail_reads(None);
    harness.vault.remove(&reference);
    let missing = start(&harness, "Second").await;
    let missing_transcript = harness
        .wait_for_run(&missing.conversation_id, &missing.run_id)
        .await;

    assert_eq!(
        failure_of(&locked_transcript, 0),
        Some(RunFailure::CredentialStoreUnavailable)
    );
    assert_eq!(
        failure_of(&missing_transcript, 0),
        Some(RunFailure::CredentialMissing)
    );
    assert_eq!(harness.model.requests(), Vec::new());
}

#[tokio::test]
async fn reported_usage_feeds_the_context_meter() {
    let harness = Harness::start().await;
    harness.model.push_reply(reply_with_usage("Hi", 1_000, 50));
    harness.model.push_reply(reply("Hi again"));

    let first = start(&harness, "Hello").await;
    let reported = harness
        .wait_for_run(&first.conversation_id, &first.run_id)
        .await;
    let second = harness
        .runtime
        .send_message(SendMessage {
            mentions: Vec::new(),
            images: Vec::new(),
            conversation_id: first.conversation_id.clone(),
            text: "Hello again".to_owned(),
        })
        .await
        .unwrap();
    let estimated = harness
        .wait_for_run(&first.conversation_id, &second.run_id)
        .await;

    let last_usage = |transcript: &mannie_runtime::testing::Transcript| {
        transcript
            .events
            .iter()
            .rev()
            .find_map(|event| match event {
                ConversationEvent::ContextUsageChanged { usage } => Some(*usage),
                _ => None,
            })
    };
    let reported = last_usage(&reported).unwrap();
    assert_eq!((reported.used_tokens, reported.estimated), (1_050, false));
    let estimated = last_usage(&estimated).unwrap();
    assert!(estimated.estimated);
    assert!(estimated.used_tokens > 1_000, "{estimated:?}");
    let run = harness.snapshot(&first.conversation_id).await;
    let usage = run
        .run(&first.run_id)
        .and_then(|run| run.outcome.as_ref())
        .and_then(|outcome| outcome.usage);
    assert_eq!(
        usage.map(|usage| (usage.input, usage.output)),
        Some((1_000, 50))
    );
}

#[tokio::test]
async fn deleting_a_conversation_stops_its_reply() {
    let harness = Harness::start().await;
    harness.model.push_reply(vec![Step::Hang]);

    let started = start(&harness, "Question").await;
    harness
        .runtime
        .delete_conversation(&started.conversation_id)
        .await
        .unwrap();
    let deleted_again = harness
        .runtime
        .delete_conversation(&started.conversation_id)
        .await;

    assert_eq!(deleted_again.unwrap_err(), ChatError::ConversationNotFound);
    assert_eq!(harness.runtime.active_run_count(), 0);
    assert_eq!(
        harness.store.list_conversations().await.unwrap(),
        Vec::new()
    );
}

/// A provider whose calls panic, like a bug in an adapter.
#[derive(Debug)]
struct PanickingProvider;

impl Provider for PanickingProvider {
    fn interface(&self) -> ApiInterface {
        ApiInterface::ChatCompletions
    }

    fn stream(&self, _target: &Target, _request: ModelRequest) -> EventStream {
        Box::pin(futures_util::stream::once(async {
            panic!("the adapter has a bug");
        }))
    }
}

#[tokio::test]
async fn a_run_that_panics_fails_as_internal_and_frees_the_conversation() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(&directory.path().join("mannie.sqlite"));
    let runtime = start_runtime(
        store.clone(),
        Arc::new(PanickingProvider),
        Arc::new(MemoryVault::new()),
        fast_config(),
    )
    .await;
    runtime
        .create_profile(CreateProviderProfile {
            draft: draft(ApiInterface::ChatCompletions),
            credential: Some(Secret::new("sk-test".to_owned())),
        })
        .await
        .unwrap();

    let started = runtime
        .start_conversation(StartConversation {
            mentions: Vec::new(),
            images: Vec::new(),
            text: "Question".to_owned(),
            profile_id: None,
            project_id: None,
        })
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while runtime.active_run_count() > 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let snapshot = store
        .snapshot(&started.conversation_id, 10, 100)
        .await
        .unwrap()
        .unwrap();

    let run = snapshot.turns[0].run.as_ref().unwrap();
    assert_eq!(run.state, RunState::Failed);
    assert_eq!(
        run.outcome
            .as_ref()
            .and_then(|outcome| outcome.failure.clone()),
        Some(RunFailure::Internal)
    );
    assert_eq!(runtime.active_run_count(), 0);
}

#[tokio::test]
async fn a_new_conversation_needs_a_profile() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(&directory.path().join("mannie.sqlite"));
    let runtime = start_runtime(
        store,
        Arc::new(FakeModel::new(ApiInterface::ChatCompletions)),
        Arc::new(MemoryVault::new()),
        fast_config(),
    )
    .await;

    let refused = runtime
        .start_conversation(StartConversation {
            mentions: Vec::new(),
            images: Vec::new(),
            text: "Question".to_owned(),
            profile_id: None,
            project_id: None,
        })
        .await;
    let empty = runtime
        .start_conversation(StartConversation {
            mentions: Vec::new(),
            images: Vec::new(),
            text: " \n".to_owned(),
            profile_id: None,
            project_id: None,
        })
        .await;

    assert_eq!(refused.unwrap_err(), ChatError::NoProfile);
    assert_eq!(empty.unwrap_err(), ChatError::MessageEmpty);
}

/// Runs two turns whose replies report the same request size, although the
/// second request carries a long message, and returns whether each run says
/// that the server cut its request.
async fn reported_cuts(harness: &Harness) -> (bool, bool) {
    let prompt_cut = |transcript: &mannie_runtime::testing::Transcript, run| {
        transcript
            .run(run)
            .and_then(|run| run.outcome.as_ref())
            .map(|outcome| outcome.prompt_cut)
            .unwrap()
    };
    harness
        .model
        .push_reply(reply_with_usage("First answer", 2_000, 50));
    let first = start(harness, "First question").await;
    let transcript = harness
        .wait_for_run(&first.conversation_id, &first.run_id)
        .await;
    let first_cut = prompt_cut(&transcript, &first.run_id);

    harness
        .model
        .push_reply(reply_with_usage("Second answer", 2_000, 50));
    let second = harness
        .runtime
        .send_message(SendMessage {
            mentions: Vec::new(),
            images: Vec::new(),
            conversation_id: first.conversation_id.clone(),
            text: "A long message that the server cannot fit. ".repeat(100),
        })
        .await
        .unwrap();
    let transcript = harness
        .wait_for_run(&first.conversation_id, &second.run_id)
        .await;
    (first_cut, prompt_cut(&transcript, &second.run_id))
}

#[tokio::test]
async fn a_server_that_cuts_requests_silently_is_caught_by_its_reported_size() {
    let harness = Harness::start().await;
    // Ollama cuts a request beyond the model's `num_ctx` instead of failing.
    harness
        .store
        .record_profile_server(&harness.profile.id, ServerKind::Ollama)
        .await
        .unwrap();

    let (first, second) = reported_cuts(&harness).await;

    assert!(
        !first,
        "the first call has no earlier report to compare with"
    );
    assert!(second, "the request grew, but the reported size did not");
}

#[tokio::test]
async fn a_server_that_fails_on_overflow_is_not_suspected_of_cutting() {
    let harness = Harness::start().await;

    let (first, second) = reported_cuts(&harness).await;

    assert!(!first);
    assert!(!second, "only servers known to cut requests are checked");
}
