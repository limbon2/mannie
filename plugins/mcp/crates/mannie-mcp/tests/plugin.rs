//! The plugin as the host sees it: its manifest, its interface, its tables,
//! the SDK's rules through the real host, and the settings' commands:
//! adding, changing and removing servers with their secrets in the
//! credential store, and Paste JSON, which never sends a secret back.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use mannie_mcp::PLUGIN_MANIFEST;
use mannie_mcp::commands::interface;
use mannie_mcp::{MCP, MODULE, mannie_plugin};
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::{CallFailure, HostOptions, HostedPlugin};
use mannie_plugin_sdk::ipc::{ChannelMessage, HostRefusal};
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::testing::FakeSecrets;
use serde_json::{Value, json};

/// The plugin, on, with its secrets in memory.
async fn hosted(secrets: &Arc<FakeSecrets>) -> HostedPlugin {
    let provider: Arc<FakeSecrets> = Arc::clone(secrets);
    let options = HostOptions::new().with_provider(provider);
    let hosted = HostedPlugin::new(mannie_plugin(), options).await;
    hosted.turn_on().await;
    hosted
}

/// A local server whose program the core does not find, so nothing runs.
fn local(name: &str) -> Value {
    json!({
        "name": name,
        "transport": "stdio",
        "program": "mannie-no-such-program-for-tests",
        "arguments": ["--token=ghp_16C7e42F292c6912E7710c838347Ae178B4a"],
        "plain": [{ "name": "GITHUB_HOST", "value": "github.com" }],
        "secrets": [{ "name": "GITHUB_TOKEN", "value": "planted-secret-token" }],
        "images": true,
        "instructions": true,
        "timeoutSeconds": 300,
    })
}

/// Waits until the view of the server `id` in the channel matches.
async fn wait_for_view(hosted: &HostedPlugin, id: &str, matches: impl Fn(&Value) -> bool) -> Value {
    let mut channel = hosted
        .open_channel("follow_servers", Value::Null)
        .await
        .expect("the channel opens");
    let found = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(message) = channel.next().await {
            let ChannelMessage::Item { item } = message else {
                break;
            };
            let found = item["servers"]
                .as_array()
                .and_then(|servers| servers.iter().find(|server| server["id"] == id))
                .cloned();
            if let Some(server) = found
                && matches(&server)
            {
                return Some(server);
            }
        }
        None
    })
    .await;
    found
        .ok()
        .flatten()
        .expect("the server matches before the channel ends, in time")
}

#[test]
fn the_manifest_declares_the_services_secrets_and_crates_of_the_plugin() {
    let manifest = Manifest::parse(PLUGIN_MANIFEST).unwrap();

    assert_eq!(manifest.id.as_str(), MCP);
    for service in ["local-servers", "secrets", "store"] {
        assert!(manifest.declares(service), "{service}");
    }
    assert_eq!(manifest.secrets.keyed, ["variable", "header", "oauth"]);
    assert_eq!(manifest.crates.exclusive, ["rmcp", "oauth2"]);
    assert_eq!(MODULE.name, manifest.id.prefix());
    let sql = MODULE.migrations[0].sql();
    assert!(sql.contains("mcp_servers") && sql.contains("mcp_tools"));
}

#[test]
fn the_interface_lists_the_commands_and_the_channel() {
    let declared = interface();
    let exported = mannie_plugin().plugin.interface();

    assert_eq!(
        declared.command_names(),
        [
            "add_server",
            "update_server",
            "remove_server",
            "set_server_enabled",
            "restart_server",
            "server_tools",
            "set_tool_enabled",
            "acknowledge_tools",
            "server_log",
            "parse_configuration",
            "import_configuration",
            "locate_program",
            "begin_sign_in",
            "sign_out",
        ]
    );
    assert_eq!(declared.channel_names(), ["follow_servers"]);
    assert_eq!(exported.command_names(), declared.command_names());
}

