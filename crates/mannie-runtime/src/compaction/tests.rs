use mannie_store::{ModelImage, ModelPart, ModelTurn};
use mannie_types::ids::{PartId, TurnId};
use mannie_types::images::{ImageBytes, ImageFormat};
use mannie_types::run::RunState;
use mannie_types::transcript::{PartKind, PartState};

use super::{
    ContextSize, Plan, Trigger, chunks, content_hash, fit, needed, plan, render_turns,
    summary_allowance, turns_estimate,
};
use crate::agents::Requester;
use crate::budget::{estimate_image, estimate_text};
use crate::texts;

fn part(id: u32, kind: PartKind, text: &str) -> ModelPart {
    ModelPart {
        id: PartId::parse(&format!("0199e0f2-0000-7000-8000-{id:012}")).unwrap(),
        kind: Some(kind),
        kind_name: kind.as_str().to_owned(),
        state: PartState::Complete,
        text: text.to_owned(),
        provider_item: None,
        run_id: None,
        tool_call: None,
        task: None,
        attempt: 1,
        image: None,
        context: None,
    }
}

/// A complete turn whose question and answer each have `bytes` bytes.
fn turn(position: u32, bytes: usize) -> ModelTurn {
    ModelTurn {
        turn_id: TurnId::parse(&format!("0199e0f2-0000-7000-9000-{position:012}")).unwrap(),
        position,
        run_id: None,
        run_state: Some(RunState::Completed),
        kept_runs: Vec::new(),
        parts: vec![
            part(position * 10, PartKind::UserText, &"q".repeat(bytes)),
            part(position * 10 + 1, PartKind::Text, &"a".repeat(bytes)),
        ],
    }
}

fn turns(count: u32, bytes: usize) -> Vec<ModelTurn> {
    (0..count).map(|position| turn(position, bytes)).collect()
}

fn size(budget: u32, projected: u32) -> ContextSize {
    ContextSize {
        budget,
        projected,
        estimate: projected,
    }
}

#[test]
fn compaction_starts_past_80_percent_of_the_budget() {
    assert!(!needed(800, 1_000));
    assert!(needed(801, 1_000));
}

#[test]
fn the_summary_allowance_is_15_percent_within_its_bounds() {
    assert_eq!(summary_allowance(100_000, 128_000), 15_000);
    assert_eq!(
        summary_allowance(2_000, 128_000),
        1_024,
        "at least 1,024 tokens"
    );
    assert_eq!(
        summary_allowance(100_000, 4_096),
        4_096,
        "never past the output limit"
    );
}

#[test]
fn a_plan_keeps_the_newest_two_turns_when_that_reaches_the_target() {
    // Ten turns of about 804 tokens each in a budget of 10,000.
    let candidates = turns(10, 1_000);
    let per_turn = turns_estimate(&candidates[..1], Requester::Owner);
    let projected = per_turn * 10;

    let planned = plan(
        &candidates,
        size(10_000, projected),
        1_024,
        Trigger::Automatic,
        Requester::Owner,
    );

    assert_eq!(
        planned,
        Some(Plan {
            covered: 8,
            after: projected - per_turn * 8 + 1_024,
        })
    );
    assert!(planned.unwrap().after <= 3_000);
}

#[test]
fn a_plan_keeps_fewer_turns_when_two_do_not_fit_the_target() {
    let candidates = turns(3, 4_000);
    let per_turn = turns_estimate(&candidates[..1], Requester::Owner);

    let planned = plan(
        &candidates,
        size(12_000, per_turn * 3),
        1_024,
        Trigger::Automatic,
        Requester::Owner,
    )
    .unwrap();

    assert_eq!(
        planned.covered, 3,
        "keeping one or two turns stays above 30%"
    );
}

#[test]
fn an_automatic_compaction_that_frees_little_is_skipped_unless_the_request_does_not_fit() {
    let candidates = turns(1, 100);
    let projected = 9_000;

    assert_eq!(
        plan(
            &candidates,
            size(10_000, projected),
            1_024,
            Trigger::Automatic,
            Requester::Owner
        ),
        None
    );
    assert_eq!(
        plan(
            &candidates,
            size(10_000, 10_001),
            1_024,
            Trigger::Automatic,
            Requester::Owner
        )
        .map(|plan| plan.covered),
        Some(1),
        "a request beyond the budget is compacted however little it frees"
    );
    assert_eq!(
        plan(
            &candidates,
            size(10_000, projected),
            1_024,
            Trigger::Manual,
            Requester::Owner
        )
        .map(|plan| plan.covered),
        Some(1)
    );
    assert_eq!(
        plan(
            &[],
            size(10_000, projected),
            1_024,
            Trigger::Overflow,
            Requester::Owner
        ),
        None
    );
}

