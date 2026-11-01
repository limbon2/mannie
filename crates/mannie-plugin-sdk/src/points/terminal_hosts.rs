//! Terminal hosts (ADR-0053, decision 3): hosts that a plugin offers to
//! terminals, such as the hosts that the owner registered with the SSH
//! plugin.
//!
//! A plugin contributes a source when it starts. The core takes the source's
//! stream of hosts once; each item is the complete list of the source's hosts
//! at that moment and replaces the one before, and a stream that ends empties
//! it. The core keeps the hosts in its registry of terminals, which the
//! service `terminals` lists and opens, so no plugin reads another plugin's
//! hosts. The core opens a terminal on a host through the source's `open`,
//! when the owner opens one or when an approved command needs one.
//!
//! A source's ID follows the rule `Owned`: the plugin's ID, or the plugin's
//! ID, a dot and more (ADR-0034, decision 6).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::contributions::{ExtensionPoint, IdRule};
use crate::services::terminals::{
    ExpectedHost, ShellKind, TerminalEnding, TerminalHostState, TerminalIoError, TerminalOutput,
    TerminalSize,
};
use crate::{BoxFuture, BoxStream};

/// The point of plugins' terminal hosts, whose entries are named by their
/// IDs.
#[derive(Debug)]
pub enum TerminalHosts {}

impl ExtensionPoint for TerminalHosts {
    const NAME: &'static str = "terminal-hosts";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Owned;
    type Entry = PluginTerminalHostSource;

    fn id(entry: &PluginTerminalHostSource) -> &str {
        &entry.id
    }
}

/// The most hosts of one source that the core keeps at a time. A list with
/// more keeps its first ones.
pub const MAX_SOURCE_HOSTS: usize = 256;

/// The most characters of a host's key.
pub const MAX_HOST_KEY_CHARS: usize = 64;

/// The most characters of a host's name.
pub const MAX_HOST_LABEL_CHARS: usize = 64;

/// The most characters of a host's endpoint or identity.
pub const MAX_HOST_ENDPOINT_CHARS: usize = 512;

/// Hosts that a plugin offers to terminals.
pub trait TerminalHostSource: Send + Sync + fmt::Debug + 'static {
    /// The source's hosts, each item the complete list that replaces the one
    /// before. The core takes the stream once, when the plugin starts, and
    /// drops it when the plugin stops.
    fn hosts(&self) -> BoxStream<'static, Vec<TerminalHostEntry>>;

    /// Opens a terminal on one of the source's hosts. The source checks the
    /// request's expected endpoint and identity before it logs in, and
    /// refuses with [`TerminalOpenFailure::HostChanged`] when either differs.
    fn open(
        &self,
        request: OpenTerminal,
    ) -> BoxFuture<'static, Result<Box<dyn TerminalConnection>, TerminalOpenFailure>>;
}

/// A host as its source lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct TerminalHostEntry {
    /// The source's key for the host, which stays while the host exists and
    /// is never reused for another: 1 to [`MAX_HOST_KEY_CHARS`] letters,
    /// digits and hyphens.
    pub key: String,
    /// The host's name, as the owner gave it.
    pub label: String,
    /// Where terminals run there, such as `deploy@build-box.lan:22`, without
    /// any credential.
    pub endpoint: String,
    /// What identifies the host beyond its address, such as the fingerprint
    /// of its pinned key.
    pub identity: String,
    /// Whether terminals open there now.
    pub state: TerminalHostState,
    /// The host's login shell, when the source knows it.
    pub shell: Option<ShellKind>,
}

impl TerminalHostEntry {
    /// A ready host with these parts. The texts are easy to swap, so callers
    /// pass named locals.
    pub fn new(key: String, label: String, endpoint: String, identity: String) -> Self {
        Self {
            key,
            label,
            endpoint,
            identity,
            state: TerminalHostState::Ready,
            shell: None,
        }
    }

    /// The same host, in another state.
    #[must_use]
    pub fn with_state(mut self, state: TerminalHostState) -> Self {
        self.state = state;
        self
    }

    /// The same host, with its login shell.
    #[must_use]
    pub fn with_shell(mut self, shell: ShellKind) -> Self {
        self.shell = Some(shell);
        self
    }
}

/// What the core asks a source to open.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpenTerminal {
    /// The source's key for the host.
    pub key: String,
    /// The terminal's size.
    pub size: TerminalSize,
    /// For a terminal that an approved command opens, the endpoint and
    /// identity that the approval named.
    pub expect: Option<ExpectedHost>,
}

impl OpenTerminal {
    /// A request for a terminal of `size` on the host `key`. Only the core
    /// makes one.
    #[cfg(feature = "host")]
    pub fn new(key: String, size: TerminalSize, expect: Option<ExpectedHost>) -> Self {
        Self { key, size, expect }
    }
}

