//! The plugin's commands and channel, as specs of the plugin SDK (ADR-0034,
//! decision 8), so the host checks their input before the plugin's handlers
//! run and answers `plugin_off` while the plugin is off.
//!
//! No command runs anything on a host: they register hosts, check a host's
//! key and test its login on the owner's action, as pairing a worker does
//! (ADR-0036, decision 9). Only the tool `run_ssh_command` runs commands,
//! through the core's policy.

use mannie_plugin_sdk::contracts::{Secret, UtcMillis};
use mannie_plugin_sdk::ipc::{
    ChannelSpec, CommandSpec, InputRefusal, Interface, PluginError, Validate,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::rules::{
    DEFAULT_PORT, MAX_NAME_CHARS, MAX_NOTE_CHARS, MAX_PASSWORD_BYTES, MAX_USERNAME_CHARS,
    is_host_name, is_note, is_password, is_port, is_username, stored_address,
};

/// The shortest password that the plugin finds in a command's output and
/// replaces, as the log redactor does with its secrets. A shorter one could
/// match ordinary text, so it is left as it is.
pub const MIN_REDACTED_PASSWORD_BYTES: u32 = 8;

/// The argument of `add_host`.
#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddHost {
    /// The host's name, unique without regard to case.
    pub name: String,
    /// A host name, an IPv4 address or an IPv6 address.
    pub address: String,
    /// The port of the host's SSH server.
    pub port: u16,
    /// The account that commands run as.
    pub username: String,
    /// The account's password, which goes to the credential store and never
    /// back to the webview.
    pub password: Secret,
    /// What the owner tells the model about the host.
    #[serde(default)]
    pub note: String,
}

impl Validate for AddHost {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_host(
            &self.name,
            &self.address,
            self.port,
            &self.username,
            &self.note,
        )?;
        if !is_password(self.password.expose()) {
            return Err(InputRefusal::new("password"));
        }
        Ok(())
    }
}

/// The argument of `update_host`.
#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateHost {
    /// The host.
    pub host_id: String,
    /// The host's name, unique without regard to case.
    pub name: String,
    /// A host name, an IPv4 address or an IPv6 address. Another address or
    /// port lets go of the host's pinned key.
    pub address: String,
    /// The port of the host's SSH server.
    pub port: u16,
    /// The account that commands run as.
    pub username: String,
    /// A new password, or none to keep the stored one.
    #[serde(default)]
    pub password: Option<Secret>,
    /// What the owner tells the model about the host.
    #[serde(default)]
    pub note: String,
}

impl Validate for UpdateHost {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_host_id(&self.host_id)?;
        check_host(
            &self.name,
            &self.address,
            self.port,
            &self.username,
            &self.note,
        )?;
        let password_is_invalid = self
            .password
            .as_ref()
            .is_some_and(|password| !is_password(password.expose()));
        if password_is_invalid {
            return Err(InputRefusal::new("password"));
        }
        Ok(())
    }
}

/// The fields that adding and changing a host share.
fn check_host(
    name: &str,
    address: &str,
    port: u16,
    username: &str,
    note: &str,
) -> Result<(), InputRefusal> {
    if !is_host_name(name) {
        return Err(InputRefusal::new("name"));
    }
    if stored_address(address).is_none() {
        return Err(InputRefusal::new("address"));
    }
    if !is_port(port) {
        return Err(InputRefusal::new("port"));
    }
    if !is_username(username) {
        return Err(InputRefusal::new("username"));
    }
    if !is_note(note) {
        return Err(InputRefusal::new("note"));
    }
    Ok(())
}

/// Whether `host_id` can be a host's ID, which the plugin makes as a UUID.
fn check_host_id(host_id: &str) -> Result<(), InputRefusal> {
    if uuid::Uuid::try_parse(host_id).is_err() {
        return Err(InputRefusal::new("hostId"));
    }
    Ok(())
}

/// A host, as a command names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostRef {
    /// The host.
    pub host_id: String,
}

impl Validate for HostRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_host_id(&self.host_id)
    }
}

/// The argument of `trust_host_key`: the key that the owner checked, by the
/// fingerprint that the last scan of the host showed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrustHostKey {
    /// The host.
    pub host_id: String,
    /// The key's SHA-256 fingerprint, as `SHA256:...`.
    pub fingerprint: String,
}