#[test]
fn freed_tokens_follow_the_calibration_of_the_projection() {
    let candidates = turns(4, 2_000);
    let estimate = turns_estimate(&candidates, Requester::Owner);
    let uncalibrated = ContextSize {
        budget: 10_000,
        projected: estimate,
        estimate,
    };
    // The provider counted half of what Mannie estimated, so covering a turn
    // frees half of its estimate too.
    let calibrated = ContextSize {
        projected: estimate / 2,
        ..uncalibrated
    };

    let by_estimate = plan(
        &candidates,
        uncalibrated,
        500,
        Trigger::Manual,
        Requester::Owner,
    );
    let by_count = plan(
        &candidates,
        calibrated,
        500,
        Trigger::Manual,
        Requester::Owner,
    );

    assert_eq!(
        by_estimate,
        Some(Plan {
            covered: 3,
            after: estimate - estimate * 3 / 4 + 500,
        })
    );
    assert_eq!(
        by_count,
        Some(Plan {
            covered: 2,
            after: estimate / 2 - estimate / 4 + 500,
        })
    );
}

#[test]
fn rendered_turns_name_the_speakers_and_leave_reasoning_out() {
    let mut with_reasoning = turn(0, 1);
    with_reasoning
        .parts
        .insert(1, part(5, PartKind::Reasoning, "secret thoughts"));
    with_reasoning.parts.push(part(
        6,
        PartKind::ContinuationNote,
        texts::CONTINUATION_NOTE,
    ));

    let rendered = render_turns(&[with_reasoning], Requester::Owner);

    assert_eq!(
        rendered,
        [format!(
            "{} q\n\n{} a",
            texts::COMPACTION_OWNER,
            texts::COMPACTION_ASSISTANT
        )]
    );
}

#[test]
fn mentions_reach_summaries_cut_to_their_start() {
    let page = format!("The owner mentioned the file a.rs.\n{}", "x".repeat(5_000));
    let mut mentioned = turn(0, 1);
    mentioned.parts.insert(1, part(2, PartKind::Mention, &page));

    let rendered = render_turns(&[mentioned.clone()], Requester::Owner);

    assert!(
        rendered[0].contains("Owner's mention: The owner mentioned the file a.rs."),
        "{rendered:?}"
    );
    assert!(rendered[0].contains("[...]"), "{rendered:?}");
    assert!(rendered[0].len() < page.len(), "{}", rendered[0].len());
    assert!(turns_estimate(&[mentioned], Requester::Owner) > estimate_text(&page));
}

fn image(id: u32, digest: &str) -> ModelPart {
    ModelPart {
        image: Some(ModelImage {
            format: ImageFormat::Jpeg,
            width: 1_600,
            height: 1_200,
            digest: digest.to_owned(),
            data: ImageBytes::from(vec![1; 10_000]),
        }),
        ..part(id, PartKind::Image, "")
    }
}

#[test]
fn images_reach_summaries_as_their_size_and_count_their_pixels() {
    let mut pictured = turn(0, 1);
    pictured.parts.insert(1, image(2, "digest-a"));

    let rendered = render_turns(&[pictured.clone()], Requester::Owner);
    let with_image = turns_estimate(&[pictured], Requester::Owner);
    let without = turns_estimate(&[turn(0, 1)], Requester::Owner);

    assert!(
        rendered[0].contains("Owner's image: 1600×1200 pixels."),
        "{rendered:?}"
    );
    assert!(with_image >= without + estimate_image(1_600, 1_200));
}

#[test]
fn an_image_s_digest_is_part_of_the_hash() {
    let mut first = turn(0, 1);
    first.parts.insert(1, image(2, "digest-a"));
    let mut second = turn(0, 1);
    second.parts.insert(1, image(2, "digest-b"));

    assert_ne!(content_hash(None, &[first]), content_hash(None, &[second]));
}

#[test]
fn chunks_keep_the_order_and_respect_the_limit() {
    let rendered: Vec<String> = [
        "a".repeat(100),
        "b".repeat(100),
        "c".repeat(400),
        "d".repeat(10),
    ]
    .into_iter()
    .collect();

    let split = chunks(&rendered, 100);

    assert_eq!(
        split,
        [
            format!("{}\n\n{}", "a".repeat(100), "b".repeat(100)),
            "c".repeat(400),
            "d".repeat(10),
        ]
    );
}

