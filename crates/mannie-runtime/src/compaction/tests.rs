use mannie_store::{ModelPart, ModelTurn};
use mannie_types::ids::{PartId, TurnId};
use mannie_types::run::RunState;
use mannie_types::transcript::{PartKind, PartState};

use super::{
    ContextSize, Plan, Trigger, chunks, content_hash, fit, needed, plan, render_turns,
    summary_allowance, turns_estimate,
};
use crate::budget::estimate_text;
use crate::texts;

fn part(id: u32, kind: PartKind, text: &str) -> ModelPart {
    ModelPart {
        id: PartId::parse(&format!("0199e0f2-0000-7000-8000-{id:012}")).unwrap(),
        kind: Some(kind),
        kind_name: kind.as_str().to_owned(),
        state: PartState::Complete,
        text: text.to_owned(),
        provider_item: None,
    }
}

/// A complete turn whose question and answer each have `bytes` bytes.
fn turn(position: u32, bytes: usize) -> ModelTurn {
    ModelTurn {
        turn_id: TurnId::parse(&format!("0199e0f2-0000-7000-9000-{position:012}")).unwrap(),
        position,
        run_state: Some(RunState::Completed),
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
    let per_turn = turns_estimate(&candidates[..1]);
    let projected = per_turn * 10;

    let planned = plan(
        &candidates,
        size(10_000, projected),
        1_024,
        Trigger::Automatic,
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
    let per_turn = turns_estimate(&candidates[..1]);

    let planned = plan(
        &candidates,
        size(12_000, per_turn * 3),
        1_024,
        Trigger::Automatic,
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
            Trigger::Automatic
        ),
        None
    );
    assert_eq!(
        plan(&candidates, size(10_000, 10_001), 1_024, Trigger::Automatic).map(|plan| plan.covered),
        Some(1),
        "a request beyond the budget is compacted however little it frees"
    );
    assert_eq!(
        plan(&candidates, size(10_000, projected), 1_024, Trigger::Manual).map(|plan| plan.covered),
        Some(1)
    );
    assert_eq!(
        plan(&[], size(10_000, projected), 1_024, Trigger::Overflow),
        None
    );
}

#[test]
fn freed_tokens_follow_the_calibration_of_the_projection() {
    let candidates = turns(4, 2_000);
    let estimate = turns_estimate(&candidates);
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

    let by_estimate = plan(&candidates, uncalibrated, 500, Trigger::Manual);
    let by_count = plan(&candidates, calibrated, 500, Trigger::Manual);

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

    let rendered = render_turns(&[with_reasoning]);

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
