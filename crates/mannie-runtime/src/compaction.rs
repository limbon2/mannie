//! Compaction (docs/features/chat/conversations.md, context budget and
//! compaction; ADR-0083): older turns, or settled earlier steps of a long turn,
//! are summarized into a checkpoint that stands in for them in the model's
//! context. This module plans what a checkpoint covers, slices turns across
//! checkpoint cursors, renders the covered turns for the summary call, and
//! hashes them; `calls.rs` makes the calls and stores the checkpoint.

use std::collections::HashSet;

use mannie_provider::{Message, ModelRequest, Role, ToolChoice};
use mannie_store::{ModelPart, ModelTurn, StoredCheckpoint};
use mannie_types::ids::{PartId, TurnId};
use mannie_types::provider::ReasoningEffort;
use mannie_types::tools::ToolCallStatus;
use mannie_types::transcript::{PartKind, PartState};
use sha2::{Digest, Sha256};

use crate::agents::Requester;
use crate::budget::{budget_bytes, estimate_text};
use crate::context::{part_result, with_unsent_images};
use crate::plans::holds_saved_plan;
use crate::questions::compaction_lines;
use crate::{tasks, texts};

/// The smallest per-call character limit that a summary call reads for tool
/// arguments and results (ADR-0083).
const MIN_SUMMARY_CALL_CHARS: usize = 1_000;

/// The largest per-call character limit that a summary call reads for tool
/// arguments and results (ADR-0083).
const MAX_SUMMARY_CALL_CHARS: usize = 4_000;

/// Compaction starts when the projected context passes this share of the
/// budget, in percent.
pub(crate) const TRIGGER_PERCENT: u32 = 80;

/// Compaction aims to leave the context at this share of the budget (ADR-0083).
pub(crate) const TARGET_PERCENT: u32 = 45;

/// Automatic and manual compaction prefer keeping at least one recent completed
/// turn or step when doing so stays within this share of the budget (ADR-0083).
const KEEP_RECENT_CEILING_PERCENT: u32 = 60;

/// An automatic compaction that would free less than this share of the
/// budget is skipped unless the request exceeds the budget.
const MIN_FREED_PERCENT: u32 = 10;

/// Minimum token allowance for a checkpoint summary.
const MIN_SUMMARY_ALLOWANCE: u32 = 1_024;

/// Cap on the token allowance for a checkpoint summary (ADR-0083).
const MAX_SUMMARY_ALLOWANCE: u32 = 8_192;

/// Maximum share of the budget kept verbatim in a resumed-turn anchor message.
const MAX_ANCHOR_PERCENT: u32 = 10;

/// Minimum token budget kept for a resumed-turn anchor message.
const MIN_ANCHOR_TOKENS: u32 = 512;

/// Maximum whole earlier turns kept by automatic compaction when they fit.
const MAX_KEPT_TURNS_AUTO: usize = 4;

/// Maximum whole earlier turns kept by manual or overflow compaction.
const MAX_KEPT_TURNS_COMPACT: usize = 2;

/// Why a compaction happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// The projected context passed the threshold.
    Automatic,
    /// The provider reported a context overflow.
    Overflow,
    /// The owner asked for it.
    Manual,
}

impl Trigger {
    /// The trigger's name, as the checkpoint records it.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Overflow => "overflow",
            Self::Manual => "manual",
        }
    }
}

/// Where a checkpoint ends in a conversation: at the end of a turn (`part` is
/// `None`) or after a settled step's last part inside `turn` (ADR-0083).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckpointCursor {
    /// The position of the last covered turn.
    pub(crate) turn: u32,
    /// The last covered part inside `turn`, or `None` when the whole turn is
    /// covered.
    pub(crate) part: Option<PartId>,
}

impl CheckpointCursor {
    /// The cursor of a stored checkpoint.
    pub(crate) fn from_stored(checkpoint: &StoredCheckpoint) -> Self {
        Self {
            turn: checkpoint.covers_through_turn,
            part: checkpoint.covers_through_part.clone(),
        }
    }

