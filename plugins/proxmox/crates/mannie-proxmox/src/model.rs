//! Domain records, validation rules, error codes and IPC DTOs for the Proxmox
//! plugin (ADR-0080).

use std::net::IpAddr;

use mannie_plugin_sdk::contracts::{ConversationId, Secret, UtcMillis};
use mannie_plugin_sdk::ipc::{InputRefusal, PluginError, Validate};
use serde::{Deserialize, Serialize};
use specta::Type;

/// The plugin's ID.
pub const PROXMOX: &str = "proxmox";

/// Default Proxmox VE HTTPS API and web UI port.
pub const DEFAULT_PORT: u16 = 8006;

/// Maximum characters of a Proxmox server display name.
pub const MAX_NAME_CHARS: usize = 64;
/// Maximum characters of a Proxmox server owner note.
pub const MAX_NOTE_CHARS: usize = 512;
/// Maximum characters of a Proxmox server hostname or IP address.
pub const MAX_HOST_CHARS: usize = 255;
/// Maximum characters of a Proxmox username with realm.
pub const MAX_USERNAME_CHARS: usize = 128;
/// Maximum bytes of a Proxmox password stored in the OS credential store.
pub const MAX_PASSWORD_BYTES: usize = 4096;
/// Maximum number of explicitly allowed QEMU VMs stored per Proxmox server.
pub const MAX_ALLOWED_VMS_PER_SERVER: usize = 512;

/// TLS trust identity string representing system certificate store validation.
pub const OS_TRUST_IDENTITY: &str = "os_trust";

/// Stable failure codes returned by Proxmox plugin commands, channels and tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ProxmoxError {
    /// A server with that name already exists (case-insensitive).
    #[error("a Proxmox server with that name already exists")]
    NameTaken,
    /// The referenced server ID does not exist.
    #[error("the Proxmox server was not found")]
    ServerNotFound,
    /// The referenced server has no stored password in the credential store.
    #[error("the Proxmox server has no password stored")]
    PasswordMissing,
    /// The OS credential store could not read or write the server's password.
    #[error("the credential store failed")]
    CredentialsFailed,
    /// SQLite storage operation failed.
    #[error("the Proxmox store operation failed")]
    StorageFailed,
    /// The VM is not in the owner's per-server allowlist.
    #[error("this virtual machine is not allowed for Mannie")]
    VmNotAllowed,
    /// The VM was not found on the Proxmox server.
    #[error("the virtual machine was not found on the server")]
    VmNotFound,
    /// The Proxmox server could not be reached over the network.
    #[error("could not connect to the Proxmox server: {detail}")]
    ConnectionFailed {
        /// Sanitized transport failure detail.
        detail: String,
    },
    /// The server's certificate is not trusted by the OS trust store and no leaf pin is set.
    #[error("the server's TLS certificate is not trusted by this computer")]
    CertificateUntrusted,
    /// The server presented a different leaf certificate than the pinned SHA-256 fingerprint.
    #[error("the server's TLS certificate does not match the pinned fingerprint")]
    CertificateMismatch,
    /// The server's pinned leaf certificate is expired or not yet valid.
    #[error("the server's TLS certificate is outside its validity period")]
    CertificateExpired,
    /// TLS certificate inspection failed to capture a valid certificate.
    #[error("could not inspect the server's TLS certificate: {detail}")]
    CertificateInspectFailed {
        /// Sanitized detail.
        detail: String,
    },
    /// The presented certificate changed between inspection and pinning.
    #[error("the presented certificate fingerprint does not match the inspected certificate")]
    PinMismatch,
    /// Proxmox rejected the username/realm or password.
    #[error("Proxmox authentication failed")]
    AuthenticationFailed,
    /// The Proxmox account requires two-factor authentication, which is unsupported in v1.
    #[error("two-factor authentication is not supported for Proxmox accounts")]
    TfaUnsupported,
    /// The Proxmox account lacks permissions for this operation.
    #[error("the Proxmox account lacks permission for this operation: {detail}")]
    PermissionDenied {
        /// Sanitized Proxmox privilege detail.
        detail: String,
    },
    /// The Proxmox API returned an unexpected error or malformed response.
    #[error("the Proxmox API returned an error: {detail}")]
    ApiError {
        /// Sanitized detail.
        detail: String,
    },
    /// The Proxmox task timed out before completing.
    #[error("the Proxmox operation timed out")]
    Timeout,
    /// The owner currently controls the live VNC console, blocking agent VNC operations.
    #[error("the owner currently has control of the VNC console")]
    OwnerInControl,
    /// The agent currently controls the live VNC console, blocking owner input until takeover.
    #[error("the agent currently has control of the VNC console")]
    AgentInControl,
    /// The requested VNC session is not open.
    #[error("no live VNC session is open for this virtual machine")]
    SessionNotFound,
    /// The concurrent VNC session cap has been reached.
    #[error("too many VNC console sessions are open")]
    TooManySessions,
    /// The VNC / WebSocket / RFB connection or protocol failed.
    #[error("the VNC console connection failed: {detail}")]
    VncFailed {
        /// Sanitized detail.
        detail: String,
    },
    /// An argument or VM state did not meet the operation's rules.
    #[error("invalid Proxmox request: {detail}")]
    InvalidRequest {
        /// Sanitized detail.
        detail: String,
    },
}

