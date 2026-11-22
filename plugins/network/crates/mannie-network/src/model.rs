//! Domain types, validation limits and stable error codes of the Network
//! Client plugin (ADR-0075; plugins/network/docs/README.md).
//!
//! Every stored collection, saved request and response carries an explicit
//! [`Protocol`] tag (`http` in version 1) so later protocols arrive by
//! addition without changing existing contracts (ADR-0002).

use mannie_plugin_sdk::contracts::UtcMillis;
use mannie_plugin_sdk::ipc::PluginError;
use serde::{Deserialize, Serialize};
use specta::Type;

/// The most characters of a collection, folder, request or environment name.
pub const MAX_NAME_CHARS: usize = 128;

/// The most characters of a description.
pub const MAX_DESCRIPTION_CHARS: usize = 2_000;

/// The most bytes of a URL or URL template.
pub const MAX_URL_BYTES: usize = 4 * 1024;

/// The most entries in a list of variables, headers, parameters or form fields.
pub const MAX_ENTRIES: usize = 200;

/// The most characters of a variable, header, parameter or cookie name.
pub const MAX_KEY_CHARS: usize = 256;

/// The most bytes of a variable, header, parameter or cookie value.
pub const MAX_VALUE_BYTES: usize = 16 * 1024;

/// The most bytes of a request body.
pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// The default request timeout, in seconds.
pub const DEFAULT_TIMEOUT_SECONDS: u32 = 30;

/// The shortest allowed request timeout, in seconds.
pub const MIN_TIMEOUT_SECONDS: u32 = 1;

/// The longest allowed request timeout, in seconds.
pub const MAX_TIMEOUT_SECONDS: u32 = 300;

/// The largest `OpenAPI` specification accepted for import, in bytes (10 MiB).
pub const MAX_SPEC_BYTES: usize = 10 * 1024 * 1024;

/// The largest response body read in the owner workbench, in bytes (10 MiB).
pub const MAX_OWNER_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

/// The largest response body read for an agent tool call, in bytes (2 MiB).
pub const MAX_AGENT_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// The protocol of a collection or saved request (ADR-0075, decision 1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// HTTP/1.1 and HTTP/2 over HTTPS or plain HTTP.
    #[default]
    Http,
}

impl Protocol {
    /// The lower-case wire identifier of the protocol.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
        }
    }

    /// Parses a stored protocol identifier.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "http" => Some(Self::Http),
            _other => None,
        }
    }
}

/// An HTTP method for an HTTP request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    /// HTTP GET.
    #[default]
    Get,
    /// HTTP POST.
    Post,
    /// HTTP PUT.
    Put,
    /// HTTP PATCH.
    Patch,
    /// HTTP DELETE.
    Delete,
    /// HTTP HEAD.
    Head,
    /// HTTP OPTIONS.
    Options,
}

impl HttpMethod {
    /// The upper-case HTTP verb.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        }
    }

    /// Parses an HTTP verb without regard to case.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "GET" => Some(Self::Get),
            "POST" => Some(Self::Post),
            "PUT" => Some(Self::Put),
            "PATCH" => Some(Self::Patch),
            "DELETE" => Some(Self::Delete),
            "HEAD" => Some(Self::Head),
            "OPTIONS" => Some(Self::Options),
            _other => None,
        }
    }

    /// Converts to a `reqwest::Method`.
    pub const fn to_reqwest(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
            Self::Patch => reqwest::Method::PATCH,
            Self::Delete => reqwest::Method::DELETE,
            Self::Head => reqwest::Method::HEAD,
            Self::Options => reqwest::Method::OPTIONS,
        }
    }
}

/// Where an API key is sent on an HTTP request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyLocation {
    /// Sent as an HTTP request header.
    #[default]
    Header,
    /// Sent as a URL query parameter.
    Query,
}

/// Authentication configuration of a collection or saved request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AuthConfig {
    /// Inherit the parent collection's authentication (requests only).
    Inherit,
    /// No authentication attached.
    #[default]
    None,
    /// `Authorization: Bearer <token>`.
    Bearer {
        /// The bearer token or `{{variable}}` template. Stored in the OS
        /// credential store, never in SQLite.
        token: String,
    },
    /// HTTP Basic authentication (`Authorization: Basic <base64>`).
    Basic {
        /// The username or template.
        username: String,
        /// The password or template. Stored in the OS credential store, never
        /// in SQLite.
        password: String,
    },
    /// API key sent in a header or query parameter.
    ApiKey {
        /// The header or query parameter name.
        name: String,
        /// The API key value or template. Stored in the OS credential store,
        /// never in SQLite.
        value: String,
        /// Whether to send the key as a header or query parameter.
        location: ApiKeyLocation,
    },
}