    /// The `after_position` argument to `Store::model_turns` that loads every
    /// turn needed to read content after this cursor.
    pub(crate) fn load_after_turn(&self) -> Option<u32> {
        match &self.part {
            None => Some(self.turn),
            Some(_) => self.turn.checked_sub(1),
        }
    }
}

/// The size of a context, in tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContextSize {
    /// The context budget of the profile.
    pub(crate) budget: u32,
    /// The projected size: reported counts where known, estimates otherwise.
    pub(crate) projected: u32,
    /// Mannie's own estimate of the same context.
    pub(crate) estimate: u32,
}

impl ContextSize {
    /// Converts an estimate into the units of the projection, which the
    /// provider's reported counts calibrate.
    pub(crate) fn calibrate(self, estimate: u32) -> u32 {
        if self.estimate == 0 {
            return estimate;
        }
        let scaled = u64::from(estimate) * u64::from(self.projected) / u64::from(self.estimate);
        u32::try_from(scaled).unwrap_or(u32::MAX)
    }
}

/// `percent` percent of `value`.
pub(crate) fn share(value: u32, percent: u32) -> u32 {
    u32::try_from(u64::from(value) * u64::from(percent) / 100).unwrap_or(u32::MAX)
}

/// Whether a projected context needs compaction.
pub(crate) fn needed(projected: u32, budget: u32) -> bool {
    projected > share(budget, TRIGGER_PERCENT)
}

/// The tokens a summary may use: 15% of the budget, clamped between 1,024 and
/// 8,192 tokens, and never more than the profile's output limit (ADR-0083).
pub(crate) fn summary_allowance(budget: u32, output_limit: u32) -> u32 {
    share(budget, 15)
        .clamp(MIN_SUMMARY_ALLOWANCE, MAX_SUMMARY_ALLOWANCE)
        .min(output_limit)
}

/// What a compaction covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
    /// Where the new checkpoint ends.
    pub(crate) cursor: CheckpointCursor,
    /// The newly covered turn and part slices to summarize and hash.
    pub(crate) covered: Vec<ModelTurn>,
    /// The projected context after the compaction, assuming a summary of
    /// `allowance` tokens.
    pub(crate) after: u32,
}

/// Extracts the exact turns and part slices strictly after `previous` up
/// through `target`. Returns `None` if a referenced boundary part is missing
/// or out of order, invalidating a stale checkpoint safely (ADR-0083).
pub(crate) fn slice_between(
    turns: &[ModelTurn],
    previous: Option<&CheckpointCursor>,
    target: &CheckpointCursor,
) -> Option<Vec<ModelTurn>> {
    let start_turn = match previous {
        None => 0,
        Some(prev) if prev.part.is_some() => prev.turn,
        Some(prev) => prev.turn.checked_add(1)?,
    };
    if start_turn > target.turn {
        return None;
    }

    let mut found_prev_part = previous.is_none_or(|prev| prev.part.is_none());
    let mut found_target_part = target.part.is_none();
    let mut covered = Vec::new();

    for turn in turns {
        if turn.position < start_turn || turn.position > target.turn {
            continue;
        }

        let start_idx = match previous {
            Some(prev) if prev.turn == turn.position => match &prev.part {
                Some(prev_part) => {
                    let pos = turn.parts.iter().position(|part| part.id == *prev_part)?;
                    found_prev_part = true;
                    pos + 1
                }
                None => 0,
            },
            _ => 0,
        };

        let end_exclusive = if turn.position == target.turn {
            match &target.part {
                Some(target_part) => {
                    let pos = turn.parts.iter().position(|part| part.id == *target_part)?;
                    found_target_part = true;
                    pos + 1
                }
                None => turn.parts.len(),
            }
        } else {
            turn.parts.len()
        };

        if start_idx > end_exclusive {
            return None;
        }
        if start_idx == end_exclusive {
            continue;
        }

        let mut sliced = turn.clone();
        sliced.parts = turn.parts[start_idx..end_exclusive].to_vec();
        covered.push(sliced);
    }

    if !found_prev_part || !found_target_part {
        return None;
    }

    Some(covered)
}

