//! Opening links from model output (docs/features/chat/conversation-view.md,
//! Markdown rendering): the webview asks the core, the core checks the URL,
//! and the URL opens in the system browser only after the owner confirmed it
//! (ADR-0004, decision 7).

use serde::{Deserialize, Serialize};
use specta::Type;

/// The longest URL Mannie opens, in bytes.
pub const MAX_URL_BYTES: usize = 8 * 1024;

/// The argument of `check_link` and `open_link`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LinkRef {
    /// The URL as the model wrote it.
    pub url: String,
}

/// A URL the core checked, in the form it would open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CheckedLink {
    /// The normalized URL. Internationalized host names appear in their ASCII
    /// form, so the owner sees where the link really goes.
    pub url: String,
    /// The host the URL points to.
    pub host: String,
}

/// Why a link cannot be opened. The UI shows the message for the code from its
/// catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum LinkError {
    /// The text is not an absolute URL.
    #[error("the link is not a valid URL")]
    InvalidUrl,
    /// The URL uses a scheme other than http or https.
    #[error("links with the scheme {scheme:?} do not open")]
    UnsupportedScheme {
        /// The scheme, cut to 32 characters.
        scheme: String,
    },
    /// The URL is longer than Mannie opens.
    #[error("the link is longer than {max_bytes} bytes")]
    TooLong {
        /// The limit.
        max_bytes: u32,
    },
    /// The system could not open the browser.
    #[error("the system browser could not be opened")]
    OpenFailed,
}

impl LinkError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUrl => "link.invalid_url",
            Self::UnsupportedScheme { .. } => "link.unsupported_scheme",
            Self::TooLong { .. } => "link.too_long",
            Self::OpenFailed => "link.open_failed",
        }
    }
}
