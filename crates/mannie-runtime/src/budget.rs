//! The context budget (docs/features/chat/conversations.md, context budget and
//! compaction): estimates before a conversation's first reply, and the token
//! counts the provider reported after it, which are the truth (ADR-0012,
//! decision 7).

use mannie_provider::{Content, Message, ModelRequest, ToolDefinition};
use mannie_types::images::MAX_IMAGE_SIDE;
use mannie_types::transcript::ContextUsage;
use serde::{Deserialize, Serialize};

/// Tokens counted for each message on top of its text: the role and the
/// framing that every API adds.
const MESSAGE_OVERHEAD: u32 = 4;

/// Baseline framing tokens for an encrypted reasoning item whose ciphertext
/// bytes are not token text (ADR-0083).
const PROVIDER_ITEM_OVERHEAD: u32 = 32;

/// Cap for non-JSON provider items so an opaque blob cannot dominate estimates.
const MAX_OPAQUE_PROVIDER_ITEM_TOKENS: u32 = 256;

/// Lower bound on the calibrated ratio (`reported / estimated`).
const MIN_CALIBRATION_RATIO: f64 = 0.25;

/// Upper bound on the calibrated ratio (`reported / estimated`).
const MAX_CALIBRATION_RATIO: f64 = 2.0;

/// The growth of a request, in tokens, below which an estimate's error hides
/// whether a server cut it.
const MEASURABLE_GROWTH: u32 = 64;

/// A conservative estimate of the tokens of a text: two tokens for every five
/// bytes. English averages about four bytes per token, and the estimate stays
/// above the count for most other languages and for code.
pub(crate) fn estimate_text(text: &str) -> u32 {
    let bytes = u64::try_from(text.len()).unwrap_or(u64::MAX);
    u32::try_from((bytes * 2).div_ceil(5)).unwrap_or(u32::MAX)
}

/// A conservative estimate of the tokens of an image: its pixels divided by
/// 750, as Anthropic documents its count, which is above what OpenAI counts
/// for the same image at its detailed reading. Never the text of its `data:`
/// URL (ADR-0033, decision 5).
pub(crate) fn estimate_image(width: u32, height: u32) -> u32 {
    let pixels = u64::from(width) * u64::from(height);
    u32::try_from(pixels.div_ceil(750))
        .unwrap_or(u32::MAX)
        .saturating_add(MESSAGE_OVERHEAD)
}

/// The estimated tokens of an image of a tool's result, which providers read
/// scaled down to [`MAX_IMAGE_SIDE`] pixels on its longer side (ADR-0044,
/// decision 7). The owner's images are stored within that size already.
pub(crate) fn estimate_scaled_image(width: u32, height: u32) -> u32 {
    let longer = width.max(height);
    if longer <= MAX_IMAGE_SIDE {
        return estimate_image(width, height);
    }
    let scaled = |side: u32| {
        let side = u64::from(side) * u64::from(MAX_IMAGE_SIDE) / u64::from(longer);
        u32::try_from(side).unwrap_or(MAX_IMAGE_SIDE).max(1)
    };
    estimate_image(scaled(width), scaled(height))
}

/// The bytes of a context budget in tokens, the inverse of the estimate of
/// two tokens for every five bytes.
pub(crate) fn budget_bytes(tokens: u32) -> usize {
    usize::try_from(u64::from(tokens) * 5 / 2).unwrap_or(usize::MAX)
}

/// Estimates a provider-private item such as Responses API encrypted reasoning
/// without counting Base64 ciphertext bytes as token text (ADR-0083).
pub(crate) fn estimate_provider_item(item: &str) -> u32 {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(item) else {
        return estimate_text(item).min(MAX_OPAQUE_PROVIDER_ITEM_TOKENS);
    };

    let summary_tokens = value
        .get("summary")
        .and_then(serde_json::Value::as_array)
        .map_or(0, |entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("text").and_then(serde_json::Value::as_str))
                .map(estimate_text)
                .fold(0, u32::saturating_add)
        });
    summary_tokens.saturating_add(PROVIDER_ITEM_OVERHEAD)
}