impl ProxmoxError {
    /// Returns the stable machine-readable code for tool failures (`snake_case`, no dots).
    pub fn tool_code(&self) -> &'static str {
        match self {
            Self::NameTaken => "name_taken",
            Self::ServerNotFound => "server_not_found",
            Self::PasswordMissing => "password_missing",
            Self::CredentialsFailed => "credentials_failed",
            Self::StorageFailed => "storage_failed",
            Self::VmNotAllowed => "vm_not_allowed",
            Self::VmNotFound => "vm_not_found",
            Self::ConnectionFailed { .. } => "connection_failed",
            Self::CertificateUntrusted => "certificate_untrusted",
            Self::CertificateMismatch => "certificate_mismatch",
            Self::CertificateExpired => "certificate_expired",
            Self::CertificateInspectFailed { .. } => "certificate_inspect_failed",
            Self::PinMismatch => "pin_mismatch",
            Self::AuthenticationFailed => "authentication_failed",
            Self::TfaUnsupported => "tfa_unsupported",
            Self::PermissionDenied { .. } => "permission_denied",
            Self::ApiError { .. } => "api_error",
            Self::Timeout => "timeout",
            Self::OwnerInControl => "owner_in_control",
            Self::AgentInControl => "agent_in_control",
            Self::SessionNotFound => "session_not_found",
            Self::TooManySessions => "too_many_sessions",
            Self::VncFailed { .. } => "vnc_failed",
            Self::InvalidRequest { .. } => "invalid_request",
        }
    }
}

impl PluginError for ProxmoxError {
    fn code(&self) -> &'static str {
        match self {
            Self::NameTaken => "proxmox.name_taken",
            Self::ServerNotFound => "proxmox.server_not_found",
            Self::PasswordMissing => "proxmox.password_missing",
            Self::CredentialsFailed => "proxmox.credentials_failed",
            Self::StorageFailed => "proxmox.storage_failed",
            Self::VmNotAllowed => "proxmox.vm_not_allowed",
            Self::VmNotFound => "proxmox.vm_not_found",
            Self::ConnectionFailed { .. } => "proxmox.connection_failed",
            Self::CertificateUntrusted => "proxmox.certificate_untrusted",
            Self::CertificateMismatch => "proxmox.certificate_mismatch",
            Self::CertificateExpired => "proxmox.certificate_expired",
            Self::CertificateInspectFailed { .. } => "proxmox.certificate_inspect_failed",
            Self::PinMismatch => "proxmox.pin_mismatch",
            Self::AuthenticationFailed => "proxmox.authentication_failed",
            Self::TfaUnsupported => "proxmox.tfa_unsupported",
            Self::PermissionDenied { .. } => "proxmox.permission_denied",
            Self::ApiError { .. } => "proxmox.api_error",
            Self::Timeout => "proxmox.timeout",
            Self::OwnerInControl => "proxmox.owner_in_control",
            Self::AgentInControl => "proxmox.agent_in_control",
            Self::SessionNotFound => "proxmox.session_not_found",
            Self::TooManySessions => "proxmox.too_many_sessions",
            Self::VncFailed { .. } => "proxmox.vnc_failed",
            Self::InvalidRequest { .. } => "proxmox.invalid_request",
        }
    }
}

