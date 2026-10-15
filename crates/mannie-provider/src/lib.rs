//! Mannie's provider adapters (ADR-0011): one streamed model call each,
//! through the OpenAI Responses API or Chat Completions.
//!
//! An adapter turns a [`ModelRequest`] into an HTTP request, streams the
//! answer, and normalizes it into [`ProviderEvent`]s, classifying failures
//! into transient ones, context overflows and permanent ones. Quirk profiles
//! adjust the requests and the parsing for each server. The runtime drives the
//! loop: it decides about retries, compaction and the next step.
//!
//! The adapters use reqwest over rustls with ring and the operating system's
//! trust store, a small server-sent events parser of their own, and serde
//! types for the parts of each API that Mannie uses. No LLM framework is
//! involved.

mod chat_completions;
mod detect;
mod errors;
mod models;
mod probe;
mod profile;
mod provider;
mod quirks;
mod registry;
mod responses;
mod sse;
mod stream;
#[cfg(feature = "testing")]
pub mod testing;
mod tool_calls;
mod transport;

pub use chat_completions::ChatCompletionsAdapter;
pub use detect::{ListedModel, ModelList, detect_server, list_models, server_from_host};
pub use models::{KnownModel, known_model, known_models};
pub use probe::{TEST_PROMPT, TOOL_PROMPT, TestInput, test_profile};
pub use profile::{MAX_BASE_URL_BYTES, check_base_url, credential_transport_allowed};
pub use provider::{
    Content, EventStream, Message, ModelRequest, Provider, ProviderError, ProviderEvent, Role,
    Target, ToolChoice, ToolDefinition,
};
pub use quirks::{EffortShape, OutputLimitField, Quirks, ToolQuirks, quirks_for};
pub use registry::{DuplicateAdapter, ProviderRegistry};
pub use reqwest::Url;
pub use responses::ResponsesAdapter;
pub use transport::{HttpClient, Timeouts, TransportError};
