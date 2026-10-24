//! Paste JSON (plugins/mcp/docs/README.md, registering a server): the
//! servers of a configuration in the formats that other clients write,
//! `{"mcpServers": {...}}` and `{"servers": {...}}`, read without running
//! anything. The preview names each server and what it would run, with the
//! names of its variables and headers, and never their values: the import
//! reads the text again and stores the secrets without sending them back to
//! the webview.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::rules;

/// The most bytes of a pasted configuration.
pub(crate) const MAX_CONFIGURATION_BYTES: usize = 256 * 1024;

/// The most servers that one configuration adds.
pub(crate) const MAX_CONFIGURED_SERVERS: usize = 32;

/// How a configured server is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Endpoint {
    /// A program that Mannie starts.
    Program {
        program: String,
        arguments: Vec<String>,
        folder: Option<String>,
    },
    /// A URL that speaks Streamable HTTP.
    Url { url: String },
}

/// A value of a variable or a header, as the configuration gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConfiguredValue {
    /// A value as it is.
    Given(String),
    /// A placeholder such as `${input:token}` or `${env:TOKEN}`, which the
    /// owner fills in after the import.
    Placeholder,
}

/// A server of a configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfiguredServer {
    /// Its key in the configuration, which names it.
    pub(crate) key: String,
    /// How it is reached.
    pub(crate) endpoint: Endpoint,
    /// Its variables, by name in their order.
    pub(crate) variables: Vec<(String, ConfiguredValue)>,
    /// Its headers, by name in their order.
    pub(crate) headers: Vec<(String, ConfiguredValue)>,
    /// Whether the configuration turns it off.
    pub(crate) disabled: bool,
}

/// Why a server of a configuration cannot be added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryRefusal {
    /// It speaks HTTP with server-sent events, the transport of 2024, which
    /// Mannie does not speak.
    SseOnly,
    /// It names neither a program nor a URL, or one of the wrong kind.
    NoEndpoint,
    /// Its program, arguments or folder break the rules of servers.
    InvalidProgram,
    /// Its URL breaks the rules of servers.
    InvalidUrl,
    /// A variable's or a header's name or value breaks their rules.
    InvalidValue,
    /// Its key is not a name that a server may have.
    InvalidName,
}

/// What a configuration holds: its servers, each read or refused, by their
/// keys.
pub(crate) type Configuration = Vec<(String, Result<ConfiguredServer, EntryRefusal>)>;

/// Why a configuration cannot be read at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigurationRefusal {
    /// It is longer than [`MAX_CONFIGURATION_BYTES`].
    TooLarge,
    /// It is not JSON.
    NotJson,
    /// It has neither `mcpServers` nor `servers` with servers in it.
    NoServers,
    /// It has more servers than [`MAX_CONFIGURED_SERVERS`].
    TooManyServers,
}

/// Reads the servers of a pasted configuration.
pub(crate) fn read(text: &str) -> Result<Configuration, ConfigurationRefusal> {
    if text.len() > MAX_CONFIGURATION_BYTES {
        return Err(ConfigurationRefusal::TooLarge);
    }
    let value: Value =
        serde_json::from_str(text).map_err(|_error| ConfigurationRefusal::NotJson)?;
    let servers = ["mcpServers", "servers"]
        .iter()
        .find_map(|key| value.get(key).and_then(Value::as_object))
        .filter(|servers| !servers.is_empty())
        .ok_or(ConfigurationRefusal::NoServers)?;
    if servers.len() > MAX_CONFIGURED_SERVERS {
        return Err(ConfigurationRefusal::TooManyServers);
    }
    let mut configuration: Configuration = servers
        .iter()
        .map(|(key, entry)| (key.clone(), server(key, entry)))
        .collect();
    // By key, whichever order serde_json keeps.
    configuration.sort_by(|(left, _left), (right, _right)| left.cmp(right));
    Ok(configuration)
}