/// Extracts the turns and parts uncovered by `checkpoint`, together with the
/// user anchor message for a turn whose earlier steps are covered by a
/// step-boundary checkpoint (ADR-0083).
pub(crate) fn uncovered_turns(
    turns: Vec<ModelTurn>,
    checkpoint: Option<&CheckpointCursor>,
    budget: u32,
) -> (Vec<ModelTurn>, Option<(TurnId, String)>) {
    let Some(cursor) = checkpoint else {
        return (turns, None);
    };
    let Some(cut_part) = &cursor.part else {
        let remaining = turns
            .into_iter()
            .filter(|turn| turn.position > cursor.turn)
            .collect();
        return (remaining, None);
    };

    let has_later_turn = turns.iter().any(|turn| turn.position > cursor.turn);
    let mut remaining = Vec::with_capacity(turns.len());
    let mut anchor = None;

    for mut turn in turns {
        if turn.position < cursor.turn {
            continue;
        }
        if turn.position == cursor.turn {
            let owner_text = owner_request_anchor_text(&turn, budget);
            if let Some(pos) = turn.parts.iter().position(|part| part.id == *cut_part) {
                turn.parts.drain(..=pos);
            } else {
                turn.parts.clear();
            }
            if turn.parts.is_empty() && has_later_turn {
                continue;
            }
            anchor = Some((
                turn.turn_id.clone(),
                texts::resumed_turn_anchor(owner_text.as_deref()),
            ));
        }
        remaining.push(turn);
    }

    (remaining, anchor)
}

/// Builds a bounded text excerpt of a turn's opening owner request for a
/// resumed-turn anchor message.
fn owner_request_anchor_text(turn: &ModelTurn, budget: u32) -> Option<String> {
    let joined: Vec<&str> = turn
        .parts
        .iter()
        .filter(|part| {
            part.run_id.is_none()
                && matches!(part.kind, Some(PartKind::UserText | PartKind::CommandNote))
        })
        .map(|part| part.text.trim())
        .filter(|text| !text.is_empty())
        .collect();
    if joined.is_empty() {
        return None;
    }
    let combined = joined.join("\n\n");
    let max_tokens = share(budget, MAX_ANCHOR_PERCENT).max(MIN_ANCHOR_TOKENS);
    let max_chars = budget_bytes(max_tokens);
    Some(cut_head_tail(&combined, max_chars))
}

/// Whether a part is settled and safe to include in a checkpoint's content
/// hash (never streaming, and never an in-flight tool or task call).
fn part_is_settled(part: &ModelPart) -> bool {
    if part.state == PartState::Streaming {
        return false;
    }
    let Some(call) = &part.tool_call else {
        return true;
    };
    matches!(
        call.status,
        ToolCallStatus::Completed
            | ToolCallStatus::Denied
            | ToolCallStatus::Uncertain
            | ToolCallStatus::NotRun
            | ToolCallStatus::Canceled
    )
}

/// Returns the part indices inside `turn` that end a settled reply step:
/// never splitting an `(run_id, attempt)` group, never splitting a tool call
/// from its result, and stopping before any streaming or unsettled part
/// (ADR-0083).
pub(crate) fn step_boundaries(turn: &ModelTurn) -> Vec<usize> {
    let mut boundaries = Vec::new();

    for (index, part) in turn.parts.iter().enumerate() {
        if !part_is_settled(part) {
            break;
        }
        if part.run_id.is_none() {
            continue;
        }
        let ends_step = match turn.parts.get(index + 1) {
            None => true,
            Some(next) => next.run_id != part.run_id || next.attempt != part.attempt,
        };
        if ends_step {
            boundaries.push(index);
        }
    }

    boundaries
}

/// Evaluated candidate compaction cut.
struct Candidate {
    cursor: CheckpointCursor,
    freed: u32,
    after: u32,
}

