//! Paste JSON (plugins/mcp/docs/README.md, registering a server): the
//! preview of a pasted configuration, which names each server, what it
//! would run and the names of its variables and headers, and the import of
//! the servers that the owner chose, which reads the text again, so the
//! secrets in it cross the bridge only toward the core.
//!
//! A variable or a header whose name looks like a secret's is secret, and
//! so is one whose value is a placeholder of another client, such as
//! `${input:token}`: the owner enters its value after the import, and the
//! server starts off until then.

use mannie_plugin_sdk::contracts::Secret;
use mannie_plugin_sdk::ipc::Validate as _;

use crate::commands::{
    ConfiguredServerView, EntryRefusalCode, ImportOutcome, ImportSkip, McpError, NamedValue,
    SecretValue, ServerInput, ServerTransport, ValueRules,
};
use crate::config::{
    self, ConfigurationRefusal, ConfiguredServer, ConfiguredValue, Endpoint, EntryRefusal,
};
use crate::rules::DEFAULT_TIMEOUT_SECONDS;
use crate::servers::Servers;

/// The servers of a pasted configuration, as the preview shows them.
pub(crate) fn preview(
    servers: &Servers,
    text: &str,
) -> Result<Vec<ConfiguredServerView>, McpError> {
    let configuration = config::read(text).map_err(configuration_error)?;
    let names = servers.names();
    Ok(configuration
        .iter()
        .map(|(key, entry)| view(key, entry, &names))
        .collect())
}

/// Adds the servers of a pasted configuration that `keys` name.
pub(crate) async fn import(
    servers: &Servers,
    text: &str,
    keys: &[String],
) -> Result<ImportOutcome, McpError> {
    let configuration = config::read(text).map_err(configuration_error)?;
    let mut added = Vec::new();
    let mut skipped = Vec::new();
    for key in keys {
        let entry = configuration
            .iter()
            .find(|(listed, _entry)| listed == key)
            .map(|(_key, entry)| entry);
        let server = match entry {
            None => Err(EntryRefusalCode::NoEndpoint),
            Some(Err(refusal)) => Err(refusal_code(*refusal)),
            Some(Ok(server)) => Ok(server),
        };
        let server = match server {
            Ok(server) => server,
            Err(reason) => {
                skipped.push(ImportSkip {
                    key: key.clone(),
                    reason,
                });
                continue;
            }
        };
        let (input, enabled) = input_of(server);
        if input.validate().is_err() {
            skipped.push(ImportSkip {
                key: key.clone(),
                reason: EntryRefusalCode::InvalidValue,
            });
            continue;
        }
        match servers.import(input, enabled).await {
            Ok(view) => added.push(view),
            Err(McpError::NameTaken | McpError::PrefixTaken) => skipped.push(ImportSkip {
                key: key.clone(),
                reason: EntryRefusalCode::NameTaken,
            }),
            Err(error) => return Err(error),
        }
    }
    Ok(ImportOutcome { added, skipped })
}

/// A configuration that cannot be read, for the owner.
fn configuration_error(refusal: ConfigurationRefusal) -> McpError {
    match refusal {
        ConfigurationRefusal::TooLarge => McpError::ConfigurationTooLarge,
        ConfigurationRefusal::NotJson => McpError::ConfigurationNotJson,
        ConfigurationRefusal::NoServers => McpError::ConfigurationEmpty,
        ConfigurationRefusal::TooManyServers => McpError::ConfigurationTooManyServers,
    }
}

/// A refused server, for the owner.
fn refusal_code(refusal: EntryRefusal) -> EntryRefusalCode {
    match refusal {
        EntryRefusal::SseOnly => EntryRefusalCode::SseOnly,
        EntryRefusal::NoEndpoint => EntryRefusalCode::NoEndpoint,
        EntryRefusal::InvalidProgram => EntryRefusalCode::InvalidProgram,
        EntryRefusal::InvalidUrl => EntryRefusalCode::InvalidUrl,
        EntryRefusal::InvalidValue => EntryRefusalCode::InvalidValue,
        EntryRefusal::InvalidName => EntryRefusalCode::InvalidName,
    }
}

