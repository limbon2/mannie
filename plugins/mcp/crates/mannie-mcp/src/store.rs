//! The plugin's tables (plugins/mcp/docs/README.md, data): `mcp_servers`,
//! one row for each server that the owner registered, and `mcp_tools`, the
//! tools that each server listed last, with the owner's switches.
//!
//! The secrets are not here: the credential store keeps them (`secrets.rs`),
//! and the row names the secret variables or headers and counts the parts of
//! each stored text, so the plugin reads and deletes exactly those parts. A
//! listing replaces a server's tools in one transaction, so the table never
//! holds half of one.

use std::sync::Arc;

use mannie_plugin_sdk::contracts::{ToolHint, UtcMillis};
use mannie_plugin_sdk::services::store::{
    Constraint, Durability, Expect, Migration, ModuleStoreApi, Row, Statement, StoreError,
    StoreModule, WriteBatch,
};
use serde_json::Value;

/// The prefix of the plugin's tables, which names its module of the store.
pub(crate) const PREFIX: &str = "mcp";

const MIGRATIONS: &[Migration] = &[Migration::new(
    1,
    "create the tables of the servers and of their tools",
    "CREATE TABLE mcp_servers (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE,
        prefix TEXT NOT NULL UNIQUE,
        transport TEXT NOT NULL CHECK (transport IN ('stdio', 'http')),
        program TEXT,
        arguments TEXT NOT NULL CHECK (json_valid(arguments)),
        folder TEXT,
        url TEXT,
        plain TEXT NOT NULL CHECK (json_valid(plain)),
        secret_names TEXT NOT NULL CHECK (json_valid(secret_names)),
        secret_parts INTEGER NOT NULL CHECK (secret_parts BETWEEN 0 AND 1000),
        missing_secrets TEXT NOT NULL CHECK (json_valid(missing_secrets)),
        oauth INTEGER NOT NULL CHECK (oauth IN (0, 1)),
        client_id TEXT,
        client_secret_set INTEGER NOT NULL CHECK (client_secret_set IN (0, 1)),
        signed_in INTEGER NOT NULL CHECK (signed_in IN (0, 1)),
        oauth_parts INTEGER NOT NULL CHECK (oauth_parts BETWEEN 0 AND 1000),
        enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
        images INTEGER NOT NULL CHECK (images IN (0, 1)),
        instructions INTEGER NOT NULL CHECK (instructions IN (0, 1)),
        timeout_seconds INTEGER NOT NULL CHECK (timeout_seconds BETWEEN 10 AND 3600),
        era TEXT CHECK (era IS NULL OR era IN ('initialize', 'discover')),
        listed INTEGER NOT NULL CHECK (listed IN (0, 1)),
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        CONSTRAINT mcp_servers_transport CHECK (
            (transport = 'stdio' AND program IS NOT NULL AND url IS NULL AND oauth = 0
                AND client_id IS NULL)
            OR (transport = 'http' AND url IS NOT NULL AND program IS NULL AND folder IS NULL
                AND arguments = '[]')
        )
    ) STRICT;
    CREATE TABLE mcp_tools (
        server_id TEXT NOT NULL REFERENCES mcp_servers (id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        position INTEGER NOT NULL CHECK (position >= 0),
        title TEXT,
        description TEXT NOT NULL,
        schema TEXT NOT NULL CHECK (json_valid(schema)),
        hints TEXT NOT NULL CHECK (json_valid(hints)),
        enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
        new INTEGER NOT NULL CHECK (new IN (0, 1)),
        PRIMARY KEY (server_id, name)
    ) STRICT",
)];

const SELECT_SERVERS: &str = "SELECT id, name, prefix, transport, program, arguments, folder, \
    url, plain, secret_names, secret_parts, missing_secrets, oauth, client_id, \
    client_secret_set, signed_in, oauth_parts, enabled, images, instructions, timeout_seconds, \
    era, listed FROM mcp_servers ORDER BY name COLLATE NOCASE, id";

const SELECT_TOOLS: &str = "SELECT server_id, name, position, title, description, schema, hints, \
    enabled, new FROM mcp_tools ORDER BY server_id, position, name";

const INSERT_SERVER: &str = "INSERT INTO mcp_servers (id, name, prefix, transport, program, \
    arguments, folder, url, plain, secret_names, secret_parts, missing_secrets, oauth, client_id, \
    client_secret_set, signed_in, oauth_parts, enabled, images, instructions, timeout_seconds, \
    era, listed, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, \
    '[]', ?11, ?12, 0, 0, 0, ?13, ?14, ?15, ?16, NULL, 0, ?17, ?17)";

