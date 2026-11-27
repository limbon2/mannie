//! Compaction keeps a conversation usable at the context limit of every known
//! model and of small local models, and leaves the stored transcript as it
//! was (docs/features/chat/conversations.md, context budget and compaction).
//!
//! The fake model refuses requests beyond its context window, as a server
//! would, and writes summaries as long as their output limit allows. Each turn
//! adds a fifth of the budget, so the conversation passes the budget several
//! times over.

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use std::fs;

use mannie_provider::testing::Step;
use mannie_provider::{Content, ProviderEvent, known_models};
use mannie_runtime::testing::{Harness, HarnessOptions, estimate, reply, tool_calls};
use mannie_runtime::texts;
use mannie_types::agents::{AgentDraft, AgentKind};
use mannie_types::chat::{
    ChatError, CheckpointRef, MAX_MESSAGE_BYTES, SendMessage, StartConversation,
};
use mannie_types::provider::{
    ApiInterface, CreateProviderProfile, ProfileAccess, ProfileDraft, ReasoningEffort,
    context_budget,
};
use mannie_types::run::{FinishReason, RunFailure, RunState, Usage};
use mannie_types::secret::Secret;
use mannie_types::transcript::PartContent;

const TURNS: usize = 9;

/// The limits to check: every distinct pair of the known models, and small
/// windows of local models.
fn limits() -> Vec<(u32, u32)> {
    let mut limits: Vec<(u32, u32)> = known_models()
        .iter()
        .map(|model| (model.limits.context_window, model.limits.output_limit))
        .collect();
    limits.extend([
        (4_096, 1_024),
        (8_192, 2_048),
        (32_768, 4_096),
        (131_072, 8_192),
    ]);
    limits.sort_unstable();
    limits.dedup();
    limits
}

/// Text of `bytes` bytes that starts with a marker of its own.
fn filler(marker: &str, bytes: usize) -> String {
    let mut text = format!("{marker} ");
    text.push_str(&"word ".repeat(bytes.saturating_sub(text.len()) / 5));
    text
}

/// Multi-line text of roughly `bytes` bytes so `read_file` returns it without
/// clipping single long lines.
fn multiline_filler(marker: &str, bytes: usize) -> String {
    let mut text = format!("{marker}\n");
    let line = "word word word word word word word word word word word word word word word\n";
    while text.len() < bytes {
        text.push_str(line);
    }
    text
}

fn mentions(request: &mannie_provider::ModelRequest, marker: &str) -> bool {
    request.instructions.contains(marker)
        || request.messages.iter().any(|message| {
            message.content.iter().any(|content| match content {
                Content::Text(text)
                | Content::ProviderItem(text)
                | Content::ToolResult { output: text, .. } => text.contains(marker),
                Content::ToolCall { arguments, .. } => arguments.contains(marker),
                Content::Image(_) => false,
            })
        })
}