/// The estimated tokens of one message.
pub(crate) fn estimate_message(message: &Message) -> u32 {
    message
        .content
        .iter()
        .map(|content| match content {
            Content::Text(text) => estimate_text(text),
            Content::ProviderItem(text) => estimate_provider_item(text),
            Content::ToolCall {
                call_id,
                name,
                arguments,
            } => estimate_text(call_id)
                .saturating_add(estimate_text(name))
                .saturating_add(estimate_text(arguments))
                .saturating_add(MESSAGE_OVERHEAD),
            Content::ToolResult {
                call_id,
                output,
                images,
            } => images.iter().fold(
                estimate_text(call_id)
                    .saturating_add(estimate_text(output))
                    .saturating_add(MESSAGE_OVERHEAD),
                |tokens, image| {
                    tokens.saturating_add(estimate_scaled_image(image.width, image.height))
                },
            ),
            Content::Image(image) => estimate_image(image.width, image.height),
        })
        .fold(MESSAGE_OVERHEAD, u32::saturating_add)
}

/// The estimated tokens of a tool definition, which every request with tools
/// carries.
pub(crate) fn estimate_tool(tool: &ToolDefinition) -> u32 {
    estimate_text(&tool.name)
        .saturating_add(estimate_text(&tool.description))
        .saturating_add(estimate_text(&tool.parameters.to_string()))
        .saturating_add(MESSAGE_OVERHEAD)
}

/// The estimated tokens of a whole request's input.
pub(crate) fn estimate_request(request: &ModelRequest) -> u32 {
    let tools = request
        .tools
        .iter()
        .map(estimate_tool)
        .fold(0, u32::saturating_add);
    request.messages.iter().map(estimate_message).fold(
        estimate_text(&request.instructions)
            .saturating_add(MESSAGE_OVERHEAD)
            .saturating_add(tools),
        u32::saturating_add,
    )
}

/// What the runtime remembers about the last model call of a conversation,
/// as a versioned JSON document in the store.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Ledger {
    /// The profile binding of the call.
    pub(crate) binding: Option<String>,
    /// The checkpoint the call's context used.
    pub(crate) checkpoint: Option<String>,
    /// The input tokens the provider reported.
    pub(crate) reported_input: Option<u32>,
    /// What Mannie estimated for the same input.
    pub(crate) estimated_input: Option<u32>,
    /// Calibrated `reported / estimated` ratio in thousandths, preserved across
    /// checkpoints for the same binding so post-checkpoint projections stay
    /// calibrated (ADR-0083).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ratio_milli: Option<u32>,
}

/// Scales `tokens` by `ratio`, rounding up and capping at `u32::MAX`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "scaled token counts are non-negative and capped at u32::MAX"
)]
pub(crate) fn scale_tokens(tokens: u32, ratio: f64) -> u32 {
    let scaled = (f64::from(tokens) * ratio).ceil();
    scaled.clamp(0.0, f64::from(u32::MAX)) as u32
}

/// Encodes a calibration ratio as thousandths.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped calibration ratios are strictly positive and <= 2_000"
)]
fn ratio_to_milli(ratio: f64) -> u32 {
    (ratio.clamp(MIN_CALIBRATION_RATIO, MAX_CALIBRATION_RATIO) * 1_000.0).round() as u32
}

impl Ledger {
    /// Reads the ledger, or an empty one when it is missing or unreadable.
    pub(crate) fn parse(text: Option<&str>) -> Self {
        text.and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_default()
    }