const UPDATE_SERVER: &str = "UPDATE mcp_servers SET name = ?2, prefix = ?3, transport = ?4, \
    program = ?5, arguments = ?6, folder = ?7, url = ?8, plain = ?9, secret_names = ?10, \
    oauth = ?11, client_id = ?12, images = ?13, instructions = ?14, timeout_seconds = ?15, \
    updated_at = ?16 WHERE id = ?1";

const FORGET_ERA: &str = "UPDATE mcp_servers SET era = NULL WHERE id = ?1";

const SET_SECRETS: &str = "UPDATE mcp_servers SET secret_parts = ?2, missing_secrets = ?3, \
    updated_at = ?4 WHERE id = ?1";

const SET_OAUTH_STATE: &str = "UPDATE mcp_servers SET oauth_parts = ?2, client_secret_set = ?3, \
    signed_in = ?4, updated_at = ?5 WHERE id = ?1";

const SET_ENABLED: &str = "UPDATE mcp_servers SET enabled = ?2, updated_at = ?3 WHERE id = ?1";

const SET_ERA: &str = "UPDATE mcp_servers SET era = ?2 WHERE id = ?1";

const SET_LISTED: &str = "UPDATE mcp_servers SET listed = 1 WHERE id = ?1";

const DELETE_TOOLS: &str = "DELETE FROM mcp_tools WHERE server_id = ?1";

const INSERT_TOOL: &str = "INSERT INTO mcp_tools (server_id, name, position, title, description, \
    schema, hints, enabled, new) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";

const SET_TOOL_ENABLED: &str =
    "UPDATE mcp_tools SET enabled = ?3, new = 0 WHERE server_id = ?1 AND name = ?2";

const ACKNOWLEDGE_TOOLS: &str = "UPDATE mcp_tools SET new = 0 WHERE server_id = ?1";

const DELETE_SERVER: &str = "DELETE FROM mcp_servers WHERE id = ?1";

/// The plugin's module of the store.
pub const MODULE: StoreModule = StoreModule::new(
    PREFIX,
    MIGRATIONS,
    &[
        SELECT_SERVERS,
        SELECT_TOOLS,
        INSERT_SERVER,
        UPDATE_SERVER,
        FORGET_ERA,
        SET_SECRETS,
        SET_OAUTH_STATE,
        SET_ENABLED,
        SET_ERA,
        SET_LISTED,
        DELETE_TOOLS,
        INSERT_TOOL,
        SET_TOOL_ENABLED,
        ACKNOWLEDGE_TOOLS,
        DELETE_SERVER,
    ],
);

/// The code of a name that another server has, which a batch maps from the
/// table's unique name.
pub(crate) const NAME_TAKEN: &str = "name_taken";

/// The code of a prefix that another server has.
pub(crate) const PREFIX_TAKEN: &str = "prefix_taken";

/// How the plugin reaches a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StoredEndpoint {
    /// A program that the core starts, with its arguments and its folder.
    Program {
        program: String,
        arguments: Vec<String>,
        folder: Option<String>,
    },
    /// A URL that speaks Streamable HTTP.
    Url { url: String },
}

impl StoredEndpoint {
    fn transport(&self) -> &'static str {
        match self {
            Self::Program { .. } => "stdio",
            Self::Url { .. } => "http",
        }
    }
}

/// The protocol era that a server's connection found, which the next one
/// starts in (plugins/mcp/docs/adr/0001-speak-mcp-through-rmcp-in-both-
/// protocol-eras.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Era {
    /// The `initialize` handshake, before 2026-07-28.
    Initialize,
    /// `server/discover` and the metadata of each request, from 2026-07-28.
    Discover,
}

impl Era {
    fn as_str(self) -> &'static str {
        match self {
            Self::Initialize => "initialize",
            Self::Discover => "discover",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "initialize" => Some(Self::Initialize),
            "discover" => Some(Self::Discover),
            _other => None,
        }
    }
}