/// Request body mode and payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum BodyConfig {
    /// No request body.
    #[default]
    None,
    /// JSON body (`application/json`).
    Json {
        /// Raw JSON text, which may contain `{{variable}}` templates.
        content: String,
    },
    /// Raw text body with an explicit `Content-Type`.
    Text {
        /// Raw text content, which may contain `{{variable}}` templates.
        content: String,
        /// Media type sent in `Content-Type`, such as `text/plain`.
        content_type: String,
    },
    /// `application/x-www-form-urlencoded` form fields.
    FormUrlEncoded {
        /// Key-value form entries; secret entries keep their values in the OS
        /// credential store.
        entries: Vec<KeyValueEntry>,
    },
}

/// A header, query parameter, path parameter or form field entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyValueEntry {
    /// The entry's name.
    pub key: String,
    /// The entry's value (empty in SQLite when `secret` is true; hydrated as-is
    /// for the owner workbench and replaced with `[redacted]` for agents).
    pub value: String,
    /// Whether this entry is active when sending the request.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether this entry's value is a secret kept in the OS credential store.
    #[serde(default)]
    pub secret: bool,
    /// Optional description, such as from an `OpenAPI` specification.
    #[serde(default)]
    pub description: String,
}

/// A custom variable (global, collection or environment).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VariableEntry {
    /// The variable's name, referenced as `{{key}}`.
    pub key: String,
    /// The variable's value (empty in SQLite when `secret` is true; hydrated
    /// as-is for the owner workbench and replaced with `[redacted]` for
    /// agents).
    pub value: String,
    /// Whether this variable participates in interpolation.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether this variable's value is a secret kept in the OS credential
    /// store.
    #[serde(default)]
    pub secret: bool,
    /// Optional description of the variable.
    #[serde(default)]
    pub description: String,
}

const fn default_true() -> bool {
    true
}

/// A folder inside a collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderView {
    /// Immutable folder ID (UUIDv7).
    pub id: String,
    /// Owning collection ID.
    pub collection_id: String,
    /// Optional parent folder ID for nested folders.
    pub parent_id: Option<String>,
    /// Folder display name.
    pub name: String,
    /// Sort position within its parent.
    pub sort_order: u32,
    /// Creation time.
    pub created_at: UtcMillis,
    /// Last modification time.
    pub updated_at: UtcMillis,
}

/// A named environment belonging to a collection (such as Local, Staging or
/// Production).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentView {
    /// Immutable environment ID (UUIDv7).
    pub id: String,
    /// Owning collection ID.
    pub collection_id: String,
    /// Environment name, unique within its collection without regard to case.
    pub name: String,
    /// Environment variables, overriding collection and global variables.
    pub variables: Vec<VariableEntry>,
    /// Creation time.
    pub created_at: UtcMillis,
    /// Last modification time.
    pub updated_at: UtcMillis,
}

/// A saved request inside a collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RequestView {
    /// Immutable request ID (UUIDv7).
    pub id: String,
    /// Owning collection ID.
    pub collection_id: String,
    /// Optional folder ID.
    pub folder_id: Option<String>,
    /// Request display name.
    pub name: String,
    /// Optional description (for example, `OpenAPI` operation summary/notes).
    pub description: String,
    /// Protocol (`http` in version 1).
    pub protocol: Protocol,
    /// HTTP method (`GET`, `POST`, etc.).
    pub method: HttpMethod,
    /// URL template, relative to the collection's base URL or absolute. May
    /// contain `{{variable}}` and `{pathParam}` placeholders.
    pub url_template: String,
    /// Path parameter definitions for `{param}` segments.
    pub path_params: Vec<KeyValueEntry>,
    /// Query parameters attached to the request.
    pub query_params: Vec<KeyValueEntry>,
    /// Request headers (override matching collection headers).
    pub headers: Vec<KeyValueEntry>,
    /// Request authentication configuration.
    pub auth: AuthConfig,
    /// Request body configuration.
    pub body: BodyConfig,
    /// Request timeout in seconds (1 to 300).
    pub timeout_seconds: u32,
    /// Whether matching cookies from the cookie jar are sent.
    pub use_cookies: bool,
    /// Sort position within its collection or folder.
    pub sort_order: u32,
    /// Creation time.
    pub created_at: UtcMillis,
    /// Last modification time.
    pub updated_at: UtcMillis,
}

