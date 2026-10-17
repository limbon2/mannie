//! Subscriptions replay from any cursor without gaps or repeats (ADR-0012;
//! docs/features/chat/conversations.md, transcript and events).
//!
//! Each case runs a conversation whose replies stream, retry, fail and get
//! retried, while subscribers join at random times, from a snapshot or from
//! the first event, read at their own pace, reconnect from their cursor and
//! fall behind their small queues. Every subscriber must end with the state
//! of a fresh snapshot, and the sequence numbers it sees must only increase.

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use std::time::Duration;

use mannie_provider::ProviderError;
use mannie_provider::ProviderEvent;
use mannie_provider::testing::Step;
use mannie_runtime::testing::{
    Harness, HarnessOptions, Transcript, fast_config, reply, server_error, tool_calls,
};
use mannie_runtime::{Runtime, RuntimeConfig, SubscriptionLimits};
use mannie_store::Store;
use mannie_types::chat::{SendMessage, StartConversation, SubscribeConversation};
use mannie_types::ids::{ConversationId, RunId};
use mannie_types::run::{FinishReason, RunFailure};
use mannie_types::transcript::{ConversationUpdate, SubscriptionEnd};
use proptest::prelude::*;
use tokio::sync::watch;

/// How a reply goes.
#[derive(Debug, Clone)]
enum Outcome {
    Completes,
    /// A transient failure after the deltas, then a short reply.
    RetriesOnce,
    /// A permanent failure after the deltas, then a retry by the owner.
    FailsThenRetried,
    /// A task in the background whose child streams the deltas, a wait for
    /// its report, then the reply.
    GivesATask,
}

#[derive(Debug, Clone)]
struct TurnPlan {
    deltas: Vec<(String, u8)>,
    outcome: Outcome,
}

#[derive(Debug, Clone)]
struct SubscriberPlan {
    start_delay_ms: u8,
    from_first_event: bool,
    reconnect_after: u8,
    reconnects: u8,
    pace_ms: u8,
}

fn turn_plan() -> impl Strategy<Value = TurnPlan> {
    (
        prop::collection::vec(("[a-z ]{1,6}", 0_u8..3), 1..25),
        prop_oneof![
            3 => Just(Outcome::Completes),
            1 => Just(Outcome::RetriesOnce),
            1 => Just(Outcome::FailsThenRetried),
            1 => Just(Outcome::GivesATask),
        ],
    )
        .prop_map(|(deltas, outcome)| TurnPlan { deltas, outcome })
}

fn subscriber_plan() -> impl Strategy<Value = SubscriberPlan> {
    (0_u8..40, any::<bool>(), 1_u8..12, 0_u8..3, 0_u8..3).prop_map(
        |(start_delay_ms, from_first_event, reconnect_after, reconnects, pace_ms)| SubscriberPlan {
            start_delay_ms,
            from_first_event,
            reconnect_after,
            reconnects,
            pace_ms,
        },
    )
}

fn streamed(plan: &TurnPlan) -> Vec<Step> {
    let mut steps = Vec::new();
    for (text, delay) in &plan.deltas {
        steps.push(Step::Event(ProviderEvent::TextDelta(text.clone())));
        if *delay > 0 {
            steps.push(Step::Delay(Duration::from_millis(u64::from(*delay))));
        }
    }
    steps
}

fn script(harness: &Harness, index: usize, plan: &TurnPlan) {
    let mut steps = streamed(plan);
    match plan.outcome {
        Outcome::Completes => {
            steps.push(Step::Event(ProviderEvent::Finished(FinishReason::Stop)));
            harness.model.push_reply(steps);
        }
        Outcome::RetriesOnce => {
            steps.push(Step::Error(server_error()));
            harness.model.push_reply(steps);
            harness.model.push_reply(reply("after a retry"));
        }
        Outcome::FailsThenRetried => {
            steps.push(Step::Error(ProviderError::Permanent(
                RunFailure::ContentFiltered,
            )));
            harness.model.push_reply(steps);
            harness.model.push_reply(reply("retried by the owner"));
        }
        Outcome::GivesATask => {
            let marker = format!("CHILD-{index}");
            let start = serde_json::json!({
                "activity": "Delegating",
                "title": format!("Task {index}"),
                "agent": "explore",
                "instruction": format!("{marker}: look around"),
                "background": true,
            })
            .to_string();
            harness.model.push_reply(tool_calls(&[(
                &format!("start_{index}"),
                "start_task",
                &start,
            )]));
            harness.model.push_reply(tool_calls(&[(
                &format!("wait_{index}"),
                "wait_for_tasks",
                r#"{"tasks":[]}"#,
            )]));
            let mut child = streamed(plan);
            child.push(Step::Event(ProviderEvent::Finished(FinishReason::Stop)));
            harness.model.push_reply_for(&marker, child);
            harness.model.push_reply_for(&marker, reply("Report."));
            steps.push(Step::Event(ProviderEvent::Finished(FinishReason::Stop)));
            harness.model.push_reply(steps);
        }
    }
}

