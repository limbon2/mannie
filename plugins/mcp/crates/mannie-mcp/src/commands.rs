//! The plugin's commands and channel, as specs of the plugin SDK (ADR-0034,
//! decision 8), so the host checks their input before the plugin's handlers
//! run and answers `plugin_off` while the plugin is off.
//!
//! The commands register servers and change the owner's switches. A command
//! connects to a server only where the owner asks for it: adding, changing,
//! turning on or restarting one, and signing in (ADR-0043, decision 7). No
//! command calls a server's tool: only the core's policy lets a call run.
//!
//! The settings take a program, its arguments, a folder and a URL that the
//! owner typed (ADR-0043, decision 5). Each is checked here and again where
//! it is used, since the webview is not trusted. Secrets come in and never
//! go back: a view names a secret, and says whether its value is stored.

use mannie_plugin_sdk::contracts::{Secret, ToolHint, UtcMillis};
use mannie_plugin_sdk::ipc::{
    ChannelSpec, CommandSpec, InputRefusal, Interface, PluginError, Validate,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::config::MAX_CONFIGURATION_BYTES;
use crate::log::{MAX_LOG_BYTES, MIN_REDACTED_BYTES};
use crate::naming::{MAX_PREFIX_CHARS, is_prefix};
use crate::rules::{
    DEFAULT_TIMEOUT_SECONDS, MAX_ARGUMENT_BYTES, MAX_ARGUMENTS, MAX_CLIENT_ID_CHARS,
    MAX_CLIENT_SECRET_BYTES, MAX_FOLDER_CHARS, MAX_HEADERS, MAX_NAME_CHARS,
    MAX_NAME_OF_VALUE_CHARS, MAX_PROGRAM_CHARS, MAX_TIMEOUT_SECONDS, MAX_URL_BYTES,
    MAX_VALUE_BYTES, MAX_VARIABLES, MIN_TIMEOUT_SECONDS, are_arguments, header_is_secret,
    is_client_id, is_client_secret, is_folder, is_header_name, is_header_value, is_program,
    is_server_name, is_timeout, is_variable_name, is_variable_value, server_url,
    variable_is_secret,
};

/// The most tools of all servers together that agents may use at once
/// (plugins/mcp/docs/README.md, Show tools).
pub const MAX_ENABLED_TOOLS: u32 = 64;

/// The most characters of a tool's name in a command.
const MAX_TOOL_NAME_CHARS: usize = 256;

/// The most characters of each text of the page that a sign-in shows.
pub const MAX_PAGE_TEXT_CHARS: usize = 500;

/// The most servers that one import adds.
const MAX_IMPORTED_SERVERS: usize = 32;

/// How the plugin reaches a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ServerTransport {
    /// A program that the core starts, which speaks MCP over its standard
    /// input and output.
    Stdio,
    /// A URL that speaks Streamable HTTP.
    Http,
}

/// A plain variable of a local server, or a plain header of a remote one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedValue {
    /// The variable's or the header's name.
    pub name: String,
    /// Its value.
    pub value: String,
}

/// A secret variable of a local server, or a secret header of a remote one.
#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretValue {
    /// The variable's or the header's name.
    pub name: String,
    /// Its value, which goes to the credential store and never back to the
    /// webview, or `None` to keep the stored value of a server that is
    /// changed.
    #[serde(default)]
    pub value: Option<Secret>,
}

/// A server as the owner sets it, which `add_server` adds and
/// `update_server` changes.
#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each switch is one of the settings' checkboxes"
)]
pub struct ServerInput {
    /// The server's name, unique without regard to case.
    pub name: String,
    /// The start of its tools' names after `mcp_`, or empty to take one
    /// from the name.
    #[serde(default)]
    pub prefix: String,
    /// How the plugin reaches the server. A server keeps its transport.
    pub transport: ServerTransport,
    /// The program of a local server: an absolute path, or a name that the
    /// core finds on Mannie's `PATH`, such as `npx`.
    #[serde(default)]
    pub program: String,
    /// The program's arguments, each as the program receives it.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// The absolute folder that the program runs in, or empty for the
    /// owner's home folder.
    #[serde(default)]
    pub folder: String,
    /// The URL of a remote server.
    #[serde(default)]
    pub url: String,
    /// The plain variables of a local server, or the plain headers of a
    /// remote one.
    #[serde(default)]
    pub plain: Vec<NamedValue>,
    /// The secret variables or headers.
    #[serde(default)]
    pub secrets: Vec<SecretValue>,
    /// Whether a remote server signs in with OAuth.
    #[serde(default)]
    pub oauth: bool,
    /// The client ID that the owner registered with the authorization
    /// server, or empty to register Mannie on its own.
    #[serde(default)]
    pub client_id: String,
    /// A new client secret, or `None` to keep the stored one.
    #[serde(default)]
    pub client_secret: Option<Secret>,
    /// Whether to delete the stored client secret.
    #[serde(default)]
    pub forget_client_secret: bool,
    /// Whether the images of the server's results reach the model.
    pub images: bool,
    /// Whether the server's instructions reach the model.
    pub instructions: bool,
    /// Seconds before a call of the server's tool ends.
    pub timeout_seconds: u32,
}