    /// The ledger as the store keeps it.
    pub(crate) fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_unencodable| "{}".to_owned())
    }

    /// Builds a ledger after a model call that reported its input tokens.
    pub(crate) fn from_report(
        binding: String,
        checkpoint: Option<String>,
        reported_input: u32,
        estimated_input: u32,
    ) -> Self {
        let ratio_milli = (estimated_input > 0)
            .then(|| ratio_to_milli(f64::from(reported_input) / f64::from(estimated_input)));
        Self {
            binding: Some(binding),
            checkpoint,
            reported_input: Some(reported_input),
            estimated_input: Some(estimated_input),
            ratio_milli,
        }
    }

    /// Builds a post-checkpoint ledger that carries the calibrated ratio of the
    /// same binding across the compaction boundary (ADR-0083).
    pub(crate) fn after_checkpoint(&self, binding: &str, checkpoint_id: &str) -> Self {
        let ratio_milli = self.calibration_ratio(binding).map(ratio_to_milli);
        Self {
            binding: Some(binding.to_owned()),
            checkpoint: Some(checkpoint_id.to_owned()),
            reported_input: None,
            estimated_input: None,
            ratio_milli,
        }
    }

    /// The calibrated `reported / estimated` ratio for `binding`, when known.
    pub(crate) fn calibration_ratio(&self, binding: &str) -> Option<f64> {
        if self.binding.as_deref() != Some(binding) {
            return None;
        }
        if let (Some(reported), Some(estimated)) = (self.reported_input, self.estimated_input)
            && estimated > 0
        {
            return Some(
                (f64::from(reported) / f64::from(estimated))
                    .clamp(MIN_CALIBRATION_RATIO, MAX_CALIBRATION_RATIO),
            );
        }
        self.ratio_milli.filter(|milli| *milli > 0).map(|milli| {
            (f64::from(milli) / 1_000.0).clamp(MIN_CALIBRATION_RATIO, MAX_CALIBRATION_RATIO)
        })
    }

    /// The tokens a request is expected to use. With a report from a call of
    /// the same binding and checkpoint, the reported input is the base, and
    /// only the delta since is scaled by how far the estimate was off. Across a
    /// checkpoint of the same binding, the preserved calibration ratio scales
    /// the new estimate (ADR-0083).
    pub(crate) fn project(&self, binding: &str, checkpoint: Option<&str>, estimate: u32) -> u32 {
        let same_binding = self.binding.as_deref() == Some(binding);
        if !same_binding {
            return estimate;
        }
        let same_checkpoint = self.checkpoint.as_deref() == checkpoint;
        if same_checkpoint
            && let (Some(reported), Some(estimated)) = (self.reported_input, self.estimated_input)
            && estimated > 0
        {
            let ratio = (f64::from(reported) / f64::from(estimated))
                .clamp(MIN_CALIBRATION_RATIO, MAX_CALIBRATION_RATIO);
            if estimate >= estimated {
                let added = scale_tokens(estimate - estimated, ratio);
                return reported.saturating_add(added);
            }
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "the dropped estimate is non-negative and capped at u32::MAX"
            )]
            let dropped = (f64::from(estimated - estimate) * ratio)
                .floor()
                .clamp(0.0, f64::from(u32::MAX)) as u32;
            return reported.saturating_sub(dropped);
        }
        if let Some(ratio) = self.calibration_ratio(binding) {
            return scale_tokens(estimate, ratio);
        }
        estimate
    }

    /// Whether a call's reported input shows that the server cut its request.
    /// Between two calls of one binding and checkpoint the request only grows,
    /// so its reported size grows by about what was added since this ledger's
    /// call. A server that cuts requests beyond its context instead of failing
    /// reports less: growth of half the addition or less counts as cut.
    pub(crate) fn shows_cut(
        &self,
        binding: &str,
        checkpoint: Option<&str>,
        projected: u32,
        reported: u32,
    ) -> bool {
        let same_context =
            self.binding.as_deref() == Some(binding) && self.checkpoint.as_deref() == checkpoint;
        let Some(previous) = self.reported_input.filter(|_| same_context) else {
            return false;
        };
        let added = projected.saturating_sub(previous);
        added >= MEASURABLE_GROWTH && reported.saturating_add(added / 2) < projected
    }
}

/// The usage that the conversation view shows.
pub(crate) fn usage(used: u32, budget: u32, estimated: bool) -> ContextUsage {
    ContextUsage {
        used_tokens: used,
        budget_tokens: budget,
        estimated,
        results_pruned: false,
        unpruned_tokens: None,
    }
}

