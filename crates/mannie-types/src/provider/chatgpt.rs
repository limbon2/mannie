//! The ChatGPT account through which profiles reach OpenAI on the owner's
//! ChatGPT plan (ADR-0062), as the UI and the core exchange it. Its tokens
//! never leave the core.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::profile::CredentialStoreProblem;
use crate::run::RunFailure;
use crate::transcript::SubscriptionEnd;

/// The most characters of a text of the page that the browser shows when a
/// sign-in returns.
pub const MAX_CALLBACK_TEXT_CHARACTERS: usize = 500;

/// Where the ChatGPT account stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChatGptAccountState {
    /// No account is signed in: the owner never signed in, or signed out.
    SignedOut,
    /// An account is signed in, and profiles that use the ChatGPT plan can
    /// call their models.
    SignedIn,
    /// The account's session ended, for example because OpenAI revoked it or
    /// its refresh token expired, so the owner signs in again.
    SignInNeeded,
}

impl ChatGptAccountState {
    /// The state's name as the store keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SignedOut => "signed_out",
            Self::SignedIn => "signed_in",
            Self::SignInNeeded => "sign_in_needed",
        }
    }

    /// The state with this name.
    pub fn parse(name: &str) -> Option<Self> {
        [Self::SignedOut, Self::SignedIn, Self::SignInNeeded]
            .into_iter()
            .find(|state| state.as_str() == name)
    }
}

/// A sign-in, from the owner's click to its end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(
    tag = "step",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ChatGptSignIn {
    /// No sign-in runs.
    Idle,
    /// The browser shows OpenAI's sign-in page, and Mannie waits for it to
    /// come back.
    WaitingForBrowser,
    /// The browser came back, and Mannie redeems the code and checks the
    /// account.
    Finishing,
    /// The last sign-in failed. The state stays until the next one starts.
    Failed {
        /// Why.
        error: ChatGptError,
    },
}

/// The ChatGPT account as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptAccountView {
    /// Where the account stands.
    pub state: ChatGptAccountState,
    /// The account's email address, when Mannie knows the account.
    pub email: Option<String>,
    /// Whether Mannie knows an account from an earlier sign-in, so that
    /// signing in again reuses its registration and "Use another account"
    /// applies.
    pub known: bool,
    /// Whether the owner saw the notice that Mannie uses their ChatGPT plan.
    pub notice_seen: bool,
    /// The sign-in that runs or last failed.
    pub sign_in: ChatGptSignIn,
}

/// A message of the subscription to the ChatGPT account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ChatGptAccountUpdate {
    /// The account as it is now, first when the subscription starts and then
    /// after each change.
    Account {
        /// The account.
        account: ChatGptAccountView,
    },
    /// The subscription ended.
    Ended {
        /// Why.
        reason: SubscriptionEnd,
    },
}

/// A model that the ChatGPT plan offers the signed-in account, in the order
/// of OpenAI's catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptModel {
    /// The name that requests use.
    pub slug: String,
    /// The name that the owner reads.
    pub display_name: String,
}

/// The texts of the page that the browser shows when a sign-in returns, from
/// the UI's catalog, since the core has none (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptCallbackPage {
    /// The page's title.
    pub title: String,
    /// What the page says after the sign-in worked.
    pub signed_in: String,
    /// What the page says after it failed.
    pub failed: String,
}

impl ChatGptCallbackPage {
    /// Checks that each text has 1 to 500 characters and no control
    /// characters. The webview is untrusted (ADR-0004).
    pub fn check(&self) -> Result<(), ChatGptError> {
        let texts = [&self.title, &self.signed_in, &self.failed];
        let all_valid = texts.iter().all(|text| {
            let count = text.chars().count();
            (1..=MAX_CALLBACK_TEXT_CHARACTERS).contains(&count)
                && !text.chars().any(char::is_control)
        });
        if all_valid {
            Ok(())
        } else {
            Err(ChatGptError::InvalidPage)
        }
    }
}

/// The argument of `start_chatgpt_sign_in`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StartChatGptSignIn {
    /// Whether to sign in to a new account instead of the known one.
    pub another_account: bool,
    /// The page that the browser shows when the sign-in returns.
    pub page: ChatGptCallbackPage,
}

/// What signing out did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SignOutReport {
    /// Whether OpenAI confirmed that it revoked the session. When it did not,
    /// Mannie still forgot the tokens, and the owner can disconnect Mannie in
    /// ChatGPT's settings.
    pub revoked: bool,
}