/// Normalizes a server host string (hostname, IPv4, or bracketed/bare IPv6)
/// into its stored host form (bare IPv6 without brackets, lowercase DNS host),
/// or returns `None` when invalid or containing scheme/path/userinfo.
pub fn normalize_host(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let length_ok = !trimmed.is_empty() && trimmed.chars().count() <= MAX_HOST_CHARS;

    if !length_ok {
        return None;
    }

    let has_forbidden = trimmed.contains("://")
        || trimmed.contains('/')
        || trimmed.contains('?')
        || trimmed.contains('#')
        || trimmed.contains('@')
        || trimmed
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace());

    if has_forbidden {
        return None;
    }

    let unwrapped = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(trimmed);

    if let Ok(ip) = unwrapped.parse::<IpAddr>() {
        return Some(ip.to_string());
    }

    // Reject stray colons (ports belong in the separate `port` field) or brackets.
    if unwrapped.contains(':') || unwrapped.contains('[') || unwrapped.contains(']') {
        return None;
    }

    let candidate = format!("https://{unwrapped}");
    let parsed = url::Url::parse(&candidate).ok()?;
    let host = parsed.host_str()?;

    if host.is_empty() || host.len() > MAX_HOST_CHARS {
        return None;
    }

    Some(host.to_ascii_lowercase())
}

/// Formats the canonical HTTPS `Host` header value for a stored host and port,
/// omitting the default HTTPS port `443` (RFC 9110 §7.2).
pub fn canonical_host_header(host: &str, port: u16) -> String {
    let host_part = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };

    if port == 443 {
        host_part
    } else {
        format!("{host_part}:{port}")
    }
}

/// Formats the canonical HTTPS origin for a stored host and port, matching
/// `Url::origin().ascii_serialization()` (e.g. `https://pve.lan:8006` or
/// `https://pve.lan` when port is 443).
pub fn canonical_endpoint(host: &str, port: u16) -> String {
    format!("https://{}", canonical_host_header(host, port))
}

/// Validates a server display name.
pub fn is_valid_server_name(name: &str) -> bool {
    let trimmed = name.trim();
    let len = trimmed.chars().count();
    let len_ok = (1..=MAX_NAME_CHARS).contains(&len);
    len_ok && !trimmed.chars().any(char::is_control)
}

/// Validates a Proxmox username with realm (e.g. `user@pve` or `alice@pam`).
pub fn is_valid_username(username: &str) -> bool {
    let trimmed = username.trim();
    let len = trimmed.chars().count();
    let len_ok = (3..=MAX_USERNAME_CHARS).contains(&len);

    if !len_ok {
        return false;
    }

    let clean = !trimmed
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace());

    if !clean {
        return false;
    }

    let Some((user, realm)) = trimmed.rsplit_once('@') else {
        return false;
    };
    let parts_non_empty = !user.is_empty() && !realm.is_empty();
    let user_ok = user
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+'));
    let realm_ok = realm
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    parts_non_empty && user_ok && realm_ok
}