/// A server as its row keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the row keeps each of the owner's switches and of the stored states as a column"
)]
pub(crate) struct ServerRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) prefix: String,
    pub(crate) endpoint: StoredEndpoint,
    /// The plain variables of a local server, or the plain headers of a
    /// remote one, by name in the owner's order.
    pub(crate) plain: Vec<(String, String)>,
    /// The names of the secret variables or headers, in the owner's order.
    pub(crate) secret_names: Vec<String>,
    /// How many parts the credential store keeps of the secrets' text.
    pub(crate) secret_parts: u32,
    /// The secrets whose values the credential store does not keep, such as
    /// those that an imported configuration left to the owner.
    pub(crate) missing_secrets: Vec<String>,
    /// Whether the server signs in with OAuth.
    pub(crate) oauth: bool,
    /// The client ID that the owner registered with the authorization
    /// server, or `None` for dynamic registration.
    pub(crate) client_id: Option<String>,
    /// Whether the credential store keeps a client secret for it.
    pub(crate) client_secret_set: bool,
    /// Whether the credential store keeps OAuth tokens for it.
    pub(crate) signed_in: bool,
    /// How many parts the credential store keeps of the OAuth text.
    pub(crate) oauth_parts: u32,
    pub(crate) enabled: bool,
    /// Whether the images of its results reach the model.
    pub(crate) images: bool,
    /// Whether its instructions reach the model.
    pub(crate) instructions: bool,
    pub(crate) timeout_seconds: u32,
    /// The era that its last connection found.
    pub(crate) era: Option<Era>,
    /// Whether it listed its tools once, so a tool that it adds later
    /// starts off.
    pub(crate) listed: bool,
}

/// The fields of a server that the owner sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerFields {
    pub(crate) name: String,
    pub(crate) prefix: String,
    pub(crate) endpoint: StoredEndpoint,
    pub(crate) plain: Vec<(String, String)>,
    pub(crate) secret_names: Vec<String>,
    pub(crate) oauth: bool,
    pub(crate) client_id: Option<String>,
    pub(crate) images: bool,
    pub(crate) instructions: bool,
    pub(crate) timeout_seconds: u32,
}

/// A tool as its row keeps it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolRecord {
    /// The tool's name, as the server gives it.
    pub(crate) name: String,
    /// Its place in the server's list.
    pub(crate) position: u32,
    pub(crate) title: Option<String>,
    pub(crate) description: String,
    /// The schema of its arguments, as the server gives it, or `Null` when
    /// it was too large to keep.
    pub(crate) schema: Value,
    /// What the server says of it.
    pub(crate) hints: Vec<ToolHint>,
    /// Whether the owner lets agents use it.
    pub(crate) enabled: bool,
    /// Whether it arrived after the server's first listing and the owner
    /// has not looked at it yet.
    pub(crate) new: bool,
}

/// The plugin's tables, through its module of the store.
#[derive(Debug, Clone)]
pub(crate) struct McpStore {
    store: Arc<dyn ModuleStoreApi>,
}

impl McpStore {
    pub(crate) fn new(store: Arc<dyn ModuleStoreApi>) -> Self {
        Self { store }
    }

    /// Every server, by name.
    pub(crate) async fn servers(&self) -> Result<Vec<ServerRecord>, StoreError> {
        let rows = self.store.query(Statement::new(SELECT_SERVERS)).await?;
        rows.iter().map(server_record).collect()
    }

    /// Every tool, with its server's ID, by server and position.
    pub(crate) async fn tools(&self) -> Result<Vec<(String, ToolRecord)>, StoreError> {
        let rows = self.store.query(Statement::new(SELECT_TOOLS)).await?;
        rows.iter().map(tool_record).collect()
    }

    /// Adds a server without secrets, on or off. `NAME_TAKEN` and
    /// `PREFIX_TAKEN` name a taken name or prefix.
    pub(crate) async fn insert(
        &self,
        id: &str,
        fields: &ServerFields,
        enabled: bool,
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let insert = bind_fields(Statement::new(INSERT_SERVER).bind(id), fields)?
            .bind(enabled)
            .bind(fields.images)
            .bind(fields.instructions)
            .bind(fields.timeout_seconds)
            .bind(now.as_millis());
        let batch = taken_codes(WriteBatch::new().push(insert, Expect::Changes(1)));
        self.write(batch).await
    }

    /// Changes a server's fields, and forgets its era when `forget_era` says
    /// that its endpoint changed, in one transaction.
    pub(crate) async fn update(
        &self,
        id: &str,
        fields: &ServerFields,
        forget_era: bool,
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let update = bind_fields(Statement::new(UPDATE_SERVER).bind(id), fields)?
            .bind(fields.images)
            .bind(fields.instructions)
            .bind(fields.timeout_seconds)
            .bind(now.as_millis());
        let mut batch = taken_codes(WriteBatch::new().push(update, Expect::Changes(1)));
        if forget_era {
            batch = batch.push(Statement::new(FORGET_ERA).bind(id), Expect::Changes(1));
        }
        self.write(batch).await
    }

