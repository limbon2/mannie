//! IPC commands and state channel of the Network Client plugin (ADR-0034,
//! decision 8; ADR-0075).
//!
//! Every input implements [`Validate`] with `deny_unknown_fields` so the host
//! rejects malformed IPC inputs before plugin handlers execute.

use mannie_plugin_sdk::contracts::UtcMillis;
use mannie_plugin_sdk::ipc::{ChannelSpec, CommandSpec, InputRefusal, Interface, Validate};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::model::{
    AuthConfig, BodyConfig, CollectionView, CookieView, DEFAULT_TIMEOUT_SECONDS, EnvironmentView,
    FolderView, HttpMethod, KeyValueEntry, MAX_KEY_CHARS, MAX_SPEC_BYTES, MAX_VALUE_BYTES,
    ManualResponseView, NetworkError, NetworkStateSnapshot, OpenApiPreview, Protocol, RequestView,
    VariableEntry, are_valid_entries, are_valid_variables, is_valid_auth, is_valid_body,
    is_valid_description, is_valid_name, is_valid_timeout, is_valid_url_template,
};

const MAX_FILTER_CHARS: usize = 100;
const MAX_ID_CHARS: usize = 128;

fn is_valid_id(id: &str) -> bool {
    let trimmed = id.trim();
    let fits = !trimmed.is_empty() && trimmed.len() <= MAX_ID_CHARS;
    let clean = !trimmed.chars().any(char::is_control);
    fits && clean
}

/// Input for creating or updating a collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectionInput {
    /// Collection name, unique without regard to case.
    pub name: String,
    /// Optional collection description.
    #[serde(default)]
    pub description: String,
    /// Protocol (`http` in version 1).
    #[serde(default)]
    pub protocol: Protocol,
    /// Default base URL or `{{variable}}` template.
    #[serde(default)]
    pub base_url: String,
    /// Whether agents may list and run saved requests from this collection.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether `Set-Cookie` responses update the cookie jar automatically.
    #[serde(default = "default_true")]
    pub auto_cookies: bool,
    /// Collection-level default headers.
    #[serde(default)]
    pub headers: Vec<KeyValueEntry>,
    /// Collection-level default query parameters.
    #[serde(default)]
    pub query_params: Vec<KeyValueEntry>,
    /// Collection-level default authentication.
    #[serde(default)]
    pub auth: AuthConfig,
    /// Collection-level variables.
    #[serde(default)]
    pub variables: Vec<VariableEntry>,
}

const fn default_true() -> bool {
    true
}

const fn default_timeout() -> u32 {
    DEFAULT_TIMEOUT_SECONDS
}

impl Validate for CollectionInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_name(&self.name) {
            return Err(InputRefusal::new("name"));
        }
        if !is_valid_description(&self.description) {
            return Err(InputRefusal::new("description"));
        }
        if !is_valid_url_template(&self.base_url) {
            return Err(InputRefusal::new("baseUrl"));
        }
        if !are_valid_entries(&self.headers) {
            return Err(InputRefusal::new("headers"));
        }
        if !are_valid_entries(&self.query_params) {
            return Err(InputRefusal::new("queryParams"));
        }
        if !is_valid_auth(&self.auth) || matches!(self.auth, AuthConfig::Inherit) {
            return Err(InputRefusal::new("auth"));
        }
        if !are_valid_variables(&self.variables) {
            return Err(InputRefusal::new("variables"));
        }
        Ok(())
    }
}

/// Argument of `update_collection`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateCollection {
    /// Collection ID.
    pub collection_id: String,
    /// Updated collection fields.
    pub input: CollectionInput,
}

impl Validate for UpdateCollection {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.collection_id) {
            return Err(InputRefusal::new("collectionId"));
        }
        self.input.validate()
    }
}

/// Argument referencing one collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectionRef {
    /// Collection ID.
    pub collection_id: String,
}

impl Validate for CollectionRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.collection_id) {
            return Err(InputRefusal::new("collectionId"));
        }
        Ok(())
    }
}

/// Argument of `set_collection_enabled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetCollectionEnabled {
    /// Collection ID.
    pub collection_id: String,
    /// Whether agents may list and run saved requests from this collection.
    pub enabled: bool,
}