/// The usage that the conversation view shows when older tool results of the
/// active turn are trimmed to stay inside the budget (ADR-0083).
pub(crate) fn pruned_usage(
    used: u32,
    budget: u32,
    estimated: bool,
    unpruned_tokens: u32,
) -> ContextUsage {
    ContextUsage {
        used_tokens: used,
        budget_tokens: budget,
        estimated,
        results_pruned: true,
        unpruned_tokens: Some(unpruned_tokens),
    }
}

#[cfg(test)]
mod tests {
    use mannie_provider::{Content, Message, ModelRequest, Role, ToolChoice, ToolDefinition};
    use mannie_types::provider::ReasoningEffort;

    use super::{
        Ledger, estimate_image, estimate_message, estimate_provider_item, estimate_request,
        estimate_scaled_image, estimate_text,
    };

    #[test]
    fn encrypted_reasoning_items_do_not_count_ciphertext_bytes() {
        let ciphertext = "A".repeat(120_000);
        let item = serde_json::json!({
            "type": "reasoning",
            "id": "rs_1",
            "encrypted_content": ciphertext,
            "summary": [{ "type": "summary_text", "text": "Checked the config." }]
        })
        .to_string();

        let tokens = estimate_provider_item(&item);

        assert!(tokens < 200, "expected bounded estimate, got {tokens}");
        assert!(tokens >= estimate_text("Checked the config."));
    }

    #[test]
    fn an_image_counts_its_pixels_never_its_bytes() {
        let image = Content::Image(mannie_provider::ImageInput {
            format: mannie_types::images::ImageFormat::Png,
            width: 2_048,
            height: 1_536,
            data: mannie_types::images::ImageBytes::from(vec![0; 3_000_000]),
        });
        let message = Message {
            role: Role::User,
            content: vec![image],
        };

        assert_eq!(estimate_image(2_048, 1_536), 4_195 + 4);
        assert_eq!(estimate_image(1, 1), 1 + 4);
        assert_eq!(estimate_message(&message), 4 + 4_195 + 4);
    }

    #[test]
    fn an_image_of_a_result_counts_as_the_provider_reads_it_scaled_down() {
        let image = |width, height| mannie_provider::ImageInput {
            format: mannie_types::images::ImageFormat::Png,
            width,
            height,
            data: mannie_types::images::ImageBytes::from(vec![0; 16]),
        };
        let result = Message {
            role: Role::Tool,
            content: vec![Content::ToolResult {
                call_id: "call_1".to_owned(),
                output: "Shot.".to_owned(),
                images: vec![image(4_096, 3_072), image(800, 600)],
            }],
        };

        // 4096 by 3072 reads as 2048 by 1536.
        assert_eq!(
            estimate_scaled_image(4_096, 3_072),
            estimate_image(2_048, 1_536)
        );
        assert_eq!(estimate_scaled_image(800, 600), estimate_image(800, 600));
        assert_eq!(estimate_scaled_image(4_096, 1), estimate_image(2_048, 1));
        assert_eq!(
            estimate_message(&result),
            4 + estimate_text("call_1")
                + estimate_text("Shot.")
                + 4
                + estimate_image(2_048, 1_536)
                + estimate_image(800, 600)
        );
    }

    #[test]
    fn estimates_round_up_two_tokens_per_five_bytes() {
        assert_eq!(estimate_text(""), 0);
        assert_eq!(estimate_text("a"), 1);
        assert_eq!(estimate_text("hello"), 2);
        assert_eq!(estimate_text("日本語"), 4, "nine bytes of three characters");
    }

    #[test]
    fn a_request_counts_its_instructions_and_messages() {
        let request = ModelRequest {
            model: "m".to_owned(),
            instructions: "hello".to_owned(),
            messages: vec![
                Message::text(Role::User, "hello"),
                Message::text(Role::Assistant, ""),
            ],
            max_output_tokens: 1,
            reasoning_effort: ReasoningEffort::ServerDefault,
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
        };

        assert_eq!(estimate_request(&request), (2 + 4) + (2 + 4) + 4);
    }