async fn check(window: u32, output: u32) {
    let harness = Harness::with_options(HarnessOptions {
        context_window: window,
        output_limit: output,
        enforce_context_window: true,
        full_summaries: true,
        ..HarnessOptions::default()
    })
    .await;
    let budget = context_budget(window, output);
    // Two tokens for every five bytes: a part of budget/4 bytes is a tenth of
    // the budget, and a turn of two parts a fifth.
    let part_bytes = (usize::try_from(budget).unwrap() / 4).min(MAX_MESSAGE_BYTES);
    let messages: Vec<String> = (0..TURNS)
        .map(|turn| filler(&format!("[message {turn}]"), part_bytes))
        .collect();
    let replies: Vec<String> = (0..TURNS)
        .map(|turn| filler(&format!("[reply {turn}]"), part_bytes))
        .collect();
    for text in &replies {
        harness.model.push_reply(reply(text));
    }

    let started = harness
        .runtime
        .start_conversation(StartConversation::new(messages[0].clone()))
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    let mut runs = vec![started.run_id];
    harness.wait_for_run(&conversation, &runs[0]).await;
    for text in &messages[1..] {
        let sent = harness
            .runtime
            .send_message(SendMessage::new(conversation.clone(), text))
            .await
            .unwrap();
        harness.wait_for_run(&conversation, &sent.run_id).await;
        runs.push(sent.run_id);
    }

    let label = format!("window {window}, output {output}");
    let transcript = harness.snapshot(&conversation).await;
    for (position, run) in runs.iter().enumerate() {
        let position = u32::try_from(position).unwrap();
        assert_eq!(
            transcript.run_state(run),
            Some(RunState::Completed),
            "{label}: turn {position}"
        );
        let parts = transcript.parts(position);
        assert_eq!(
            parts[0].content,
            PartContent::UserText {
                text: messages[position as usize].clone()
            },
            "{label}: the owner's message {position} is stored as it was"
        );
        assert_eq!(
            transcript.reply_text(position),
            replies[position as usize],
            "{label}: reply {position}"
        );
    }
    let checkpoints = harness.store.checkpoints(&conversation).await.unwrap();
    assert!(
        !checkpoints.is_empty(),
        "{label}: the conversation was compacted"
    );
    for request in harness.model.requests() {
        assert!(
            estimate(&request) <= budget,
            "{label}: a request of {} tokens exceeds the budget of {budget}",
            estimate(&request)
        );
    }
    assert_eq!(
        harness.model.reply_requests().len(),
        TURNS,
        "{label}: no call overflowed"
    );
    let last = harness.model.reply_requests().pop().unwrap();
    assert!(
        last.instructions.contains(texts::CHECKPOINT_HEADING),
        "{label}"
    );
    assert!(
        !mentions(&last, "[message 0]"),
        "{label}: covered turns leave the context"
    );
    assert!(
        mentions(&last, &format!("[message {}]", TURNS - 1)),
        "{label}"
    );
}

#[tokio::test]
async fn compaction_keeps_conversations_usable_at_every_known_limit() {
    let limits = limits();
    assert!(limits.len() >= 8, "{limits:?}");
    for (window, output) in limits {
        check(window, output).await;
    }
}

#[tokio::test]
async fn the_owner_can_compact_by_hand_and_read_the_checkpoint() {
    let harness = Harness::start().await;
    for reply_text in ["One", "Two", "Three"] {
        harness.model.push_reply(reply(reply_text));
    }
    let started = harness
        .runtime
        .start_conversation(StartConversation::new("First"))
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    harness.wait_for_run(&conversation, &started.run_id).await;

    let too_early = harness.runtime.compact_conversation(&conversation).await;
    let second = harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "Second"))
        .await
        .unwrap();
    harness.wait_for_run(&conversation, &second.run_id).await;
    harness
        .runtime
        .compact_conversation(&conversation)
        .await
        .unwrap();
    let checkpoints = harness.store.checkpoints(&conversation).await.unwrap();
    let checkpoint = harness
        .runtime
        .checkpoint(CheckpointRef {
            conversation_id: conversation.clone(),
            checkpoint_id: checkpoints[0].id.clone(),
        })
        .await
        .unwrap();
    let third = harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "Third"))
        .await
        .unwrap();
    harness.wait_for_run(&conversation, &third.run_id).await;

    assert_eq!(too_early.unwrap_err(), ChatError::NothingToCompact);
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(
        checkpoints[0].covers_through_turn, 0,
        "the latest turn is never covered"
    );
    assert_eq!(checkpoint.summary, "Summary 1.");
    let summary_request = &harness.model.summary_requests()[0];
    let summarized = match &summary_request.messages[0].content[0] {
        Content::Text(text) => text.clone(),
        _ => String::new(),
    };
    assert!(summarized.contains("Owner: First"), "{summarized}");
    assert!(summarized.contains("Assistant: One"), "{summarized}");
    assert!(!summarized.contains("Second"), "{summarized}");
    let after = harness.model.reply_requests().pop().unwrap();
    assert!(
        after.instructions.ends_with("Summary 1."),
        "{}",
        after.instructions
    );
    assert_eq!(
        after.messages.first(),
        Some(&mannie_provider::Message::text(
            mannie_provider::Role::User,
            "Second"
        ))
    );
}

