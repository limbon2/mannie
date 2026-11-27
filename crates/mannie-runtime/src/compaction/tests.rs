use mannie_store::{ModelImage, ModelPart, ModelTurn};
use mannie_types::ids::{PartId, RunId, TurnId};
use mannie_types::images::{ImageBytes, ImageFormat};
use mannie_types::run::RunState;
use mannie_types::tools::{ToolCallDocument, ToolCallStatus, ToolOutcome, ToolResult};
use mannie_types::transcript::{PartKind, PartState};

use super::{
    CheckpointCursor, ContextSize, Trigger, chunks, content_hash, cut_head_tail, fit, needed,
    plan_compaction, render_turns, slice_between, step_boundaries, summary_allowance,
    uncovered_turns,
};
use crate::agents::Requester;
use crate::budget::estimate_text;
use crate::texts;

fn part_id(id: u32) -> PartId {
    PartId::parse(&format!("0199e0f2-0000-7000-8000-{id:012}")).unwrap()
}

fn run_id(id: u32) -> RunId {
    RunId::parse(&format!("0199e0f2-0000-7000-b000-{id:012}")).unwrap()
}

fn part(id: u32, kind: PartKind, text: &str) -> ModelPart {
    let from_model = !matches!(
        kind,
        PartKind::UserText | PartKind::Mention | PartKind::Image | PartKind::Context
    );
    ModelPart {
        id: part_id(id),
        kind: Some(kind),
        kind_name: kind.as_str().to_owned(),
        state: PartState::Complete,
        text: text.to_owned(),
        provider_item: None,
        run_id: from_model.then(|| run_id(1)),
        tool_call: None,
        task: None,
        attempt: u32::from(from_model),
        image: None,
        context: None,
    }
}