impl Validate for ServerInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_server_name(&self.name) {
            return Err(InputRefusal::new("name"));
        }
        if !self.prefix.is_empty() && !is_prefix(&self.prefix) {
            return Err(InputRefusal::new("prefix"));
        }
        match self.transport {
            ServerTransport::Stdio => self.check_program()?,
            ServerTransport::Http => self.check_url()?,
        }
        self.check_values()?;
        if !is_timeout(self.timeout_seconds) {
            return Err(InputRefusal::new("timeoutSeconds"));
        }
        Ok(())
    }
}

impl ServerInput {
    /// The fields of a local server, and none of a remote one.
    fn check_program(&self) -> Result<(), InputRefusal> {
        if !is_program(&self.program) {
            return Err(InputRefusal::new("program"));
        }
        if !are_arguments(&self.arguments) {
            return Err(InputRefusal::new("arguments"));
        }
        if !self.folder.is_empty() && !is_folder(&self.folder) {
            return Err(InputRefusal::new("folder"));
        }
        if !self.url.is_empty() {
            return Err(InputRefusal::new("url"));
        }
        let has_oauth = self.oauth || !self.client_id.is_empty() || self.client_secret.is_some();
        if has_oauth {
            return Err(InputRefusal::new("oauth"));
        }
        Ok(())
    }

    /// The fields of a remote server, and none of a local one.
    fn check_url(&self) -> Result<(), InputRefusal> {
        if server_url(&self.url).is_err() {
            return Err(InputRefusal::new("url"));
        }
        if !self.program.is_empty() {
            return Err(InputRefusal::new("program"));
        }
        if !self.arguments.is_empty() {
            return Err(InputRefusal::new("arguments"));
        }
        if !self.folder.is_empty() {
            return Err(InputRefusal::new("folder"));
        }
        let client_id_is_invalid = !self.client_id.is_empty() && !is_client_id(&self.client_id);
        let has_client_without_oauth = !self.oauth && !self.client_id.is_empty();
        if client_id_is_invalid || has_client_without_oauth {
            return Err(InputRefusal::new("clientId"));
        }
        let secret_is_invalid = self
            .client_secret
            .as_ref()
            .is_some_and(|secret| !is_client_secret(secret.expose()));
        let has_secret_without_client = self.client_secret.is_some() && self.client_id.is_empty();
        if secret_is_invalid || has_secret_without_client {
            return Err(InputRefusal::new("clientSecret"));
        }
        Ok(())
    }

    /// The plain and secret variables or headers: valid names that appear
    /// once without regard to case, valid values, a name that looks like a
    /// secret's only among the secrets, and no `Authorization` header beside
    /// OAuth.
    fn check_values(&self) -> Result<(), InputRefusal> {
        let rules = ValueRules::of(self.transport);
        if self.plain.len() + self.secrets.len() > rules.limit {
            return Err(InputRefusal::new("plain"));
        }
        let mut names: Vec<String> = Vec::new();
        for value in &self.plain {
            let is_valid = (rules.is_name)(&value.name)
                && (rules.is_value)(&value.value)
                && !(rules.is_secret)(&value.name);
            let lower = value.name.to_ascii_lowercase();
            if !is_valid || names.contains(&lower) {
                return Err(InputRefusal::new("plain"));
            }
            names.push(lower);
        }
        for secret in &self.secrets {
            let value_is_invalid = secret
                .value
                .as_ref()
                .is_some_and(|value| value.is_empty() || !(rules.is_value)(value.expose()));
            let lower = secret.name.to_ascii_lowercase();
            let is_authorization_beside_oauth = self.oauth && lower == "authorization";
            let is_refused = !(rules.is_name)(&secret.name)
                || value_is_invalid
                || names.contains(&lower)
                || is_authorization_beside_oauth;
            if is_refused {
                return Err(InputRefusal::new("secrets"));
            }
            names.push(lower);
        }
        Ok(())
    }
}

/// The rules of the variables of a local server, or of the headers of a
/// remote one.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ValueRules {
    /// The most of them, plain and secret together.
    pub(crate) limit: usize,
    /// Whether a name is one of theirs.
    pub(crate) is_name: fn(&str) -> bool,
    /// Whether a value is one of theirs.
    pub(crate) is_value: fn(&str) -> bool,
    /// Whether a name must be secret.
    pub(crate) is_secret: fn(&str) -> bool,
}

impl ValueRules {
    /// The rules of a server of `transport`.
    pub(crate) fn of(transport: ServerTransport) -> Self {
        match transport {
            ServerTransport::Stdio => Self {
                limit: MAX_VARIABLES,
                is_name: is_variable_name,
                is_value: is_variable_value,
                is_secret: variable_is_secret,
            },
            ServerTransport::Http => Self {
                limit: MAX_HEADERS,
                is_name: is_header_name,
                is_value: is_header_value,
                is_secret: header_is_secret,
            },
        }
    }
}

/// Whether `server_id` can be a server's ID, which the plugin makes as a
/// UUID.
fn check_server_id(server_id: &str) -> Result<(), InputRefusal> {
    if uuid::Uuid::try_parse(server_id).is_err() {
        return Err(InputRefusal::new("serverId"));
    }
    Ok(())
}

/// The argument of `update_server`.
#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateServer {
    /// The server.
    pub server_id: String,
    /// Its fields, as the owner set them.
    pub server: ServerInput,
}

impl Validate for UpdateServer {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_id(&self.server_id)?;
        self.server.validate()
    }
}

/// A server, as a command names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerRef {
    /// The server.
    pub server_id: String,
}

