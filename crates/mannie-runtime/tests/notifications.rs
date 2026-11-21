//! Owner-conversation lifecycle signals emitted by the runtime hub for
//! desktop notifications and the tray badge (ADR-0074, decision 3).

#![cfg(test)]

use std::time::Duration;

use mannie_provider::ProviderError;
use mannie_runtime::testing::{Harness, HarnessOptions, failure, reply, tool_calls};
use mannie_runtime::{OwnerSignal, SignalSubscription};
use mannie_types::chat::StartConversation;
use mannie_types::ids::ConversationId;
use mannie_types::run::RunFailure;
use mannie_types::tools::{
    AnswerQuestion, ApprovalChoice, DecideApproval, QuestionAnswer, QuestionReply, ToolCallStatus,
};
use serde_json::json;

/// Receives signals until `predicate` matches, failing if the stream ends or
/// times out.
async fn next_matching(
    signals: &mut SignalSubscription,
    mut predicate: impl FnMut(&OwnerSignal) -> bool,
) -> Vec<OwnerSignal> {
    let mut seen = Vec::new();
    let deadline = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(signal) = signals.next().await {
            let matched = predicate(&signal);
            seen.push(signal);
            if matched {
                return;
            }
        }
        panic!("the signal stream ended before the expected signal arrived; saw {seen:#?}");
    })
    .await;
    assert!(
        deadline.is_ok(),
        "timed out waiting for expected signal; saw {seen:#?}"
    );
    seen
}

async fn tools_harness() -> Harness {
    Harness::with_options(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await
}

#[tokio::test]
async fn completed_and_failed_runs_emit_owner_signals_and_delete_emits_deleted() {
    let harness = Harness::start().await;
    let mut signals = harness.runtime.subscribe_signals();

    harness.model.push_reply(reply("All done."));

    let started = harness
        .runtime
        .start_conversation(StartConversation::new("Finish this job"))
        .await
        .unwrap();

    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let completed = next_matching(&mut signals, |signal| {
        matches!(signal, OwnerSignal::Completed { .. })
    })
    .await;
    assert!(completed.iter().any(|signal| {
        matches!(
            signal,
            OwnerSignal::Completed {
                conversation_id,
                title,
                run_id,
            } if conversation_id == &started.conversation_id
                && title == "Finish this job"
                && run_id == &started.run_id
        )
    }));

    harness.model.push_reply(failure(ProviderError::Permanent(
        RunFailure::InvalidCredential,
    )));

    let failing = harness
        .runtime
        .start_conversation(StartConversation::new("Broken provider"))
        .await
        .unwrap();

    harness
        .wait_for_run(&failing.conversation_id, &failing.run_id)
        .await;

    let failed = next_matching(&mut signals, |signal| {
        matches!(signal, OwnerSignal::Failed { .. })
    })
    .await;
    assert!(failed.iter().any(|signal| {
        matches!(
            signal,
            OwnerSignal::Failed {
                conversation_id,
                title,
                run_id,
            } if conversation_id == &failing.conversation_id
                && title == "Broken provider"
                && run_id == &failing.run_id
        )
    }));

    harness
        .runtime
        .delete_conversation(&started.conversation_id)
        .await
        .unwrap();

    let deleted = next_matching(&mut signals, |signal| {
        matches!(signal, OwnerSignal::Deleted { .. })
    })
    .await;
    assert!(deleted.iter().any(|signal| {
        matches!(
            signal,
            OwnerSignal::Deleted { conversation_id } if conversation_id == &started.conversation_id
        )
    }));
}

#[tokio::test]
async fn ask_owner_emits_awaiting_answer_then_active_and_completed() {
    let harness = tools_harness().await;
    let mut signals = harness.runtime.subscribe_signals();

    let ask_arguments = json!({
        "activity": "Asking the owner",
        "questions": [{
            "question": "Pick an option",
            "multiple": false,
            "options": [
                { "label": "Alpha", "description": "First" },
                { "label": "Beta", "description": "Second" }
            ]
        }]
    })
    .to_string();
    harness
        .model
        .push_reply(tool_calls(&[("call_q", "ask_owner", &ask_arguments)]));
    harness.model.push_reply(reply("Thanks!"));

    let asking = harness
        .runtime
        .start_conversation(StartConversation::new("Need your input"))
        .await
        .unwrap();

    let wait_signals = next_matching(&mut signals, |signal| {
        matches!(signal, OwnerSignal::AwaitingAnswer { .. })
    })
    .await;
    let part_id = wait_signals
        .iter()
        .find_map(|signal| match signal {
            OwnerSignal::AwaitingAnswer {
                conversation_id,
                part_id,
                ..
            } if conversation_id == &asking.conversation_id => Some(part_id.clone()),
            _ => None,
        })
        .expect("question signal should include the waiting part ID");

    harness
        .runtime
        .answer_question(AnswerQuestion {
            conversation_id: asking.conversation_id.clone(),
            part_id,
            reply: QuestionReply::Answered {
                answers: vec![QuestionAnswer {
                    selected: vec![0],
                    own_words: None,
                }],
            },
        })
        .unwrap();
    harness
        .wait_for_run(&asking.conversation_id, &asking.run_id)
        .await;

    let after_answer = next_matching(&mut signals, |signal| {
        matches!(signal, OwnerSignal::Completed { .. })
    })
    .await;
    assert!(after_answer.iter().any(|signal| {
        matches!(
            signal,
            OwnerSignal::Active { conversation_id } if conversation_id == &asking.conversation_id
        )
    }));
}

fn write_file_arguments(directory: &tempfile::TempDir) -> String {
    let target = directory.path().join("note.txt");
    json!({
        "activity": "Writing a file",
        "path": target.to_str().unwrap(),
        "content": "hello",
    })
    .to_string()
}

#[tokio::test]
async fn stopped_approval_emits_awaiting_approval_then_ended_without_alert() {
    let harness = tools_harness().await;
    let mut signals = harness.runtime.subscribe_signals();
    let outside = tempfile::tempdir().unwrap();
    let write_arguments = write_file_arguments(&outside);

    harness
        .model
        .push_reply(tool_calls(&[("call_w", "write_file", &write_arguments)]));

    let approving = harness
        .runtime
        .start_conversation(StartConversation::new("Write a file"))
        .await
        .unwrap();

    let approval_signals = next_matching(&mut signals, |signal| {
        matches!(signal, OwnerSignal::AwaitingApproval { .. })
    })
    .await;
    assert!(approval_signals.iter().any(|signal| {
        matches!(
            signal,
            OwnerSignal::AwaitingApproval { conversation_id, .. }
                if conversation_id == &approving.conversation_id
        )
    }));

    harness.runtime.stop_reply(&approving.conversation_id).await;
    harness
        .wait_for_run(&approving.conversation_id, &approving.run_id)
        .await;

    let stopped = next_matching(&mut signals, |signal| {
        matches!(signal, OwnerSignal::EndedWithoutAlert { .. })
    })
    .await;
    assert!(!stopped.iter().any(|signal| {
        matches!(
            signal,
            OwnerSignal::Completed { .. } | OwnerSignal::Failed { .. }
        )
    }));
}

#[tokio::test]
async fn allowed_approval_emits_awaiting_approval_then_completed() {
    let harness = tools_harness().await;
    let mut signals = harness.runtime.subscribe_signals();
    let outside = tempfile::tempdir().unwrap();
    let write_arguments = write_file_arguments(&outside);

    harness
        .model
        .push_reply(tool_calls(&[("call_w2", "write_file", &write_arguments)]));
    harness.model.push_reply(reply("Written."));

    let approving = harness
        .runtime
        .start_conversation(StartConversation::new("Write again"))
        .await
        .unwrap();
    let approval_signals = next_matching(&mut signals, |signal| {
        matches!(signal, OwnerSignal::AwaitingApproval { .. })
    })
    .await;
    let approval_part = approval_signals
        .iter()
        .find_map(|signal| match signal {
            OwnerSignal::AwaitingApproval {
                conversation_id,
                part_id,
                ..
            } if conversation_id == &approving.conversation_id => Some(part_id.clone()),
            _ => None,
        })
        .expect("approval signal should carry the part ID");

    let transcript = harness
        .wait_until(&approving.conversation_id, |view| {
            view.tool_calls(0)
                .iter()
                .any(|call| call.status == ToolCallStatus::AwaitingApproval)
        })
        .await;
    let digest = transcript
        .tool_calls(0)
        .into_iter()
        .find(|call| call.status == ToolCallStatus::AwaitingApproval)
        .and_then(|call| call.request.map(|request| request.digest))
        .expect("pending approval should expose its digest");
    harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: approving.conversation_id.clone(),
            part_id: approval_part,
            digest,
            choice: ApprovalChoice::AllowOnce,
        })
        .await
        .unwrap();
    harness
        .wait_for_run(&approving.conversation_id, &approving.run_id)
        .await;

    let completed = next_matching(&mut signals, |signal| {
        matches!(signal, OwnerSignal::Completed { .. })
    })
    .await;
    assert!(completed.iter().any(|signal| {
        matches!(
            signal,
            OwnerSignal::Completed { conversation_id, .. }
                if conversation_id == &approving.conversation_id
        )
    }));
}