/// A complete turn whose question and answer each have `bytes` bytes.
fn turn(position: u32, bytes: usize) -> ModelTurn {
    ModelTurn {
        turn_id: TurnId::parse(&format!("0199e0f2-0000-7000-9000-{position:012}")).unwrap(),
        position,
        run_id: Some(run_id(1)),
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

fn tool_step_part(id: u32, attempt: u32, status: ToolCallStatus, output: &str) -> ModelPart {
    ModelPart {
        attempt,
        tool_call: Some(ToolCallDocument {
            status,
            result: Some(ToolResult {
                outcome: ToolOutcome::Unknown,
                output: output.to_owned(),
                archives: Vec::new(),
                images: Vec::new(),
            }),
            ..ToolCallDocument::receiving(format!("call_{id}"), "read_file".to_owned())
        }),
        ..part(id, PartKind::ToolCall, r#"{"path":"a.txt"}"#)
    }
}

fn estimate_slice(turns: &[ModelTurn]) -> u32 {
    turns
        .iter()
        .flat_map(|turn| &turn.parts)
        .map(|part| {
            let text = estimate_text(&part.text);
            let result = part
                .tool_call
                .as_ref()
                .and_then(|call| call.result.as_ref())
                .map_or(0, |result| estimate_text(&result.output));
            text.saturating_add(result).saturating_add(4)
        })
        .fold(0, u32::saturating_add)
}

fn remaining_estimator(
    all: &[ModelTurn],
    budget: u32,
) -> impl FnMut(&CheckpointCursor) -> u32 + '_ {
    move |cursor| {
        let (remaining, anchor) = uncovered_turns(all.to_vec(), Some(cursor), budget);
        let anchor_tokens = anchor.map_or(0, |(_, text)| estimate_text(&text) + 4);
        estimate_slice(&remaining).saturating_add(anchor_tokens)
    }
}

fn size(budget: u32, projected: u32, estimate: u32) -> ContextSize {
    ContextSize {
        budget,
        projected,
        estimate,
    }
}

#[test]
fn compaction_starts_past_80_percent_of_the_budget() {
    assert!(!needed(800, 1_000));
    assert!(needed(801, 1_000));
}

#[test]
fn the_summary_allowance_is_15_percent_within_its_bounds() {
    assert_eq!(summary_allowance(30_000, 128_000), 4_500);
    assert_eq!(
        summary_allowance(200_000, 128_000),
        8_192,
        "capped at 8,192 tokens"
    );
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
fn a_plan_keeps_recent_completed_turns_when_they_fit_under_target() {
    // 10 turns of ~808 tokens each, plus a tiny active latest turn (turn 10).
    let mut all = turns(10, 1_000);
    all.push(turn(10, 10));
    let total = estimate_slice(&all);

    let planned = plan_compaction(
        &all,
        size(10_000, total, total),
        1_024,
        Trigger::Automatic,
        false,
        remaining_estimator(&all, 10_000),
    )
    .unwrap();

    // Target is 45% (4,500): keeping 4 earlier turns (~3,232) + latest (~16)
    // + allowance (1,024) = ~4,272 <= 4,500, so turns 0..=5 are compacted.
    assert_eq!(
        planned.cursor,
        CheckpointCursor {
            turn: 5,
            part: None
        }
    );
    assert_eq!(planned.covered.len(), 6);
    assert!(planned.after <= 4_500, "{}", planned.after);
}

#[test]
fn a_plan_cuts_on_a_settled_step_boundary_inside_a_large_previous_turn() {
    // Turn 0 is a huge 6-step turn (~12K tokens); Turn 1 is a small follow-up.
    let mut parts = vec![part(1, PartKind::UserText, "Configure the wizard")];
    for step in 1..=6_u32 {
        parts.push(tool_step_part(
            step + 1,
            step,
            ToolCallStatus::Completed,
            &"x".repeat(5_000),
        ));
    }
    let big_turn = ModelTurn {
        turn_id: TurnId::parse("0199e0f2-0000-7000-9000-000000000000").unwrap(),
        position: 0,
        run_id: Some(run_id(1)),
        run_state: Some(RunState::Completed),
        kept_runs: Vec::new(),
        parts,
    };
    let follow_up = turn(1, 20);
    let all = vec![big_turn, follow_up];
    let total = estimate_slice(&all);

    let planned = plan_compaction(
        &all,
        size(14_000, total, total),
        1_024,
        Trigger::Automatic,
        false,
        remaining_estimator(&all, 14_000),
    )
    .unwrap();

    // Should cut inside turn 0 rather than wiping all of turn 0.
    assert_eq!(planned.cursor.turn, 0);
    assert!(
        planned.cursor.part.is_some(),
        "expected step-boundary cut inside turn 0, got {:?}",
        planned.cursor
    );
    assert!(planned.after <= 6_300, "{}", planned.after);
}

#[test]
fn a_plan_cuts_inside_an_active_single_turn_with_many_steps() {
    let mut parts = vec![part(1, PartKind::UserText, "Long single-turn task")];
    for step in 1..=6_u32 {
        parts.push(tool_step_part(
            step + 1,
            step,
            ToolCallStatus::Completed,
            &"y".repeat(5_000),
        ));
    }
    let active = ModelTurn {
        turn_id: TurnId::parse("0199e0f2-0000-7000-9000-000000000000").unwrap(),
        position: 0,
        run_id: Some(run_id(1)),
        run_state: Some(RunState::Running),
        kept_runs: Vec::new(),
        parts,
    };
    let all = vec![active];
    let total = estimate_slice(&all);

    let planned = plan_compaction(
        &all,
        size(14_000, total, total),
        1_024,
        Trigger::Automatic,
        false,
        remaining_estimator(&all, 14_000),
    )
    .unwrap();

    assert_eq!(planned.cursor.turn, 0);
    assert!(planned.cursor.part.is_some());
    assert_ne!(
        planned.cursor.part,
        Some(part_id(7)),
        "keeps at least the newest step"
    );
}

#[test]
fn step_boundaries_never_cross_unsettled_or_streaming_parts() {
    let mut streaming_task = tool_step_part(4, 2, ToolCallStatus::Running, "working");
    streaming_task.state = PartState::Streaming;

    let model_turn = ModelTurn {
        turn_id: TurnId::parse("0199e0f2-0000-7000-9000-000000000000").unwrap(),
        position: 0,
        run_id: Some(run_id(1)),
        run_state: Some(RunState::Running),
        kept_runs: Vec::new(),
        parts: vec![
            part(1, PartKind::UserText, "Run"),
            tool_step_part(2, 1, ToolCallStatus::Completed, "ok 1"),
            tool_step_part(3, 1, ToolCallStatus::Completed, "ok 2"),
            streaming_task,
            tool_step_part(5, 3, ToolCallStatus::Completed, "later"),
        ],
    };

    // Only index 2 (end of settled attempt 1) is safe; attempt 1's index 1 is
    // mid-step, and indices >= 3 are at or after a streaming part.
    assert_eq!(step_boundaries(&model_turn), vec![2]);
}

#[test]
fn slice_between_and_uncovered_turns_match_across_step_checkpoints() {
    let mut parts = vec![part(1, PartKind::UserText, "Install and configure")];
    for step in 1..=4_u32 {
        parts.push(tool_step_part(
            step + 1,
            step,
            ToolCallStatus::Completed,
            &format!("step {step}"),
        ));
    }
    let full = vec![ModelTurn {
        turn_id: TurnId::parse("0199e0f2-0000-7000-9000-000000000000").unwrap(),
        position: 0,
        run_id: Some(run_id(1)),
        run_state: Some(RunState::Running),
        kept_runs: Vec::new(),
        parts,
    }];

    let cp1 = CheckpointCursor {
        turn: 0,
        part: Some(part_id(3)),
    };
    let cp2 = CheckpointCursor {
        turn: 0,
        part: Some(part_id(5)),
    };

    let slice1 = slice_between(&full, None, &cp1).unwrap();
    assert_eq!(slice1[0].parts.len(), 3);

    let (uncovered1, anchor1) = uncovered_turns(full.clone(), Some(&cp1), 100_000);
    assert_eq!(uncovered1[0].parts.len(), 2);
    assert!(anchor1.unwrap().1.contains("Install and configure"));

    let slice2_from_full = slice_between(&full, Some(&cp1), &cp2).unwrap();
    let slice2_from_uncovered = slice_between(&uncovered1, None, &cp2).unwrap();
    assert_eq!(
        content_hash(Some("h1"), &slice2_from_full),
        content_hash(Some("h1"), &slice2_from_uncovered)
    );
}

#[test]
fn an_automatic_compaction_that_frees_little_is_skipped_unless_the_request_does_not_fit() {
    let mut all = turns(1, 100);
    all.push(turn(1, 20_000));
    let estimate = estimate_slice(&all);

    assert_eq!(
        plan_compaction(
            &all,
            size(10_000, 9_000, estimate),
            1_024,
            Trigger::Automatic,
            false,
            remaining_estimator(&all, 10_000),
        ),
        None
    );
    assert_eq!(
        plan_compaction(
            &all,
            size(10_000, 10_001, estimate),
            1_024,
            Trigger::Automatic,
            false,
            remaining_estimator(&all, 10_000),
        )
        .map(|plan| plan.cursor),
        Some(CheckpointCursor {
            turn: 0,
            part: None
        }),
        "a request beyond the budget is compacted however little it frees"
    );
    assert_eq!(
        plan_compaction(
            &all,
            size(10_000, 9_000, estimate),
            1_024,
            Trigger::Manual,
            false,
            remaining_estimator(&all, 10_000),
        )
        .map(|plan| plan.cursor),
        Some(CheckpointCursor {
            turn: 0,
            part: None
        })
    );
    assert_eq!(
        plan_compaction(
            &[],
            size(10_000, 9_000, 9_000),
            1_024,
            Trigger::Overflow,
            true,
            |_| 0,
        ),
        None
    );
}

#[test]
fn cut_head_tail_preserves_start_and_end_of_long_outputs() {
    let text = format!("HEADER {} FOOTER_STATE", "m".repeat(5_000));
    let cut = cut_head_tail(&text, 1_200);

    assert!(cut.starts_with("HEADER "));
    assert!(cut.ends_with("FOOTER_STATE"));
    assert!(cut.contains("chars omitted"));
    assert_eq!(cut_head_tail("short", 1_200), "short");
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

    let rendered = render_turns(&[with_reasoning], Requester::Owner, 10_000);

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
fn mentions_reach_summaries_as_head_and_tail_excerpts() {
    let page = format!(
        "The owner mentioned the file a.rs.\n{}\nFINAL_LINE",
        "x".repeat(8_000)
    );
    let mut mentioned = turn(0, 1);
    mentioned.parts.insert(1, part(2, PartKind::Mention, &page));

    let rendered = render_turns(&[mentioned], Requester::Owner, 10_000);

    assert!(
        rendered[0].contains("Owner's mention: The owner mentioned the file a.rs."),
        "{rendered:?}"
    );
    assert!(rendered[0].contains("FINAL_LINE"), "{rendered:?}");
    assert!(rendered[0].contains("chars omitted"), "{rendered:?}");
    assert!(rendered[0].len() < page.len(), "{}", rendered[0].len());
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
fn images_reach_summaries_as_their_size() {
    let mut pictured = turn(0, 1);
    pictured.parts.insert(1, image(2, "digest-a"));

    let rendered = render_turns(&[pictured], Requester::Owner, 10_000);

    assert!(
        rendered[0].contains("Owner's image: 1600×1200 pixels."),
        "{rendered:?}"
    );
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
fn tool_calls_hash_their_results_and_reach_summaries() {
    let call = |output: &str| tool_step_part(2, 1, ToolCallStatus::Completed, output);
    let with_result = |output: &str| {
        let mut turn = turn(0, 10);
        turn.parts.push(call(output));
        vec![turn]
    };

    let short = with_result("short");
    let long = with_result(&"x".repeat(10_000));

    assert_ne!(content_hash(None, &short), content_hash(None, &long));
    assert!(
        render_turns(&short, Requester::Owner, 10_000)[0].contains(&texts::compaction_tool_call(
            "read_file",
            r#"{"path":"a.txt"}"#,
            "short"
        )),
        "{:?}",
        render_turns(&short, Requester::Owner, 10_000)
    );
}

#[test]
fn task_calls_reach_summaries_and_the_hash_covers_their_results() {
    use mannie_types::tasks::{AwaitedTask, TaskPartDocument, TaskState};

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

    let rendered = render_turns(&short, Requester::Owner, 10_000);
    assert!(
        rendered[0].contains(&texts::compaction_task_call(
            "wait_for_tasks",
            r#"{"tasks":[2]}"#,
            "Task 2 reported:\n\nAll green."
        )),
        "{rendered:?}"
    );
    assert_ne!(content_hash(None, &short), content_hash(None, &other));
}