/// A collection of saved network requests with its folders, environments and
/// requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CollectionView {
    /// Immutable collection ID (UUIDv7).
    pub id: String,
    /// Collection name, unique without regard to case.
    pub name: String,
    /// Optional collection description.
    pub description: String,
    /// Protocol (`http` in version 1).
    pub protocol: Protocol,
    /// Default base URL, such as `https://api.example.com/v1` or
    /// `{{base_url}}`.
    pub base_url: String,
    /// Whether agents may list and run saved requests from this collection.
    pub enabled: bool,
    /// Active environment ID used when sending requests without an explicit
    /// environment override.
    pub active_environment_id: Option<String>,
    /// Whether `Set-Cookie` response headers automatically update the cookie
    /// jar.
    pub auto_cookies: bool,
    /// Collection-level default headers.
    pub headers: Vec<KeyValueEntry>,
    /// Collection-level default query parameters.
    pub query_params: Vec<KeyValueEntry>,
    /// Collection-level default authentication.
    pub auth: AuthConfig,
    /// Collection-level variables.
    pub variables: Vec<VariableEntry>,
    /// Folders in this collection.
    pub folders: Vec<FolderView>,
    /// Environments in this collection.
    pub environments: Vec<EnvironmentView>,
    /// Saved requests in this collection.
    pub requests: Vec<RequestView>,
    /// Creation time.
    pub created_at: UtcMillis,
    /// Last modification time.
    pub updated_at: UtcMillis,
}

/// A stored cookie in the cookie jar. Its `value` lives only in the OS
/// credential store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "RFC 6265 cookie attributes (host_only, secure, http_only) plus enabled toggle"
)]
pub struct CookieView {
    /// Immutable cookie ID (UUIDv7).
    pub id: String,
    /// Cookie name.
    pub name: String,
    /// Cookie value (stored only in the OS credential store; visible as-is in
    /// the owner workbench and `[redacted]` to agents).
    pub value: String,
    /// Domain scope (such as `api.example.com` or `.example.com`).
    pub domain: String,
    /// Whether this cookie is host-only per RFC 6265 section 5.4 (exact host
    /// match only, never subdomain suffix match).
    #[serde(default)]
    pub host_only: bool,
    /// Path prefix (such as `/`).
    pub path: String,
    /// Whether the cookie travels only over HTTPS.
    pub secure: bool,
    /// Whether the cookie carries the `HttpOnly` attribute.
    pub http_only: bool,
    /// Optional expiration timestamp.
    pub expires_at: Option<UtcMillis>,
    /// Whether the cookie is active for outbound requests.
    pub enabled: bool,
    /// Creation time.
    pub created_at: UtcMillis,
    /// Last modification time.
    pub updated_at: UtcMillis,
}

/// Complete owner workbench state pushed over `follow_state` and returned by
/// `get_state`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStateSnapshot {
    /// Every saved collection with its folders, environments and requests.
    pub collections: Vec<CollectionView>,
    /// Global variables shared across all collections.
    pub global_variables: Vec<VariableEntry>,
    /// Cookies in the owner's cookie jar.
    pub cookies: Vec<CookieView>,
}

/// A response header returned to the owner workbench.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ResponseHeaderView {
    /// Header name.
    pub name: String,
    /// Header value (as-is for the owner workbench).
    pub value: String,
}

/// A `Set-Cookie` item returned to the owner workbench.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ResponseCookieView {
    /// Cookie name.
    pub name: String,
    /// Cookie value (as-is for the owner workbench).
    pub value: String,
    /// Effective domain.
    pub domain: String,
    /// Whether the `Set-Cookie` header omitted `Domain` (host-only scope).
    #[serde(default)]
    pub host_only: bool,
    /// Effective path.
    pub path: String,
    /// Whether `Secure` was set.
    pub secure: bool,
    /// Whether `HttpOnly` was set.
    pub http_only: bool,
    /// Effective expiry, if any.
    pub expires_at: Option<UtcMillis>,
}