impl Validate for SetCollectionEnabled {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.collection_id) {
            return Err(InputRefusal::new("collectionId"));
        }
        Ok(())
    }
}

/// Argument of `create_folder`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FolderCreateInput {
    /// Owning collection ID.
    pub collection_id: String,
    /// Optional parent folder ID.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Folder display name.
    pub name: String,
}

impl Validate for FolderCreateInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.collection_id) {
            return Err(InputRefusal::new("collectionId"));
        }
        if self.parent_id.as_deref().is_some_and(|id| !is_valid_id(id)) {
            return Err(InputRefusal::new("parentId"));
        }
        if !is_valid_name(&self.name) {
            return Err(InputRefusal::new("name"));
        }
        Ok(())
    }
}

/// Argument of `rename_folder`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenameFolder {
    /// Folder ID.
    pub folder_id: String,
    /// New display name.
    pub name: String,
}

impl Validate for RenameFolder {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.folder_id) {
            return Err(InputRefusal::new("folderId"));
        }
        if !is_valid_name(&self.name) {
            return Err(InputRefusal::new("name"));
        }
        Ok(())
    }
}

/// Argument referencing one folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FolderRef {
    /// Folder ID.
    pub folder_id: String,
}

impl Validate for FolderRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.folder_id) {
            return Err(InputRefusal::new("folderId"));
        }
        Ok(())
    }
}

/// Input fields for creating, updating or test-sending a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestInput {
    /// Optional folder ID inside the collection.
    #[serde(default)]
    pub folder_id: Option<String>,
    /// Request display name.
    pub name: String,
    /// Optional request description.
    #[serde(default)]
    pub description: String,
    /// Protocol (`http` in version 1).
    #[serde(default)]
    pub protocol: Protocol,
    /// HTTP method (`GET`, `POST`, etc.).
    #[serde(default)]
    pub method: HttpMethod,
    /// Relative or absolute URL template.
    #[serde(default)]
    pub url_template: String,
    /// Path parameters for `{param}` segments.
    #[serde(default)]
    pub path_params: Vec<KeyValueEntry>,
    /// Query parameters.
    #[serde(default)]
    pub query_params: Vec<KeyValueEntry>,
    /// Request headers.
    #[serde(default)]
    pub headers: Vec<KeyValueEntry>,
    /// Request authentication.
    #[serde(default = "default_inherit_auth")]
    pub auth: AuthConfig,
    /// Request body.
    #[serde(default)]
    pub body: BodyConfig,
    /// Request timeout in seconds (1 to 300).
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u32,
    /// Whether matching cookies from the cookie jar are attached.
    #[serde(default = "default_true")]
    pub use_cookies: bool,
}

const fn default_inherit_auth() -> AuthConfig {
    AuthConfig::Inherit
}

impl Validate for RequestInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.folder_id.as_deref().is_some_and(|id| !is_valid_id(id)) {
            return Err(InputRefusal::new("folderId"));
        }
        if !is_valid_name(&self.name) {
            return Err(InputRefusal::new("name"));
        }
        if !is_valid_description(&self.description) {
            return Err(InputRefusal::new("description"));
        }
        if !is_valid_url_template(&self.url_template) {
            return Err(InputRefusal::new("urlTemplate"));
        }
        if !are_valid_entries(&self.path_params) {
            return Err(InputRefusal::new("pathParams"));
        }
        if !are_valid_entries(&self.query_params) {
            return Err(InputRefusal::new("queryParams"));
        }
        if !are_valid_entries(&self.headers) {
            return Err(InputRefusal::new("headers"));
        }
        if !is_valid_auth(&self.auth) {
            return Err(InputRefusal::new("auth"));
        }
        if !is_valid_body(&self.body) {
            return Err(InputRefusal::new("body"));
        }
        if !is_valid_timeout(self.timeout_seconds) {
            return Err(InputRefusal::new("timeoutSeconds"));
        }
        Ok(())
    }
}

/// Argument of `create_request`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateRequest {
    /// Owning collection ID.
    pub collection_id: String,
    /// New request fields.
    pub input: RequestInput,
}