impl Validate for ServerRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_id(&self.server_id)
    }
}

/// The argument of `set_server_enabled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetServerEnabled {
    /// The server.
    pub server_id: String,
    /// Whether the plugin connects to it and agents use its tools.
    pub enabled: bool,
}

impl Validate for SetServerEnabled {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_id(&self.server_id)
    }
}

/// The argument of `set_tool_enabled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetToolEnabled {
    /// The server.
    pub server_id: String,
    /// The tool, as the server names it.
    pub tool: String,
    /// Whether agents may use it.
    pub enabled: bool,
}

impl Validate for SetToolEnabled {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_id(&self.server_id)?;
        let count = self.tool.chars().count();
        if !(1..=MAX_TOOL_NAME_CHARS).contains(&count) {
            return Err(InputRefusal::new("tool"));
        }
        Ok(())
    }
}

/// A configuration that the owner pasted, in the formats that other clients
/// write.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationText {
    /// The configuration's JSON.
    pub text: String,
}

impl Validate for ConfigurationText {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.text.len() > MAX_CONFIGURATION_BYTES {
            return Err(InputRefusal::new("text"));
        }
        Ok(())
    }
}

/// The argument of `import_configuration`: the pasted configuration again,
/// so its secrets cross the bridge only toward the core, and the servers to
/// add from it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportConfiguration {
    /// The configuration's JSON.
    pub text: String,
    /// The keys of the servers to add.
    pub keys: Vec<String>,
}

impl Validate for ImportConfiguration {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.text.len() > MAX_CONFIGURATION_BYTES {
            return Err(InputRefusal::new("text"));
        }
        let keys_are_invalid = self.keys.is_empty()
            || self.keys.len() > MAX_IMPORTED_SERVERS
            || self.keys.iter().any(|key| !is_server_name(key));
        if keys_are_invalid {
            return Err(InputRefusal::new("keys"));
        }
        Ok(())
    }
}

/// The argument of `locate_program`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocateProgram {
    /// The program, as the owner typed it.
    pub program: String,
}

impl Validate for LocateProgram {
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_program(&self.program) {
            return Err(InputRefusal::new("program"));
        }
        Ok(())
    }
}

/// The texts of the page that the browser shows when a sign-in returns,
/// from the UI's catalog, since the core has none (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignInPage {
    /// The page's title.
    pub title: String,
    /// What the page says after the sign-in worked.
    pub signed_in: String,
    /// What the page says after it failed.
    pub failed: String,
}

/// The argument of `begin_sign_in`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeginSignIn {
    /// The server.
    pub server_id: String,
    /// The page that the browser shows when the sign-in returns.
    pub page: SignInPage,
}

impl Validate for BeginSignIn {
    fn validate(&self) -> Result<(), InputRefusal> {
        check_server_id(&self.server_id)?;
        let texts = [&self.page.title, &self.page.signed_in, &self.page.failed];
        let texts_are_invalid = texts.iter().any(|text| {
            let count = text.chars().count();
            !(1..=MAX_PAGE_TEXT_CHARS).contains(&count) || text.chars().any(char::is_control)
        });
        if texts_are_invalid {
            return Err(InputRefusal::new("page"));
        }
        Ok(())
    }
}

/// Where a server stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ServerStatus {
    /// The owner turned it off, or the plugin is stopping.
    Off,
    /// The plugin connects to it, or waits to connect again.
    Starting,
    /// It answers, and agents use its tools.
    Connected,
    /// It asks for a sign-in with OAuth, which the owner starts.
    SignInNeeded,
    /// Its sign-in lacks a permission that a call asked for, so the owner
    /// signs in again.
    AccessNeeded,
    /// It failed, for the reason in the view, until the owner restarts it.
    Failed,
}

/// Why a server failed. More reasons may come, by addition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ServerFailure {
    /// The program is neither an absolute path nor found on Mannie's `PATH`.
    ProgramNotFound,
    /// The program is a file that the core does not start.
    ProgramRefused,
    /// The working folder does not exist.
    WorkingDirectoryInvalid,
    /// An argument cannot be given to the program.
    ArgumentRefused,
    /// A variable cannot be given to the server.
    VariableRefused {
        /// The variable's name.
        name: String,
    },
    /// The plugin runs as many local servers as it may.
    TooManyServers {
        /// The limit.
        limit: u32,
    },
    /// The program could not be started.
    SpawnFailed,
    /// The program ended before it answered, with its exit code if it had
    /// one.
    Exited {
        /// The exit code.
        exit_code: Option<i32>,
    },
    /// The server ended or dropped its connection too often.
    Crashed {
        /// How often in the window of restarts.
        restarts: u32,
    },
    /// The server did not answer in time.
    TimedOut {
        /// The seconds that it had.
        seconds: u32,
    },
    /// The URL could not be reached.
    Unreachable,
    /// The server answered with an HTTP status that ends the connection.
    HttpStatus {
        /// The status.
        status: u16,
    },
    /// The server does not speak MCP as the plugin does.
    Protocol,
    /// The server speaks only HTTP with server-sent events, the transport
    /// of 2024, which the plugin does not speak.
    SseOnly,
    /// A secret of the server is not stored, so the owner enters it again.
    SecretMissing,
    /// The credential store could not be used.
    CredentialStoreUnavailable,
    /// The authorization server does not offer what a sign-in needs, such
    /// as PKCE with S256.
    AuthorizationUnsupported,
    /// The sign-in failed, for a reason that the log names.
    SignInFailed,
}