    #[test]
    fn a_request_counts_its_tools_their_calls_and_their_results() {
        let plain = ModelRequest {
            model: "m".to_owned(),
            instructions: String::new(),
            messages: Vec::new(),
            max_output_tokens: 1,
            reasoning_effort: ReasoningEffort::ServerDefault,
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
        };
        let with_tools = ModelRequest {
            tools: vec![ToolDefinition {
                name: "read_file".to_owned(),
                description: "Reads lines of a text file.".to_owned(),
                parameters: serde_json::json!({ "type": "object", "properties": {} }),
                strict: true,
            }],
            messages: vec![
                Message {
                    role: Role::Assistant,
                    content: vec![Content::ToolCall {
                        call_id: "call_1".to_owned(),
                        name: "read_file".to_owned(),
                        arguments: "{}".to_owned(),
                    }],
                },
                Message {
                    role: Role::Tool,
                    content: vec![Content::ToolResult {
                        call_id: "call_1".to_owned(),
                        output: "x".repeat(1_000),
                        images: Vec::new(),
                    }],
                },
            ],
            ..plain.clone()
        };

        assert!(estimate_request(&with_tools) > estimate_request(&plain) + 400);
    }

    #[test]
    fn projections_calibrate_same_binding_and_preserve_ratio_across_checkpoints() {
        let ledger = Ledger::from_report("p:1".to_owned(), None, 1_000, 1_250);

        assert_eq!(ledger.project("p:1", None, 1_750), 1_400);
        assert_eq!(ledger.project("p:1", None, 750), 600, "shrinks cleanly");
        assert_eq!(ledger.project("p:2", None, 1_750), 1_750, "another binding");
        assert_eq!(
            ledger.project("p:1", Some("c"), 1_750),
            1_400,
            "another checkpoint on the same binding keeps calibration"
        );
        let after = ledger.after_checkpoint("p:1", "c");
        assert_eq!(after.reported_input, None);
        assert_eq!(after.project("p:1", Some("c"), 1_750), 1_400);
        assert_eq!(after.project("p:2", Some("c"), 1_750), 1_750);
        assert_eq!(Ledger::default().project("p:1", None, 900), 900);
    }

    #[test]
    fn a_report_that_grew_less_than_the_request_shows_a_cut() {
        let ledger = Ledger {
            binding: Some("p:1".to_owned()),
            checkpoint: None,
            reported_input: Some(3_900),
            estimated_input: Some(6_000),
            ratio_milli: None,
        };

        assert!(
            !ledger.shows_cut("p:1", None, 4_500, 4_480),
            "it grew by the addition"
        );
        assert!(
            !ledger.shows_cut("p:1", None, 4_500, 4_250),
            "it grew by half the addition, within the estimate's error"
        );
        assert!(
            ledger.shows_cut("p:1", None, 4_500, 4_096),
            "the server stopped at its window"
        );
        assert!(
            ledger.shows_cut("p:1", None, 4_500, 3_900),
            "it did not grow at all"
        );
        assert!(
            !ledger.shows_cut("p:1", None, 3_940, 3_900),
            "the addition is too small to measure"
        );
        assert!(
            !ledger.shows_cut("p:2", None, 4_500, 4_096),
            "another binding"
        );
        assert!(
            !ledger.shows_cut("p:1", Some("c"), 4_500, 4_096),
            "another checkpoint"
        );
        assert!(
            !Ledger::default().shows_cut("p:1", None, 4_500, 1_000),
            "no earlier report"
        );
    }

    #[test]
    fn the_ledger_survives_the_store_and_tolerates_garbage() {
        let ledger = Ledger {
            binding: Some("p:1".to_owned()),
            checkpoint: Some("c".to_owned()),
            reported_input: Some(10),
            estimated_input: Some(12),
            ratio_milli: Some(833),
        };

        assert_eq!(Ledger::parse(Some(&ledger.to_json())), ledger);
        assert_eq!(Ledger::parse(Some("not json")), Ledger::default());
        assert_eq!(Ledger::parse(None), Ledger::default());
    }
}