impl Validate for CreateRequest {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.collection_id) {
            return Err(InputRefusal::new("collectionId"));
        }
        self.input.validate()
    }
}

/// Argument of `update_request`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateRequest {
    /// Request ID.
    pub request_id: String,
    /// Updated request fields.
    pub input: RequestInput,
}

impl Validate for UpdateRequest {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.request_id) {
            return Err(InputRefusal::new("requestId"));
        }
        self.input.validate()
    }
}

/// Argument referencing one request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestRef {
    /// Request ID.
    pub request_id: String,
}

impl Validate for RequestRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.request_id) {
            return Err(InputRefusal::new("requestId"));
        }
        Ok(())
    }
}

/// Argument of `save_environment`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentInput {
    /// Owning collection ID.
    pub collection_id: String,
    /// Existing environment ID when editing, or `None` when creating.
    #[serde(default)]
    pub environment_id: Option<String>,
    /// Environment name (unique within the collection without regard to case).
    pub name: String,
    /// Environment variables.
    #[serde(default)]
    pub variables: Vec<VariableEntry>,
}

impl Validate for EnvironmentInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.collection_id) {
            return Err(InputRefusal::new("collectionId"));
        }
        if self
            .environment_id
            .as_deref()
            .is_some_and(|id| !is_valid_id(id))
        {
            return Err(InputRefusal::new("environmentId"));
        }
        if !is_valid_name(&self.name) {
            return Err(InputRefusal::new("name"));
        }
        if !are_valid_variables(&self.variables) {
            return Err(InputRefusal::new("variables"));
        }
        Ok(())
    }
}

/// Argument referencing one environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentRef {
    /// Environment ID.
    pub environment_id: String,
}

impl Validate for EnvironmentRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.environment_id) {
            return Err(InputRefusal::new("environmentId"));
        }
        Ok(())
    }
}

/// Argument of `set_active_environment`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetActiveEnvironment {
    /// Collection ID.
    pub collection_id: String,
    /// Environment ID to activate, or `None` to clear.
    #[serde(default)]
    pub environment_id: Option<String>,
}

impl Validate for SetActiveEnvironment {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.collection_id) {
            return Err(InputRefusal::new("collectionId"));
        }
        if self
            .environment_id
            .as_deref()
            .is_some_and(|id| !is_valid_id(id))
        {
            return Err(InputRefusal::new("environmentId"));
        }
        Ok(())
    }
}

/// Argument of `save_global_variables`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveGlobalVariables {
    /// Full list of global variables.
    pub variables: Vec<VariableEntry>,
}

impl Validate for SaveGlobalVariables {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !are_valid_variables(&self.variables) {
            return Err(InputRefusal::new("variables"));
        }
        Ok(())
    }
}

/// Argument of `save_cookie`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "RFC 6265 cookie attributes (host_only, secure, http_only) plus enabled toggle"
)]
pub struct CookieInput {
    /// Existing cookie ID when editing, or `None` to add/upsert.
    #[serde(default)]
    pub cookie_id: Option<String>,
    /// Cookie name.
    pub name: String,
    /// Cookie value (stored only in the OS credential store).
    pub value: String,
    /// Domain scope (such as `api.example.com` or `example.com`).
    pub domain: String,
    /// Whether this cookie is host-only (exact host match, no subdomain suffix
    /// matching). Defaults to `false` unless set.
    #[serde(default)]
    pub host_only: bool,
    /// Path prefix (defaults to `/`).
    #[serde(default)]
    pub path: String,
    /// Whether the cookie travels only over HTTPS.
    #[serde(default)]
    pub secure: bool,
    /// Whether the cookie is `HttpOnly`.
    #[serde(default)]
    pub http_only: bool,
    /// Optional expiration timestamp.
    #[serde(default)]
    pub expires_at: Option<UtcMillis>,
    /// Whether the cookie is active for outbound requests.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Validate for CookieInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.cookie_id.as_deref().is_some_and(|id| !is_valid_id(id)) {
            return Err(InputRefusal::new("cookieId"));
        }

        let name = self.name.trim();
        let name_ok = !name.is_empty()
            && name.chars().count() <= MAX_KEY_CHARS
            && !name
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, ';' | '=' | ' '));

        if !name_ok {
            return Err(InputRefusal::new("name"));
        }

        let value_ok =
            self.value.len() <= MAX_VALUE_BYTES && !self.value.chars().any(char::is_control);

        if !value_ok {
            return Err(InputRefusal::new("value"));
        }

        let domain = self.domain.trim();
        let domain_ok = !domain.is_empty()
            && domain.chars().count() <= MAX_KEY_CHARS
            && !domain
                .chars()
                .any(|ch| ch.is_control() || ch.is_whitespace());

        if !domain_ok {
            return Err(InputRefusal::new("domain"));
        }

        let path_ok = self.path.chars().count() <= MAX_KEY_CHARS
            && !self.path.chars().any(char::is_control)
            && (self.path.is_empty() || self.path.trim().starts_with('/'));

        if !path_ok {
            return Err(InputRefusal::new("path"));
        }
        Ok(())
    }
}

