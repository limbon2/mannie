//! A plugin's commands and channels (ADR-0034, decisions 8 and 9): what its
//! UI calls, declared in Rust with no Tauri.
//!
//! A plugin declares each command with [`CommandSpec`] and each channel with
//! [`ChannelSpec`]: a name, an input that refuses unknown fields and states
//! its checks with [`Validate`], an output or an item, an error tagged by its
//! `code`, and a size limit for the input. [`Plugin::interface`] lists them
//! with the constants that the UI needs, and a start hands the host one
//! handler for each through [`Handlers`], so a plugin that is off has nothing
//! that can be called.
//!
//! The host dispatches every call in one place. It checks, in order, that the
//! store is available and Mannie is not quitting, that the plugin is on, the
//! size of the input, its decoding and its checks, and only then calls the
//! handler. Its refusals are [`HostRefusal`]s, which cross IPC beside the
//! plugin's own errors in a [`PluginFailure`], so the UI tells them apart by
//! code alone.
//!
//! [`Plugin::interface`]: crate::plugin::Plugin::interface

mod handlers;
mod interface;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub use self::handlers::{
    Answer, ChannelHandler, CommandHandler, ErrorValue, Handlers, InputFailure, Opened,
};
pub use self::interface::{Constant, Endpoint, Interface};
use crate::BoxStream;
use crate::contracts::SubscriptionEnd;

/// The most bytes of a command's or a channel's input as JSON, unless its
/// spec says otherwise.
pub const DEFAULT_MAX_INPUT_BYTES: u32 = 64 * 1024;

/// Checks of meaning on an input from the webview, which is untrusted
/// (ADR-0004, decision 2). It has no default, so every input type states its
/// checks, even when it has none.
pub trait Validate {
    /// Refuses an input that decoded but breaks a rule, such as a name that
    /// is too long.
    fn validate(&self) -> Result<(), InputRefusal>;
}

impl Validate for () {
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// Why an input that decoded breaks a rule: the field whose value the
/// rule refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct InputRefusal {
    /// The field, as the input names it in JSON, such as `filterName`.
    pub field: String,
}

impl InputRefusal {
    /// The refusal of `field`.
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
        }
    }
}

/// An error of a plugin's command or channel. It serializes as an enum
/// tagged by `code` (ADR-0007), with codes that are not the host's, which
/// the bindings exporter checks.
pub trait PluginError: Serialize + std::error::Error + Send + Sync + 'static {
    /// The error's stable code for the log, such as `images.too_large`.
    fn code(&self) -> &'static str;
}

/// A command of a plugin.
pub trait CommandSpec: 'static {
    /// The command's name: lower-case words joined by underscores, unique
    /// among the plugin's commands and channels.
    const NAME: &'static str;
    /// What the command does, for the generated function's documentation.
    const DOCS: &'static str;
    /// The most bytes of the input as JSON.
    const MAX_INPUT_BYTES: u32 = DEFAULT_MAX_INPUT_BYTES;
    /// The input, which refuses unknown fields
    /// (`#[serde(deny_unknown_fields)]`).
    type Input: DeserializeOwned + Validate + specta::Type + Send + 'static;
    /// The output.
    type Output: Serialize + specta::Type + Send + 'static;
    /// The plugin's errors.
    type Error: PluginError + specta::Type;
}

/// A channel of a plugin: a subscription whose items the host carries to the
/// UI in batches. Its stream runs until the plugin stops or Mannie quits, and
/// the host then ends the channel with `plugin_off` or `shutdown`. A stream
/// that ends before ends the channel with `lagged`, so the UI subscribes
/// again.
pub trait ChannelSpec: 'static {
    /// The channel's name: lower-case words joined by underscores, unique
    /// among the plugin's commands and channels.
    const NAME: &'static str;
    /// What the channel carries, for the generated function's
    /// documentation.
    const DOCS: &'static str;
    /// The most bytes of the input as JSON.
    const MAX_INPUT_BYTES: u32 = DEFAULT_MAX_INPUT_BYTES;
    /// The input, which refuses unknown fields
    /// (`#[serde(deny_unknown_fields)]`).
    type Input: DeserializeOwned + Validate + specta::Type + Send + 'static;
    /// An item of the channel.
    type Item: Serialize + specta::Type + Send + 'static;
    /// The plugin's errors, when the channel cannot open.
    type Error: PluginError + specta::Type;
}

/// The items of a channel, which own their data and end the channel when
/// they are dropped (decision 3).
pub type PluginStream<T> = BoxStream<'static, T>;

/// A message of a plugin's channel: an item of the plugin, or the end that
/// the host sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelMessage<T> {
    /// An item of the plugin.
    Item {
        /// The item.
        item: T,
    },
    /// The host ended the channel.
    Ended {
        /// Why.
        reason: SubscriptionEnd,
    },
}