/// A server of a configuration as the owner's input, and whether it starts
/// on: not when the configuration turns it off or leaves a secret out.
fn input_of(server: &ConfiguredServer) -> (ServerInput, bool) {
    let (transport, values) = match &server.endpoint {
        Endpoint::Program { .. } => (ServerTransport::Stdio, &server.variables),
        Endpoint::Url { .. } => (ServerTransport::Http, &server.headers),
    };
    let (program, arguments, folder, url) = match &server.endpoint {
        Endpoint::Program {
            program,
            arguments,
            folder,
        } => (
            program.clone(),
            arguments.clone(),
            folder.clone().unwrap_or_default(),
            String::new(),
        ),
        Endpoint::Url { url } => (String::new(), Vec::new(), String::new(), url.clone()),
    };
    let is_secret = ValueRules::of(transport).is_secret;
    let mut plain = Vec::new();
    let mut secrets = Vec::new();
    for (name, value) in values {
        match value {
            ConfiguredValue::Given(value) if !is_secret(name) => plain.push(NamedValue {
                name: name.clone(),
                value: value.clone(),
            }),
            ConfiguredValue::Given(value) => secrets.push(SecretValue {
                name: name.clone(),
                value: Some(Secret::new(value.clone())),
            }),
            ConfiguredValue::Placeholder => secrets.push(SecretValue {
                name: name.clone(),
                value: None,
            }),
        }
    }
    let has_every_secret = secrets.iter().all(|secret| secret.value.is_some());
    let input = ServerInput {
        name: server.key.clone(),
        prefix: String::new(),
        transport,
        program,
        arguments,
        folder,
        url,
        plain,
        secrets,
        oauth: false,
        client_id: String::new(),
        client_secret: None,
        forget_client_secret: false,
        images: true,
        instructions: true,
        timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
    };
    (input, has_every_secret && !server.disabled)
}

/// A server of a configuration as the preview shows it.
fn view(
    key: &str,
    entry: &Result<ConfiguredServer, EntryRefusal>,
    names: &[String],
) -> ConfiguredServerView {
    let server = match entry {
        Ok(server) => server,
        Err(refusal) => {
            return ConfiguredServerView {
                key: key.to_owned(),
                transport: None,
                program: None,
                arguments: Vec::new(),
                url: None,
                plain_names: Vec::new(),
                secret_names: Vec::new(),
                missing_secrets: Vec::new(),
                disabled: false,
                refusal: Some(refusal_code(*refusal)),
            };
        }
    };
    let (input, _enabled) = input_of(server);
    let name_taken = names.iter().any(|name| name.eq_ignore_ascii_case(key));
    let refusal = if name_taken {
        Some(EntryRefusalCode::NameTaken)
    } else if input.validate().is_err() {
        Some(EntryRefusalCode::InvalidValue)
    } else {
        None
    };
    ConfiguredServerView {
        key: key.to_owned(),
        transport: Some(input.transport),
        program: (!input.program.is_empty()).then(|| input.program.clone()),
        arguments: input.arguments.clone(),
        url: (!input.url.is_empty()).then(|| input.url.clone()),
        plain_names: input.plain.iter().map(|value| value.name.clone()).collect(),
        secret_names: input
            .secrets
            .iter()
            .map(|secret| secret.name.clone())
            .collect(),
        missing_secrets: input
            .secrets
            .iter()
            .filter(|secret| secret.value.is_none())
            .map(|secret| secret.name.clone())
            .collect(),
        disabled: server.disabled,
        refusal,
    }
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::Secret;

    use super::{input_of, view};
    use crate::commands::{EntryRefusalCode, ServerTransport};
    use crate::config::read;

    const CONFIGURATION: &str = r#"{
        "mcpServers": {
            "github": {
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-github"],
                "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_planted", "GITHUB_HOST": "github.com" }
            },
            "remote": {
                "type": "http",
                "url": "https://mcp.example.com/mcp",
                "headers": { "Authorization": "Bearer ${input:token}", "X-Team": "core" }
            },
            "old": { "url": "https://mcp.example.com/sse" }
        }
    }"#;

    #[test]
    fn secrets_and_placeholders_become_secret_and_a_missing_one_keeps_the_server_off() {
        let configuration = read(CONFIGURATION).unwrap();
        let github = configuration[0].1.as_ref().unwrap();
        let remote = configuration[2].1.as_ref().unwrap();

        let (local, local_on) = input_of(github);
        let (http, http_on) = input_of(remote);

        assert_eq!(local.transport, ServerTransport::Stdio);
        assert_eq!(local.plain[0].name, "GITHUB_HOST");
        assert_eq!(local.secrets[0].name, "GITHUB_PERSONAL_ACCESS_TOKEN");
        assert_eq!(
            local.secrets[0].value.as_ref().map(Secret::expose),
            Some("ghp_planted")
        );
        assert!(local_on);
        assert_eq!(http.secrets[0].name, "Authorization");
        assert!(http.secrets[0].value.is_none());
        assert_eq!(http.plain[0].name, "X-Team");
        assert!(!http_on);
    }

    #[test]
    fn the_preview_names_values_without_them_and_marks_taken_names_and_refusals() {
        let configuration = read(CONFIGURATION).unwrap();

        let views: Vec<_> = configuration
            .iter()
            .map(|(key, entry)| view(key, entry, &["GitHub".to_owned()]))
            .collect();

        assert_eq!(views[0].refusal, Some(EntryRefusalCode::NameTaken));
        assert_eq!(views[0].secret_names, ["GITHUB_PERSONAL_ACCESS_TOKEN"]);
        assert_eq!(views[1].refusal, Some(EntryRefusalCode::SseOnly));
        assert_eq!(views[2].missing_secrets, ["Authorization"]);
        assert_eq!(views[2].refusal, None);
        let text = serde_json::to_string(&views).unwrap();
        assert!(!text.contains("ghp_planted"));
    }
}