/// Why a command of the ChatGPT account, or a sign-in, failed. The UI shows
/// the message for the code from its catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ChatGptError {
    /// Another program listens on the port of the first sign-in, 1455, such
    /// as the Codex CLI while it signs in, or the system reserved it.
    #[error("another program uses port {port}, which the first sign-in needs")]
    PortInUse {
        /// The port.
        port: u16,
    },
    /// The browser did not come back within 5 minutes.
    #[error("the browser did not come back in time")]
    TimedOut,
    /// The owner declined in the browser, or OpenAI ended the sign-in there.
    #[error("the sign-in was declined in the browser")]
    AccessDenied,
    /// OpenAI's answer to a first sign-in named no client, so Mannie's
    /// registration is incomplete.
    #[error("OpenAI did not complete Mannie's registration")]
    RegistrationIncomplete,
    /// OpenAI refused to redeem the sign-in, or its answer was not usable.
    #[error("OpenAI refused the sign-in")]
    SignInRefused,
    /// The ID token of the sign-in failed a check, such as its signature,
    /// issuer, audience, nonce or expiry.
    #[error("the ID token of the sign-in is not valid")]
    IdTokenInvalid,
    /// The account did not allow Mannie to use its ChatGPT plan.
    #[error("the account did not allow Mannie to use its ChatGPT plan")]
    PlanUsageNotGranted,
    /// OpenAI could not be reached.
    #[error("OpenAI could not be reached")]
    Network,
    /// The owner canceled the sign-in.
    #[error("the sign-in was canceled")]
    Canceled,
    /// A sign-in runs already.
    #[error("a sign-in runs already")]
    AlreadySigningIn,
    /// No sign-in runs.
    #[error("no sign-in runs")]
    NotSigningIn,
    /// A text of the sign-in's page is empty, too long or has control
    /// characters.
    #[error("a text of the sign-in's page is not valid")]
    InvalidPage,
    /// No account is signed in.
    #[error("no ChatGPT account is signed in")]
    SignedOut,
    /// The system browser could not be opened.
    #[error("the system browser could not be opened")]
    BrowserUnavailable,
    /// A call to OpenAI failed, such as reading the catalog of models.
    #[error("a call to OpenAI failed: {failure}")]
    Provider {
        /// Why.
        failure: RunFailure,
    },
    /// The credential store cannot be used.
    #[error("the credential store cannot be used")]
    CredentialStoreUnavailable {
        /// Why.
        reason: CredentialStoreProblem,
    },
    /// The store failed. The log has the cause.
    #[error("the ChatGPT account could not be read or saved")]
    StorageFailed,
    /// The core started without its database.
    #[error("the database is not open")]
    StoreUnavailable,
}

impl ChatGptError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::PortInUse { .. } => "chatgpt.port_in_use",
            Self::TimedOut => "chatgpt.timed_out",
            Self::AccessDenied => "chatgpt.access_denied",
            Self::RegistrationIncomplete => "chatgpt.registration_incomplete",
            Self::SignInRefused => "chatgpt.sign_in_refused",
            Self::IdTokenInvalid => "chatgpt.id_token_invalid",
            Self::PlanUsageNotGranted => "chatgpt.plan_usage_not_granted",
            Self::Network => "chatgpt.network",
            Self::Canceled => "chatgpt.canceled",
            Self::AlreadySigningIn => "chatgpt.already_signing_in",
            Self::NotSigningIn => "chatgpt.not_signing_in",
            Self::InvalidPage => "chatgpt.invalid_page",
            Self::SignedOut => "chatgpt.signed_out",
            Self::BrowserUnavailable => "chatgpt.browser_unavailable",
            Self::Provider { .. } => "chatgpt.provider",
            Self::CredentialStoreUnavailable { .. } => "chatgpt.credential_store_unavailable",
            Self::StorageFailed => "chatgpt.storage_failed",
            Self::StoreUnavailable => "chatgpt.store_unavailable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChatGptAccountState, ChatGptCallbackPage, ChatGptError, ChatGptSignIn,
        MAX_CALLBACK_TEXT_CHARACTERS,
    };

    fn page() -> ChatGptCallbackPage {
        ChatGptCallbackPage {
            title: "Mannie".to_owned(),
            signed_in: "You are signed in. You can close this page.".to_owned(),
            failed: "The sign-in failed. Go back to Mannie.".to_owned(),
        }
    }

    #[test]
    fn states_round_trip_through_their_names() {
        for state in [
            ChatGptAccountState::SignedOut,
            ChatGptAccountState::SignedIn,
            ChatGptAccountState::SignInNeeded,
        ] {
            assert_eq!(ChatGptAccountState::parse(state.as_str()), Some(state));
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::json!(state.as_str())
            );
        }
        assert_eq!(ChatGptAccountState::parse("signed_up"), None);
    }

    #[test]
    fn a_failed_sign_in_carries_its_error_by_code() {
        let failed = ChatGptSignIn::Failed {
            error: ChatGptError::PortInUse { port: 1455 },
        };

        assert_eq!(
            serde_json::to_value(&failed).unwrap(),
            serde_json::json!({ "step": "failed", "error": { "code": "port_in_use", "port": 1455 } })
        );
        assert_eq!(
            serde_json::to_value(ChatGptSignIn::WaitingForBrowser).unwrap(),
            serde_json::json!({ "step": "waiting_for_browser" })
        );
    }

    #[test]
    fn page_texts_are_checked() {
        assert_eq!(page().check(), Ok(()));
        for invalid in [
            ChatGptCallbackPage {
                title: String::new(),
                ..page()
            },
            ChatGptCallbackPage {
                signed_in: "a".repeat(MAX_CALLBACK_TEXT_CHARACTERS + 1),
                ..page()
            },
            ChatGptCallbackPage {
                failed: "line\nbreak".to_owned(),
                ..page()
            },
        ] {
            assert_eq!(invalid.check(), Err(ChatGptError::InvalidPage));
        }
    }
}