/// Why the host refused a call of a plugin's command or channel, before its
/// handler or instead of its answer (decision 9). The host answers these the
/// same way for every plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum HostRefusal {
    /// The plugin is not on, or it was turned off while the call ran.
    #[error("the plugin {plugin} is off")]
    PluginOff {
        /// The plugin's ID.
        plugin: String,
    },
    /// The core started without its database.
    #[error("the database is not open")]
    StoreUnavailable,
    /// Mannie is quitting.
    #[error("Mannie is quitting")]
    ShuttingDown,
    /// The plugin has no such command or channel.
    #[error("the plugin has no such command")]
    UnknownCommand,
    /// The input did not decode, or a check refused it.
    #[error("the input is not valid")]
    InvalidInput {
        /// The field that a check refused, when one did.
        field: Option<String>,
    },
    /// The input is larger than the command accepts.
    #[error("the input is larger than {max_bytes} bytes")]
    InputTooLarge {
        /// The limit.
        max_bytes: u32,
    },
    /// The host could not complete the call, such as when the handler
    /// panicked. The log has the cause.
    #[error("the host could not complete the call")]
    HostServiceFailed,
    /// Tauri's access control refused the call before it reached the host.
    /// The generated transport reports it, and the host never answers it.
    #[error("the call is not allowed")]
    NotAllowed,
}

impl HostRefusal {
    /// The codes of the host's refusals. A plugin's error uses none of them,
    /// which the bindings exporter checks, so the UI tells a refusal from
    /// the plugin's own error by its code.
    pub const CODES: &'static [&'static str] = &[
        "plugin_off",
        "store_unavailable",
        "shutting_down",
        "unknown_command",
        "invalid_input",
        "input_too_large",
        "host_service_failed",
        "not_allowed",
    ];

    /// The refusal of a call of `plugin`, which is off.
    pub fn plugin_off(plugin: &str) -> Self {
        Self::PluginOff {
            plugin: plugin.to_owned(),
        }
    }

    /// The refusal's code, as it crosses IPC and as the host logs it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::PluginOff { .. } => "plugin_off",
            Self::StoreUnavailable => "store_unavailable",
            Self::ShuttingDown => "shutting_down",
            Self::UnknownCommand => "unknown_command",
            Self::InvalidInput { .. } => "invalid_input",
            Self::InputTooLarge { .. } => "input_too_large",
            Self::HostServiceFailed => "host_service_failed",
            Self::NotAllowed => "not_allowed",
        }
    }
}

/// Why a call of a plugin's command or channel failed, as it crosses IPC
/// inside a `CommandError` (decision 9): a refusal of the host, or the
/// plugin's own error, each with its `code` beside the `origin`.
///
/// The bindings exporter writes its TypeScript itself, so it derives no
/// `specta::Type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum PluginFailure<E> {
    /// The host refused the call.
    Host(HostRefusal),
    /// The plugin's own error.
    Plugin(E),
}

#[cfg(test)]
mod tests {
    use super::{HostRefusal, InputRefusal, Validate};

    fn every_refusal() -> Vec<HostRefusal> {
        vec![
            HostRefusal::plugin_off("images"),
            HostRefusal::StoreUnavailable,
            HostRefusal::ShuttingDown,
            HostRefusal::UnknownCommand,
            HostRefusal::InvalidInput { field: None },
            HostRefusal::InputTooLarge { max_bytes: 65_536 },
            HostRefusal::HostServiceFailed,
            HostRefusal::NotAllowed,
        ]
    }

    #[test]
    fn every_refusal_serializes_with_its_code_and_the_codes_list_them_all() {
        let codes: Vec<String> = every_refusal()
            .iter()
            .map(|refusal| {
                let json = serde_json::to_value(refusal).unwrap();
                let code = json["code"].as_str().unwrap().to_owned();
                assert_eq!(code, refusal.code());
                code
            })
            .collect();

        assert_eq!(codes, HostRefusal::CODES);
    }

    #[test]
    fn a_refusal_carries_its_arguments_in_camel_case() {
        assert_eq!(
            serde_json::to_value(HostRefusal::InputTooLarge { max_bytes: 10 }).unwrap(),
            serde_json::json!({ "code": "input_too_large", "maxBytes": 10 })
        );
        assert_eq!(
            serde_json::to_value(HostRefusal::InvalidInput {
                field: Some("name".to_owned())
            })
            .unwrap(),
            serde_json::json!({ "code": "invalid_input", "field": "name" })
        );
    }

    #[test]
    fn an_input_without_fields_has_no_checks() {
        assert_eq!(().validate(), Ok(()));
        assert_eq!(InputRefusal::new("name").field, "name");
    }
}