/// Argument referencing one cookie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CookieRef {
    /// Cookie ID.
    pub cookie_id: String,
}

impl Validate for CookieRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.cookie_id) {
            return Err(InputRefusal::new("cookieId"));
        }
        Ok(())
    }
}

/// Argument of `pick_openapi_file`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PickOpenApiFile {
    /// File-dialog filter label from the UI message catalog.
    pub filter_name: String,
}

impl Validate for PickOpenApiFile {
    fn validate(&self) -> Result<(), InputRefusal> {
        let trimmed = self.filter_name.trim();
        let ok = !trimmed.is_empty()
            && trimmed.chars().count() <= MAX_FILTER_CHARS
            && !trimmed.chars().any(char::is_control);

        if !ok {
            return Err(InputRefusal::new("filterName"));
        }
        Ok(())
    }
}

/// Argument of `preview_openapi_text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenApiTextInput {
    /// Raw `OpenAPI` v2 or v3 JSON or YAML specification text.
    pub text: String,
}

impl Validate for OpenApiTextInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.text.trim().is_empty() || self.text.len() > MAX_SPEC_BYTES {
            return Err(InputRefusal::new("text"));
        }
        Ok(())
    }
}

/// Argument of `confirm_openapi_import`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenApiConfirmInput {
    /// Ephemeral preview ID returned by `pick_openapi_file` or
    /// `preview_openapi_text`.
    pub preview_id: String,
    /// Optional custom collection name override.
    #[serde(default)]
    pub name: Option<String>,
    /// Optional base URL override.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Whether the imported collection starts enabled for agents.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Validate for OpenApiConfirmInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.preview_id) {
            return Err(InputRefusal::new("previewId"));
        }
        if let Some(name) = &self.name
            && !name.trim().is_empty()
            && !is_valid_name(name)
        {
            return Err(InputRefusal::new("name"));
        }
        if let Some(url) = &self.base_url
            && !is_valid_url_template(url)
        {
            return Err(InputRefusal::new("baseUrl"));
        }
        Ok(())
    }
}

/// Argument of `send_owner_request`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManualSendInput {
    /// Client-generated run ID so the workbench can cancel the request.
    pub run_id: String,
    /// Owning collection ID.
    pub collection_id: String,
    /// Saved request ID (optional when sending an edited workbench draft).
    #[serde(default)]
    pub request_id: Option<String>,
    /// Optional environment ID override (defaults to the collection's active
    /// environment).
    #[serde(default)]
    pub environment_id: Option<String>,
    /// Optional edited workbench request draft to send without saving first.
    #[serde(default)]
    pub draft: Option<RequestInput>,
}

impl Validate for ManualSendInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.run_id) {
            return Err(InputRefusal::new("runId"));
        }
        if !is_valid_id(&self.collection_id) {
            return Err(InputRefusal::new("collectionId"));
        }
        if self
            .request_id
            .as_deref()
            .is_some_and(|id| !is_valid_id(id))
        {
            return Err(InputRefusal::new("requestId"));
        }
        if self
            .environment_id
            .as_deref()
            .is_some_and(|id| !is_valid_id(id))
        {
            return Err(InputRefusal::new("environmentId"));
        }
        if let Some(draft) = &self.draft {
            draft.validate()?;
        } else if self.request_id.is_none() {
            return Err(InputRefusal::new("requestId"));
        }
        Ok(())
    }
}