    /// Records how many parts of the secrets' text the credential store
    /// keeps, and which secrets have no value there.
    pub(crate) async fn set_secrets(
        &self,
        id: &str,
        parts: u32,
        missing: &[String],
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let missing = serde_json::to_string(missing).map_err(|_error| StoreError::Failed)?;
        let update = Statement::new(SET_SECRETS)
            .bind(id)
            .bind(parts)
            .bind(missing)
            .bind(now.as_millis());
        self.write(WriteBatch::new().push(update, Expect::Changes(1)))
            .await
    }

    /// Records what the credential store keeps of a server's OAuth: how many
    /// parts, whether a client secret and whether tokens.
    pub(crate) async fn set_oauth_state(
        &self,
        id: &str,
        state: OAuthState,
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let update = Statement::new(SET_OAUTH_STATE)
            .bind(id)
            .bind(state.parts)
            .bind(state.client_secret_set)
            .bind(state.signed_in)
            .bind(now.as_millis());
        self.write(WriteBatch::new().push(update, Expect::Changes(1)))
            .await
    }

    /// Turns a server on or off.
    pub(crate) async fn set_enabled(
        &self,
        id: &str,
        enabled: bool,
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let update = Statement::new(SET_ENABLED)
            .bind(id)
            .bind(enabled)
            .bind(now.as_millis());
        self.write(WriteBatch::new().push(update, Expect::Changes(1)))
            .await
    }

    /// Records the era that a server's connection found. A server removed
    /// meanwhile is not a failure.
    pub(crate) async fn set_era(&self, id: &str, era: Era) -> Result<(), StoreError> {
        let update = Statement::new(SET_ERA).bind(id).bind(era.as_str());
        self.write(WriteBatch::new().push(update, Expect::Any))
            .await
    }

    /// Replaces a server's tools with a listing and marks the server as
    /// listed, in one transaction. A server removed meanwhile keeps no tools.
    pub(crate) async fn replace_tools(
        &self,
        id: &str,
        tools: &[ToolRecord],
    ) -> Result<(), StoreError> {
        let mut batch = WriteBatch::new()
            .push(Statement::new(DELETE_TOOLS).bind(id), Expect::Any)
            .push(Statement::new(SET_LISTED).bind(id), Expect::Changes(1));
        for tool in tools {
            let hints = serde_json::to_string(&tool.hints).map_err(|_error| StoreError::Failed)?;
            let schema =
                serde_json::to_string(&tool.schema).map_err(|_error| StoreError::Failed)?;
            let insert = Statement::new(INSERT_TOOL)
                .bind(id)
                .bind(tool.name.as_str())
                .bind(tool.position)
                .bind(tool.title.as_deref())
                .bind(tool.description.as_str())
                .bind(schema)
                .bind(hints)
                .bind(tool.enabled)
                .bind(tool.new);
            batch = batch.push(insert, Expect::Changes(1));
        }
        self.write(batch).await
    }

    /// Turns a tool on or off, which also clears its mark as new. Returns
    /// whether the tool exists.
    pub(crate) async fn set_tool_enabled(
        &self,
        id: &str,
        tool: &str,
        enabled: bool,
    ) -> Result<bool, StoreError> {
        let update = Statement::new(SET_TOOL_ENABLED)
            .bind(id)
            .bind(tool)
            .bind(enabled);
        let changed = self
            .store
            .write(
                WriteBatch::new().push(update, Expect::Any),
                Durability::Full,
            )
            .await?;
        Ok(changed.first().is_some_and(|rows| *rows == 1))
    }

    /// Clears the marks of a server's new tools, once the owner saw them.
    pub(crate) async fn acknowledge_tools(&self, id: &str) -> Result<(), StoreError> {
        let update = Statement::new(ACKNOWLEDGE_TOOLS).bind(id);
        self.write(WriteBatch::new().push(update, Expect::Any))
            .await
    }