/// Validates a Proxmox node name.
pub fn is_valid_node(node: &str) -> bool {
    let mut bytes = node.bytes();
    let starts_alnum = bytes.next().is_some_and(|b| b.is_ascii_alphanumeric());
    let len_ok = starts_alnum && node.len() <= 64;

    if !len_ok {
        return false;
    }

    bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Validates an optional server note.
pub fn is_valid_note(note: &str) -> bool {
    note.chars().count() <= MAX_NOTE_CHARS && !note.contains('\0')
}

/// Validates a non-empty password within size bounds.
pub fn is_valid_password(password: &str) -> bool {
    !password.is_empty() && password.len() <= MAX_PASSWORD_BYTES && !password.contains('\0')
}

/// Validates a lowercase 64-hex-digit SHA-256 certificate digest (without prefix).
pub fn is_valid_sha256_hex(hex: &str) -> bool {
    hex.len() == 64
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

/// Normalizes a SHA-256 hex digest or `sha256:<hex>` or colon-separated uppercase
/// fingerprint into lowercase 64-character hex.
pub fn normalize_sha256_hex(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let without_prefix = trimmed
        .strip_prefix("sha256:")
        .or_else(|| trimmed.strip_prefix("SHA256:"))
        .unwrap_or(trimmed);
    let stripped: String = without_prefix
        .chars()
        .filter(|ch| *ch != ':' && *ch != ' ')
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    is_valid_sha256_hex(&stripped).then_some(stripped)
}

/// Formats a 64-character lowercase SHA-256 hex digest as `AA:BB:...` for human
/// verification in Settings.
pub fn format_fingerprint_colon(hex: &str) -> String {
    let upper = hex.to_ascii_uppercase();
    upper
        .as_bytes()
        .chunks(2)
        .filter_map(|pair| std::str::from_utf8(pair).ok())
        .collect::<Vec<_>>()
        .join(":")
}

/// Validates a UUID string for `server_id`.
fn check_server_id(server_id: &str) -> Result<(), InputRefusal> {
    if uuid::Uuid::try_parse(server_id).is_err() {
        return Err(InputRefusal::new("serverId"));
    }

    Ok(())
}

/// A Proxmox server row stored in SQLite. Passwords never live here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRecord {
    /// Server UUID.
    pub id: String,
    /// Owner-chosen server name.
    pub name: String,
    /// Hostname or IP address (IPv6 without brackets).
    pub host: String,
    /// HTTPS port (default 8006).
    pub port: u16,
    /// Proxmox user with realm (e.g. `user@pve`).
    pub username: String,
    /// Whether the credential store holds the password.
    pub password_set: bool,
    /// Pinned leaf certificate SHA-256 hex (64 lowercase chars), or `None` for OS trust.
    pub pinned_cert_sha256: Option<String>,
    /// Subject display string of the pinned certificate.
    pub pinned_cert_subject: Option<String>,
    /// When the certificate was pinned.
    pub pinned_cert_at: Option<i64>,
    /// Optional owner note for the model.
    pub note: String,
}

impl ServerRecord {
    /// Canonical `https://host[:port]` origin.
    pub fn endpoint(&self) -> String {
        canonical_endpoint(&self.host, self.port)
    }

    /// Canonical HTTPS `Host` header value (omits `:443`).
    pub fn host_header(&self) -> String {
        canonical_host_header(&self.host, self.port)
    }

    /// TLS trust identity used in `Access::ProxmoxVm` and `GrantScope::ProxmoxVm`.
    pub fn tls_identity(&self) -> String {
        match &self.pinned_cert_sha256 {
            Some(hex) => format!("sha256:{hex}"),
            None => OS_TRUST_IDENTITY.to_owned(),
        }
    }

    /// Computes readiness status from stored credentials and allowlist count.
    pub fn status(&self, allowed_vm_count: usize) -> ServerStatus {
        if !self.password_set {
            return ServerStatus::PasswordMissing;
        }

        if allowed_vm_count == 0 {
            return ServerStatus::NoVmsAllowed;
        }

        ServerStatus::Ready
    }

    /// Builds the UI view of this server with its explicitly allowed VMs.
    pub fn view(&self, allowed_vms: &[AllowedVmRecord]) -> ServerView {
        let pinned_fingerprint = self
            .pinned_cert_sha256
            .as_deref()
            .map(format_fingerprint_colon);
        ServerView {
            id: self.id.clone(),
            name: self.name.clone(),
            host: self.host.clone(),
            port: self.port,
            endpoint: self.endpoint(),
            username: self.username.clone(),
            password_set: self.password_set,
            pinned_cert_sha256: self.pinned_cert_sha256.clone(),
            pinned_cert_fingerprint: pinned_fingerprint,
            pinned_cert_subject: self.pinned_cert_subject.clone(),
            pinned_cert_at: self.pinned_cert_at.map(UtcMillis::from_millis),
            note: self.note.clone(),
            status: self.status(allowed_vms.len()),
            allowed_vms: allowed_vms.iter().map(AllowedVmRecord::view).collect(),
        }
    }
}

/// An explicitly allowed QEMU VM row in `proxmox_allowed_vms`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedVmRecord {
    /// Owning Proxmox server ID.
    pub server_id: String,
    /// Positive Proxmox QEMU VMID.
    pub vmid: u32,
    /// Last known Proxmox node name.
    pub node: String,
    /// Last known VM display name.
    pub name: String,
    /// When the owner added this VM to the allowlist.
    pub added_at: i64,
}