#[test]
fn an_oversized_chunk_loses_its_middle_and_fits() {
    let chunk = format!(
        "{}{}{}",
        "start ".repeat(200),
        "é".repeat(5_000),
        " end".repeat(200)
    );

    let fitted = fit(&chunk, 1_000);

    assert!(
        estimate_text(&fitted) <= 1_000,
        "{} tokens",
        estimate_text(&fitted)
    );
    assert!(fitted.starts_with("start start"));
    assert!(fitted.ends_with("end end"));
    assert!(fitted.contains(texts::COMPACTION_OMITTED));
    assert_eq!(fit("short", 1_000), "short");
}

#[test]
fn the_hash_covers_every_part_and_chains_the_previous_checkpoint() {
    let original = turns(2, 10);
    let mut changed = original.clone();
    changed[1].parts[1].text.push('!');
    let mut stopped = original.clone();
    stopped[0].parts[1].state = PartState::Stopped;

    let hash = content_hash(None, &original);

    assert_eq!(hash.len(), 64);
    assert_eq!(hash, content_hash(None, &original));
    assert_ne!(hash, content_hash(None, &changed));
    assert_ne!(hash, content_hash(None, &stopped));
    assert_ne!(hash, content_hash(Some("previous"), &original));
}

#[test]
fn tool_calls_count_and_hash_their_results_and_reach_summaries() {
    use mannie_types::tools::{ToolCallDocument, ToolCallStatus, ToolOutcome, ToolResult};

    let call = |output: &str| ModelPart {
        tool_call: Some(ToolCallDocument {
            status: ToolCallStatus::Completed,
            result: Some(ToolResult {
                outcome: ToolOutcome::Unknown,
                output: output.to_owned(),
                archives: Vec::new(),
                images: Vec::new(),
            }),
            ..ToolCallDocument::receiving("call_1".to_owned(), "read_file".to_owned())
        }),
        ..part(2, PartKind::ToolCall, r#"{"path":"a.txt"}"#)
    };
    let with_result = |output: &str| {
        let mut turn = turn(0, 10);
        turn.parts.push(call(output));
        vec![turn]
    };

    let short = with_result("short");
    let long = with_result(&"x".repeat(10_000));

    assert!(
        turns_estimate(&long, Requester::Owner) > turns_estimate(&short, Requester::Owner) + 3_000
    );
    assert_ne!(content_hash(None, &short), content_hash(None, &long));
    assert!(
        render_turns(&short, Requester::Owner)[0].contains(&texts::compaction_tool_call(
            "read_file",
            r#"{"path":"a.txt"}"#,
            "short"
        )),
        "{:?}",
        render_turns(&short, Requester::Owner)
    );
}

#[test]
fn task_calls_reach_summaries_and_the_hash_covers_their_results() {
    use mannie_types::tasks::{AwaitedTask, TaskPartDocument, TaskState};
    use mannie_types::tools::{ToolCallDocument, ToolCallStatus};

    let wait = |output: &str| ModelPart {
        tool_call: Some(ToolCallDocument {
            status: ToolCallStatus::Completed,
            ..ToolCallDocument::receiving("call_1".to_owned(), "wait_for_tasks".to_owned())
        }),
        task: Some(TaskPartDocument {
            awaited: vec![AwaitedTask {
                number: 2,
                title: "Read the tests".to_owned(),
                state: TaskState::Reported,
            }],
            output: Some(output.to_owned()),
            ..TaskPartDocument::default()
        }),
        ..part(2, PartKind::Task, r#"{"tasks":[2]}"#)
    };
    let with_result = |output: &str| {
        let mut turn = turn(0, 10);
        turn.parts.push(wait(output));
        vec![turn]
    };

    let short = with_result("Task 2 reported:\n\nAll green.");
    let other = with_result("Task 2 reported:\n\nTwo failures.");

    let rendered = render_turns(&short, Requester::Owner);
    assert!(
        rendered[0].contains(&texts::compaction_task_call(
            "wait_for_tasks",
            r#"{"tasks":[2]}"#,
            "Task 2 reported:\n\nAll green."
        )),
        "{rendered:?}"
    );
    assert_ne!(content_hash(None, &short), content_hash(None, &other));
    assert!(
        turns_estimate(&short, Requester::Owner) > turns_estimate(&[turn(0, 10)], Requester::Owner)
    );
}