/// Plans what a checkpoint covers across `uncovered` turns (oldest first),
/// considering both whole-turn boundaries and settled step boundaries inside
/// large turns (ADR-0083).
///
/// `estimate_remaining` returns Mannie's uncalibrated token estimate for the
/// request that would remain after cutting at a candidate cursor (excluding the
/// previous and new checkpoint summary), computed with the same projection as
/// real model calls.
pub(crate) fn plan_compaction<F>(
    uncovered: &[ModelTurn],
    size: ContextSize,
    allowance: u32,
    trigger: Trigger,
    allow_active_turn_end: bool,
    mut estimate_remaining: F,
) -> Option<Plan>
where
    F: FnMut(&CheckpointCursor) -> u32,
{
    let (latest, earlier) = uncovered.split_last()?;
    let target = share(size.budget, TARGET_PERCENT);
    let keep_ceiling = share(size.budget, KEEP_RECENT_CEILING_PERCENT);
    let min_freed = share(size.budget, MIN_FREED_PERCENT);

    let worth_it = |freed: u32| {
        trigger != Trigger::Automatic
            || size.projected > size.budget
            || freed.saturating_sub(allowance) >= min_freed
    };

    let mut eval = |cursor: CheckpointCursor| -> Candidate {
        let remaining_est = estimate_remaining(&cursor);
        let freed_est = size.estimate.saturating_sub(remaining_est);
        let freed = size.calibrate(freed_est);
        let after = size
            .projected
            .saturating_sub(freed)
            .saturating_add(allowance);
        Candidate {
            cursor,
            freed,
            after,
        }
    };

    let max_kept = if trigger == Trigger::Automatic {
        MAX_KEPT_TURNS_AUTO
    } else {
        MAX_KEPT_TURNS_COMPACT
    };

    // 1. Try whole-turn cuts across `earlier` while keeping recent completed
    // turns whenever they fit within the target.
    let mut keep_one_fallback: Option<Candidate> = None;
    if earlier.len() >= 2 {
        let top_keep = (earlier.len() - 1).min(max_kept);
        for keep in (1..=top_keep).rev() {
            let covered_count = earlier.len() - keep;
            let cursor = CheckpointCursor {
                turn: earlier[covered_count - 1].position,
                part: None,
            };
            let candidate = eval(cursor);
            if candidate.after <= target && worth_it(candidate.freed) {
                return build_plan(uncovered, candidate);
            }
            if keep == 1
                && trigger != Trigger::Overflow
                && candidate.after <= keep_ceiling
                && worth_it(candidate.freed)
            {
                keep_one_fallback = Some(candidate);
            }
        }
    }

    // 2. Before wiping the most recent completed turn completely, try cutting
    // on a settled internal step boundary inside it so its recent steps stay
    // verbatim (ADR-0083).
    if let Some(prev_turn) = earlier.last() {
        let ceiling = (trigger != Trigger::Overflow).then_some(keep_ceiling);
        let step_cut = best_step_cut(prev_turn, false, target, ceiling, &worth_it, &mut eval);
        if let Some(candidate) = step_cut.or(keep_one_fallback) {
            return build_plan(uncovered, candidate);
        }
    }

    // 3. Evaluate covering all of `earlier` (whole-turn `keep = 0`).
    let whole_earlier = earlier.last().map(|last| {
        eval(CheckpointCursor {
            turn: last.position,
            part: None,
        })
    });
    if let Some(candidate) = &whole_earlier
        && candidate.after <= target
        && worth_it(candidate.freed)
    {
        return build_plan(uncovered, whole_earlier?);
    }

    // 4. When `earlier` is empty or covering all of `earlier` still leaves the
    // context above `target`, cut on a settled step boundary inside `latest`
    // (ADR-0083).
    if let Some(candidate) = best_step_cut(
        latest,
        allow_active_turn_end,
        target,
        Some(u32::MAX),
        &worth_it,
        &mut eval,
    ) {
        return build_plan(uncovered, candidate);
    }

    if let Some(candidate) = whole_earlier
        && worth_it(candidate.freed)
    {
        return build_plan(uncovered, candidate);
    }

    None
}