impl Validate for TrustHostKey {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_host_id(&self.host_id)?;
        let is_fingerprint = self.fingerprint.starts_with("SHA256:")
            && self.fingerprint.len() <= 128
            && self.fingerprint.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'+' | b'/' | b'=')
            });
        if !is_fingerprint {
            return Err(InputRefusal::new("fingerprint"));
        }
        Ok(())
    }
}

/// A host's key, as the owner compares it with the host's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyView {
    /// The key's algorithm, such as `ssh-ed25519`.
    pub algorithm: String,
    /// The key's SHA-256 fingerprint, as `ssh-keygen -l` prints it.
    pub fingerprint: String,
    /// When the plugin pinned or saw the key.
    pub at: UtcMillis,
}

/// Whether a host can run commands, and what the owner does about it if not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HostStatus {
    /// Its key is pinned and its password stored.
    Ready,
    /// No key is pinned yet: the owner checks the key's fingerprint and
    /// trusts it.
    KeyNotChecked,
    /// The host showed another key than the pinned one, so no password goes
    /// to it until the owner trusts the new key.
    KeyChanged,
    /// The credential store holds no password for the host.
    PasswordMissing,
}

/// A host, as the settings show it. The password never leaves the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HostView {
    /// The host's ID.
    pub id: String,
    /// The host's name.
    pub name: String,
    /// What the owner tells the model about the host.
    pub note: String,
    /// The host's address, an IPv6 address without brackets.
    pub address: String,
    /// The port of its SSH server.
    pub port: u16,
    /// The account that commands run as.
    pub username: String,
    /// Where commands run, as approvals show it: `user@address:port`.
    pub endpoint: String,
    /// Whether the credential store holds the host's password.
    pub password_set: bool,
    /// The pinned key, or `None` before the owner trusted one.
    pub key: Option<HostKeyView>,
    /// The key that the host showed last, when it is not the pinned one.
    pub seen_key: Option<HostKeyView>,
    /// Whether the host can run commands.
    pub status: HostStatus,
}

/// How a scanned key compares with the pinned one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum KeyComparison {
    /// No key is pinned yet.
    NotPinned,
    /// The host showed the pinned key.
    Same,
    /// The host showed another key than the pinned one.
    Changed,
}

/// What `scan_host_key` found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyScan {
    /// The key that the host showed.
    pub key: HostKeyView,
    /// How it compares with the pinned key.
    pub comparison: KeyComparison,
}

/// What the channel `follow_hosts` carries: the whole list after each
/// change. More kinds of updates may come, by addition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostListUpdate {
    /// The hosts, by name.
    Hosts {
        /// The hosts.
        hosts: Vec<HostView>,
    },
}

/// Why a command or the channel of the plugin failed. The host's refusals,
/// such as `plugin_off` and `invalid_input` with the refused field, come
/// beside these (ADR-0034, decision 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum SshError {
    /// Another host has the name, without regard to case.
    #[error("another host has this name")]
    NameTaken,
    /// The host is not registered, or was removed meanwhile.
    #[error("the host is not registered")]
    HostNotFound,
    /// The address could not be resolved, or nothing answered at it.
    #[error("the host could not be reached")]
    Unreachable,
    /// The host did not answer in time.
    #[error("the host did not answer in time")]
    TimedOut,
    /// The host does not speak SSH as the plugin does, or ended the
    /// connection during the handshake.
    #[error("the SSH handshake with the host failed")]
    HandshakeFailed,
    /// The host offers only keys that the plugin does not accept, such as
    /// RSA keys.
    #[error("the host offers only keys that the plugin does not accept")]
    HostKeyUnsupported,
    /// The host showed another key than the pinned one, so the password was
    /// not sent.
    #[error("the host showed another key than the pinned one")]
    HostKeyChanged,
    /// No key is pinned for the host yet.
    #[error("no key is pinned for the host")]
    HostKeyNotTrusted,
    /// The key to trust is not the one that the last scan of the host saw.
    #[error("the key is not the one that the last scan saw")]
    ScanOutdated,
    /// The host refused the user name or the password.
    #[error("the host refused the login")]
    AuthFailed,
    /// The host takes neither a password nor a password through
    /// keyboard-interactive login.
    #[error("the host takes no password login")]
    AuthMethodUnsupported,
    /// The credential store holds no password for the host.
    #[error("the host has no password")]
    PasswordMissing,
    /// The credential store could not be used.
    #[error("the credential store is not available")]
    CredentialStoreUnavailable,
    /// The store did not keep the change.
    #[error("the change could not be stored")]
    StorageFailed,
}