#[tokio::test]
async fn the_plugin_keeps_the_rules_of_the_sdk() {
    let options = HostOptions::new().with_provider(Arc::new(FakeSecrets::new()));

    assert_eq!(
        conformance(mannie_plugin(), options).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn a_server_keeps_its_secrets_in_the_credential_store_and_its_view_only_names_them() {
    let secrets = Arc::new(FakeSecrets::new());
    let hosted = hosted(&secrets).await;

    let added = hosted.invoke("add_server", local("GitHub")).await.unwrap();

    let id = added["id"].as_str().unwrap().to_owned();
    assert_eq!(added["prefix"], "github");
    assert_eq!(added["secretNames"], json!(["GITHUB_TOKEN"]));
    assert_eq!(added["missingSecrets"], json!([]));
    assert_eq!(
        added["warnings"][0],
        json!({ "code": "token_in_argument", "index": 0 })
    );
    assert!(!added.to_string().contains("planted-secret-token"));
    let stored = secrets
        .stored(&format!("mcp/variable/{id}-0"))
        .expect("the secret is stored");
    assert!(stored.expose().contains("planted-secret-token"));
    let failed = wait_for_view(&hosted, &id, |server| server["status"] == "failed").await;
    assert_eq!(failed["failure"], json!({ "code": "program_not_found" }));
    let log = hosted
        .invoke("server_log", json!({ "serverId": id }))
        .await
        .unwrap();
    assert!(
        log["entries"]
            .as_array()
            .is_some_and(|entries| !entries.is_empty())
    );
}

#[tokio::test]
async fn a_change_keeps_the_secrets_that_the_owner_does_not_replace_and_removal_deletes_them() {
    let secrets = Arc::new(FakeSecrets::new());
    let hosted = hosted(&secrets).await;
    let added = hosted.invoke("add_server", local("GitHub")).await.unwrap();
    let id = added["id"].as_str().unwrap().to_owned();
    let mut changed = local("GitHub tools");
    changed["secrets"] = json!([
        { "name": "GITHUB_TOKEN" },
        { "name": "OTHER_TOKEN", "value": "planted-other-token" },
    ]);

    let updated = hosted
        .invoke(
            "update_server",
            json!({ "serverId": id, "server": changed }),
        )
        .await
        .unwrap();
    let stored = secrets.stored(&format!("mcp/variable/{id}-0")).unwrap();

    assert_eq!(updated["name"], "GitHub tools");
    assert_eq!(updated["prefix"], "github");
    assert!(stored.expose().contains("planted-secret-token"));
    assert!(stored.expose().contains("planted-other-token"));
    hosted
        .invoke("remove_server", json!({ "serverId": id }))
        .await
        .unwrap();
    assert!(secrets.stored(&format!("mcp/variable/{id}-0")).is_none());
    let missing = hosted
        .invoke("server_log", json!({ "serverId": id }))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "mcp.server_not_found");
}

#[tokio::test]
async fn names_and_prefixes_stay_apart_and_inputs_break_no_rule() {
    let secrets = Arc::new(FakeSecrets::new());
    let hosted = hosted(&secrets).await;
    hosted.invoke("add_server", local("GitHub")).await.unwrap();

    let second = hosted.invoke("add_server", local("GitHub!")).await.unwrap();
    let taken = hosted
        .invoke("add_server", local("github"))
        .await
        .unwrap_err();
    let mut insecure = local("Remote");
    insecure["transport"] = json!("http");
    insecure["program"] = json!("");
    insecure["arguments"] = json!([]);
    insecure["url"] = json!("http://mcp.example.com/mcp");
    let refused = hosted.invoke("add_server", insecure).await.unwrap_err();

    assert_eq!(second["prefix"], "github2");
    assert_eq!(taken.code(), "mcp.name_taken");
    assert_eq!(
        refused,
        CallFailure::Host(HostRefusal::InvalidInput {
            field: Some("url".to_owned())
        })
    );
}

#[tokio::test]
async fn a_pasted_configuration_previews_without_values_and_imports_the_chosen_servers() {
    let secrets = Arc::new(FakeSecrets::new());
    let hosted = hosted(&secrets).await;
    let text = json!({
        "mcpServers": {
            "files": {
                "command": "mannie-no-such-program-for-tests",
                "env": { "FILES_TOKEN": "planted-files-token" }
            },
            "remote": {
                "type": "http",
                "url": "https://mcp.example.com/mcp",
                "headers": { "Authorization": "Bearer ${input:token}" }
            },
            "events": { "url": "https://mcp.example.com/sse" }
        }
    })
    .to_string();

    let preview = hosted
        .invoke("parse_configuration", json!({ "text": text }))
        .await
        .unwrap();
    let imported = hosted
        .invoke(
            "import_configuration",
            json!({ "text": text, "keys": ["files", "remote", "events"] }),
        )
        .await
        .unwrap();

    assert!(!preview.to_string().contains("planted-files-token"));
    assert_eq!(preview[0]["key"], "events");
    assert_eq!(preview[0]["refusal"], "sse_only");
    assert_eq!(preview[1]["secretNames"], json!(["FILES_TOKEN"]));
    assert_eq!(preview[2]["missingSecrets"], json!(["Authorization"]));
    let added: Vec<&str> = imported["added"]
        .as_array()
        .unwrap()
        .iter()
        .map(|server| server["name"].as_str().unwrap())
        .collect();
    assert_eq!(added, ["files", "remote"]);
    assert_eq!(imported["added"][1]["enabled"], false);
    assert_eq!(
        imported["skipped"],
        json!([{ "key": "events", "reason": "sse_only" }])
    );
    assert!(!imported.to_string().contains("planted-files-token"));
}

#[tokio::test]
async fn the_core_says_where_it_finds_a_program() {
    let secrets = Arc::new(FakeSecrets::new());
    let hosted = hosted(&secrets).await;

    let missing = hosted
        .invoke(
            "locate_program",
            json!({ "program": "mannie-no-such-program-for-tests" }),
        )
        .await
        .unwrap_err();

    assert_eq!(missing.code(), "mcp.program_not_found");
}
