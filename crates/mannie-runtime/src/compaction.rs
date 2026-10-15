//! Compaction (docs/features/chat/conversations.md, context budget and
//! compaction): older turns are summarized into a checkpoint that stands in
//! for them in the model's context. This module plans what a checkpoint
//! covers, renders the covered turns for the summary call, and hashes them;
//! `calls.rs` makes the calls and stores the checkpoint.

use std::collections::HashSet;

use mannie_provider::{Message, ModelRequest, Role, ToolChoice};
use mannie_store::{ModelPart, ModelTurn};
use mannie_types::provider::ReasoningEffort;
use mannie_types::transcript::PartKind;
use sha2::{Digest, Sha256};

use crate::budget::{estimate_message, estimate_text};
use crate::context::call_result;
use crate::texts;

/// The most characters of a tool call's arguments, and of its result, that a
/// summary call reads.
const MAX_SUMMARY_CALL_CHARS: usize = 500;

/// Compaction starts when the projected context passes this share of the
/// budget, in percent.
const TRIGGER_PERCENT: u32 = 80;

/// Compaction aims to leave the context at this share of the budget.
const TARGET_PERCENT: u32 = 30;

/// An automatic compaction that would free less than this share of the
/// budget is skipped.
const MIN_FREED_PERCENT: u32 = 10;

/// The newest complete turns that compaction keeps when it can.
const KEPT_TURNS: [usize; 3] = [2, 1, 0];

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
    fn calibrate(self, estimate: u32) -> u32 {
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

/// The tokens a summary may use: 15% of the budget, at least 1,024, and
/// never more than the profile's output limit.
pub(crate) fn summary_allowance(budget: u32, output_limit: u32) -> u32 {
    share(budget, 15).max(1_024).min(output_limit)
}

/// What a compaction covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Plan {
    /// How many of the candidate turns, from the oldest, the checkpoint
    /// covers.
    pub(crate) covered: usize,
    /// The projected context after the compaction.
    pub(crate) after: u32,
}

/// Plans a compaction of `candidates`: the complete turns after the current
/// checkpoint, oldest first, never the turn being answered. It keeps the
/// newest two turns when that reaches the target, then one, then none.
/// `None` means there is nothing worth compacting: no candidates, or an
/// automatic compaction of a request that fits and would free less than 10%
/// of the budget.
pub(crate) fn plan(
    candidates: &[ModelTurn],
    size: ContextSize,
    allowance: u32,
    trigger: Trigger,
) -> Option<Plan> {
    let target = share(size.budget, TARGET_PERCENT);
    let mut best = None;
    for keep in KEPT_TURNS {
        if candidates.len() <= keep {
            continue;
        }
        let covered = candidates.len() - keep;
        let freed = size.calibrate(turns_estimate(&candidates[..covered]));
        let after = size
            .projected
            .saturating_sub(freed)
            .saturating_add(allowance);
        best = Some((covered, freed, after));
        if after <= target {
            break;
        }
    }
    let (covered, freed, after) = best?;
    let freed_net = freed.saturating_sub(allowance);
    // A request that does not fit the budget is compacted however little it
    // frees; otherwise the call would only fail with an overflow.
    let worth_it = trigger != Trigger::Automatic
        || size.projected > size.budget
        || freed_net >= share(size.budget, MIN_FREED_PERCENT);
    worth_it.then_some(Plan { covered, after })
}

/// The estimated tokens of turns as the model reads them. A tool call counts
/// its arguments and its result.
pub(crate) fn turns_estimate(turns: &[ModelTurn]) -> u32 {
    turns
        .iter()
        .flat_map(|turn| &turn.parts)
        .map(|part| {
            let text = estimate_message(&Message::text(Role::User, part.text.clone()));
            let result = tool_result(part).map_or(0, |result| estimate_text(&result));
            text.saturating_add(result)
        })
        .fold(0, u32::saturating_add)
}

/// What the model reads as the result of a tool call part.
fn tool_result(part: &ModelPart) -> Option<String> {
    part.tool_call
        .as_ref()
        .map(|call| call_result(&part.id, call, &HashSet::new()))
}

/// The covered turns as the summary call reads them: the owner's lines and
/// the assistant's, in order. Reasoning stays out; it was the model's own.
pub(crate) fn render_turns(turns: &[ModelTurn]) -> Vec<String> {
    turns
        .iter()
        .map(|turn| {
            let mut lines = Vec::new();
            for part in &turn.parts {
                let speaker = match part.kind {
                    Some(PartKind::UserText) => texts::COMPACTION_OWNER,
                    Some(PartKind::Text) => texts::COMPACTION_ASSISTANT,
                    Some(PartKind::ToolCall) => {
                        if let (Some(call), Some(result)) = (&part.tool_call, tool_result(part)) {
                            lines.push(texts::compaction_tool_call(
                                &call.tool,
                                &cut(&part.text),
                                &cut(&result),
                            ));
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
        })
        .filter(|turn| !turn.is_empty())
        .collect()
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
        // A summary needs no deliberate reasoning; the server's default keeps
        // the call as the server would answer any request.
        reasoning_effort: ReasoningEffort::ServerDefault,
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
    }
}

/// A text cut for a summary call.
fn cut(text: &str) -> String {
    let mut characters = text.trim().chars();
    let kept: String = characters.by_ref().take(MAX_SUMMARY_CALL_CHARS).collect();
    if characters.next().is_some() {
        format!("{kept} [...]")
    } else {
        kept
    }
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
            hasher.update([0x1e]);
        }
        hasher.update([0x1d]);
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
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