/// Picks the best settled step-boundary cut inside `turn`: returns the first
/// cut that reaches `target`, or otherwise the deepest cut that stays within
/// `fallback_ceiling` (when set).
fn best_step_cut<W, E>(
    turn: &ModelTurn,
    include_end: bool,
    target: u32,
    fallback_ceiling: Option<u32>,
    worth_it: &W,
    eval: &mut E,
) -> Option<Candidate>
where
    W: Fn(u32) -> bool,
    E: FnMut(CheckpointCursor) -> Candidate,
{
    let mut deepest: Option<Candidate> = None;

    for index in step_boundaries(turn) {
        if !include_end && index + 1 >= turn.parts.len() {
            continue;
        }
        let cursor = CheckpointCursor {
            turn: turn.position,
            part: Some(turn.parts[index].id.clone()),
        };
        let candidate = eval(cursor);
        if candidate.after <= target && worth_it(candidate.freed) {
            return Some(candidate);
        }
        if fallback_ceiling.is_some_and(|ceiling| candidate.after <= ceiling)
            && worth_it(candidate.freed)
        {
            deepest = Some(candidate);
        }
    }

    deepest
}

fn build_plan(uncovered: &[ModelTurn], candidate: Candidate) -> Option<Plan> {
    let covered = slice_between(uncovered, None, &candidate.cursor)?;
    if covered.is_empty() {
        return None;
    }
    Some(Plan {
        cursor: candidate.cursor,
        covered,
        after: candidate.after,
    })
}

/// What the model reads as the result of a call part: a tool call or a task.
/// The summary's model reads no image of a result, only its size (ADR-0044,
/// decision 5).
fn call_result(part: &ModelPart, requester: Requester) -> Option<String> {
    part.tool_call.as_ref().map(|call| {
        with_unsent_images(
            part_result(part, call, &HashSet::new(), requester),
            call,
            &[],
        )
    })
}

/// The covered turns as the summary call reads them: the owner's lines,
/// assistant replies, and tool calls in order, using an adaptive head-and-tail
/// character limit sized to `chunk_tokens` and boosted for the newest covered
/// turns so live state survives compaction (ADR-0083). Reasoning stays out.
pub(crate) fn render_turns(
    turns: &[ModelTurn],
    requester: Requester,
    chunk_tokens: u32,
) -> Vec<String> {
    // The model edits a plan from its newest text, so that call stays whole, as
    // an answer does (ADR-0060, decision 9).
    let newest_plan = turns
        .iter()
        .rev()
        .flat_map(|turn| turn.parts.iter().rev())
        .find(|part| part.tool_call.as_ref().is_some_and(holds_saved_plan))
        .map(|part| part.id.clone());

    let call_count: usize = turns
        .iter()
        .flat_map(|turn| &turn.parts)
        .filter(|part| part.kind.is_some_and(PartKind::is_call))
        .count();
    let base_limit = adaptive_call_limit(chunk_tokens, call_count);
    let recent_start = turns.len().saturating_sub((turns.len() / 4).max(1));

    turns
        .iter()
        .enumerate()
        .map(|(turn_index, turn)| {
            let char_limit = if turn_index >= recent_start {
                base_limit.saturating_mul(2).min(MAX_SUMMARY_CALL_CHARS)
            } else {
                base_limit
            };
            render_one_turn(turn, requester, newest_plan.as_ref(), char_limit)
        })
        .filter(|turn| !turn.is_empty())
        .collect()
}

fn adaptive_call_limit(chunk_tokens: u32, call_count: usize) -> usize {
    if call_count == 0 {
        return MAX_SUMMARY_CALL_CHARS;
    }
    // Allocate up to half of one summary chunk across all tool calls (two
    // fields each: arguments and result).
    let available_chars = budget_bytes(chunk_tokens) / 2;
    let per_field = available_chars / (call_count.saturating_mul(2).max(1));
    per_field.clamp(MIN_SUMMARY_CALL_CHARS, MAX_SUMMARY_CALL_CHARS)
}

