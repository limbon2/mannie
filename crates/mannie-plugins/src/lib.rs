//! What Mannie's internal plugins build on (ADR-0030): the definition of a
//! plugin, the context that its start receives with the services that the
//! composition root grants it, and the host that starts and stops plugins and
//! keeps the owner's choices.
//!
//! A plugin's code lives in `plugins/<id>/` and depends on the core; the core
//! never depends on a plugin (decision 10). The desktop app's composition root
//! lists the plugins in code and runs the host.

mod host;

use std::fmt;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use mannie_runtime::{CredentialLender, ExecutorServices, PluginContributions};
use mannie_store::{ModuleStore, StoreModule};
use mannie_types::clock::Clock;
use mannie_types::secret::Secret;

pub use host::{HostError, HostedPlugin, PluginHost};

/// An internal plugin (ADR-0030, decision 3).
pub trait Plugin: Send + Sync + fmt::Debug + 'static {
    /// The plugin's ID: lower-case words joined by hyphens, such as
    /// `workers`. It names the plugin's folders and, with underscores in
    /// place of hyphens, prefixes its tables, its settings and its messages.
    fn id(&self) -> &'static str;

    /// The plugin's modules of the store. The composition root registers
    /// them whether or not the plugin is on, so the schema never depends on
    /// the plugin's state (decision 5).
    fn store_modules(&self) -> Vec<StoreModule>;

    /// Whether the plugin starts when the owner has not chosen yet, for the
    /// data that `stores`, the plugin's modules, hold (decision 7).
    fn default_on(&self, stores: Vec<ModuleStore>) -> BoxFuture<'static, bool>;

    /// Starts the plugin and returns what it contributes while it is on
    /// (decision 6). A start that fails leaves nothing of the plugin running.
    fn start(
        &self,
        context: StartContext,
    ) -> BoxFuture<'static, Result<PluginContributions, PluginStartError>>;

    /// Ends the plugin's background work, after the host dropped its
    /// contributions. Work in progress ends as it ends when Mannie quits, and
    /// nothing is repeated or lost.
    fn stop(&self) -> BoxFuture<'static, ()>;
}

/// Why a plugin could not start, as its stable code, which the Plugins page
/// shows with the plugin's message for it and "Try again".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the plugin could not start: {code}")]
pub struct PluginStartError {
    /// The plugin's code, such as `storage_failed`.
    pub code: String,
}

impl PluginStartError {
    /// The error with this code.
    pub fn new(code: &str) -> Self {
        Self {
            code: code.to_owned(),
        }
    }
}

/// The code of a start whose context lacks a service that the plugin needs,
/// which the composition root did not grant it.
pub const GRANT_MISSING: &str = "grant_missing";

/// The code of a plugin whose contributions the runtime refused, such as a
/// tool whose name another plugin uses.
pub const CONTRIBUTIONS_REFUSED: &str = "contributions_refused";

/// The core services that the composition root grants a plugin, which its
/// start receives. The start context is the only way that a plugin reaches
/// the core, so the composition root shows in one place what each plugin can
/// use (ADR-0030, decision 13).
#[derive(Debug, Clone, Default)]
pub struct Grants {
    /// The runtime's services for a plugin's task executor.
    pub executors: Option<ExecutorServices>,
    /// Lends provider credentials for the tasks of the plugin's executor.
    pub credentials: Option<CredentialLender>,
    /// The operating system's credential store, limited to the plugin's own
    /// entries.
    pub secrets: Option<Arc<dyn PluginSecrets>>,
}

/// Why a plugin's secret could not be read or kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SecretsError {
    /// The credential store cannot be used.
    #[error("the credential store cannot be used")]
    Unavailable,
    /// The entry does not belong to the plugin.
    #[error("the entry does not belong to the plugin")]
    NotPermitted,
}

/// The operating system's credential store, limited to the entries that a
/// plugin owns (ADR-0030, decision 13). The calls may block; a plugin makes
/// them on blocking threads.
pub trait PluginSecrets: Send + Sync + fmt::Debug + 'static {
    /// The secret of one of the plugin's entries, or `None` when it has none.
    fn load(&self, entry: &str) -> Result<Option<Secret>, SecretsError>;

    /// Keeps a secret under one of the plugin's entries.
    fn save(&self, entry: &str, secret: &Secret) -> Result<(), SecretsError>;
}

/// What a plugin's start receives.
#[derive(Debug, Clone)]
pub struct StartContext {
    /// The store, opened to the plugin's modules, in the order of
    /// [`Plugin::store_modules`].
    pub stores: Vec<ModuleStore>,
    /// The services that the composition root granted the plugin.
    pub grants: Grants,
    /// The clock of every timestamp.
    pub clock: Arc<dyn Clock>,
    /// Mannie's version, such as `0.3.0`.
    pub version: String,
}

impl StartContext {
    /// The store of one of the plugin's modules.
    pub fn store(&self, module: &StoreModule) -> Option<&ModuleStore> {
        self.stores.iter().find(|store| store.name() == module.name)
    }
}

/// Whether an ID is lower-case words of letters and digits joined by single
/// hyphens, starting with a letter.
pub fn is_valid_plugin_id(id: &str) -> bool {
    let starts_with_letter = id
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase());
    starts_with_letter
        && id.split('-').all(|word| {
            !word.is_empty()
                && word
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::is_valid_plugin_id;

    #[test]
    fn plugin_ids_are_lower_case_words_joined_by_hyphens() {
        for valid in ["workers", "log-viewer", "jira2", "a-b-c"] {
            assert!(is_valid_plugin_id(valid), "{valid}");
        }
        for invalid in [
            "",
            "Workers",
            "log_viewer",
            "-workers",
            "workers-",
            "log--viewer",
            "2fa",
            "a b",
        ] {
            assert!(!is_valid_plugin_id(invalid), "{invalid}");
        }
    }
}