/// The live HTTP response returned directly to the owner workbench (unredacted
/// per ADR-0075, decision 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManualResponseView {
    /// Protocol (`http`).
    pub protocol: Protocol,
    /// HTTP method that was sent.
    pub method: HttpMethod,
    /// Final resolved URL that was sent.
    pub url: String,
    /// HTTP status code (for example, `200`).
    pub status: u16,
    /// Canonical status reason phrase (for example, `OK`).
    pub status_text: String,
    /// Round-trip elapsed time in milliseconds.
    pub duration_ms: u32,
    /// Total response body bytes received (before any preview cap).
    pub bytes: u32,
    /// Whether the response body exceeded the 10 MiB workbench limit and was
    /// cut off.
    pub truncated: bool,
    /// Response `Content-Type` header, if present.
    pub content_type: Option<String>,
    /// Response body text, or a base64 representation when `is_binary` is true.
    pub body: String,
    /// Whether the response body was non-UTF-8 binary data.
    pub is_binary: bool,
    /// Response headers, in arrival order.
    pub headers: Vec<ResponseHeaderView>,
    /// Parsed `Set-Cookie` headers received in the response.
    pub cookies: Vec<ResponseCookieView>,
}

/// One endpoint summary shown in an `OpenAPI` import preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenApiPreviewEndpoint {
    /// Derived folder name (from the first `OpenAPI` tag), if any.
    pub folder: Option<String>,
    /// Operation display name.
    pub name: String,
    /// HTTP verb.
    pub method: HttpMethod,
    /// Endpoint path template.
    pub path: String,
    /// Short operation description.
    pub description: String,
}

/// Which `OpenAPI` field was missing when no base URL could be derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MissingBaseUrlSource {
    /// Swagger 2.0 specification omitted `host`.
    V2Host,
    /// `OpenAPI` 3.x specification omitted `servers` or declared no usable server URLs.
    V3Servers,
}

/// Structured non-fatal warning detected while previewing an `OpenAPI` specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum OpenApiWarning {
    /// The specification declared no server or host; resolved when the owner provides a base URL.
    MissingBaseUrl {
        /// Whether the spec was v2 (`host`) or v3 (`servers`).
        source: MissingBaseUrlSource,
    },
    /// An endpoint declared a cookie parameter; resolved when that cookie exists in the Cookie Jar.
    CookieParameter {
        /// Endpoint name or summary.
        endpoint: String,
        /// Cookie name expected by the endpoint.
        cookie_name: String,
    },
    /// A security scheme uses a cookie API key; resolved when that cookie exists in the Cookie Jar.
    CookieSecurityScheme {
        /// Security scheme name.
        scheme: String,
        /// Cookie name expected by the scheme.
        cookie_name: String,
    },
    /// A remote `$ref` was skipped because remote references are never fetched.
    SkippedExternalRef {
        /// Original `$ref` value.
        reference: String,
    },
    /// A cyclic local `$ref` was broken.
    CyclicRef {
        /// Original `$ref` value.
        reference: String,
    },
    /// A local `$ref` pointer could not be resolved.
    UnresolvedLocalRef {
        /// Original `$ref` value.
        reference: String,
    },
    /// A `$ref` chain exceeded the maximum depth.
    DeepRefChain,
    /// The importer stopped after the per-spec endpoint cap.
    EndpointLimitReached {
        /// Maximum number of endpoints imported.
        max_endpoints: u32,
    },
    /// One security requirement combined multiple schemes; only the first supported scheme is attached.
    CombinedSecuritySchemes {
        /// Security context label (root security or operation name).
        context: String,
    },
    /// An operation requires a security scheme that cannot be attached automatically.
    UnsupportedOperationSecurity {
        /// Endpoint name or summary.
        operation: String,
    },
}

/// Preview of an `OpenAPI` v2 or v3 specification before importing it as a
/// collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenApiPreview {
    /// Ephemeral preview ID cached in memory for `confirm_openapi_import`.
    pub preview_id: String,
    /// Suggested collection title from `info.title`.
    pub title: String,
    /// Collection description from `info.description`.
    pub description: String,
    /// Detected `OpenAPI` version string (such as `2.0`, `3.0.3` or `3.1.0`).
    pub spec_version: String,
    /// Primary base URL derived from `servers` or `host`/`basePath`.
    pub base_url: String,
    /// Derived environment names (when multiple servers are defined).
    pub environments: Vec<String>,
    /// Derived folder names (from operation tags).
    pub folders: Vec<String>,
    /// Endpoints that will be created.
    pub endpoints: Vec<OpenApiPreviewEndpoint>,
    /// Detected security scheme labels (never credential values).
    pub security_schemes: Vec<String>,
    /// Non-fatal warnings found while reading the specification (deprecated in favor of `warning_items`).
    pub warnings: Vec<String>,
    /// Structured non-fatal warnings so the UI can localize and dismiss notes once fixed.
    #[serde(default)]
    pub warning_items: Vec<OpenApiWarning>,
}