impl PluginError for SshError {
    fn code(&self) -> &'static str {
        match self {
            Self::NameTaken => "ssh.name_taken",
            Self::HostNotFound => "ssh.host_not_found",
            Self::Unreachable => "ssh.unreachable",
            Self::TimedOut => "ssh.timed_out",
            Self::HandshakeFailed => "ssh.handshake_failed",
            Self::HostKeyUnsupported => "ssh.host_key_unsupported",
            Self::HostKeyChanged => "ssh.host_key_changed",
            Self::HostKeyNotTrusted => "ssh.host_key_not_trusted",
            Self::ScanOutdated => "ssh.scan_outdated",
            Self::AuthFailed => "ssh.auth_failed",
            Self::AuthMethodUnsupported => "ssh.auth_method_unsupported",
            Self::PasswordMissing => "ssh.password_missing",
            Self::CredentialStoreUnavailable => "ssh.credential_store_unavailable",
            Self::StorageFailed => "ssh.storage_failed",
        }
    }
}

/// `add_host`: registers a host and keeps its password in the credential
/// store.
#[derive(Debug)]
pub enum AddHostCommand {}

impl CommandSpec for AddHostCommand {
    const NAME: &'static str = "add_host";
    const DOCS: &'static str = "Registers a host and keeps its password in the credential store. Nothing connects to the host.";
    type Input = AddHost;
    type Output = HostView;
    type Error = SshError;
}

/// `update_host`: changes a host, and its password when one is given.
#[derive(Debug)]
pub enum UpdateHostCommand {}

impl CommandSpec for UpdateHostCommand {
    const NAME: &'static str = "update_host";
    const DOCS: &'static str = "Changes a host, and its password when the input has one. Another address or port lets go of its pinned key.";
    type Input = UpdateHost;
    type Output = HostView;
    type Error = SshError;
}

/// `remove_host`: forgets a host and deletes its password.
#[derive(Debug)]
pub enum RemoveHostCommand {}

impl CommandSpec for RemoveHostCommand {
    const NAME: &'static str = "remove_host";
    const DOCS: &'static str = "Deletes a host's password from the credential store, then forgets the host and closes its connections.";
    type Input = HostRef;
    type Output = ();
    type Error = SshError;
}

/// `scan_host_key`: reads the key that a host shows, without logging in.
#[derive(Debug)]
pub enum ScanHostKeyCommand {}

impl CommandSpec for ScanHostKeyCommand {
    const NAME: &'static str = "scan_host_key";
    const DOCS: &'static str = "Connects to a host up to the key exchange, records the key that it shows and compares it with the pinned key. No password is sent.";
    type Input = HostRef;
    type Output = HostKeyScan;
    type Error = SshError;
}

/// `trust_host_key`: pins the key that the last scan saw.
#[derive(Debug)]
pub enum TrustHostKeyCommand {}

impl CommandSpec for TrustHostKeyCommand {
    const NAME: &'static str = "trust_host_key";
    const DOCS: &'static str = "Pins the key that the last scan of the host saw, when its fingerprint is the one that the owner checked.";
    type Input = TrustHostKey;
    type Output = HostView;
    type Error = SshError;
}

/// `test_host`: logs in to a host and runs nothing.
#[derive(Debug)]
pub enum TestHostCommand {}

impl CommandSpec for TestHostCommand {
    const NAME: &'static str = "test_host";
    const DOCS: &'static str =
        "Connects to a host with its pinned key, logs in with its password and runs nothing.";
    type Input = HostRef;
    type Output = ();
    type Error = SshError;
}

/// `follow_hosts`: the hosts as they are, then after each change.
#[derive(Debug)]
pub enum FollowHostsChannel {}

impl ChannelSpec for FollowHostsChannel {
    const NAME: &'static str = "follow_hosts";
    const DOCS: &'static str =
        "The registered hosts as they are, then after each change, until the plugin stops.";
    type Input = ();
    type Item = HostListUpdate;
    type Error = SshError;
}

