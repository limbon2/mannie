//! Limits of known models: the defaults a new profile gets for its context
//! window and output limit (docs/features/chat/providers.md, provider
//! profiles).
//!
//! The table lives in code rather than in the documentation, because model
//! names are not words that the documentation checks accept. Each entry names
//! the date it was verified against the provider's model page; check the
//! entries again before relying on them after that.

use mannie_types::provider::ModelLimits;

/// What Mannie knows about a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownModel {
    /// The model's name, without a snapshot date.
    pub name: &'static str,
    /// Its limits.
    pub limits: ModelLimits,
    /// Whether it reasons before it answers.
    pub reasons: bool,
}

const fn model(
    name: &'static str,
    context_window: u32,
    output_limit: u32,
    reasons: bool,
) -> KnownModel {
    KnownModel {
        name,
        limits: ModelLimits {
            context_window,
            output_limit,
        },
        reasons,
    }
}

/// OpenAI's models, verified on <https://developers.openai.com/api/docs/models>
/// on 2026-10-14. Older models are kept while profiles may still name them.
const KNOWN_MODELS: &[KnownModel] = &[
    model("gpt-6-astra", 1_050_000, 128_000, true),
    model("gpt-6.1-sol", 1_050_000, 128_000, true),
    model("gpt-6-luna", 1_050_000, 128_000, true),
    model("gpt-5.1", 400_000, 128_000, true),
    model("gpt-5", 400_000, 128_000, true),
    model("gpt-5-mini", 400_000, 128_000, true),
    model("gpt-5-nano", 400_000, 128_000, true),
    model("gpt-4.1", 1_047_576, 32_768, false),
    model("gpt-4.1-mini", 1_047_576, 32_768, false),
    model("gpt-4.1-nano", 1_047_576, 32_768, false),
    model("gpt-4o", 128_000, 16_384, false),
    model("gpt-4o-mini", 128_000, 16_384, false),
    model("o3", 200_000, 100_000, true),
    model("o3-mini", 200_000, 100_000, true),
    model("o4-mini", 200_000, 100_000, true),
    model("o1", 200_000, 100_000, true),
];

/// Every known model.
pub fn known_models() -> &'static [KnownModel] {
    KNOWN_MODELS
}

/// The known model that a model name refers to: the same name, or the name
/// followed by a snapshot such as `-2025-04-14`. A provider prefix such as
/// `openai/`, as OpenRouter writes it, is ignored. The longest match wins, so
/// `gpt-4.1-mini` is not taken for `gpt-4.1`.
pub fn known_model(name: &str) -> Option<&'static KnownModel> {
    let bare = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
    KNOWN_MODELS
        .iter()
        .filter(|known| {
            let Some(rest) = bare.strip_prefix(known.name) else {
                return false;
            };
            let is_snapshot = rest
                .strip_prefix('-')
                .is_some_and(|date| date.starts_with(|first: char| first.is_ascii_digit()));
            rest.is_empty() || is_snapshot
        })
        .max_by_key(|known| known.name.len())
}

#[cfg(test)]
mod tests {
    use super::{KNOWN_MODELS, known_model};
    use mannie_types::provider::check_limits;

    #[test]
    fn names_and_snapshots_match_the_longest_known_model() {
        let cases = [
            ("gpt-4.1", Some("gpt-4.1")),
            ("gpt-4.1-mini-2025-04-14", Some("gpt-4.1-mini")),
            ("openai/gpt-5-mini", Some("gpt-5-mini")),
            ("GPT-6-Astra", Some("gpt-6-astra")),
            ("gpt-4.1-turbo", None),
            ("o3-pro", None),
            ("qwen3:32b", None),
        ];
        for (name, expected) in cases {
            assert_eq!(
                known_model(name).map(|known| known.name),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn every_known_model_has_limits_that_a_profile_accepts() {
        for known in KNOWN_MODELS {
            assert_eq!(
                check_limits(known.limits.context_window, known.limits.output_limit),
                Ok(()),
                "{}",
                known.name
            );
        }
    }
}
