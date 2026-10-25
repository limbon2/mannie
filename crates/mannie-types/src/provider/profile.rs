//! Provider profiles: one model on one server, with an optional credential in
//! the operating system's credential store.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::clock::UtcMillis;
use crate::ids::ProfileId;
use crate::secret::Secret;

/// The longest profile name, in characters.
pub const MAX_NAME_CHARACTERS: usize = 100;

/// The longest model name, in characters.
pub const MAX_MODEL_CHARACTERS: usize = 200;

/// The smallest context window a profile accepts, in tokens.
pub const MIN_CONTEXT_WINDOW: u32 = 1_024;

/// The largest context window a profile accepts, in tokens.
pub const MAX_CONTEXT_WINDOW: u32 = 10_000_000;

/// The smallest output limit a profile accepts, in tokens.
pub const MIN_OUTPUT_LIMIT: u32 = 16;

/// The smallest context budget a profile must leave, in tokens.
pub const MIN_CONTEXT_BUDGET: u32 = 1_024;

/// The longest credential Mannie accepts, in bytes.
pub const MAX_CREDENTIAL_BYTES: usize = 4_096;

/// The API a profile speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ApiInterface {
    /// The OpenAI Responses API.
    Responses,
    /// The Chat Completions API of OpenAI and compatible servers.
    ChatCompletions,
}

impl ApiInterface {
    /// The interface's name as the store keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Responses => "responses",
            Self::ChatCompletions => "chat_completions",
        }
    }

    /// The interface with this name.
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Responses, Self::ChatCompletions]
            .into_iter()
            .find(|interface| interface.as_str() == name)
    }
}

/// How much reasoning a model spends before it answers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    /// Mannie sends no effort, and the server uses its default.
    #[default]
    ServerDefault,
    /// No reasoning.
    None,
    /// The least reasoning.
    Minimal,
    /// Little reasoning.
    Low,
    /// Medium reasoning.
    Medium,
    /// Much reasoning.
    High,
    /// More reasoning than high, for the models that offer it.
    #[serde(rename = "xhigh")]
    XHigh,
    /// The most reasoning, for the models that offer it.
    Max,
}

impl ReasoningEffort {
    /// The effort's name in the provider APIs, or `None` when Mannie sends no
    /// effort.
    pub fn api_name(self) -> Option<&'static str> {
        match self {
            Self::ServerDefault => None,
            Self::None => Some("none"),
            Self::Minimal => Some("minimal"),
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
            Self::XHigh => Some("xhigh"),
            Self::Max => Some("max"),
        }
    }
}

/// The servers whose behavior Mannie knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ServerKind {
    /// OpenAI's API.
    OpenAi,
    /// OpenRouter.
    OpenRouter,
    /// vLLM.
    Vllm,
    /// SGLang.
    Sglang,
    /// LM Studio.
    LmStudio,
    /// Ollama.
    Ollama,
    /// An OpenAI-compatible server that Mannie does not recognize.
    Other,
}

impl ServerKind {
    /// Every kind.
    pub const ALL: [Self; 7] = [
        Self::OpenAi,
        Self::OpenRouter,
        Self::Vllm,
        Self::Sglang,
        Self::LmStudio,
        Self::Ollama,
        Self::Other,
    ];

    /// The kind's name as the store keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "open_ai",
            Self::OpenRouter => "open_router",
            Self::Vllm => "vllm",
            Self::Sglang => "sglang",
            Self::LmStudio => "lm_studio",
            Self::Ollama => "ollama",
            Self::Other => "other",
        }
    }

    /// The kind with this name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

/// A model's context window and output limit, in tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelLimits {
    /// The tokens the model reads and writes in one call.
    pub context_window: u32,
    /// The tokens the model writes in one call.
    pub output_limit: u32,
}

/// The argument of `get_model_defaults`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelDefaultsRef {
    /// The model's name as the owner typed it.
    pub model: String,
}