    /// Forgets a server and its tools. A server that is gone already is not
    /// a failure.
    pub(crate) async fn delete(&self, id: &str) -> Result<(), StoreError> {
        let batch = WriteBatch::new()
            .push(Statement::new(DELETE_TOOLS).bind(id), Expect::Any)
            .push(Statement::new(DELETE_SERVER).bind(id), Expect::Any);
        self.write(batch).await
    }

    async fn write(&self, batch: WriteBatch) -> Result<(), StoreError> {
        self.store.write(batch, Durability::Full).await.map(drop)
    }
}

/// What the credential store keeps of a server's OAuth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OAuthState {
    /// How many parts of the OAuth text.
    pub(crate) parts: u32,
    /// Whether a client secret.
    pub(crate) client_secret_set: bool,
    /// Whether tokens.
    pub(crate) signed_in: bool,
}

/// The table's unique name and prefix, as SQLite reports their failures.
fn taken_codes(batch: WriteBatch) -> WriteBatch {
    batch
        .on_constraint(
            Constraint::Key {
                table: "mcp_servers",
                columns: &["name"],
            },
            NAME_TAKEN,
        )
        .on_constraint(
            Constraint::Key {
                table: "mcp_servers",
                columns: &["prefix"],
            },
            PREFIX_TAKEN,
        )
}

/// Binds the owner's fields, from `?2` to `?12`.
fn bind_fields(statement: Statement, fields: &ServerFields) -> Result<Statement, StoreError> {
    let (program, arguments, folder, url) = match &fields.endpoint {
        StoredEndpoint::Program {
            program,
            arguments,
            folder,
        } => (
            Some(program.as_str()),
            arguments.as_slice(),
            folder.as_deref(),
            None,
        ),
        StoredEndpoint::Url { url } => (None, [].as_slice(), None, Some(url.as_str())),
    };
    let arguments = serde_json::to_string(arguments).map_err(|_error| StoreError::Failed)?;
    let plain = serde_json::to_string(&fields.plain).map_err(|_error| StoreError::Failed)?;
    let secret_names =
        serde_json::to_string(&fields.secret_names).map_err(|_error| StoreError::Failed)?;
    Ok(statement
        .bind(fields.name.as_str())
        .bind(fields.prefix.as_str())
        .bind(fields.endpoint.transport())
        .bind(program)
        .bind(arguments)
        .bind(folder)
        .bind(url)
        .bind(plain)
        .bind(secret_names)
        .bind(fields.oauth)
        .bind(fields.client_id.as_deref()))
}

/// A JSON column read as `T`.
fn json_column<T: serde::de::DeserializeOwned>(row: &Row, column: usize) -> Result<T, StoreError> {
    let text: String = row.get(column)?;
    serde_json::from_str(&text).map_err(|_error| StoreError::InvalidValue { column })
}

/// A server from a row of `SELECT_SERVERS`.
fn server_record(row: &Row) -> Result<ServerRecord, StoreError> {
    let transport: String = row.get(3)?;
    let endpoint = match transport.as_str() {
        "stdio" => StoredEndpoint::Program {
            program: row.get(4)?,
            arguments: json_column(row, 5)?,
            folder: row.get(6)?,
        },
        "http" => StoredEndpoint::Url { url: row.get(7)? },
        _other => return Err(StoreError::InvalidValue { column: 3 }),
    };
    let era: Option<String> = row.get(21)?;
    Ok(ServerRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        prefix: row.get(2)?,
        endpoint,
        plain: json_column(row, 8)?,
        secret_names: json_column(row, 9)?,
        secret_parts: row.get(10)?,
        missing_secrets: json_column(row, 11)?,
        oauth: row.get(12)?,
        client_id: row.get(13)?,
        client_secret_set: row.get(14)?,
        signed_in: row.get(15)?,
        oauth_parts: row.get(16)?,
        enabled: row.get(17)?,
        images: row.get(18)?,
        instructions: row.get(19)?,
        timeout_seconds: row.get(20)?,
        era: era.as_deref().and_then(Era::parse),
        listed: row.get(22)?,
    })
}

/// A tool and its server's ID from a row of `SELECT_TOOLS`.
fn tool_record(row: &Row) -> Result<(String, ToolRecord), StoreError> {
    Ok((
        row.get(0)?,
        ToolRecord {
            name: row.get(1)?,
            position: row.get(2)?,
            title: row.get(3)?,
            description: row.get(4)?,
            schema: json_column(row, 5)?,
            hints: json_column(row, 6)?,
            enabled: row.get(7)?,
            new: row.get(8)?,
        },
    ))
}