/// What the settings warn about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ServerWarning {
    /// An argument looks like a token, which approvals and the journal show;
    /// a secret variable keeps it apart.
    TokenInArgument {
        /// The argument's place, from 0.
        index: u32,
    },
    /// The server added tools after the owner's first look, which start off.
    NewTools {
        /// How many.
        count: u32,
    },
    /// Some of its tools cannot be offered: their names or schemas break
    /// the rules of tools.
    UnusableTools {
        /// How many.
        count: u32,
    },
}

/// A server, as the settings show it. Its secrets never leave the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each switch is a checkbox or a state that the settings show"
)]
pub struct ServerView {
    /// The server's ID.
    pub id: String,
    /// Its name.
    pub name: String,
    /// The start of its tools' names after `mcp_`.
    pub prefix: String,
    /// How the plugin reaches it.
    pub transport: ServerTransport,
    /// The program of a local server.
    pub program: Option<String>,
    /// The program's arguments.
    pub arguments: Vec<String>,
    /// The folder that the program runs in, or `None` for the owner's home
    /// folder.
    pub folder: Option<String>,
    /// The URL of a remote server.
    pub url: Option<String>,
    /// Its plain variables or headers.
    pub plain: Vec<NamedValue>,
    /// The names of its secret variables or headers.
    pub secret_names: Vec<String>,
    /// The secrets among them whose values are not stored.
    pub missing_secrets: Vec<String>,
    /// Whether it signs in with OAuth.
    pub oauth: bool,
    /// The client ID that the owner registered.
    pub client_id: Option<String>,
    /// Whether a client secret is stored.
    pub client_secret_set: bool,
    /// Whether OAuth tokens are stored.
    pub signed_in: bool,
    /// Whether the owner turned it on.
    pub enabled: bool,
    /// Whether the images of its results reach the model.
    pub images: bool,
    /// Whether its instructions reach the model.
    pub instructions: bool,
    /// Seconds before a call of its tool ends.
    pub timeout_seconds: u32,
    /// Where it stands.
    pub status: ServerStatus,
    /// Why it failed, when it did.
    pub failure: Option<ServerFailure>,
    /// The protocol version of its connection.
    pub protocol_version: Option<String>,
    /// How many tools it listed last.
    pub tool_count: u32,
    /// How many of them agents may use.
    pub enabled_tool_count: u32,
    /// How many resources it lists.
    pub resource_count: u32,
    /// What the settings warn about.
    pub warnings: Vec<ServerWarning>,
}

/// Why a listed tool cannot be offered to agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ToolProblem {
    /// Its schema is not an object schema, or was too large to keep.
    SchemaInvalid,
    /// Its schema names both `activity` and `activity_value`.
    ActivityClash,
    /// Its description or its schema is longer than a published tool's may
    /// be.
    TooLong,
}

/// A tool of a server, as Show tools lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolView {
    /// The tool, as the server names it.
    pub name: String,
    /// The name that agents call it by.
    pub model_name: String,
    /// Its title, as the server gives it.
    pub title: Option<String>,
    /// Its description, as the server gives it.
    pub description: String,
    /// What the server says of it, which decides nothing.
    pub hints: Vec<ToolHint>,
    /// Whether agents may use it.
    pub enabled: bool,
    /// Whether it arrived after the server's first listing.
    pub new: bool,
    /// Why it cannot be offered, when it cannot.
    pub problem: Option<ToolProblem>,
}

/// Why a server of a pasted configuration cannot be added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EntryRefusalCode {
    /// It speaks only HTTP with server-sent events.
    SseOnly,
    /// It names neither a program nor a URL.
    NoEndpoint,
    /// Its program, arguments or folder break the rules of servers.
    InvalidProgram,
    /// Its URL breaks the rules of servers.
    InvalidUrl,
    /// A variable's or a header's name or value breaks their rules.
    InvalidValue,
    /// Its key is not a name that a server may have.
    InvalidName,
    /// A server of that name is registered already.
    NameTaken,
}

/// A server of a pasted configuration, as the preview shows it, without the
/// values of its variables and headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredServerView {
    /// Its key in the configuration, which names it.
    pub key: String,
    /// How the plugin would reach it.
    pub transport: Option<ServerTransport>,
    /// The program that it runs.
    pub program: Option<String>,
    /// The program's arguments.
    pub arguments: Vec<String>,
    /// Its URL.
    pub url: Option<String>,
    /// The names of its plain variables or headers.
    pub plain_names: Vec<String>,
    /// The names of its secret variables or headers.
    pub secret_names: Vec<String>,
    /// The secrets that the configuration leaves for the owner to enter.
    pub missing_secrets: Vec<String>,
    /// Whether the configuration turns it off.
    pub disabled: bool,
    /// Why it cannot be added, when it cannot.
    pub refusal: Option<EntryRefusalCode>,
}

/// What `import_configuration` did with each server that it was asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    /// The servers that it added. A server whose secrets the configuration
    /// leaves out starts off.
    pub added: Vec<ServerView>,
    /// The servers that it did not add, with the reasons.
    pub skipped: Vec<ImportSkip>,
}

/// A server that `import_configuration` did not add.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportSkip {
    /// Its key.
    pub key: String,
    /// Why.
    pub reason: EntryRefusalCode,
}