impl AllowedVmRecord {
    /// Builds the UI view for this allowed VM.
    pub fn view(&self) -> AllowedVmView {
        AllowedVmView {
            vmid: self.vmid,
            node: self.node.clone(),
            name: self.name.clone(),
            added_at: UtcMillis::from_millis(self.added_at),
        }
    }
}

/// Combined snapshot of one server and its allowed VMs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerState {
    /// Stored server record.
    pub server: ServerRecord,
    /// Explicitly allowed VMs on this server, sorted by VMID.
    pub allowed_vms: Vec<AllowedVmRecord>,
}

impl ServerState {
    /// Builds the Settings UI view.
    pub fn view(&self) -> ServerView {
        self.server.view(&self.allowed_vms)
    }
}

/// Readiness status of a registered Proxmox server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ServerStatus {
    /// Password is stored and at least one VM is allowed for Mannie.
    Ready,
    /// Server is configured, but no VMs have been selected in the allowlist yet.
    NoVmsAllowed,
    /// The credential store holds no password for this server.
    PasswordMissing,
}

/// An allowed QEMU VM shown in Settings and selectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AllowedVmView {
    /// Proxmox QEMU VMID.
    pub vmid: u32,
    /// Proxmox cluster node name.
    pub node: String,
    /// VM name.
    pub name: String,
    /// When the VM was added to the allowlist.
    pub added_at: UtcMillis,
}

/// A Proxmox server shown in Settings. Passwords never cross IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    /// Server ID.
    pub id: String,
    /// Owner-chosen server name.
    pub name: String,
    /// Hostname or IP address.
    pub host: String,
    /// HTTPS port.
    pub port: u16,
    /// Canonical HTTPS endpoint origin.
    pub endpoint: String,
    /// Proxmox username with realm.
    pub username: String,
    /// Whether a password is stored in the OS credential store.
    pub password_set: bool,
    /// Pinned leaf SHA-256 hex digest (64 lowercase chars), if pinned.
    pub pinned_cert_sha256: Option<String>,
    /// Colon-separated uppercase SHA-256 fingerprint for display, if pinned.
    pub pinned_cert_fingerprint: Option<String>,
    /// Subject DN of the pinned certificate, if pinned.
    pub pinned_cert_subject: Option<String>,
    /// Timestamp when the certificate was pinned, if pinned.
    pub pinned_cert_at: Option<UtcMillis>,
    /// Owner note describing this server.
    pub note: String,
    /// Readiness status.
    pub status: ServerStatus,
    /// Explicitly allowed QEMU VMs on this server.
    pub allowed_vms: Vec<AllowedVmView>,
}

/// Live server list channel payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerListUpdate {
    /// Full snapshot of all registered servers.
    Servers {
        /// All registered Proxmox servers in display order.
        servers: Vec<ServerView>,
    },
}

/// Input to `add_server`.
#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddServer {
    /// Display name, unique case-insensitively.
    pub name: String,
    /// Hostname, IPv4, or IPv6 address.
    pub host: String,
    /// HTTPS port (1..=65535, typically 8006).
    pub port: u16,
    /// Proxmox username with realm (e.g. `user@pve`).
    pub username: String,
    /// Account password stored only in the OS credential store.
    pub password: Secret,
    /// Optional note for the model.
    #[serde(default)]
    pub note: String,
}

impl Validate for AddServer {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_fields(
            &self.name,
            &self.host,
            self.port,
            &self.username,
            &self.note,
        )?;
        if !is_valid_password(self.password.expose()) {
            return Err(InputRefusal::new("password"));
        }

        Ok(())
    }
}

