//! Provider profiles and the provider test, as the UI and the core exchange
//! them (docs/features/chat/providers.md).
//!
//! This crate carries the profiles and checks what needs no parser; the
//! provider crate checks URLs and knows the servers.

mod profile;
mod test_report;

pub use profile::{
    ApiInterface, CreateProviderProfile, CredentialChange, CredentialStoreProblem,
    MAX_CONTEXT_WINDOW, MAX_CREDENTIAL_BYTES, MAX_MODEL_CHARACTERS, MAX_NAME_CHARACTERS,
    MIN_CONTEXT_BUDGET, MIN_CONTEXT_WINDOW, MIN_OUTPUT_LIMIT, ModelDefaults, ModelDefaultsRef,
    ModelLimits, ModelName, ProfileDraft, ProfileRef, ProviderProfile, ProviderProfileError,
    ReasoningEffort, ServerKind, UpdateProviderProfile, check_credential, check_limits,
    context_budget,
};
pub use test_report::{
    CheckKind, CheckNote, CheckOutcome, ProviderCheck, ProviderTestReport, ReportedLimits,
    SkipReason,
};