#[tokio::test]
async fn a_summary_cut_at_the_output_limit_is_not_kept() {
    let harness = Harness::start().await;
    for reply_text in ["One", "Two"] {
        harness.model.push_reply(reply(reply_text));
    }
    // A model that reasons first may spend the whole limit and stop in the
    // middle of its summary.
    harness.model.push_summary(vec![
        Step::Event(ProviderEvent::ReasoningDelta("Let me think.".to_owned())),
        Step::Event(ProviderEvent::TextDelta("Goal: the owner".to_owned())),
        Step::Event(ProviderEvent::Finished(FinishReason::OutputLimit)),
    ]);
    let started = harness
        .runtime
        .start_conversation(StartConversation::new("First"))
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    harness.wait_for_run(&conversation, &started.run_id).await;
    let second = harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "Second"))
        .await
        .unwrap();
    harness.wait_for_run(&conversation, &second.run_id).await;

    let cut = harness.runtime.compact_conversation(&conversation).await;
    let after_cut = harness.store.checkpoints(&conversation).await.unwrap();
    let whole = harness.runtime.compact_conversation(&conversation).await;
    let checkpoints = harness.store.checkpoints(&conversation).await.unwrap();

    assert_eq!(
        cut.unwrap_err(),
        ChatError::CompactionFailed {
            failure: RunFailure::SummaryCut
        }
    );
    assert!(after_cut.is_empty(), "{after_cut:?}");
    assert_eq!(whole, Ok(()));
    assert_eq!(checkpoints.len(), 1);
    let checkpoint = harness
        .runtime
        .checkpoint(CheckpointRef {
            conversation_id: conversation.clone(),
            checkpoint_id: checkpoints[0].id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(checkpoint.summary, "Summary 1.");
}

#[tokio::test]
async fn step_boundary_compaction_inside_a_single_turn_tool_loop() {
    let harness = Harness::with_options(HarnessOptions {
        context_window: 16_384,
        output_limit: 2_048,
        enforce_context_window: true,
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await;
    let workspace = tempfile::tempdir().unwrap();
    let project = harness
        .runtime
        .add_project(workspace.path().to_str().unwrap().to_owned())
        .await
        .unwrap();

    for index in 0..6 {
        let content = multiline_filler(&format!("[file {index}]"), 6_500);
        fs::write(workspace.path().join(format!("step_{index}.txt")), content).unwrap();
        let call_id = format!("call_{index}");
        let args = format!(r#"{{"path":"step_{index}.txt"}}"#);
        harness
            .model
            .push_reply(tool_calls(&[(&call_id, "read_file", &args)]));
    }
    harness.model.push_reply(reply("All six files checked."));

    let started = harness
        .runtime
        .start_conversation(
            StartConversation::new("Inspect all six files carefully.")
                .with_project(Some(project.id)),
        )
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    let transcript = harness.wait_for_run(&conversation, &started.run_id).await;

    assert_eq!(
        transcript.run_state(&started.run_id),
        Some(RunState::Completed),
        "outcome: {:?}",
        transcript
            .run(&started.run_id)
            .and_then(|run| run.outcome.clone())
    );
    let checkpoints = harness.store.checkpoints(&conversation).await.unwrap();
    assert!(
        !checkpoints.is_empty(),
        "a long single-turn tool loop compacts at a step boundary"
    );
    assert_eq!(checkpoints[0].covers_through_turn, 0);
    assert!(
        checkpoints[0].covers_through_part.is_some(),
        "single-turn compaction records a step boundary part"
    );

    let snapshot = harness
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.checkpoints.len(), checkpoints.len());
    let latest_cp = snapshot.checkpoints.last().unwrap();
    assert_eq!(latest_cp.trigger.as_deref(), Some("automatic"));
    assert_eq!(
        latest_cp.covers_through_part.as_ref(),
        checkpoints[0].covers_through_part.as_ref()
    );
    assert_eq!(transcript.reply_text(0), "All six files checked.");

    let last = harness.model.reply_requests().pop().unwrap();
    assert!(last.instructions.contains(texts::CHECKPOINT_HEADING));
    assert!(
        mentions(&last, "Inspect all six files carefully."),
        "the resumed turn anchor keeps the owner's active request"
    );
    assert!(
        mentions(&last, "[file 5]"),
        "the newest tool step stays verbatim after step-boundary compaction"
    );
    assert!(
        !mentions(&last, "[file 0]"),
        "the earliest covered step left the model's context"
    );
}

#[tokio::test]
async fn step_boundary_compaction_preserves_recent_steps_of_a_heavy_earlier_turn() {
    let harness = Harness::with_options(HarnessOptions {
        context_window: 32_768,
        output_limit: 2_048,
        enforce_context_window: true,
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await;
    let workspace = tempfile::tempdir().unwrap();
    let project = harness
        .runtime
        .add_project(workspace.path().to_str().unwrap().to_owned())
        .await
        .unwrap();

    // Budget = 29_081 (80% = 23_264, 60% = 17_448). Four medium reads stay
    // below 80% during turn zero, then turn zero's final reply pushes the start
    // of turn one over 80% while keeping turn one itself short.
    for index in 0..4 {
        let content = multiline_filler(&format!("[heavy step {index}]"), 6_000);
        fs::write(workspace.path().join(format!("heavy_{index}.txt")), content).unwrap();
        let call_id = format!("heavy_{index}");
        let args = format!(r#"{{"path":"heavy_{index}.txt"}}"#);
        harness
            .model
            .push_reply(tool_calls(&[(&call_id, "read_file", &args)]));
    }
    let conclusion = filler("Final conclusion of turn zero.", 14_000);
    harness.model.push_reply(reply(&conclusion));
    harness.model.push_reply(reply("Follow-up answered."));

    let started = harness
        .runtime
        .start_conversation(
            StartConversation::new("Investigate the four files.").with_project(Some(project.id)),
        )
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    harness.wait_for_run(&conversation, &started.run_id).await;
    assert!(
        harness
            .store
            .checkpoints(&conversation)
            .await
            .unwrap()
            .is_empty(),
        "turn zero stays below the automatic threshold"
    );

    let follow_up = harness
        .runtime
        .send_message(SendMessage::new(
            conversation.clone(),
            "Continue from your final conclusion.",
        ))
        .await
        .unwrap();
    let transcript = harness.wait_for_run(&conversation, &follow_up.run_id).await;

    assert_eq!(
        transcript.run_state(&follow_up.run_id),
        Some(RunState::Completed)
    );
    let checkpoints = harness.store.checkpoints(&conversation).await.unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0].covers_through_turn, 0);
    assert!(
        checkpoints[0].covers_through_part.is_some(),
        "cuts inside the heavy earlier turn instead of wiping all of it"
    );

    let last = harness.model.reply_requests().pop().unwrap();
    assert!(
        mentions(&last, "Final conclusion of turn zero."),
        "turn zero's newest step stays verbatim"
    );
    assert!(
        !mentions(&last, "[heavy step 0]"),
        "turn zero's earliest step left the context"
    );
}

#[tokio::test]
async fn proactive_automatic_compaction_failure_fails_soft_when_request_fits() {
    let harness = Harness::with_options(HarnessOptions {
        context_window: 8_192,
        output_limit: 1_024,
        ..HarnessOptions::default()
    })
    .await;
    // Budget = 8_192 - 1_024 - 410 = 6_758; 80% threshold = 5_406.
    // Three turns of ~1_850 tokens each cross 80% on turn 3 while still
    // fitting comfortably under 6_758.
    let part = filler("[turn data]", 2_200);
    for _ in 0..4 {
        harness.model.push_reply(reply(&part));
    }
    harness.model.push_summary(vec![
        Step::Event(ProviderEvent::TextDelta("Incomplete".to_owned())),
        Step::Event(ProviderEvent::Finished(FinishReason::OutputLimit)),
    ]);

    let started = harness
        .runtime
        .start_conversation(StartConversation::new(format!("[turn 0] {part}")))
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    harness.wait_for_run(&conversation, &started.run_id).await;

    for index in 1..=3 {
        let sent = harness
            .runtime
            .send_message(SendMessage::new(
                conversation.clone(),
                format!("[turn {index}] {part}"),
            ))
            .await
            .unwrap();
        let transcript = harness.wait_for_run(&conversation, &sent.run_id).await;
        assert_eq!(
            transcript.run_state(&sent.run_id),
            Some(RunState::Completed),
            "turn {index} succeeds even when proactive compaction fails"
        );
    }

    assert!(
        !harness.model.summary_requests().is_empty(),
        "proactive compaction was attempted"
    );
    assert!(
        harness
            .store
            .checkpoints(&conversation)
            .await
            .unwrap()
            .is_empty(),
        "a failed summary writes no checkpoint"
    );
    let last = harness.model.reply_requests().pop().unwrap();
    assert!(mentions(&last, "[turn 0]"));
}

#[tokio::test]
async fn encrypted_provider_items_do_not_trigger_premature_compaction() {
    let harness = Harness::with_options(HarnessOptions {
        interface: ApiInterface::Responses,
        context_window: 16_384,
        output_limit: 2_048,
        ..HarnessOptions::default()
    })
    .await;
    let encrypted = "A".repeat(60_000);
    let item = format!(
        r#"{{"type":"reasoning","summary":[{{"type":"summary_text","text":"Checked state."}}],"encrypted_content":"{encrypted}"}}"#
    );
    harness.model.push_reply(vec![
        Step::Event(ProviderEvent::ProviderItem(item)),
        Step::Event(ProviderEvent::TextDelta("Done.".to_owned())),
        Step::Event(ProviderEvent::Usage(Usage {
            input: 600,
            output: 120,
            cached_input: None,
            reasoning: Some(80),
        })),
        Step::Event(ProviderEvent::Finished(FinishReason::Stop)),
    ]);
    harness.model.push_reply(reply("Continued."));

    let started = harness
        .runtime
        .start_conversation(StartConversation::new("First step."))
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    harness.wait_for_run(&conversation, &started.run_id).await;

    let second = harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "Second step."))
        .await
        .unwrap();
    let transcript = harness.wait_for_run(&conversation, &second.run_id).await;

    assert_eq!(
        transcript.run_state(&second.run_id),
        Some(RunState::Completed)
    );
    assert!(
        harness.model.summary_requests().is_empty(),
        "encrypted reasoning ciphertext must not inflate token estimates into compaction"
    );
    assert_eq!(
        harness
            .store
            .checkpoints(&conversation)
            .await
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn checkpoint_tokens_after_measures_actual_summary_size() {
    let harness = Harness::start().await;
    harness.model.push_reply(reply("First reply."));
    harness.model.push_reply(reply("Second reply."));

    let started = harness
        .runtime
        .start_conversation(StartConversation::new("First question."))
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    harness.wait_for_run(&conversation, &started.run_id).await;

    let second = harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "Second question."))
        .await
        .unwrap();
    harness.wait_for_run(&conversation, &second.run_id).await;

    harness
        .runtime
        .compact_conversation(&conversation)
        .await
        .unwrap();

    let checkpoints = harness.store.checkpoints(&conversation).await.unwrap();
    assert_eq!(checkpoints.len(), 1);
    let checkpoint = harness
        .runtime
        .checkpoint(CheckpointRef {
            conversation_id: conversation.clone(),
            checkpoint_id: checkpoints[0].id.clone(),
        })
        .await
        .unwrap();
    assert!(
        checkpoint.view.tokens_after < 500,
        "tokens_after should reflect the short summary rather than worst-case allowance, got {}",
        checkpoint.view.tokens_after
    );
}

#[tokio::test]
async fn summary_requests_use_lowest_supported_reasoning_effort() {
    let harness = Harness::with_options(HarnessOptions {
        interface: ApiInterface::Responses,
        ..HarnessOptions::default()
    })
    .await;
    let created = harness
        .runtime
        .create_profile(CreateProviderProfile {
            draft: ProfileDraft {
                name: "Reasoning model".to_owned(),
                access: ProfileAccess::Endpoint,
                interface: ApiInterface::Responses,
                base_url: "https://api.openai.com/v1".to_owned(),
                model: "o3".to_owned(),
                context_window: 200_000,
                output_limit: 16_384,
                reasoning_effort: ReasoningEffort::High,
                use_tools: false,
            },
            credential: Some(Secret::new("sk-reasoning-test".to_owned())),
        })
        .await
        .unwrap();
    harness
        .runtime
        .set_default_profile(&created.id)
        .await
        .unwrap();

    harness.model.push_reply(reply("Reply one."));
    harness.model.push_reply(reply("Reply two."));

    let started = harness
        .runtime
        .start_conversation(StartConversation::new("Question one."))
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    harness.wait_for_run(&conversation, &started.run_id).await;

    let second = harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "Question two."))
        .await
        .unwrap();
    harness.wait_for_run(&conversation, &second.run_id).await;

    harness
        .runtime
        .compact_conversation(&conversation)
        .await
        .unwrap();

    let replies = harness.model.reply_requests();
    assert_eq!(replies[0].reasoning_effort, ReasoningEffort::High);
    let summaries = harness.model.summary_requests();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].reasoning_effort, ReasoningEffort::Low);
}