/// Input to `update_server`.
#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateServer {
    /// Target server ID.
    pub server_id: String,
    /// Display name.
    pub name: String,
    /// Hostname or IP address. Changing host or port clears TLS pin and VM allowlist.
    pub host: String,
    /// HTTPS port.
    pub port: u16,
    /// Proxmox username with realm.
    pub username: String,
    /// Optional replacement password; omit or leave `None` to keep stored password.
    #[serde(default)]
    pub password: Option<Secret>,
    /// Optional note for the model.
    #[serde(default)]
    pub note: String,
}

impl Validate for UpdateServer {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_id(&self.server_id)?;
        check_server_fields(
            &self.name,
            &self.host,
            self.port,
            &self.username,
            &self.note,
        )?;
        let password_invalid = self
            .password
            .as_ref()
            .is_some_and(|secret| !is_valid_password(secret.expose()));

        if password_invalid {
            return Err(InputRefusal::new("password"));
        }

        Ok(())
    }
}

fn check_server_fields(
    name: &str,
    host: &str,
    port: u16,
    username: &str,
    note: &str,
) -> Result<(), InputRefusal> {
    if !is_valid_server_name(name) {
        return Err(InputRefusal::new("name"));
    }

    if normalize_host(host).is_none() {
        return Err(InputRefusal::new("host"));
    }

    if port == 0 {
        return Err(InputRefusal::new("port"));
    }

    if !is_valid_username(username) {
        return Err(InputRefusal::new("username"));
    }

    if !is_valid_note(note) {
        return Err(InputRefusal::new("note"));
    }

    Ok(())
}

/// Reference to a registered server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerRef {
    /// Server ID.
    pub server_id: String,
}

impl Validate for ServerRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_id(&self.server_id)
    }
}

/// Parsed X.509 leaf certificate metadata returned by `inspect_certificate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CertificateInfo {
    /// Lowercase 64-character SHA-256 hex digest of the leaf DER bytes.
    pub sha256: String,
    /// Colon-separated uppercase SHA-256 fingerprint for visual verification.
    pub fingerprint: String,
    /// Certificate subject string (e.g. `CN=pve.lan`).
    pub subject: String,
    /// Certificate issuer string (e.g. `CN=Proxmox Virtual Environment`).
    pub issuer: String,
    /// Subject Alternative Names (DNS names and IP addresses).
    pub sans: Vec<String>,
    /// Start of validity window (ISO-8601 UTC string, if parsed).
    pub not_before: String,
    /// End of validity window (ISO-8601 UTC string, if parsed).
    pub not_after: String,
    /// Whether the certificate is currently within its validity window.
    pub valid_now: bool,
    /// Whether the operating system certificate store trusts this certificate chain.
    pub os_trusted: bool,
    /// Whether this certificate matches the currently pinned fingerprint (when one is pinned).
    pub matches_pin: Option<bool>,
}

/// Input to `pin_certificate`: pins only the leaf SHA-256 fingerprint that
/// matches a live TLS inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinCertificate {
    /// Target server ID.
    pub server_id: String,
    /// Expected leaf SHA-256 hex digest (or colon fingerprint).
    pub sha256: String,
}

impl Validate for PinCertificate {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_id(&self.server_id)?;
        if normalize_sha256_hex(&self.sha256).is_none() {
            return Err(InputRefusal::new("sha256"));
        }

        Ok(())
    }
}

/// Outcome of `test_connection`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestConnectionResult {
    /// Canonical server endpoint tested.
    pub endpoint: String,
    /// Number of QEMU VMs visible to this Proxmox account.
    pub visible_vm_count: u32,
    /// Number of QEMU VMs currently allowed for Mannie on this server.
    pub allowed_vm_count: u32,
}

/// A QEMU VM returned by `discover_server_vms` during the Settings allowlist
/// pre-configure step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverableVmView {
    /// Proxmox QEMU VMID.
    pub vmid: u32,
    /// Proxmox node name.
    pub node: String,
    /// VM name.
    pub name: String,
    /// Current power status (`running`, `stopped`, `paused`, etc.).
    pub status: String,
    /// Number of configured CPU cores, if reported.
    pub maxcpu: Option<u32>,
    /// Configured RAM in MiB, if reported.
    pub maxmem_mib: Option<u32>,
    /// Whether this VM is currently selected in the server's allowlist.
    pub allowed: bool,
}