/// One server of a configuration.
fn server(key: &str, entry: &Value) -> Result<ConfiguredServer, EntryRefusal> {
    if !rules::is_server_name(key) {
        return Err(EntryRefusal::InvalidName);
    }
    let entry = entry.as_object().ok_or(EntryRefusal::NoEndpoint)?;
    let kind = entry.get("type").and_then(Value::as_str);
    let url = ["url", "serverUrl", "httpUrl"]
        .iter()
        .find_map(|field| entry.get(*field).and_then(Value::as_str));
    let command = entry.get("command").and_then(Value::as_str);
    let endpoint = match (kind, url, command) {
        (Some("sse"), _, _) => return Err(EntryRefusal::SseOnly),
        (Some("http" | "streamable-http" | "streamableHttp") | None, Some(url), None) => {
            // An old client's entry without a type names an endpoint of
            // server-sent events by its path.
            if kind.is_none() && url.trim_end_matches('/').ends_with("/sse") {
                return Err(EntryRefusal::SseOnly);
            }
            rules::server_url(url).map_err(|_refusal| EntryRefusal::InvalidUrl)?;
            Endpoint::Url {
                url: url.to_owned(),
            }
        }
        (Some("stdio") | None, None, Some(command)) => program(command, entry)?,
        _ => return Err(EntryRefusal::NoEndpoint),
    };
    let is_program = matches!(endpoint, Endpoint::Program { .. });
    let variables = if is_program {
        values(
            entry.get("env"),
            rules::is_variable_name,
            rules::is_variable_value,
        )?
    } else {
        Vec::new()
    };
    let headers = if is_program {
        Vec::new()
    } else {
        values(
            entry.get("headers"),
            rules::is_header_name,
            rules::is_header_value,
        )?
    };
    let disabled = entry.get("disabled").and_then(Value::as_bool) == Some(true);
    Ok(ConfiguredServer {
        key: key.to_owned(),
        endpoint,
        variables,
        headers,
        disabled,
    })
}

/// The program of an entry, with its arguments and its folder.
fn program(command: &str, entry: &Map<String, Value>) -> Result<Endpoint, EntryRefusal> {
    let arguments: Vec<String> = match entry.get("args") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .ok_or(EntryRefusal::InvalidProgram)?,
        Some(_) => return Err(EntryRefusal::InvalidProgram),
    };
    let folder = entry.get("cwd").and_then(Value::as_str).map(str::to_owned);
    let fits = rules::is_program(command)
        && rules::are_arguments(&arguments)
        && folder.as_deref().is_none_or(rules::is_folder);
    if !fits {
        return Err(EntryRefusal::InvalidProgram);
    }
    Ok(Endpoint::Program {
        program: command.to_owned(),
        arguments,
        folder,
    })
}

/// The variables or headers of an entry, by name in their order.
fn values(
    values: Option<&Value>,
    is_name: fn(&str) -> bool,
    is_value: fn(&str) -> bool,
) -> Result<Vec<(String, ConfiguredValue)>, EntryRefusal> {
    let Some(values) = values else {
        return Ok(Vec::new());
    };
    let values = values.as_object().ok_or(EntryRefusal::InvalidValue)?;
    if values.len() > rules::MAX_VARIABLES {
        return Err(EntryRefusal::InvalidValue);
    }
    let mut read = BTreeSet::new();
    let mut ordered = Vec::with_capacity(values.len());
    for (name, value) in values {
        let value = value.as_str().ok_or(EntryRefusal::InvalidValue)?;
        if !is_name(name) {
            return Err(EntryRefusal::InvalidValue);
        }
        let configured = if is_placeholder(value) {
            ConfiguredValue::Placeholder
        } else if is_value(value) {
            ConfiguredValue::Given(value.to_owned())
        } else {
            return Err(EntryRefusal::InvalidValue);
        };
        if !read.insert(name.to_ascii_lowercase()) {
            return Err(EntryRefusal::InvalidValue);
        }
        ordered.push((name.clone(), configured));
    }
    Ok(ordered)
}