/// What happened to a server's connections, as its log keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum LogEvent {
    /// The plugin started to connect.
    Connecting,
    /// The server answered.
    Connected {
        /// The protocol version.
        version: String,
    },
    /// The server listed its tools.
    Listed {
        /// How many.
        tools: u32,
    },
    /// The program ended, with its exit code if it had one.
    Exited {
        /// The exit code.
        exit_code: Option<i32>,
    },
    /// The connection ended.
    Disconnected,
    /// The plugin connects again after a pause.
    Restarting {
        /// The pause in seconds.
        seconds: u32,
    },
    /// The server failed.
    Failed {
        /// Why.
        failure: ServerFailure,
    },
    /// The owner or the plugin stopped the server.
    Stopped,
    /// The owner signed in.
    SignedIn,
    /// The owner signed out.
    SignedOut,
}

/// An entry of a server's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum LogEntry {
    /// A line of the server's error output, with its secrets replaced.
    Output {
        /// When it arrived.
        at: UtcMillis,
        /// The line.
        text: String,
    },
    /// Something that happened to the server's connections.
    Event {
        /// When.
        at: UtcMillis,
        /// What.
        event: LogEvent,
    },
}

/// What `server_log` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ServerLog {
    /// The entries, oldest first.
    pub entries: Vec<LogEntry>,
    /// Whether older entries left the log.
    pub cut: bool,
}

/// Where the core would find a program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LocatedProgramView {
    /// The program's path.
    pub path: String,
    /// Whether it lies inside one of the owner's projects, where a model's
    /// changes could replace it (T-63).
    pub in_project: bool,
}

/// What `begin_sign_in` returns: the page that the browser opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SignInStarted {
    /// The authorization server's page.
    pub url: String,
    /// The authorization server's host, which the settings show.
    pub host: String,
}

/// What the channel `follow_servers` carries: the whole list after each
/// change. More kinds of updates may come, by addition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ServerListUpdate {
    /// The servers, by name.
    Servers {
        /// The servers.
        servers: Vec<ServerView>,
        /// How many tools of all servers agents may use.
        enabled_tools: u32,
    },
}

/// Why a command or the channel of the plugin failed. The host's refusals,
/// such as `plugin_off` and `invalid_input` with the refused field, come
/// beside these (ADR-0034, decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum McpError {
    /// Another server has the name, without regard to case.
    #[error("another server has this name")]
    NameTaken,
    /// Another server has the prefix.
    #[error("another server has this prefix")]
    PrefixTaken,
    /// The server is not registered, or was removed meanwhile.
    #[error("the server is not registered")]
    ServerNotFound,
    /// A server keeps its transport; a new server takes another.
    #[error("a server keeps its transport")]
    TransportChanged,
    /// The server has not listed this tool.
    #[error("the server has no such tool")]
    ToolNotFound,
    /// Agents may use at most this many tools of all servers together.
    #[error("agents may use at most {limit} tools of the servers")]
    TooManyTools {
        /// The limit.
        limit: u32,
    },
    /// The configuration is longer than the plugin reads.
    #[error("the configuration is too long")]
    ConfigurationTooLarge,
    /// The configuration is not JSON.
    #[error("the configuration is not JSON")]
    ConfigurationNotJson,
    /// The configuration lists no servers in a format that the plugin reads.
    #[error("the configuration lists no servers")]
    ConfigurationEmpty,
    /// The configuration lists more servers than one import adds.
    #[error("the configuration lists too many servers")]
    ConfigurationTooManyServers,
    /// The program is neither an absolute path nor found on Mannie's `PATH`.
    #[error("the program was not found")]
    ProgramNotFound,
    /// The program is a file that the core does not start.
    #[error("the program is not one that a server may run")]
    ProgramRefused,
    /// The server does not sign in with OAuth.
    #[error("the server does not sign in with OAuth")]
    NoSignIn,
    /// The authorization server could not be reached or answered with an
    /// error.
    #[error("the authorization server failed")]
    AuthorizationFailed,
    /// The authorization server does not offer what a sign-in needs.
    #[error("the authorization server does not offer what a sign-in needs")]
    AuthorizationUnsupported,
    /// The authorization server takes no client registration, so the owner
    /// enters a client ID.
    #[error("the authorization server needs a client ID")]
    ClientIdNeeded,
    /// The credential store could not be used.
    #[error("the credential store is not available")]
    CredentialStoreUnavailable,
    /// The store did not keep the change.
    #[error("the change could not be stored")]
    StorageFailed,
}

impl PluginError for McpError {
    fn code(&self) -> &'static str {
        match self {
            Self::NameTaken => "mcp.name_taken",
            Self::PrefixTaken => "mcp.prefix_taken",
            Self::ServerNotFound => "mcp.server_not_found",
            Self::TransportChanged => "mcp.transport_changed",
            Self::ToolNotFound => "mcp.tool_not_found",
            Self::TooManyTools { .. } => "mcp.too_many_tools",
            Self::ConfigurationTooLarge => "mcp.configuration_too_large",
            Self::ConfigurationNotJson => "mcp.configuration_not_json",
            Self::ConfigurationEmpty => "mcp.configuration_empty",
            Self::ConfigurationTooManyServers => "mcp.configuration_too_many_servers",
            Self::ProgramNotFound => "mcp.program_not_found",
            Self::ProgramRefused => "mcp.program_refused",
            Self::NoSignIn => "mcp.no_sign_in",
            Self::AuthorizationFailed => "mcp.authorization_failed",
            Self::AuthorizationUnsupported => "mcp.authorization_unsupported",
            Self::ClientIdNeeded => "mcp.client_id_needed",
            Self::CredentialStoreUnavailable => "mcp.credential_store_unavailable",
            Self::StorageFailed => "mcp.storage_failed",
        }
    }
}