/// Argument of `cancel_owner_request`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelRunInput {
    /// Client-generated run ID passed to `send_owner_request`.
    pub run_id: String,
}

impl Validate for CancelRunInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_valid_id(&self.run_id) {
            return Err(InputRefusal::new("runId"));
        }
        Ok(())
    }
}

/// Maximum JSON IPC input size for pasted `OpenAPI` specifications (10 MiB
/// spec plus JSON escaping overhead).
const MAX_OPENAPI_IPC_BYTES: u32 = 28 * 1024 * 1024;

/// Maximum JSON IPC input size for saving or manually sending requests (2 MiB
/// body, headers, params and JSON escaping overhead).
const MAX_REQUEST_IPC_BYTES: u32 = 16 * 1024 * 1024;

/// Maximum JSON IPC input size for collections, environments and global
/// variable lists (up to 200 entries plus JSON escaping overhead).
const MAX_COLLECTION_IPC_BYTES: u32 = 12 * 1024 * 1024;

/// Maximum JSON IPC input size for saving a single cookie (16 KiB value plus
/// JSON escaping overhead).
const MAX_COOKIE_IPC_BYTES: u32 = 128 * 1024;

macro_rules! command_spec {
    ($name:ident, $wire:literal, $docs:literal, $input:ty, $output:ty $(, max_input = $max_input:expr)?) => {
        /// IPC command specification.
        #[derive(Debug)]
        pub struct $name;

        impl CommandSpec for $name {
            const NAME: &'static str = $wire;
            const DOCS: &'static str = $docs;
            $(const MAX_INPUT_BYTES: u32 = $max_input;)?
            type Input = $input;
            type Output = $output;
            type Error = NetworkError;
        }
    };
}

