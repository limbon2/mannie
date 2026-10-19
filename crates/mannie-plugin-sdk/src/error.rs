//! Why a plugin did not start (ADR-0034, decisions 4 and 5), as a stable
//! code that the Plugins page words with "Try again".
//!
//! A plugin gives its own codes, such as Workers' `storage_failed`, which its
//! messages word. The host's codes are below; the host gives them for what
//! went wrong around the plugin's code, and a plugin gives `grant_missing`
//! and `service_unavailable` through [`StartContext::service`].
//!
//! [`StartContext::service`]: crate::context::StartContext::service

use serde::{Deserialize, Serialize};

/// Why a plugin could not start, as its code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(transparent)]
#[error("the plugin could not start: {code}")]
pub struct StartFailure {
    code: String,
}

impl StartFailure {
    /// The failure with this code, such as `storage_failed`.
    pub fn new(code: &str) -> Self {
        Self {
            code: code.to_owned(),
        }
    }

    /// The code.
    pub fn code(&self) -> &str {
        &self.code
    }
}

/// The code of a start that asked for a service that the plugin's manifest
/// does not declare.
pub const GRANT_MISSING: &str = "grant_missing";

/// The code of a start for which a declared service could not be bound: no
/// provider offers it, the provider offers another major, or the provider
/// refused to bind it to the plugin.
pub const SERVICE_UNAVAILABLE: &str = "service_unavailable";

/// The code of a plugin whose manifest names another major of the SDK. The
/// host never calls such a plugin's code.
pub const SDK_INCOMPATIBLE: &str = "sdk_incompatible";

/// The code of a plugin whose contributions the core refused, such as a tool
/// whose name another plugin uses, or a contribution to a point that no part
/// of the core takes (decision 6).
pub const CONTRIBUTIONS_REFUSED: &str = "contributions_refused";

#[cfg(test)]
mod tests {
    use super::{GRANT_MISSING, StartFailure};

    #[test]
    fn a_failure_keeps_its_code_and_says_it() {
        let failure = StartFailure::new(GRANT_MISSING);

        assert_eq!(failure.code(), "grant_missing");
        assert_eq!(
            failure.to_string(),
            "the plugin could not start: grant_missing"
        );
    }
}