/// `add_server`: registers a server, keeps its secrets in the credential
/// store and connects to it.
#[derive(Debug)]
pub enum AddServerCommand {}

impl CommandSpec for AddServerCommand {
    const NAME: &'static str = "add_server";
    const DOCS: &'static str = "Registers a server, keeps its secrets in the credential store and connects to it to list its tools.";
    type Input = ServerInput;
    type Output = ServerView;
    type Error = McpError;
}

/// `update_server`: changes a server and connects to it again.
#[derive(Debug)]
pub enum UpdateServerCommand {}

impl CommandSpec for UpdateServerCommand {
    const NAME: &'static str = "update_server";
    const DOCS: &'static str =
        "Changes a server and its secrets, and connects to it again when it is on.";
    type Input = UpdateServer;
    type Output = ServerView;
    type Error = McpError;
}

/// `remove_server`: forgets a server and deletes its secrets.
#[derive(Debug)]
pub enum RemoveServerCommand {}

impl CommandSpec for RemoveServerCommand {
    const NAME: &'static str = "remove_server";
    const DOCS: &'static str = "Stops a server, deletes its secrets and tokens from the credential store, and forgets it and its tools. Its grants stay listed and cover nothing.";
    type Input = ServerRef;
    type Output = ();
    type Error = McpError;
}

/// `set_server_enabled`: turns a server on or off.
#[derive(Debug)]
pub enum SetServerEnabledCommand {}

impl CommandSpec for SetServerEnabledCommand {
    const NAME: &'static str = "set_server_enabled";
    const DOCS: &'static str = "Turns a server on, which connects to it, or off, which stops it and takes its tools from agents.";
    type Input = SetServerEnabled;
    type Output = ServerView;
    type Error = McpError;
}

/// `restart_server`: connects to a server again.
#[derive(Debug)]
pub enum RestartServerCommand {}

impl CommandSpec for RestartServerCommand {
    const NAME: &'static str = "restart_server";
    const DOCS: &'static str =
        "Stops a server that is on and connects to it again, also after it failed.";
    type Input = ServerRef;
    type Output = ();
    type Error = McpError;
}

/// `server_tools`: the tools that a server listed last.
#[derive(Debug)]
pub enum ServerToolsCommand {}

impl CommandSpec for ServerToolsCommand {
    const NAME: &'static str = "server_tools";
    const DOCS: &'static str =
        "The tools that a server listed last, with what it says of them and the owner's switches.";
    type Input = ServerRef;
    type Output = Vec<ToolView>;
    type Error = McpError;
}

/// `set_tool_enabled`: lets agents use a tool or not.
#[derive(Debug)]
pub enum SetToolEnabledCommand {}

impl CommandSpec for SetToolEnabledCommand {
    const NAME: &'static str = "set_tool_enabled";
    const DOCS: &'static str = "Lets agents use a tool of a server or not. At most MAX_ENABLED_TOOLS tools of all servers are on.";
    type Input = SetToolEnabled;
    type Output = ();
    type Error = McpError;
}

/// `acknowledge_tools`: clears the marks of a server's new tools.
#[derive(Debug)]
pub enum AcknowledgeToolsCommand {}

impl CommandSpec for AcknowledgeToolsCommand {
    const NAME: &'static str = "acknowledge_tools";
    const DOCS: &'static str = "Clears the marks of the tools that a server added after its first listing, once the owner saw them.";
    type Input = ServerRef;
    type Output = ();
    type Error = McpError;
}

/// `server_log`: a server's log.
#[derive(Debug)]
pub enum ServerLogCommand {}

impl CommandSpec for ServerLogCommand {
    const NAME: &'static str = "server_log";
    const DOCS: &'static str = "The last MAX_LOG_BYTES of a server's error output and of what happened to its connections, with its secrets replaced.";
    type Input = ServerRef;
    type Output = ServerLog;
    type Error = McpError;
}

/// `parse_configuration`: the servers of a pasted configuration.
#[derive(Debug)]
pub enum ParseConfigurationCommand {}

impl CommandSpec for ParseConfigurationCommand {
    const NAME: &'static str = "parse_configuration";
    const DOCS: &'static str = "Reads the servers of a pasted configuration without running anything, and names their variables and headers without their values.";
    type Input = ConfigurationText;
    type Output = Vec<ConfiguredServerView>;
    type Error = McpError;
}

/// `import_configuration`: adds servers of a pasted configuration.
#[derive(Debug)]
pub enum ImportConfigurationCommand {}

impl CommandSpec for ImportConfigurationCommand {
    const NAME: &'static str = "import_configuration";
    const DOCS: &'static str = "Adds the chosen servers of a pasted configuration, with their secrets in the credential store. A server whose secrets the configuration leaves out starts off.";
    type Input = ImportConfiguration;
    type Output = ImportOutcome;
    type Error = McpError;
}

/// `locate_program`: where the core would find a program.
#[derive(Debug)]
pub enum LocateProgramCommand {}