/// Stable error codes returned by the Network Client plugin's commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum NetworkError {
    /// Another collection already has this name.
    #[error("a collection with this name already exists")]
    NameTaken,
    /// Another environment in this collection already has this name.
    #[error("an environment with this name already exists in this collection")]
    EnvironmentNameTaken,
    /// No collection exists with this ID.
    #[error("the collection was not found")]
    CollectionNotFound,
    /// No folder exists with this ID.
    #[error("the folder was not found")]
    FolderNotFound,
    /// No saved request exists with this ID.
    #[error("the request was not found")]
    RequestNotFound,
    /// No environment exists with this ID.
    #[error("the environment was not found")]
    EnvironmentNotFound,
    /// No cookie exists with this ID.
    #[error("the cookie was not found")]
    CookieNotFound,
    /// The `OpenAPI` preview expired or does not exist.
    #[error("the OpenAPI import preview is no longer available")]
    PreviewNotFound,
    /// The resolved URL is malformed, unsupported or embeds userinfo.
    #[error("the request URL is not a valid http or https URL")]
    InvalidUrl,
    /// Credentials or cookies cannot travel over plain HTTP to a non-local
    /// host.
    #[error("credentials and cookies require https or a local or private network address")]
    InsecureCredentialTransport,
    /// The `OpenAPI` document could not be parsed.
    #[error("the OpenAPI specification is not valid: {detail}")]
    InvalidSpec {
        /// Parser diagnostic detail (never contains secret values).
        detail: String,
    },
    /// The `OpenAPI` document exceeds the 10 MiB limit.
    #[error("the OpenAPI specification is larger than 10 MiB")]
    SpecTooLarge,
    /// The `OpenAPI` YAML document exceeded nesting, node or alias limits.
    #[error("the OpenAPI specification exceeded safe complexity limits")]
    SpecTooComplex,
    /// The document is neither `OpenAPI` v2 (Swagger 2.0) nor `OpenAPI` v3.
    #[error("unsupported OpenAPI version: {version}")]
    UnsupportedSpecVersion {
        /// The version field that was read, if any.
        version: String,
    },
    /// The system file dialog could not open.
    #[error("the file dialog could not open")]
    FileDialogFailed,
    /// The chosen file could not be read or was not UTF-8 text.
    #[error("the chosen file could not be read")]
    FileUnreadable,
    /// The HTTP request failed before reaching or completing on the remote host.
    #[error("the request failed: {detail}")]
    RequestFailed {
        /// Sanitized transport failure detail.
        detail: String,
    },
    /// The connection broke after sending the request or while reading its
    /// response; whether the remote server processed it is unknown.
    #[error("the connection closed before the response finished: {detail}")]
    ResponseLost {
        /// Sanitized transport failure detail.
        detail: String,
    },
    /// The HTTP request timed out.
    #[error("the request timed out")]
    RequestTimedOut,
    /// The owner canceled the in-flight request.
    #[error("the request was canceled")]
    RequestCanceled,
    /// The operating system credential store could not be reached.
    #[error("the system credential store is not available")]
    CredentialStoreUnavailable,
    /// The SQLite store failed.
    #[error("the change could not be saved")]
    StorageFailed,
}

impl PluginError for NetworkError {
    fn code(&self) -> &'static str {
        match self {
            Self::NameTaken => "name_taken",
            Self::EnvironmentNameTaken => "environment_name_taken",
            Self::CollectionNotFound => "collection_not_found",
            Self::FolderNotFound => "folder_not_found",
            Self::RequestNotFound => "request_not_found",
            Self::EnvironmentNotFound => "environment_not_found",
            Self::CookieNotFound => "cookie_not_found",
            Self::PreviewNotFound => "preview_not_found",
            Self::InvalidUrl => "invalid_url",
            Self::InsecureCredentialTransport => "insecure_credential_transport",
            Self::InvalidSpec { .. } => "invalid_spec",
            Self::SpecTooLarge => "spec_too_large",
            Self::SpecTooComplex => "spec_too_complex",
            Self::UnsupportedSpecVersion { .. } => "unsupported_spec_version",
            Self::FileDialogFailed => "file_dialog_failed",
            Self::FileUnreadable => "file_unreadable",
            Self::RequestFailed { .. } => "request_failed",
            Self::ResponseLost { .. } => "response_lost",
            Self::RequestTimedOut => "request_timed_out",
            Self::RequestCanceled => "request_canceled",
            Self::CredentialStoreUnavailable => "credential_store_unavailable",
            Self::StorageFailed => "storage_failed",
        }
    }
}

