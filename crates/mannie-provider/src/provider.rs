//! The `Provider` trait: one streamed model call (ADR-0011, decision 1).

use std::fmt;
use std::pin::Pin;
use std::time::Duration;

use futures_util::Stream;
use mannie_types::provider::{ApiInterface, ReasoningEffort};
use mannie_types::run::{FinishReason, RunFailure, Usage};
use mannie_types::secret::Secret;
use reqwest::Url;

use crate::quirks::Quirks;

/// Where a call goes: the server, the credential and the server's quirks.
#[derive(Debug, Clone)]
pub struct Target {
    /// The server's base URL, such as `https://api.openai.com/v1`.
    pub base_url: Url,
    /// The credential, sent as a bearer token, if the profile has one.
    pub credential: Option<Secret>,
    /// The documented behavior of the server and the model.
    pub quirks: Quirks,
}

/// Who wrote a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The owner, or the runtime on the owner's behalf.
    User,
    /// The model.
    Assistant,
}

/// One element of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// Text.
    Text(String),
    /// Provider-private data from an earlier call with the same binding, such
    /// as an encrypted reasoning item of the Responses API: a JSON document
    /// that the adapter wrote and sends back as it is.
    ProviderItem(String),
}

/// A message of the conversation that the model reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Who wrote it.
    pub role: Role,
    /// What it holds, in order.
    pub content: Vec<Content>,
}

impl Message {
    /// A message with one piece of text.
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![Content::Text(text.into())],
        }
    }
}

/// One model call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRequest {
    /// The model.
    pub model: String,
    /// The system prompt, assembled from the instruction sections.
    pub instructions: String,
    /// The conversation after the instructions, oldest first.
    pub messages: Vec<Message>,
    /// The most tokens the answer may use.
    pub max_output_tokens: u32,
    /// How much the model reasons.
    pub reasoning_effort: ReasoningEffort,
}

/// What a model call produces, in order, normalized across the APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    /// Text of the answer.
    TextDelta(String),
    /// Reasoning that the owner may read: a summary or the reasoning text.
    ReasoningDelta(String),
    /// Provider-private data to store with the reasoning and send back to the
    /// same binding on the next call: a JSON document.
    ProviderItem(String),
    /// The token counts of the call.
    Usage(Usage),
    /// The call ended. Nothing follows.
    Finished(FinishReason),
}

/// Why a model call failed (ADR-0011, decision 3).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// The call may succeed when it is tried again: a rate limit, a server
    /// error, a timeout or a broken stream.
    #[error("the call failed and may succeed later: {cause}")]
    Transient {
        /// What failed.
        cause: RunFailure,
        /// How long the provider asked to wait, if it said so.
        retry_after: Option<Duration>,
    },
    /// The request is longer than the model's context.
    #[error("the request is longer than the model's context")]
    ContextOverflow,
    /// The call fails the same way when it is tried again.
    #[error("the call failed: {0}")]
    Permanent(RunFailure),
}

impl ProviderError {
    /// The failure to show the owner.
    pub fn failure(&self) -> RunFailure {
        match self {
            Self::Transient { cause, .. } => cause.clone(),
            Self::ContextOverflow => RunFailure::ContextOverflow,
            Self::Permanent(failure) => failure.clone(),
        }
    }
}

/// The events of one call. Dropping the stream cancels the call at once and
/// closes its connection.
pub type EventStream = Pin<Box<dyn Stream<Item = Result<ProviderEvent, ProviderError>> + Send>>;

/// An adapter for one API interface (docs/architecture/extension-points.md,
/// provider adapters).
///
/// An adapter streams exactly one model call. It never retries, never runs
/// tools and never decides what happens next: the runtime does
/// (docs/architecture/agent-runtime.md, the loop).
pub trait Provider: Send + Sync + fmt::Debug {
    /// The API interface it speaks.
    fn interface(&self) -> ApiInterface;

    /// Starts a call. Errors arrive as items of the stream, so a call that
    /// fails before its first event and one that fails midway look the same to
    /// the runtime.
    fn stream(&self, target: &Target, request: ModelRequest) -> EventStream;
}