/// What Mannie knows about a model: the defaults of a profile for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelDefaults {
    /// The model's limits.
    pub limits: ModelLimits,
    /// Whether the model reasons before it answers, so that a reasoning
    /// effort applies to it.
    pub reasons: bool,
}

/// A provider profile as the UI shows it. The credential itself never leaves
/// the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProfile {
    /// The profile.
    pub id: ProfileId,
    /// Its name.
    pub name: String,
    /// The API it speaks.
    pub interface: ApiInterface,
    /// The server's base URL.
    pub base_url: String,
    /// The model.
    pub model: String,
    /// Whether a credential is stored for it.
    pub has_credential: bool,
    /// The model's context window, in tokens.
    pub context_window: u32,
    /// The longest answer, in tokens.
    pub output_limit: u32,
    /// The reasoning effort.
    pub reasoning_effort: ReasoningEffort,
    /// Whether the model gets tools (docs/features/system-tools/tool-calls.md,
    /// use tools).
    pub use_tools: bool,
    /// Whether it is the default profile.
    pub is_default: bool,
    /// The server the provider test recognized, if it ran.
    pub server: Option<ServerKind>,
    /// When it was created.
    pub created_at: UtcMillis,
    /// When it last changed.
    pub updated_at: UtcMillis,
}

/// What the owner entered for a profile, before the core checks it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDraft {
    /// The name.
    pub name: String,
    /// The API interface.
    pub interface: ApiInterface,
    /// The base URL.
    pub base_url: String,
    /// The model.
    pub model: String,
    /// The context window, in tokens.
    pub context_window: u32,
    /// The output limit, in tokens.
    pub output_limit: u32,
    /// The reasoning effort.
    pub reasoning_effort: ReasoningEffort,
    /// Whether the model gets tools.
    pub use_tools: bool,
}

impl ProfileDraft {
    /// Checks everything except the base URL, which the provider crate checks:
    /// the name, the model and the limits. Returns the trimmed name and model.
    pub fn check_fields(&self) -> Result<(String, ModelName), ProviderProfileError> {
        let name = self.name.trim();
        let name_is_valid =
            !name.is_empty() && name.chars().count() <= MAX_NAME_CHARACTERS && !has_control(name);
        if !name_is_valid {
            return Err(ProviderProfileError::InvalidName);
        }
        let model = ModelName::parse(&self.model)?;
        check_limits(self.context_window, self.output_limit)?;
        Ok((name.to_owned(), model))
    }
}

/// A model name: 1 to 200 characters without control characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct ModelName(String);

impl ModelName {
    /// Checks a model name and trims surrounding white space.
    pub fn parse(text: &str) -> Result<Self, ProviderProfileError> {
        let model = text.trim();
        let is_valid = !model.is_empty()
            && model.chars().count() <= MAX_MODEL_CHARACTERS
            && !has_control(model);
        if is_valid {
            Ok(Self(model.to_owned()))
        } else {
            Err(ProviderProfileError::InvalidModel)
        }
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The context budget that the limits leave: the window minus the output limit
/// and a margin of 5% of the window (docs/architecture/agent-runtime.md,
/// context management).
pub fn context_budget(context_window: u32, output_limit: u32) -> u32 {
    let margin = context_window.div_ceil(20);
    context_window
        .saturating_sub(output_limit)
        .saturating_sub(margin)
}

/// Checks that the limits are within range and leave a usable budget.
pub fn check_limits(context_window: u32, output_limit: u32) -> Result<(), ProviderProfileError> {
    let window_in_range = (MIN_CONTEXT_WINDOW..=MAX_CONTEXT_WINDOW).contains(&context_window);
    let output_in_range = output_limit >= MIN_OUTPUT_LIMIT;
    let budget_in_range = context_budget(context_window, output_limit) >= MIN_CONTEXT_BUDGET;
    if window_in_range && output_in_range && budget_in_range {
        Ok(())
    } else {
        Err(ProviderProfileError::InvalidLimits {
            min_context_window: MIN_CONTEXT_WINDOW,
            max_context_window: MAX_CONTEXT_WINDOW,
            min_budget: MIN_CONTEXT_BUDGET,
        })
    }
}

/// Checks a credential's text and returns it without surrounding white space,
/// which pasting often adds.
pub fn check_credential(credential: &Secret) -> Result<Secret, ProviderProfileError> {
    let trimmed = credential.expose().trim();
    let is_valid =
        !trimmed.is_empty() && trimmed.len() <= MAX_CREDENTIAL_BYTES && !has_control(trimmed);
    if is_valid {
        Ok(Secret::new(trimmed.to_owned()))
    } else {
        Err(ProviderProfileError::InvalidCredential)
    }
}

fn has_control(text: &str) -> bool {
    text.chars().any(char::is_control)
}

/// What happens to a profile's credential when the profile is updated.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum CredentialChange {
    /// The stored credential stays.
    Keep,
    /// A new credential replaces the stored one.
    Replace {
        /// The new credential.
        credential: Secret,
    },
    /// The profile no longer has a credential.
    Remove,
}

/// The argument of `create_provider_profile`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateProviderProfile {
    /// The profile's fields.
    pub draft: ProfileDraft,
    /// The credential, if the server needs one.
    pub credential: Option<Secret>,
}