/// One VM selection entry sent by `set_allowed_vms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllowedVmEntry {
    /// Positive Proxmox QEMU VMID.
    pub vmid: u32,
    /// Proxmox node name.
    pub node: String,
    /// VM display name.
    pub name: String,
}

/// Input to `set_allowed_vms`: replaces the server's explicit VM allowlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetAllowedVms {
    /// Target server ID.
    pub server_id: String,
    /// Complete set of VMs the owner chooses to make available to Mannie.
    pub vms: Vec<AllowedVmEntry>,
}

impl Validate for SetAllowedVms {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_id(&self.server_id)?;
        if self.vms.len() > MAX_ALLOWED_VMS_PER_SERVER {
            return Err(InputRefusal::new("vms"));
        }

        for vm in &self.vms {
            let name_ok = !vm.name.trim().is_empty()
                && vm.name.chars().count() <= 128
                && !vm.name.chars().any(char::is_control);
            let valid = vm.vmid > 0 && is_valid_node(&vm.node) && name_ok;

            if !valid {
                return Err(InputRefusal::new("vms"));
            }
        }

        Ok(())
    }
}

/// Who currently controls a live VNC console session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum VncControlOwner {
    /// The agent may inspect screenshots and send pointer/keyboard input.
    /// Owner canvas input is disabled until the owner takes over.
    Agent,
    /// The owner has taken over the console. All agent VNC operations
    /// (including screenshots) are refused in Rust until the owner resumes.
    Owner,
}

/// Live connection state of a VNC session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum VncSessionState {
    /// Connecting to Proxmox `vncproxy` and establishing RFB 3.8.
    Connecting,
    /// Live RFB session is active.
    Connected,
    /// Session disconnected or was closed.
    Closed,
}

/// Summary of an active live VNC session for a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VncSessionSummary {
    /// Conversation that owns this VNC session.
    pub conversation_id: ConversationId,
    /// Server ID.
    pub server_id: String,
    /// Server display name.
    pub server_name: String,
    /// Proxmox node name.
    pub node: String,
    /// QEMU VMID.
    pub vmid: u32,
    /// VM display name.
    pub vm_name: String,
    /// Current control owner (`agent` or `owner`).
    pub control: VncControlOwner,
    /// Connection state.
    pub state: VncSessionState,
    /// Current framebuffer width in pixels (0 before first `ServerInit`).
    pub width: u16,
    /// Current framebuffer height in pixels (0 before first `ServerInit`).
    pub height: u16,
}

/// Conversation reference for listing active VNC sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VncConversationRef {
    /// Conversation ID.
    pub conversation_id: ConversationId,
}

impl Validate for VncConversationRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// Identifies a specific VNC console target within a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VncSessionTarget {
    /// Conversation ID.
    pub conversation_id: ConversationId,
    /// Server ID.
    pub server_id: String,
    /// QEMU VMID.
    pub vmid: u32,
}

impl Validate for VncSessionTarget {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_id(&self.server_id)?;
        if self.vmid == 0 {
            return Err(InputRefusal::new("vmid"));
        }

        Ok(())
    }
}

/// Input to `set_vnc_control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetVncControl {
    /// Conversation ID.
    pub conversation_id: ConversationId,
    /// Server ID.
    pub server_id: String,
    /// QEMU VMID.
    pub vmid: u32,
    /// Desired control owner (`owner` to take over, `agent` to resume).
    pub control: VncControlOwner,
}

impl Validate for SetVncControl {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_id(&self.server_id)?;
        if self.vmid == 0 {
            return Err(InputRefusal::new("vmid"));
        }

        Ok(())
    }
}

/// Owner pointer event sent from the live VNC side panel while in `owner` mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VncPointerInput {
    /// Conversation ID.
    pub conversation_id: ConversationId,
    /// Server ID.
    pub server_id: String,
    /// QEMU VMID.
    pub vmid: u32,
    /// X coordinate in framebuffer pixels.
    pub x: u16,
    /// Y coordinate in framebuffer pixels.
    pub y: u16,
    /// RFB button mask (bit 0 = left, bit 1 = middle, bit 2 = right, bit 3 = scroll up, bit 4 = scroll down).
    pub button_mask: u8,
}