/// Validates a short single-line non-empty label such as a collection or
/// folder name.
pub(crate) fn is_valid_name(text: &str) -> bool {
    let trimmed = text.trim();
    let fits = !trimmed.is_empty() && trimmed.chars().count() <= MAX_NAME_CHARS;
    let clean = !text.chars().any(char::is_control);
    fits && clean
}

/// Validates an optional multi-line description.
pub(crate) fn is_valid_description(text: &str) -> bool {
    let fits = text.chars().count() <= MAX_DESCRIPTION_CHARS;
    let clean = !text.contains('\0');
    fits && clean
}

/// Validates a URL template or base URL string (may be empty or contain
/// `{{var}}` and `{param}` placeholders).
pub(crate) fn is_valid_url_template(text: &str) -> bool {
    let fits = text.len() <= MAX_URL_BYTES;
    let clean = !text.chars().any(char::is_control);
    fits && clean
}

/// Validates a timeout value in seconds.
pub(crate) const fn is_valid_timeout(seconds: u32) -> bool {
    seconds >= MIN_TIMEOUT_SECONDS && seconds <= MAX_TIMEOUT_SECONDS
}

/// Validates a list of key-value entries.
pub(crate) fn are_valid_entries(entries: &[KeyValueEntry]) -> bool {
    if entries.len() > MAX_ENTRIES {
        return false;
    }

    entries.iter().all(|entry| {
        let key_ok =
            entry.key.chars().count() <= MAX_KEY_CHARS && !entry.key.chars().any(char::is_control);
        let value_ok = entry.value.len() <= MAX_VALUE_BYTES
            && !entry
                .value
                .chars()
                .any(|ch| ch == '\0' || ch == '\r' || ch == '\n');
        let desc_ok = is_valid_description(&entry.description);
        key_ok && value_ok && desc_ok
    })
}

/// Validates a list of custom variables.
pub(crate) fn are_valid_variables(variables: &[VariableEntry]) -> bool {
    if variables.len() > MAX_ENTRIES {
        return false;
    }

    variables.iter().all(|entry| {
        let key_ok =
            entry.key.chars().count() <= MAX_KEY_CHARS && !entry.key.chars().any(char::is_control);
        let value_ok = entry.value.len() <= MAX_VALUE_BYTES && !entry.value.contains('\0');
        let desc_ok = is_valid_description(&entry.description);
        key_ok && value_ok && desc_ok
    })
}

/// Validates an authentication configuration.
pub(crate) fn is_valid_auth(auth: &AuthConfig) -> bool {
    match auth {
        AuthConfig::Inherit | AuthConfig::None => true,
        AuthConfig::Bearer { token } => {
            token.len() <= MAX_VALUE_BYTES
                && !token
                    .chars()
                    .any(|ch| ch == '\0' || ch == '\r' || ch == '\n')
        }
        AuthConfig::Basic { username, password } => {
            let user_ok = username.len() <= MAX_VALUE_BYTES
                && !username
                    .chars()
                    .any(|ch| ch == '\0' || ch == '\r' || ch == '\n');
            let pass_ok = password.len() <= MAX_VALUE_BYTES
                && !password
                    .chars()
                    .any(|ch| ch == '\0' || ch == '\r' || ch == '\n');
            user_ok && pass_ok
        }
        AuthConfig::ApiKey { name, value, .. } => {
            let name_ok = !name.trim().is_empty()
                && name.chars().count() <= MAX_KEY_CHARS
                && !name.chars().any(char::is_control);
            let value_ok = value.len() <= MAX_VALUE_BYTES
                && !value
                    .chars()
                    .any(|ch| ch == '\0' || ch == '\r' || ch == '\n');
            name_ok && value_ok
        }
    }
}

/// Validates a request body configuration.
pub(crate) fn is_valid_body(body: &BodyConfig) -> bool {
    match body {
        BodyConfig::None => true,
        BodyConfig::Json { content } => content.len() <= MAX_BODY_BYTES && !content.contains('\0'),
        BodyConfig::Text {
            content,
            content_type,
        } => {
            let content_ok = content.len() <= MAX_BODY_BYTES && !content.contains('\0');
            let type_ok = content_type.chars().count() <= MAX_KEY_CHARS
                && !content_type.chars().any(char::is_control);
            content_ok && type_ok
        }
        BodyConfig::FormUrlEncoded { entries } => are_valid_entries(entries),
    }
}