/// Whether a value is a placeholder that another client fills in, such as
/// `${input:github_token}` or `${env:TOKEN}`.
fn is_placeholder(value: &str) -> bool {
    value.contains("${")
}

#[cfg(test)]
mod tests {
    use super::{ConfigurationRefusal, ConfiguredValue, Endpoint, EntryRefusal, read};

    #[test]
    fn the_servers_of_both_formats_are_read_by_their_keys() {
        let claude = r#"{
            "mcpServers": {
                "github": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-github"],
                    "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_planted", "GITHUB_HOST": "github.com" }
                },
                "linear": { "url": "https://mcp.linear.app/mcp" }
            }
        }"#;
        let vscode = r#"{
            "inputs": [{ "id": "token", "type": "promptString", "password": true }],
            "servers": {
                "files": { "type": "stdio", "command": "uvx", "args": ["mcp-server-files"], "cwd": "D:\\work" },
                "remote": {
                    "type": "http",
                    "url": "https://mcp.example.com/mcp",
                    "headers": { "Authorization": "Bearer ${input:token}" }
                }
            }
        }"#;

        let first = read(claude).unwrap();
        let second = read(vscode).unwrap();

        let keys: Vec<&str> = first.iter().map(|(key, _server)| key.as_str()).collect();
        assert_eq!(keys, ["github", "linear"]);
        let github = first[0].1.as_ref().unwrap();
        assert_eq!(
            github.endpoint,
            Endpoint::Program {
                program: "npx".to_owned(),
                arguments: vec![
                    "-y".to_owned(),
                    "@modelcontextprotocol/server-github".to_owned()
                ],
                folder: None,
            }
        );
        assert!(github.variables.contains(&(
            "GITHUB_PERSONAL_ACCESS_TOKEN".to_owned(),
            ConfiguredValue::Given("ghp_planted".to_owned())
        )));
        let remote = second[1].1.as_ref().unwrap();
        assert_eq!(
            remote.headers,
            [("Authorization".to_owned(), ConfiguredValue::Placeholder)]
        );
        let files = second[0].1.as_ref().unwrap();
        assert!(
            matches!(&files.endpoint, Endpoint::Program { folder: Some(folder), .. } if folder == "D:\\work")
        );
    }

    #[test]
    fn servers_of_server_sent_events_and_broken_entries_are_refused_alone() {
        let text = r#"{
            "mcpServers": {
                "old": { "url": "https://mcp.example.com/sse" },
                "typed": { "type": "sse", "url": "https://mcp.example.com/events" },
                "nothing": {},
                "insecure": { "url": "http://mcp.example.com/mcp" },
                "bad env": { "command": "npx", "env": { "A-B": "x" } },
                "good": { "command": "server" }
            }
        }"#;

        let read = read(text).unwrap();
        let refusals: Vec<Option<EntryRefusal>> = read
            .iter()
            .map(|(_key, server)| server.as_ref().err().copied())
            .collect();

        // By their keys: bad env, good, insecure, nothing, old, typed.
        assert_eq!(
            refusals,
            [
                Some(EntryRefusal::InvalidValue),
                None,
                Some(EntryRefusal::InvalidUrl),
                Some(EntryRefusal::NoEndpoint),
                Some(EntryRefusal::SseOnly),
                Some(EntryRefusal::SseOnly),
            ]
        );
    }

    #[test]
    fn a_configuration_without_servers_or_json_is_refused() {
        assert_eq!(read("not json"), Err(ConfigurationRefusal::NotJson));
        assert_eq!(read("{}"), Err(ConfigurationRefusal::NoServers));
        assert_eq!(
            read(r#"{"mcpServers": {}}"#),
            Err(ConfigurationRefusal::NoServers)
        );
        assert_eq!(
            read(&" ".repeat(super::MAX_CONFIGURATION_BYTES + 1)),
            Err(ConfigurationRefusal::TooLarge)
        );
    }
}