#[tokio::test]
async fn child_tasks_never_emit_separate_owner_signals_and_shutdown_closes_stream() {
    let harness = tools_harness().await;
    let mut signals = harness.runtime.subscribe_signals();
    let task_arguments = json!({
        "activity": "Delegating to explore child",
        "agent": "explore",
        "title": "Inspect workspace",
        "instruction": "Report back immediately",
        "background": false,
    })
    .to_string();

    harness
        .model
        .push_reply(tool_calls(&[("call_t", "start_task", &task_arguments)]));
    harness
        .model
        .push_reply_for("Report back immediately", reply("Child done."));
    harness
        .model
        .push_reply_for("Report back immediately", reply("Report: child done."));
    harness.model.push_reply(reply("Parent done."));

    let parent = harness
        .runtime
        .start_conversation(StartConversation::new("Parent work"))
        .await
        .unwrap();

    harness
        .wait_for_run(&parent.conversation_id, &parent.run_id)
        .await;

    let parent_signals = next_matching(&mut signals, |signal| {
        matches!(
            signal,
            OwnerSignal::Completed { conversation_id, .. }
                if conversation_id == &parent.conversation_id
        )
    })
    .await;
    let completed_ids: Vec<&ConversationId> = parent_signals
        .iter()
        .filter_map(|signal| match signal {
            OwnerSignal::Completed {
                conversation_id, ..
            } => Some(conversation_id),
            _ => None,
        })
        .collect();
    assert_eq!(completed_ids, vec![&parent.conversation_id]);

    harness.runtime.shutdown().await;

    let shutdown = next_matching(&mut signals, |signal| {
        matches!(signal, OwnerSignal::Shutdown)
    })
    .await;
    assert!(
        shutdown
            .iter()
            .any(|signal| matches!(signal, OwnerSignal::Shutdown))
    );
    assert_eq!(signals.next().await, None);
}
