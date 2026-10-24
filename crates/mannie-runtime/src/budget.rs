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

/// The estimated tokens of one message.
pub(crate) fn estimate_message(message: &Message) -> u32 {
    message
        .content
        .iter()
        .map(|content| match content {
            Content::Text(text) | Content::ProviderItem(text) => estimate_text(text),
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

    /// The tokens a request is expected to use. With a report from a call of
    /// the same binding and checkpoint, the reported input is the base, and
    /// only what was added since is estimated, scaled by how far the estimate
    /// was off on that call.
    pub(crate) fn project(&self, binding: &str, checkpoint: Option<&str>, estimate: u32) -> u32 {
        let same_context =
            self.binding.as_deref() == Some(binding) && self.checkpoint.as_deref() == checkpoint;
        let (Some(reported), Some(estimated)) = (self.reported_input, self.estimated_input) else {
            return estimate;
        };
        if !same_context || estimated == 0 {
            return estimate;
        }
        let ratio = (f64::from(reported) / f64::from(estimated)).clamp(0.5, 2.0);
        let added = f64::from(estimate.saturating_sub(estimated)) * ratio;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the added estimate is non-negative and capped at u32::MAX below"
        )]
        let added = added.ceil().min(f64::from(u32::MAX)) as u32;
        reported.saturating_add(added)
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
    }
}

#[cfg(test)]
mod tests {
    use mannie_provider::{Content, Message, ModelRequest, Role, ToolChoice, ToolDefinition};
    use mannie_types::provider::ReasoningEffort;

    use super::{
        Ledger, estimate_image, estimate_message, estimate_request, estimate_scaled_image,
        estimate_text,
    };

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
    fn projections_use_reported_counts_for_the_same_context_only() {
        let ledger = Ledger {
            binding: Some("p:1".to_owned()),
            checkpoint: None,
            reported_input: Some(1_000),
            estimated_input: Some(1_250),
        };

        assert_eq!(ledger.project("p:1", None, 1_750), 1_400);
        assert_eq!(ledger.project("p:2", None, 1_750), 1_750, "another binding");
        assert_eq!(
            ledger.project("p:1", Some("c"), 1_750),
            1_750,
            "another checkpoint"
        );
        assert_eq!(Ledger::default().project("p:1", None, 900), 900);
    }

    #[test]
    fn a_report_that_grew_less_than_the_request_shows_a_cut() {
        let ledger = Ledger {
            binding: Some("p:1".to_owned()),
            checkpoint: None,
            reported_input: Some(3_900),
            estimated_input: Some(6_000),
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
        };

        assert_eq!(Ledger::parse(Some(&ledger.to_json())), ledger);
        assert_eq!(Ledger::parse(Some("not json")), Ledger::default());
        assert_eq!(Ledger::parse(None), Ledger::default());
    }
}