impl CommandSpec for LocateProgramCommand {
    const NAME: &'static str = "locate_program";
    const DOCS: &'static str = "Where the core would find a program that the owner typed, and whether it lies inside a project. Nothing runs.";
    type Input = LocateProgram;
    type Output = LocatedProgramView;
    type Error = McpError;
}

/// `begin_sign_in`: starts a sign-in with OAuth.
#[derive(Debug)]
pub enum BeginSignInCommand {}

impl CommandSpec for BeginSignInCommand {
    const NAME: &'static str = "begin_sign_in";
    const DOCS: &'static str = "Starts a sign-in to a remote server with OAuth and returns the authorization server's page, which the settings open in the browser. The plugin waits for the browser's return on a loopback port.";
    type Input = BeginSignIn;
    type Output = SignInStarted;
    type Error = McpError;
}

/// `sign_out`: forgets a server's OAuth tokens.
#[derive(Debug)]
pub enum SignOutCommand {}

impl CommandSpec for SignOutCommand {
    const NAME: &'static str = "sign_out";
    const DOCS: &'static str =
        "Deletes a server's OAuth tokens and registration from the credential store.";
    type Input = ServerRef;
    type Output = ServerView;
    type Error = McpError;
}

/// `follow_servers`: the servers as they are, then after each change.
#[derive(Debug)]
pub enum FollowServersChannel {}

impl ChannelSpec for FollowServersChannel {
    const NAME: &'static str = "follow_servers";
    const DOCS: &'static str =
        "The registered servers as they are, then after each change, until the plugin stops.";
    type Input = ();
    type Item = ServerListUpdate;
    type Error = McpError;
}