fn render_one_turn(
    turn: &ModelTurn,
    requester: Requester,
    newest_plan: Option<&PartId>,
    char_limit: usize,
) -> String {
    let mut lines = Vec::new();
    for part in &turn.parts {
        let speaker = match part.kind {
            Some(PartKind::UserText) => texts::COMPACTION_OWNER,
            Some(PartKind::Text) => texts::COMPACTION_ASSISTANT,
            Some(kind @ (PartKind::ToolCall | PartKind::Task)) => {
                // The owner's answers to questions stay whole, as the
                // owner's messages do (ADR-0058, decision 7).
                let answers = part.tool_call.as_ref().and_then(compaction_lines);
                if let Some(answers) = answers {
                    lines.extend(answers);
                    continue;
                }
                let result = call_result(part, requester);
                if let (Some(call), Some(result)) = (&part.tool_call, result) {
                    let render = if kind == PartKind::Task {
                        texts::compaction_task_call
                    } else {
                        texts::compaction_tool_call
                    };
                    let arguments = if newest_plan == Some(&part.id) {
                        part.text.clone()
                    } else {
                        cut_head_tail(&part.text, char_limit)
                    };
                    lines.push(render(
                        &call.tool,
                        &arguments,
                        &cut_head_tail(&result, char_limit),
                    ));
                }
                continue;
            }
            // A mention's text can be a whole page, so the summary reads a
            // bounded head-and-tail excerpt (ADR-0032, decision 7).
            Some(PartKind::Mention) => {
                lines.push(texts::compaction_mention(&cut_head_tail(
                    &part.text, char_limit,
                )));
                continue;
            }
            // A wake-up's request names the commands that ended (ADR-0041).
            Some(PartKind::CommandNote) => {
                lines.push(texts::compaction_command_note(&cut_head_tail(
                    &part.text, char_limit,
                )));
                continue;
            }
            // What the attachments gave can be long, so the summary reads a
            // bounded excerpt (ADR-0046, decision 4).
            Some(PartKind::Context) => {
                lines.push(texts::compaction_context(&cut_head_tail(
                    &part.text, char_limit,
                )));
                continue;
            }
            // The summary's model reads no image, only its size
            // (ADR-0033, decision 6).
            Some(PartKind::Image) => {
                if let Some(image) = &part.image {
                    lines.push(texts::compaction_image(image.width, image.height));
                }
                continue;
            }
            _ => continue,
        };
        if !part.text.trim().is_empty() {
            lines.push(format!("{speaker} {}", part.text.trim()));
        }
    }
    lines.join("\n\n")
}