/// The plugin's commands, its channel and the limits that the UI checks.
pub fn interface() -> Interface {
    Interface::new()
        .command::<AddHostCommand>()
        .command::<UpdateHostCommand>()
        .command::<RemoveHostCommand>()
        .command::<ScanHostKeyCommand>()
        .command::<TrustHostKeyCommand>()
        .command::<TestHostCommand>()
        .channel::<FollowHostsChannel>()
        .constant(
            "MAX_NAME_CHARS",
            "The most characters of a host's name.",
            MAX_NAME_CHARS,
        )
        .constant(
            "MAX_USERNAME_CHARS",
            "The most characters of a user name.",
            MAX_USERNAME_CHARS,
        )
        .constant(
            "MAX_PASSWORD_BYTES",
            "The most bytes of a password.",
            MAX_PASSWORD_BYTES,
        )
        .constant(
            "MAX_NOTE_CHARS",
            "The most characters of the note about a host.",
            MAX_NOTE_CHARS,
        )
        .constant(
            "DEFAULT_PORT",
            "The port of SSH, which a new host starts with.",
            u32::from(DEFAULT_PORT),
        )
        .constant(
            "MIN_REDACTED_PASSWORD_BYTES",
            "The shortest password that the plugin replaces in a command's output.",
            MIN_REDACTED_PASSWORD_BYTES,
        )
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::Secret;
    use mannie_plugin_sdk::ipc::Validate;

    use super::{AddHost, HostRef, TrustHostKey, UpdateHost};

    fn add_host() -> AddHost {
        AddHost {
            name: "build-box".to_owned(),
            address: "build-box.lan".to_owned(),
            port: 22,
            username: "deploy".to_owned(),
            password: Secret::new("correct horse".to_owned()),
            note: "Builds the releases.".to_owned(),
        }
    }

    fn refused_field(input: &impl Validate) -> Option<String> {
        input.validate().err().map(|refusal| refusal.field)
    }

    /// A change that breaks one rule of a host.
    type Break = fn(&mut AddHost);

    #[test]
    fn a_host_is_refused_by_the_first_field_that_breaks_a_rule() {
        assert_eq!(refused_field(&add_host()), None);
        let cases: [(&str, Break); 6] = [
            ("name", |host| host.name = "-box".to_owned()),
            ("address", |host| host.address = "ssh://box".to_owned()),
            ("port", |host| host.port = 0),
            ("username", |host| host.username = "two words".to_owned()),
            ("note", |host| host.note = "bell\u{7}".to_owned()),
            ("password", |host| {
                host.password = Secret::new(String::new());
            }),
        ];
        for (field, change) in cases {
            let mut host = add_host();
            change(&mut host);

            assert_eq!(refused_field(&host).as_deref(), Some(field));
        }
    }

    #[test]
    fn a_change_keeps_the_password_unless_it_names_a_valid_one() {
        let change = |password: Option<&str>| UpdateHost {
            host_id: "0199e0f2-0000-7000-8000-0000000000aa".to_owned(),
            name: "build-box".to_owned(),
            address: "10.0.0.5".to_owned(),
            port: 2222,
            username: "deploy".to_owned(),
            password: password.map(|password| Secret::new(password.to_owned())),
            note: String::new(),
        };

        assert_eq!(refused_field(&change(None)), None);
        assert_eq!(refused_field(&change(Some("new password"))), None);
        assert_eq!(
            refused_field(&change(Some("nul\0byte"))).as_deref(),
            Some("password")
        );
        let mut unknown = change(None);
        unknown.host_id = "box".to_owned();
        assert_eq!(refused_field(&unknown).as_deref(), Some("hostId"));
    }

    #[test]
    fn a_host_reference_and_a_fingerprint_are_checked() {
        let host = HostRef {
            host_id: "0199e0f2-0000-7000-8000-0000000000aa".to_owned(),
        };
        let trust = |fingerprint: &str| TrustHostKey {
            host_id: host.host_id.clone(),
            fingerprint: fingerprint.to_owned(),
        };

        assert_eq!(refused_field(&host), None);
        assert_eq!(
            refused_field(&trust("SHA256:uX0dFmYbVKUH4bZmL1nOb4TaZ5WPMbgUpkOvR3EJPr0")),
            None
        );
        for fingerprint in ["", "MD5:aa:bb", "SHA256:has space", "SHA256:<script>"] {
            assert_eq!(
                refused_field(&trust(fingerprint)).as_deref(),
                Some("fingerprint"),
                "{fingerprint}"
            );
        }
    }
}