async fn wait_for_end(store: &Store, conversation: &ConversationId, run: &RunId) {
    loop {
        let snapshot = store
            .snapshot(conversation, 100, 10_000)
            .await
            .unwrap()
            .unwrap();
        let ended = snapshot
            .turns
            .iter()
            .filter_map(|turn| turn.run.as_ref())
            .any(|view| view.id == *run && !view.state.is_active());
        if ended {
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Runs the conversation and returns its ID once the first turn exists; the
/// rest runs in the background and reports the final sequence number.
async fn drive(
    harness: &Harness,
    turns: Vec<TurnPlan>,
    finished: watch::Sender<Option<u32>>,
) -> ConversationId {
    for (index, plan) in turns.iter().enumerate() {
        script(harness, index, plan);
    }
    let started = harness
        .runtime
        .start_conversation(StartConversation {
            mentions: Vec::new(),
            text: "Turn 0".to_owned(),
            profile_id: None,
            project_id: None,
        })
        .await
        .unwrap();
    let conversation = started.conversation_id.clone();
    let runtime = harness.runtime.clone();
    let store = harness.store.clone();
    let background_conversation = conversation.clone();
    let background = async move {
        let mut run = started.run_id;
        for (index, plan) in turns.iter().enumerate() {
            if index > 0 {
                run = runtime
                    .send_message(SendMessage {
                        mentions: Vec::new(),
                        conversation_id: background_conversation.clone(),
                        text: format!("Turn {index}"),
                    })
                    .await
                    .unwrap()
                    .run_id;
            }
            wait_for_end(&store, &background_conversation, &run).await;
            if matches!(plan.outcome, Outcome::FailsThenRetried) {
                run = runtime
                    .retry_reply(&background_conversation)
                    .await
                    .unwrap()
                    .run_id;
                wait_for_end(&store, &background_conversation, &run).await;
            }
        }
        let last_seq = match store
            .events_after(&background_conversation, 0, u32::MAX)
            .await
            .unwrap()
        {
            mannie_store::EventsAfter::Events { last_seq, .. } => last_seq,
            other => panic!("the conversation exists: {other:?}"),
        };
        finished.send_replace(Some(last_seq));
    };
    tokio::spawn(background);
    conversation
}

/// What a subscriber saw.
#[derive(Debug)]
struct Followed {
    transcript: Transcript,
    /// How often it fell behind.
    lagged: u32,
}

/// Follows the conversation as a window would, and returns what it saw.
async fn follow(
    runtime: Runtime,
    conversation: ConversationId,
    plan: SubscriberPlan,
    mut finished: watch::Receiver<Option<u32>>,
) -> Result<Followed, String> {
    tokio::time::sleep(Duration::from_millis(u64::from(plan.start_delay_ms))).await;
    let mut transcript = Transcript::default();
    let mut seen: Option<u32> = None;
    let mut cursor = plan.from_first_event.then_some(0);
    let mut reconnects = plan.reconnects;
    let mut lagged = 0;
    loop {
        let mut subscription = runtime
            .subscribe(SubscribeConversation {
                conversation_id: conversation.clone(),
                after_seq: cursor,
            })
            .await
            .map_err(|error| format!("subscribing failed: {error}"))?;
        let mut read = 0_u8;
        loop {
            let last = *finished.borrow();
            let caught_up = last.is_some_and(|last| seen.is_some_and(|seen| seen >= last));
            if caught_up && !subscription.is_replaying() {
                return Ok(Followed { transcript, lagged });
            }
            let update = if last.is_some() {
                subscription.next().await
            } else {
                tokio::select! {
                    update = subscription.next() => update,
                    _changed = finished.changed() => continue,
                }
            };
            let Some(update) = update else {
                return Err("the subscription ended without saying why".to_owned());
            };
            match &update {
                ConversationUpdate::Snapshot { snapshot } => {
                    if seen.is_some_and(|seen| snapshot.through_seq < seen) {
                        return Err(format!(
                            "a snapshot went back from {seen:?} to {}",
                            snapshot.through_seq
                        ));
                    }
                    seen = Some(snapshot.through_seq);
                }
                ConversationUpdate::Event { seq, .. } => {
                    if seen.is_some_and(|seen| *seq <= seen) {
                        return Err(format!("event {seq} came after {seen:?}"));
                    }
                    seen = Some(*seq);
                }
                ConversationUpdate::Ended { reason } => {
                    if *reason != SubscriptionEnd::Lagged {
                        return Err(format!("the subscription ended: {reason:?}"));
                    }
                    lagged += 1;
                    break;
                }
            }
            transcript.apply(&update);
            read = read.saturating_add(1);
            if reconnects > 0 && read >= plan.reconnect_after {
                reconnects -= 1;
                break;
            }
            if plan.pace_ms > 0 {
                tokio::time::sleep(Duration::from_millis(u64::from(plan.pace_ms))).await;
            }
        }
        // A window that reconnects or fell behind continues from its cursor.
        cursor = seen;
    }
}

fn run_case(
    turns: Vec<TurnPlan>,
    subscribers: Vec<SubscriberPlan>,
) -> Result<Vec<Followed>, TestCaseError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let harness = Harness::with_options(HarnessOptions {
            config: RuntimeConfig {
                subscriptions: SubscriptionLimits {
                    // A small queue makes slow subscribers fall behind.
                    queue: 6,
                    ..SubscriptionLimits::default()
                },
                ..fast_config()
            },
            // Tasks need tools; a reply without them streams as before.
            use_tools: true,
            ..HarnessOptions::default()
        })
        .await;
        let (finished_sender, finished) = watch::channel(None);
        let conversation = drive(&harness, turns, finished_sender).await;
        let followers: Vec<_> = subscribers
            .into_iter()
            .map(|plan| {
                tokio::spawn(follow(
                    harness.runtime.clone(),
                    conversation.clone(),
                    plan,
                    finished.clone(),
                ))
            })
            .collect();
        let mut seen = Vec::new();
        for follower in followers {
            let outcome = tokio::time::timeout(Duration::from_secs(30), follower)
                .await
                .map_err(|_elapsed| TestCaseError::fail("a subscriber did not finish"))?
                .map_err(|error| TestCaseError::fail(format!("a subscriber panicked: {error}")))?;
            seen.push(outcome.map_err(TestCaseError::fail)?);
        }
        let expected = harness.snapshot(&conversation).await.content();
        for followed in &seen {
            prop_assert_eq!(followed.transcript.content(), expected.clone());
        }
        harness.runtime.shutdown().await;
        Ok(seen)
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn every_subscriber_ends_with_the_state_of_a_fresh_snapshot(
        turns in prop::collection::vec(turn_plan(), 1..4),
        subscribers in prop::collection::vec(subscriber_plan(), 1..5),
    ) {
        run_case(turns, subscribers)?;
    }
}

#[test]
fn a_slow_subscriber_falls_behind_and_catches_up_from_its_cursor() {
    let turns = vec![TurnPlan {
        deltas: (0..40).map(|index| (format!("{index} "), 1)).collect(),
        outcome: Outcome::Completes,
    }];
    let subscribers = vec![
        SubscriberPlan {
            start_delay_ms: 0,
            from_first_event: true,
            reconnect_after: 2,
            reconnects: 0,
            pace_ms: 15,
        },
        SubscriberPlan {
            start_delay_ms: 5,
            from_first_event: false,
            reconnect_after: 3,
            reconnects: 2,
            pace_ms: 0,
        },
    ];

    let seen = run_case(turns, subscribers).unwrap();

    assert!(seen[0].lagged > 0, "the slow subscriber never fell behind");
}

#[test]
fn a_subscriber_follows_a_task_to_its_report() {
    let turns = vec![TurnPlan {
        deltas: (0..20).map(|index| (format!("{index} "), 1)).collect(),
        outcome: Outcome::GivesATask,
    }];
    let subscribers = vec![SubscriberPlan {
        start_delay_ms: 0,
        from_first_event: true,
        reconnect_after: 4,
        reconnects: 1,
        pace_ms: 1,
    }];

    let seen = run_case(turns, subscribers).unwrap();

    let tasks = seen[0].transcript.tasks(0);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].state, mannie_types::tasks::TaskState::Reported);
    assert_eq!(tasks[0].report.as_deref(), Some("Report."));
}