#[tokio::test]
async fn manual_compaction_uses_conversation_agent_and_compacts_multi_step_single_turn() {
    let harness = Harness::with_options(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await;
    let workspace = tempfile::tempdir().unwrap();
    let project = harness
        .runtime
        .add_project(workspace.path().to_str().unwrap().to_owned())
        .await
        .unwrap();
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(workspace.path().join(name), format!("Contents of {name}")).unwrap();
    }

    let planner = harness
        .runtime
        .create_agent(&AgentDraft {
            name: "Read-only planner".to_owned(),
            description: String::new(),
            kind: AgentKind::Conversation,
            read_only: true,
            profile_id: None,
            instructions: "Inspect files carefully before proposing changes.".to_owned(),
            steps: Vec::new(),
            tools: vec!["read_file".to_owned()],
            plugins: None,
            children: None,
            max_tasks: 0,
            max_tool_rounds: 50,
            max_tool_calls: 100,
        })
        .await
        .unwrap();

    harness
        .model
        .push_reply(tool_calls(&[("c1", "read_file", r#"{"path":"a.txt"}"#)]));
    harness
        .model
        .push_reply(tool_calls(&[("c2", "read_file", r#"{"path":"b.txt"}"#)]));
    harness
        .model
        .push_reply(tool_calls(&[("c3", "read_file", r#"{"path":"c.txt"}"#)]));
    harness.model.push_reply(reply("Plan ready."));

    let started = harness
        .runtime
        .start_conversation(
            StartConversation::new("Plan the change.")
                .with_project(Some(project.id))
                .with_agent(Some(planner.key)),
        )
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    harness.wait_for_run(&conversation, &started.run_id).await;

    harness
        .runtime
        .compact_conversation(&conversation)
        .await
        .unwrap();

    let checkpoints = harness.store.checkpoints(&conversation).await.unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0].covers_through_turn, 0);
    assert!(checkpoints[0].covers_through_part.is_some());
    let detail = harness
        .runtime
        .checkpoint(CheckpointRef {
            conversation_id: conversation.clone(),
            checkpoint_id: checkpoints[0].id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(detail.view.trigger.as_deref(), Some("manual"));

    let summary_request = &harness.model.summary_requests()[0];
    assert!(
        mentions(summary_request, "Contents of a.txt"),
        "early steps are summarized"
    );
    assert!(
        !mentions(summary_request, "Plan ready."),
        "the newest step stays outside the manual summary"
    );
}

fn pruned_call_ids(request: &mannie_provider::ModelRequest) -> Vec<String> {
    request
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::ToolResult {
                call_id, output, ..
            } if output.contains("was trimmed from context") => Some(call_id.clone()),
            _ => None,
        })
        .collect()
}

fn first_pruned_stub(request: &mannie_provider::ModelRequest) -> Option<&str> {
    request
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|content| match content {
            Content::ToolResult { output, .. } if output.contains("was trimmed from context") => {
                Some(output.as_str())
            }
            _ => None,
        })
}

#[tokio::test]
async fn epoch_tool_result_pruning_keeps_prefix_stable_and_reports_pruned_usage() {
    let harness = Harness::with_options(HarnessOptions {
        context_window: 24_000,
        output_limit: 2_048,
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await;
    let workspace = tempfile::tempdir().unwrap();
    let project = harness
        .runtime
        .add_project(workspace.path().to_str().unwrap().to_owned())
        .await
        .unwrap();

    // Six large files cross 80% of the 20_752 budget once older results are
    // prunable; after pruning brings the request below 80%, two tiny files
    // follow so the watermark stays fixed across consecutive steps.
    for index in 0..6 {
        let content = multiline_filler(&format!("[payload {index}]"), 5_000);
        fs::write(workspace.path().join(format!("p{index}.txt")), content).unwrap();
    }
    for index in 6..8 {
        fs::write(
            workspace.path().join(format!("p{index}.txt")),
            format!("[payload {index}] tiny"),
        )
        .unwrap();
    }

    for index in 0..8 {
        let call_id = format!("p_{index}");
        let args = format!(r#"{{"path":"p{index}.txt"}}"#);
        harness
            .model
            .push_reply(tool_calls(&[(&call_id, "read_file", &args)]));
        // Make every proactive automatic compaction fail soft so epoch pruning
        // handles the active turn.
        harness.model.push_summary(vec![
            Step::Event(ProviderEvent::TextDelta("Cut".to_owned())),
            Step::Event(ProviderEvent::Finished(FinishReason::OutputLimit)),
        ]);
    }
    harness.model.push_reply(reply("Finished pruning test."));

    let started = harness
        .runtime
        .start_conversation(
            StartConversation::new("Read every payload.").with_project(Some(project.id)),
        )
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    let transcript = harness.wait_for_run(&conversation, &started.run_id).await;

    assert_eq!(
        transcript.run_state(&started.run_id),
        Some(RunState::Completed),
        "outcome: {:?}",
        transcript
            .run(&started.run_id)
            .and_then(|run| run.outcome.clone())
    );

    let usage = harness
        .store
        .snapshot(&conversation, 50, 400)
        .await
        .unwrap()
        .unwrap()
        .context
        .expect("usage recorded after pruned steps");
    assert!(
        usage.results_pruned,
        "pruned calls report results_pruned = true: {usage:?}"
    );
    assert!(
        usage
            .unpruned_tokens
            .is_some_and(|full| full > usage.used_tokens),
        "pruned calls report a larger unpruned_tokens count: {usage:?}"
    );

    let requests = harness.model.reply_requests();
    let first_pruned = requests
        .iter()
        .find(|request| !pruned_call_ids(request).is_empty())
        .expect("at least one request pruned older tool results");
    // The final two requests follow tiny file reads (`p6.txt`, `p7.txt`) so
    // the epoch watermark stays fixed instead of shifting on every step.
    let penultimate_pruned_ids = pruned_call_ids(&requests[requests.len() - 2]);
    let final_pruned_ids = pruned_call_ids(&requests[requests.len() - 1]);
    assert!(
        !penultimate_pruned_ids.is_empty(),
        "penultimate request still has pruned tool results"
    );
    assert_eq!(
        penultimate_pruned_ids, final_pruned_ids,
        "epoch pruning keeps the watermark fixed across small steps for prompt-cache stability"
    );

    let stub_output = first_pruned_stub(first_pruned).unwrap();
    assert!(stub_output.contains("read_file"), "{stub_output}");
    assert!(stub_output.contains("p0.txt"), "{stub_output}");
    assert!(
        stub_output.contains("rely on what was already recorded"),
        "pruned stubs report factual metadata without urging re-runs: {stub_output}"
    );
}