/// Splits rendered turns into chunks that each fit `max_tokens`, oldest
/// first. A single turn larger than that is a chunk of its own.
pub(crate) fn chunks(rendered: &[String], max_tokens: u32) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for turn in rendered {
        let fits = estimate_text(&current).saturating_add(estimate_text(turn)) <= max_tokens;
        if !fits && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(turn);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Shortens a chunk that is larger than `max_tokens` by leaving out its
/// middle: the start of a turn usually holds the request, and its end the
/// conclusion.
pub(crate) fn fit(chunk: &str, max_tokens: u32) -> String {
    if estimate_text(chunk) <= max_tokens {
        return chunk.to_owned();
    }
    // The estimate counts two tokens for every five bytes.
    let max_bytes = usize::try_from(u64::from(max_tokens) * 5 / 2).unwrap_or(usize::MAX);
    let marker = format!("\n\n{}\n\n", texts::COMPACTION_OMITTED);
    let available = max_bytes.saturating_sub(marker.len());
    let head_end = floor_char_boundary(chunk, available * 2 / 3);
    let tail_start = ceil_char_boundary(chunk, chunk.len() - (available - available * 2 / 3));
    format!(
        "{}{marker}{}",
        &chunk[..head_end],
        &chunk[tail_start.max(head_end)..]
    )
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

/// The request that summarizes `transcript`, on top of an earlier summary.
pub(crate) fn summary_request(
    model: &str,
    previous: Option<&str>,
    transcript: &str,
    allowance: u32,
    reasoning_effort: ReasoningEffort,
) -> ModelRequest {
    let mut text = String::new();
    if let Some(previous) = previous.filter(|previous| !previous.trim().is_empty()) {
        text.push_str(texts::COMPACTION_PREVIOUS_SUMMARY);
        text.push_str("\n\n");
        text.push_str(previous.trim());
        text.push_str("\n\n");
    }
    text.push_str(texts::COMPACTION_TRANSCRIPT);
    text.push_str("\n\n");
    text.push_str(transcript);
    text.push_str("\n\n");
    text.push_str(texts::COMPACTION_REQUEST);
    ModelRequest {
        model: model.to_owned(),
        instructions: texts::COMPACTION_INSTRUCTIONS.to_owned(),
        messages: vec![Message::text(Role::User, text)],
        max_output_tokens: allowance,
        reasoning_effort,
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
    }
}

/// Cuts text for a summary call, keeping two-thirds from the head and one-third
/// from the tail so errors, exit summaries and final form states survive
/// (ADR-0083).
pub(crate) fn cut_head_tail(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let total = trimmed.chars().count();
    if total <= max_chars {
        return trimmed.to_owned();
    }
    let head_chars = max_chars * 2 / 3;
    let tail_chars = max_chars.saturating_sub(head_chars);
    let omitted = total.saturating_sub(head_chars + tail_chars);
    let head: String = trimmed.chars().take(head_chars).collect();
    let tail: String = trimmed.chars().skip(total - tail_chars).collect();
    format!("{head} [... {omitted} chars omitted ...] {tail}")
}

/// The tokens a summary request needs besides its transcript: the
/// instructions, the headings and a previous summary of up to `allowance`
/// tokens.
pub(crate) fn request_overhead(previous: Option<&str>, allowance: u32) -> u32 {
    let headings = [
        texts::COMPACTION_PREVIOUS_SUMMARY,
        texts::COMPACTION_TRANSCRIPT,
        texts::COMPACTION_REQUEST,
    ]
    .iter()
    .map(|heading| estimate_text(heading))
    .fold(0, u32::saturating_add);
    let previous = estimate_text(previous.unwrap_or_default()).max(allowance);
    // Two messages' framing: the instructions and the request.
    estimate_text(texts::COMPACTION_INSTRUCTIONS)
        .saturating_add(headings)
        .saturating_add(previous)
        .saturating_add(8)
}

/// The hash of what a checkpoint covers: the model's view of its turns, after
/// the previous checkpoint's hash. A change in any covered part changes it.
pub(crate) fn content_hash(previous: Option<&str>, turns: &[ModelTurn]) -> String {
    let mut hasher = Sha256::new();
    if let Some(previous) = previous {
        hasher.update(previous.as_bytes());
        hasher.update([0x1d]);
    }
    for turn in turns {
        for part in &turn.parts {
            hasher.update(part.kind_name.as_bytes());
            hasher.update([0x1f]);
            hasher.update(part.state.as_str().as_bytes());
            hasher.update([0x1f]);
            hasher.update(part.text.as_bytes());
            // A tool call also covers what it did. Other parts hash as they
            // did before tools existed, so older checkpoints stay valid.
            if let Some(call) = &part.tool_call {
                hasher.update([0x1f]);
                hasher.update(serde_json::to_vec(&call.status).unwrap_or_default());
                hasher.update([0x1f]);
                if let Some(result) = &call.result {
                    hasher.update(result.output.as_bytes());
                }
            }
            // An image part covers its image through the image's digest.
            if let Some(image) = &part.image {
                hasher.update([0x1f]);
                hasher.update(image.digest.as_bytes());
            }
            // A task part covers its tasks' numbers and the result that the
            // model read, which hold whatever matters of its task.
            if let Some(section) = &part.task {
                hasher.update([0x1f]);
                for number in tasks::reported_numbers(section) {
                    hasher.update(number.to_be_bytes());
                }
                hasher.update([0x1f]);
                if let Some(result) = tasks::task_result(section) {
                    hasher.update(result.as_bytes());
                }
            }
            hasher.update([0x1e]);
        }
        hasher.update([0x1d]);
    }
    to_hex(&hasher.finalize())
}

/// Bytes in hexadecimal, two lowercase digits each.
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(hex_digit(byte >> 4));
        hex.push(hex_digit(byte & 0x0f));
    }
    hex
}

fn hex_digit(value: u8) -> char {
    char::from_digit(u32::from(value), 16).unwrap_or('0')
}

#[cfg(test)]
mod tests;
