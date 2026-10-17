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

use mannie_provider::{Content, known_models};
use mannie_runtime::testing::{Harness, HarnessOptions, estimate, reply};
use mannie_runtime::texts;
use mannie_types::chat::{MAX_MESSAGE_BYTES, SendMessage, StartConversation};
use mannie_types::provider::context_budget;
use mannie_types::run::RunState;
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

fn mentions(request: &mannie_provider::ModelRequest, marker: &str) -> bool {
    request.instructions.contains(marker)
        || request.messages.iter().any(|message| {
            message.content.iter().any(|content| match content {
                Content::Text(text)
                | Content::ProviderItem(text)
                | Content::ToolResult { output: text, .. } => text.contains(marker),
                Content::ToolCall { arguments, .. } => arguments.contains(marker),
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
        .start_conversation(StartConversation {
            mentions: Vec::new(),
            text: messages[0].clone(),
            profile_id: None,
            project_id: None,
        })
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    let mut runs = vec![started.run_id];
    harness.wait_for_run(&conversation, &runs[0]).await;
    for text in &messages[1..] {
        let sent = harness
            .runtime
            .send_message(SendMessage {
                mentions: Vec::new(),
                conversation_id: conversation.clone(),
                text: text.clone(),
            })
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
        .start_conversation(StartConversation {
            mentions: Vec::new(),
            text: "First".to_owned(),
            profile_id: None,
            project_id: None,
        })
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    harness.wait_for_run(&conversation, &started.run_id).await;

    let too_early = harness.runtime.compact_conversation(&conversation).await;
    let second = harness
        .runtime
        .send_message(SendMessage {
            mentions: Vec::new(),
            conversation_id: conversation.clone(),
            text: "Second".to_owned(),
        })
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
        .checkpoint(mannie_types::chat::CheckpointRef {
            conversation_id: conversation.clone(),
            checkpoint_id: checkpoints[0].id.clone(),
        })
        .await
        .unwrap();
    let third = harness
        .runtime
        .send_message(SendMessage {
            mentions: Vec::new(),
            conversation_id: conversation.clone(),
            text: "Third".to_owned(),
        })
        .await
        .unwrap();
    harness.wait_for_run(&conversation, &third.run_id).await;

    assert_eq!(
        too_early.unwrap_err(),
        mannie_types::chat::ChatError::NothingToCompact
    );
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