command_spec!(
    GetStateCommand,
    "get_state",
    "Returns the full owner workbench state including collections, global variables and cookies.",
    (),
    NetworkStateSnapshot
);
command_spec!(
    CreateCollectionCommand,
    "create_collection",
    "Creates a new collection.",
    CollectionInput,
    CollectionView,
    max_input = MAX_COLLECTION_IPC_BYTES
);
command_spec!(
    UpdateCollectionCommand,
    "update_collection",
    "Updates an existing collection.",
    UpdateCollection,
    CollectionView,
    max_input = MAX_COLLECTION_IPC_BYTES
);
command_spec!(
    SetCollectionEnabledCommand,
    "set_collection_enabled",
    "Enables or disables a collection for agents.",
    SetCollectionEnabled,
    ()
);
command_spec!(
    DeleteCollectionCommand,
    "delete_collection",
    "Deletes a collection and its folders, environments, requests and secrets.",
    CollectionRef,
    ()
);
command_spec!(
    CreateFolderCommand,
    "create_folder",
    "Creates a folder inside a collection.",
    FolderCreateInput,
    FolderView
);
command_spec!(
    RenameFolderCommand,
    "rename_folder",
    "Renames a folder.",
    RenameFolder,
    ()
);
command_spec!(
    DeleteFolderCommand,
    "delete_folder",
    "Deletes a folder and moves its children to the collection root.",
    FolderRef,
    ()
);
command_spec!(
    CreateRequestCommand,
    "create_request",
    "Creates a saved request in a collection.",
    CreateRequest,
    RequestView,
    max_input = MAX_REQUEST_IPC_BYTES
);
command_spec!(
    UpdateRequestCommand,
    "update_request",
    "Updates a saved request.",
    UpdateRequest,
    RequestView,
    max_input = MAX_REQUEST_IPC_BYTES
);
command_spec!(
    DuplicateRequestCommand,
    "duplicate_request",
    "Duplicates a saved request.",
    RequestRef,
    RequestView
);
command_spec!(
    DeleteRequestCommand,
    "delete_request",
    "Deletes a saved request and its stored secrets.",
    RequestRef,
    ()
);
command_spec!(
    SaveEnvironmentCommand,
    "save_environment",
    "Creates or updates an environment in a collection.",
    EnvironmentInput,
    EnvironmentView,
    max_input = MAX_COLLECTION_IPC_BYTES
);
command_spec!(
    DeleteEnvironmentCommand,
    "delete_environment",
    "Deletes an environment and its stored secrets.",
    EnvironmentRef,
    ()
);
command_spec!(
    SetActiveEnvironmentCommand,
    "set_active_environment",
    "Sets or clears the active environment of a collection.",
    SetActiveEnvironment,
    ()
);
command_spec!(
    SaveGlobalVariablesCommand,
    "save_global_variables",
    "Replaces the global variables list.",
    SaveGlobalVariables,
    Vec<VariableEntry>,
    max_input = MAX_COLLECTION_IPC_BYTES
);
command_spec!(
    SaveCookieCommand,
    "save_cookie",
    "Creates or updates a cookie in the cookie jar.",
    CookieInput,
    CookieView,
    max_input = MAX_COOKIE_IPC_BYTES
);
command_spec!(
    DeleteCookieCommand,
    "delete_cookie",
    "Deletes one cookie from the cookie jar.",
    CookieRef,
    ()
);
command_spec!(
    ClearCookiesCommand,
    "clear_cookies",
    "Removes all cookies from the cookie jar.",
    (),
    ()
);
command_spec!(
    PickOpenApiFileCommand,
    "pick_openapi_file",
    "Opens the native file picker for an OpenAPI v2 or v3 JSON or YAML file and returns a preview.",
    PickOpenApiFile,
    Option<OpenApiPreview>
);
command_spec!(
    PreviewOpenApiTextCommand,
    "preview_openapi_text",
    "Parses pasted OpenAPI v2 or v3 JSON or YAML text and returns an import preview.",
    OpenApiTextInput,
    OpenApiPreview,
    max_input = MAX_OPENAPI_IPC_BYTES
);
command_spec!(
    ConfirmOpenApiImportCommand,
    "confirm_openapi_import",
    "Imports a previously previewed OpenAPI specification into a new collection.",
    OpenApiConfirmInput,
    CollectionView
);
command_spec!(
    SendOwnerRequestCommand,
    "send_owner_request",
    "Runs an HTTP request directly from the owner workbench and returns the unredacted response.",
    ManualSendInput,
    ManualResponseView,
    max_input = MAX_REQUEST_IPC_BYTES
);
command_spec!(
    CancelOwnerRequestCommand,
    "cancel_owner_request",
    "Cancels an in-flight workbench request by run ID.",
    CancelRunInput,
    ()
);

/// Live channel pushing the owner workbench state after every mutation.
#[derive(Debug)]
pub struct FollowStateChannel;

impl ChannelSpec for FollowStateChannel {
    const NAME: &'static str = "follow_state";
    const DOCS: &'static str =
        "Streams the owner workbench state snapshot on subscribe and after every change.";
    type Input = ();
    type Item = NetworkStateSnapshot;
    type Error = NetworkError;
}

/// Complete IPC interface exported to the host and TypeScript bindings
/// generator (`task gen`).
pub fn interface() -> Interface {
    Interface::new()
        .command::<GetStateCommand>()
        .command::<CreateCollectionCommand>()
        .command::<UpdateCollectionCommand>()
        .command::<SetCollectionEnabledCommand>()
        .command::<DeleteCollectionCommand>()
        .command::<CreateFolderCommand>()
        .command::<RenameFolderCommand>()
        .command::<DeleteFolderCommand>()
        .command::<CreateRequestCommand>()
        .command::<UpdateRequestCommand>()
        .command::<DuplicateRequestCommand>()
        .command::<DeleteRequestCommand>()
        .command::<SaveEnvironmentCommand>()
        .command::<DeleteEnvironmentCommand>()
        .command::<SetActiveEnvironmentCommand>()
        .command::<SaveGlobalVariablesCommand>()
        .command::<SaveCookieCommand>()
        .command::<DeleteCookieCommand>()
        .command::<ClearCookiesCommand>()
        .command::<PickOpenApiFileCommand>()
        .command::<PreviewOpenApiTextCommand>()
        .command::<ConfirmOpenApiImportCommand>()
        .command::<SendOwnerRequestCommand>()
        .command::<CancelOwnerRequestCommand>()
        .channel::<FollowStateChannel>()
}
