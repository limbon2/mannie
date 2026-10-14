//! Quirk profiles: the documented behavior of specific servers, as data
//! (ADR-0011, decision 5; docs/features/chat/providers.md, quirk profiles).
//!
//! A quirk is trusted only when a recorded fixture of the server shows it.
//! Until the owner records fixtures from each server, the table follows the
//! servers' documentation, verified 2026-10-14, and the conformance tests run
//! on constructed fixtures that say so in their manifests.

use mannie_types::provider::{ApiInterface, ServerKind};

/// The field of a Chat Completions request that limits the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputLimitField {
    /// `max_completion_tokens`, which OpenAI's reasoning models require.
    MaxCompletionTokens,
    /// `max_tokens`, which every OpenAI-compatible server accepts.
    MaxTokens,
}

/// How a Chat Completions request carries the reasoning effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortShape {
    /// `reasoning_effort: "low"`.
    ReasoningEffort,
    /// `reasoning: {effort: "low"}`, as OpenRouter takes it.
    ReasoningObject,
    /// Not at all: the server refuses the field.
    Omitted,
}

/// The normalizations that one server needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quirks {
    /// Whether to ask for usage with `stream_options.include_usage`.
    pub stream_options: bool,
    /// The field that limits the answer.
    pub output_limit_field: OutputLimitField,
    /// How the reasoning effort is sent.
    pub effort: EffortShape,
    /// Whether text between `<think>` and `</think>` at the start of the
    /// content is reasoning. Models served without a reasoning parser write
    /// their reasoning this way.
    pub think_tags: bool,
    /// The server's wording of a context overflow, in lower case, for servers
    /// that report it without an error code.
    pub overflow_phrases: &'static [&'static str],
    /// Whether the server shortens prompts that do not fit its context
    /// instead of failing.
    pub silent_truncation: bool,
}

/// The behavior of servers that Mannie does not recognize: the most widely
/// accepted fields.
const GENERIC: Quirks = Quirks {
    stream_options: true,
    output_limit_field: OutputLimitField::MaxTokens,
    effort: EffortShape::ReasoningEffort,
    think_tags: true,
    overflow_phrases: &["context length", "context window", "context size"],
    silent_truncation: false,
};

/// The quirks of a server, or the generic behavior when it is not known.
pub fn quirks_for(interface: ApiInterface, server: Option<ServerKind>) -> Quirks {
    if interface == ApiInterface::Responses {
        // The Responses API has no fields that vary by server in M1.
        return Quirks {
            think_tags: false,
            ..GENERIC
        };
    }
    match server {
        Some(ServerKind::OpenAi) => Quirks {
            output_limit_field: OutputLimitField::MaxCompletionTokens,
            think_tags: false,
            overflow_phrases: &[],
            ..GENERIC
        },
        Some(ServerKind::OpenRouter) => Quirks {
            effort: EffortShape::ReasoningObject,
            ..GENERIC
        },
        Some(ServerKind::Vllm) => Quirks {
            output_limit_field: OutputLimitField::MaxCompletionTokens,
            overflow_phrases: &["maximum context length", "max_model_len"],
            ..GENERIC
        },
        Some(ServerKind::Sglang) => Quirks {
            overflow_phrases: &["context length", "max_total_num_tokens"],
            ..GENERIC
        },
        Some(ServerKind::LmStudio) => Quirks {
            // Some versions refuse the field on Chat Completions.
            effort: EffortShape::Omitted,
            overflow_phrases: &["context length", "context size", "context overflow"],
            ..GENERIC
        },
        Some(ServerKind::Ollama) => Quirks {
            // Ollama cuts a prompt beyond the model's `num_ctx` without an
            // error.
            silent_truncation: true,
            ..GENERIC
        },
        Some(ServerKind::Other) | None => GENERIC,
    }
}

#[cfg(test)]
mod tests {
    use mannie_types::provider::{ApiInterface, ServerKind};

    use super::{EffortShape, OutputLimitField, quirks_for};

    #[test]
    fn openai_uses_its_own_fields_and_no_think_tags() {
        let quirks = quirks_for(ApiInterface::ChatCompletions, Some(ServerKind::OpenAi));

        assert_eq!(
            quirks.output_limit_field,
            OutputLimitField::MaxCompletionTokens
        );
        assert!(!quirks.think_tags);
    }

    #[test]
    fn unknown_servers_get_the_generic_behavior() {
        let unknown = quirks_for(ApiInterface::ChatCompletions, None);
        let other = quirks_for(ApiInterface::ChatCompletions, Some(ServerKind::Other));

        assert_eq!(unknown, other);
        assert_eq!(unknown.output_limit_field, OutputLimitField::MaxTokens);
        assert!(unknown.think_tags);
    }

    #[test]
    fn servers_differ_where_their_documentation_says() {
        assert_eq!(
            quirks_for(ApiInterface::ChatCompletions, Some(ServerKind::OpenRouter)).effort,
            EffortShape::ReasoningObject
        );
        assert_eq!(
            quirks_for(ApiInterface::ChatCompletions, Some(ServerKind::LmStudio)).effort,
            EffortShape::Omitted
        );
        assert!(
            quirks_for(ApiInterface::ChatCompletions, Some(ServerKind::Ollama)).silent_truncation
        );
    }
}