/// A terminal on a source's host, as the source hands it to the core.
pub trait TerminalConnection: Send + fmt::Debug {
    /// The terminal's output as it arrives, then how it ended. The core takes
    /// it once.
    fn output(&mut self) -> BoxStream<'static, TerminalOutput>;

    /// Writes bytes to the terminal's input, after the bytes of every earlier
    /// call.
    fn write(&self, bytes: Vec<u8>) -> BoxFuture<'static, Result<(), TerminalIoError>>;

    /// Changes the terminal's size.
    fn resize(&self, size: TerminalSize) -> BoxFuture<'static, Result<(), TerminalIoError>>;

    /// Hangs up the terminal, waits at most `grace` for its shell to end,
    /// then closes the connection. Resolves with how it ended.
    fn close(self: Box<Self>, grace: Duration) -> BoxFuture<'static, TerminalEnding>;
}

/// Why a source did not open a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum TerminalOpenFailure {
    /// The source has no host with the key.
    #[error("no host has this key")]
    UnknownHost,
    /// The host's endpoint or identity is not the expected one.
    #[error("the host changed since the approval")]
    HostChanged,
    /// The source could not open the terminal, with its code, such as
    /// `connection_failed` or `login_failed`: 1 to 64 lower-case letters and
    /// underscores.
    #[error("the terminal could not be opened: {reason}")]
    Failed {
        /// The source's code.
        reason: String,
    },
}

/// A plugin's source of terminal hosts, under its ID.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PluginTerminalHostSource {
    /// The plugin's ID, or the plugin's ID, a dot and more.
    pub id: String,
    /// The source.
    pub source: Arc<dyn TerminalHostSource>,
}

impl PluginTerminalHostSource {
    /// A source under `id`.
    pub fn new(id: impl Into<String>, source: Arc<dyn TerminalHostSource>) -> Self {
        Self {
            id: id.into(),
            source,
        }
    }
}

/// Whether a host's entry keeps the point's rules: a key of 1 to
/// [`MAX_HOST_KEY_CHARS`] letters, digits and hyphens, a name of 1 to
/// [`MAX_HOST_LABEL_CHARS`] characters, an endpoint and an identity of at
/// most [`MAX_HOST_ENDPOINT_CHARS`], none with a control character, and an
/// unavailable state's code of 1 to 64 lower-case letters and underscores.
/// The core leaves out an entry that breaks them, with a log line, and a
/// source applies the same check before it lists a host.
pub fn check_host_entry(entry: &TerminalHostEntry) -> Result<(), &'static str> {
    let key_fits = (1..=MAX_HOST_KEY_CHARS).contains(&entry.key.chars().count())
        && entry
            .key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-');
    if !key_fits {
        return Err("key");
    }
    let plain = |text: &str, most: usize| {
        let count = text.chars().count();
        count <= most && !text.chars().any(char::is_control)
    };
    if entry.label.is_empty() || !plain(&entry.label, MAX_HOST_LABEL_CHARS) {
        return Err("label");
    }
    if entry.endpoint.is_empty() || !plain(&entry.endpoint, MAX_HOST_ENDPOINT_CHARS) {
        return Err("endpoint");
    }
    if !plain(&entry.identity, MAX_HOST_ENDPOINT_CHARS) {
        return Err("identity");
    }
    if let TerminalHostState::Unavailable { code } = &entry.state
        && !is_code(code)
    {
        return Err("state");
    }
    Ok(())
}

/// Whether a text is a code: 1 to 64 lower-case letters and underscores.
pub(crate) fn is_code(code: &str) -> bool {
    (1..=64).contains(&code.len())
        && code
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::{TerminalHostEntry, check_host_entry};
    use crate::services::terminals::TerminalHostState;

    fn entry() -> TerminalHostEntry {
        TerminalHostEntry::new(
            "0199e0f2-0000-7000-8000-0000000000aa".to_owned(),
            "build-box".to_owned(),
            "deploy@build-box.lan:22".to_owned(),
            "SHA256:abc".to_owned(),
        )
    }

    #[test]
    fn an_entry_keeps_the_rules_of_the_point() {
        assert_eq!(check_host_entry(&entry()), Ok(()));

        let mut bad_key = entry();
        "host/1".clone_into(&mut bad_key.key);
        assert_eq!(check_host_entry(&bad_key), Err("key"));

        let mut bad_label = entry();
        "build\nbox".clone_into(&mut bad_label.label);
        assert_eq!(check_host_entry(&bad_label), Err("label"));

        let bad_state = entry().with_state(TerminalHostState::Unavailable {
            code: "Key Changed".to_owned(),
        });
        assert_eq!(check_host_entry(&bad_state), Err("state"));

        let unavailable = entry().with_state(TerminalHostState::Unavailable {
            code: "key_changed".to_owned(),
        });
        assert_eq!(check_host_entry(&unavailable), Ok(()));
    }
}