impl Validate for VncPointerInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_id(&self.server_id)?;
        if self.vmid == 0 {
            return Err(InputRefusal::new("vmid"));
        }

        Ok(())
    }
}

/// Owner keyboard event sent from the live VNC side panel while in `owner` mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VncKeyboardInput {
    /// Conversation ID.
    pub conversation_id: ConversationId,
    /// Server ID.
    pub server_id: String,
    /// QEMU VMID.
    pub vmid: u32,
    /// X11 `KeySym` value.
    pub keysym: u32,
    /// `true` on key down, `false` on key up.
    pub down: bool,
}

impl Validate for VncKeyboardInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_id(&self.server_id)?;
        if self.vmid == 0 {
            return Err(InputRefusal::new("vmid"));
        }

        if self.keysym == 0 {
            return Err(InputRefusal::new("keysym"));
        }

        Ok(())
    }
}

/// Updates pushed over the `follow_vnc_session` channel to the live VNC panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum VncSessionUpdate {
    /// Full framebuffer PNG snapshot (initial frame or resize).
    Snapshot {
        /// Session summary metadata.
        session: VncSessionSummary,
        /// Base64-encoded PNG bytes of the full framebuffer (empty if no frame decoded yet).
        png_base64: String,
        /// Monotonic frame sequence number.
        seq: u32,
    },
    /// Dirty-region PNG patch applied at `(x, y)` on top of the current canvas.
    Patch {
        /// Region X offset in pixels.
        x: u16,
        /// Region Y offset in pixels.
        y: u16,
        /// Region width in pixels.
        width: u16,
        /// Region height in pixels.
        height: u16,
        /// Base64-encoded PNG bytes for this region.
        png_base64: String,
        /// Monotonic frame sequence number.
        seq: u32,
    },
    /// Session metadata or control ownership changed without a new framebuffer image.
    Status {
        /// Updated session summary.
        session: VncSessionSummary,
    },
    /// The VNC session closed.
    Closed {
        /// Machine-readable closure reason code.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_endpoint, format_fingerprint_colon, is_valid_username, normalize_host,
        normalize_sha256_hex,
    };

    #[test]
    fn host_normalization_handles_dns_ipv4_and_ipv6_and_rejects_urls() {
        assert_eq!(normalize_host("PVE.Lan").as_deref(), Some("pve.lan"));
        assert_eq!(
            normalize_host("192.168.1.10").as_deref(),
            Some("192.168.1.10")
        );
        assert_eq!(normalize_host("[::1]").as_deref(), Some("::1"));
        assert_eq!(
            normalize_host("2001:db8::5").as_deref(),
            Some("2001:db8::5")
        );
        assert_eq!(normalize_host("https://pve.lan:8006"), None);
        assert_eq!(normalize_host("user@pve.lan"), None);
        assert_eq!(normalize_host("pve.lan:8006"), None);
        assert_eq!(normalize_host("pve.lan/api2"), None);
    }

    #[test]
    fn canonical_endpoint_formats_default_and_custom_ports_and_ipv6() {
        assert_eq!(canonical_endpoint("pve.lan", 8006), "https://pve.lan:8006");
        assert_eq!(canonical_endpoint("pve.lan", 443), "https://pve.lan");
        assert_eq!(canonical_endpoint("::1", 8006), "https://[::1]:8006");
    }

    #[test]
    fn username_requires_user_and_realm() {
        assert!(is_valid_username("alice@pve"));
        assert!(is_valid_username("ops.user-1@pam"));
        assert!(!is_valid_username("alice"));
        assert!(!is_valid_username("@pve"));
        assert!(!is_valid_username("alice@"));
        assert!(!is_valid_username("alice @pve"));
    }

    #[test]
    fn sha256_normalization_and_formatting_round_trip() {
        let raw = "a1".repeat(32);
        let formatted = format_fingerprint_colon(&raw);
        assert_eq!(
            normalize_sha256_hex(&formatted).as_deref(),
            Some(raw.as_str())
        );
        assert_eq!(
            normalize_sha256_hex(&format!("sha256:{raw}")).as_deref(),
            Some(raw.as_str())
        );
        assert_eq!(normalize_sha256_hex("not-hex"), None);
    }
}