/// A limit as the UI reads it.
fn limit(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// The plugin's commands, its channel and the limits that the UI checks.
pub fn interface() -> Interface {
    let interface = Interface::new()
        .command::<AddServerCommand>()
        .command::<UpdateServerCommand>()
        .command::<RemoveServerCommand>()
        .command::<SetServerEnabledCommand>()
        .command::<RestartServerCommand>()
        .command::<ServerToolsCommand>()
        .command::<SetToolEnabledCommand>()
        .command::<AcknowledgeToolsCommand>()
        .command::<ServerLogCommand>()
        .command::<ParseConfigurationCommand>()
        .command::<ImportConfigurationCommand>()
        .command::<LocateProgramCommand>()
        .command::<BeginSignInCommand>()
        .command::<SignOutCommand>()
        .channel::<FollowServersChannel>();
    with_other_limits(with_field_limits(interface))
}

/// The interface with the limits of a server's fields, which the UI checks.
fn with_field_limits(interface: Interface) -> Interface {
    interface
        .constant(
            "MAX_NAME_CHARS",
            "The most characters of a server's name.",
            limit(MAX_NAME_CHARS),
        )
        .constant(
            "MAX_PREFIX_CHARS",
            "The most characters of a server's prefix.",
            limit(MAX_PREFIX_CHARS),
        )
        .constant(
            "MAX_PROGRAM_CHARS",
            "The most characters of a local server's program.",
            limit(MAX_PROGRAM_CHARS),
        )
        .constant(
            "MAX_ARGUMENTS",
            "The most arguments of a local server's program.",
            limit(MAX_ARGUMENTS),
        )
        .constant(
            "MAX_ARGUMENT_BYTES",
            "The most bytes of one argument.",
            limit(MAX_ARGUMENT_BYTES),
        )
        .constant(
            "MAX_FOLDER_CHARS",
            "The most characters of a local server's folder.",
            limit(MAX_FOLDER_CHARS),
        )
        .constant(
            "MAX_URL_BYTES",
            "The most bytes of a remote server's URL.",
            limit(MAX_URL_BYTES),
        )
        .constant(
            "MAX_VARIABLES",
            "The most variables of a local server, plain and secret together.",
            limit(MAX_VARIABLES),
        )
        .constant(
            "MAX_HEADERS",
            "The most headers of a remote server, plain and secret together.",
            limit(MAX_HEADERS),
        )
        .constant(
            "MAX_NAME_OF_VALUE_CHARS",
            "The most characters of a variable's or a header's name.",
            limit(MAX_NAME_OF_VALUE_CHARS),
        )
        .constant(
            "MAX_VALUE_BYTES",
            "The most bytes of a variable's or a header's value.",
            limit(MAX_VALUE_BYTES),
        )
}

/// The interface with the other limits that the UI checks.
fn with_other_limits(interface: Interface) -> Interface {
    interface
        .constant(
            "MIN_TIMEOUT_SECONDS",
            "The shortest timeout of a call of a server's tool.",
            MIN_TIMEOUT_SECONDS,
        )
        .constant(
            "MAX_TIMEOUT_SECONDS",
            "The longest timeout of a call of a server's tool.",
            MAX_TIMEOUT_SECONDS,
        )
        .constant(
            "DEFAULT_TIMEOUT_SECONDS",
            "The timeout that a new server starts with.",
            DEFAULT_TIMEOUT_SECONDS,
        )
        .constant(
            "MAX_CLIENT_ID_CHARS",
            "The most characters of an OAuth client ID.",
            limit(MAX_CLIENT_ID_CHARS),
        )
        .constant(
            "MAX_CLIENT_SECRET_BYTES",
            "The most bytes of an OAuth client secret.",
            limit(MAX_CLIENT_SECRET_BYTES),
        )
        .constant(
            "MAX_ENABLED_TOOLS",
            "The most tools of all servers that agents may use at once.",
            MAX_ENABLED_TOOLS,
        )
        .constant(
            "MAX_CONFIGURATION_BYTES",
            "The most bytes of a pasted configuration.",
            limit(MAX_CONFIGURATION_BYTES),
        )
        .constant(
            "MAX_LOG_BYTES",
            "The most bytes of a server's log.",
            limit(MAX_LOG_BYTES),
        )
        .constant(
            "MIN_REDACTED_BYTES",
            "The shortest secret that the plugin replaces in a server's output.",
            limit(MIN_REDACTED_BYTES),
        )
        .constant(
            "MAX_PAGE_TEXT_CHARS",
            "The most characters of each text of a sign-in's page.",
            limit(MAX_PAGE_TEXT_CHARS),
        )
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::Secret;
    use mannie_plugin_sdk::ipc::Validate;

    use super::{NamedValue, SecretValue, ServerInput, ServerTransport, UpdateServer};

    fn local() -> ServerInput {
        ServerInput {
            name: "GitHub".to_owned(),
            prefix: String::new(),
            transport: ServerTransport::Stdio,
            program: "npx".to_owned(),
            arguments: vec![
                "-y".to_owned(),
                "@modelcontextprotocol/server-github".to_owned(),
            ],
            folder: String::new(),
            url: String::new(),
            plain: vec![NamedValue {
                name: "GITHUB_HOST".to_owned(),
                value: "github.com".to_owned(),
            }],
            secrets: vec![SecretValue {
                name: "GITHUB_PERSONAL_ACCESS_TOKEN".to_owned(),
                value: Some(Secret::new("ghp_planted".to_owned())),
            }],
            oauth: false,
            client_id: String::new(),
            client_secret: None,
            forget_client_secret: false,
            images: true,
            instructions: true,
            timeout_seconds: 300,
        }
    }

    fn remote() -> ServerInput {
        ServerInput {
            transport: ServerTransport::Http,
            program: String::new(),
            arguments: Vec::new(),
            url: "https://mcp.example.com/mcp".to_owned(),
            plain: vec![NamedValue {
                name: "X-Team".to_owned(),
                value: "core".to_owned(),
            }],
            secrets: Vec::new(),
            oauth: true,
            client_id: "mannie-desktop".to_owned(),
            ..local()
        }
    }

    fn refused_field(input: &impl Validate) -> Option<String> {
        input.validate().err().map(|refusal| refusal.field)
    }

    /// A change that breaks one rule of a server.
    type Break = fn(&mut ServerInput);

    #[test]
    fn a_local_server_is_refused_by_the_first_field_that_breaks_a_rule() {
        assert_eq!(refused_field(&local()), None);
        let cases: [(&str, Break); 9] = [
            ("name", |server| server.name = " GitHub".to_owned()),
            ("prefix", |server| server.prefix = "read".to_owned()),
            ("program", |server| server.program = String::new()),
            ("arguments", |server| {
                server.arguments = vec!["nul\0".to_owned()];
            }),
            ("folder", |server| server.folder = "relative".to_owned()),
            ("url", |server| {
                server.url = "https://example.com".to_owned();
            }),
            ("oauth", |server| server.oauth = true),
            ("plain", |server| {
                server.plain[0].name = "GITHUB_TOKEN".to_owned();
            }),
            ("timeoutSeconds", |server| server.timeout_seconds = 5),
        ];
        for (field, change) in cases {
            let mut server = local();
            change(&mut server);

            assert_eq!(refused_field(&server).as_deref(), Some(field), "{field}");
        }
    }

    #[test]
    fn a_remote_server_takes_https_headers_and_oauth_without_an_authorization_header() {
        assert_eq!(refused_field(&remote()), None);
        let cases: [(&str, Break); 6] = [
            ("url", |server| {
                server.url = "http://mcp.example.com/mcp".to_owned();
            }),
            ("url", |server| {
                server.url = "https://mcp.example.com/mcp?api_key=1".to_owned();
            }),
            ("program", |server| server.program = "npx".to_owned()),
            ("clientId", |server| server.oauth = false),
            ("plain", |server| {
                server.plain[0].name = "Mcp-Session-Id".to_owned();
            }),
            ("secrets", |server| {
                server.secrets = vec![SecretValue {
                    name: "Authorization".to_owned(),
                    value: Some(Secret::new("Bearer planted".to_owned())),
                }];
            }),
        ];
        for (field, change) in cases {
            let mut server = remote();
            change(&mut server);

            assert_eq!(refused_field(&server).as_deref(), Some(field), "{field}");
        }
    }

    #[test]
    fn names_appear_once_and_secret_values_are_not_empty() {
        let mut twice = local();
        twice.secrets.push(SecretValue {
            name: "github_personal_access_token".to_owned(),
            value: None,
        });
        let mut empty = local();
        empty.secrets[0].value = Some(Secret::new(String::new()));
        let mut kept = local();
        kept.secrets[0].value = None;

        assert_eq!(refused_field(&twice).as_deref(), Some("secrets"));
        assert_eq!(refused_field(&empty).as_deref(), Some("secrets"));
        assert_eq!(refused_field(&kept), None);
        let update = UpdateServer {
            server_id: "server".to_owned(),
            server: local(),
        };
        assert_eq!(refused_field(&update).as_deref(), Some("serverId"));
    }
}