/// The argument of `update_provider_profile`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProviderProfile {
    /// The profile.
    pub id: ProfileId,
    /// Its new fields.
    pub draft: ProfileDraft,
    /// What happens to its credential.
    pub credential: CredentialChange,
}

/// The argument of the commands that act on one profile.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRef {
    /// The profile.
    pub id: ProfileId,
}

/// Why the credential store could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStoreProblem {
    /// Linux has no Secret Service, such as GNOME Keyring or KWallet.
    NoSecretService,
    /// The operating system has no credential store that Mannie supports.
    UnsupportedPlatform,
    /// The credential store refused access, for example because it is locked.
    AccessDenied,
    /// The credential store failed. The log has the cause.
    Other,
}

/// Why a provider profile could not be saved, read or deleted. The UI shows
/// the message for the code from its catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ProviderProfileError {
    /// The name is empty, too long or has control characters.
    #[error("the profile name is not valid")]
    InvalidName,
    /// Another profile has the same name.
    #[error("another profile has the same name")]
    NameTaken,
    /// The base URL is not an absolute http or https URL without user
    /// information, query or fragment.
    #[error("the base URL is not valid")]
    InvalidBaseUrl,
    /// A credential would travel over plain HTTP to a host that is not on the
    /// local machine or the local network.
    #[error("a credential would travel over plain HTTP to a remote host")]
    InsecureCredentialTransport,
    /// The model name is empty, too long or has control characters.
    #[error("the model name is not valid")]
    InvalidModel,
    /// The limits are out of range or leave too small a context budget.
    #[error("the context window and output limit are not valid")]
    InvalidLimits {
        /// The smallest context window.
        min_context_window: u32,
        /// The largest context window.
        max_context_window: u32,
        /// The smallest budget the limits must leave.
        min_budget: u32,
    },
    /// The credential is empty, too long or has control characters.
    #[error("the credential is not valid")]
    InvalidCredential,
    /// A prefix of a search profile is too long or has control characters,
    /// or a rerank profile has one (ADR-0047).
    #[error("a prefix of the search profile is not valid")]
    InvalidPrefix {
        /// The longest prefix, in characters.
        max_characters: u32,
    },
    /// The profile does not exist.
    #[error("the provider profile does not exist")]
    NotFound,
    /// A reply that is being written uses the profile.
    #[error("a reply that is being written uses the profile")]
    ProfileInUse,
    /// The credential store cannot be used.
    #[error("the credential store cannot be used")]
    CredentialStoreUnavailable {
        /// Why.
        reason: CredentialStoreProblem,
    },
    /// The store failed. The log has the cause.
    #[error("the provider profile could not be read or saved")]
    StorageFailed,
    /// The core started without its database.
    #[error("the database is not open")]
    StoreUnavailable,
}

impl ProviderProfileError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidName => "provider.invalid_name",
            Self::NameTaken => "provider.name_taken",
            Self::InvalidBaseUrl => "provider.invalid_base_url",
            Self::InsecureCredentialTransport => "provider.insecure_credential_transport",
            Self::InvalidModel => "provider.invalid_model",
            Self::InvalidLimits { .. } => "provider.invalid_limits",
            Self::InvalidCredential => "provider.invalid_credential",
            Self::InvalidPrefix { .. } => "provider.invalid_prefix",
            Self::NotFound => "provider.not_found",
            Self::ProfileInUse => "provider.profile_in_use",
            Self::CredentialStoreUnavailable { .. } => "provider.credential_store_unavailable",
            Self::StorageFailed => "provider.storage_failed",
            Self::StoreUnavailable => "provider.store_unavailable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApiInterface, ProfileDraft, ProviderProfileError, ReasoningEffort, ServerKind,
        check_credential, check_limits, context_budget,
    };
    use crate::secret::Secret;

    fn draft() -> ProfileDraft {
        ProfileDraft {
            name: "  Local vLLM ".to_owned(),
            interface: ApiInterface::ChatCompletions,
            base_url: "http://127.0.0.1:8000/v1".to_owned(),
            model: " qwen3 ".to_owned(),
            context_window: 32_768,
            output_limit: 4_096,
            reasoning_effort: ReasoningEffort::ServerDefault,
            use_tools: true,
        }
    }

    #[test]
    fn names_round_trip() {
        for interface in [ApiInterface::Responses, ApiInterface::ChatCompletions] {
            assert_eq!(ApiInterface::parse(interface.as_str()), Some(interface));
        }
        for kind in ServerKind::ALL {
            assert_eq!(ServerKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ReasoningEffort::ServerDefault.api_name(), None);
        assert_eq!(ReasoningEffort::Low.api_name(), Some("low"));
    }

    #[test]
    fn a_valid_draft_is_trimmed() {
        let (name, model) = draft().check_fields().unwrap();

        assert_eq!(name, "Local vLLM");
        assert_eq!(model.as_str(), "qwen3");
    }

    #[test]
    fn invalid_fields_are_refused() {
        let blank_name = ProfileDraft {
            name: " ".to_owned(),
            ..draft()
        };
        let long_model = ProfileDraft {
            model: "m".repeat(201),
            ..draft()
        };
        let control_in_name = ProfileDraft {
            name: "a\u{7}b".to_owned(),
            ..draft()
        };

        assert_eq!(
            blank_name.check_fields(),
            Err(ProviderProfileError::InvalidName)
        );
        assert_eq!(
            long_model.check_fields(),
            Err(ProviderProfileError::InvalidModel)
        );
        assert_eq!(
            control_in_name.check_fields(),
            Err(ProviderProfileError::InvalidName)
        );
    }

    #[test]
    fn limits_must_leave_a_budget() {
        assert_eq!(context_budget(128_000, 16_000), 105_600);
        assert_eq!(check_limits(128_000, 16_000), Ok(()));
        assert!(
            check_limits(1_000, 100).is_err(),
            "window below the minimum"
        );
        assert!(
            check_limits(8_192, 7_000).is_err(),
            "budget below the minimum"
        );
        assert!(check_limits(8_192, 8).is_err(), "output below the minimum");
        assert!(
            check_limits(20_000_000, 8_192).is_err(),
            "window above the maximum"
        );
    }

    #[test]
    fn credentials_are_trimmed_and_checked() {
        let pasted = Secret::new("  sk-test-0123456789\n".to_owned());

        assert_eq!(
            check_credential(&pasted).unwrap().expose(),
            "sk-test-0123456789"
        );
        assert_eq!(
            check_credential(&Secret::new(" \n".to_owned())).unwrap_err(),
            ProviderProfileError::InvalidCredential
        );
        assert_eq!(
            check_credential(&Secret::new("a\u{0}b".to_owned())).unwrap_err(),
            ProviderProfileError::InvalidCredential
        );
    }
}
