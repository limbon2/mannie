//! The feature Databases of Workers through the plugin host and its dispatch
//! (plugins/workers/docs/databases.md; ADR-0055, ADR-0056): it starts off
//! and turns on under Workers, the owner registers a file through the
//! system's file dialog with a name that no other database has, the view
//! reads structures, counts, pages and statements, stops a read and runs
//! the owner's changes once, agents read without asking and change after an
//! approval, the section lists the databases and "@" names them, and turning
//! the feature off removes its tools and ends its calls and channel.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::StreamExt;
use mannie_plugin_harness::host::{CallFailure, HostOptions, HostedPlugin};
use mannie_plugin_harness::runtime::{HarnessOptions, SeenPart, Transcript, reply, tool_calls};
use mannie_plugin_harness::store::dbfiles::{OperationState, create_for_tests};
use mannie_plugin_harness::types::{
    ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision, PartContent, PartId,
    PluginState, StartConversation, ToolCallStatus,
};
use mannie_plugin_sdk::contracts::{ConversationId, DatabaseOperationId, SubscriptionEnd};
use mannie_plugin_sdk::host::ServiceProvider;
use mannie_plugin_sdk::ipc::{ChannelMessage, HostRefusal, PluginStream};
use mannie_plugin_sdk::testing::FakeSecrets;
use mannie_workers::WORKERS_KIND;
use mannie_workers::databases::DATABASES;
use mannie_workers::databases::tools::{CHANGE_DATABASE, DESCRIBE_DATABASE, QUERY_DATABASE};
use serde_json::{Value, json};
use tempfile::TempDir;

/// The tables and rows of a test's database.
const SCHEMA: &str = "CREATE TABLE parts (name TEXT PRIMARY KEY, stock INTEGER NOT NULL);
                      INSERT INTO parts (name, stock) VALUES ('bolt', 12), ('nut', 40);";

/// Workers with its feature Databases on, on the desktop app's host, and a
/// folder for the test's databases.
struct Setup {
    plugin: HostedPlugin,
    folder: TempDir,
}

impl Setup {
    async fn new() -> Self {
        let secrets: Arc<dyn ServiceProvider> = Arc::new(FakeSecrets::new());
        let options = HostOptions::new()
            .with_core(HarnessOptions {
                use_tools: true,
                ..HarnessOptions::default()
            })
            .with_provider(secrets);
        let plugin = HostedPlugin::new(mannie_workers::mannie_plugin(), options).await;
        assert_eq!(plugin.turn_on().await.state, PluginState::On);
        let feature = plugin.turn_feature_on(DATABASES).await;
        assert_eq!(feature.state, PluginState::On, "{feature:?}");
        Self {
            plugin,
            folder: tempfile::tempdir().unwrap(),
        }
    }

    /// A SQLite file with the test's tables.
    fn database(&self, name: &str) -> PathBuf {
        let path = self.folder.path().join(name);
        create_for_tests(&path, SCHEMA).unwrap();
        path
    }

    async fn invoke(&self, command: &str, input: Value) -> Result<Value, CallFailure> {
        self.plugin.invoke(command, input).await
    }

    /// Registers `path` through the dialog, with the owner's name if any.
    async fn add(&self, path: &Path, name: Option<&str>) -> Result<Value, CallFailure> {
        self.plugin.choose_file(path);
        self.invoke(
            "add_database",
            json!({
                "name": name,
                "readOnly": false,
                "databasesFilter": "SQLite databases",
                "allFilesFilter": "All files",
            }),
        )
        .await
    }

    /// Registers `path` and returns its key.
    async fn added(&self, path: &Path) -> String {
        let added = self.add(path, Some("Inventory")).await.unwrap();
        added["key"].as_str().unwrap().to_owned()
    }

    async fn follow(&self) -> PluginStream<ChannelMessage<Value>> {
        self.plugin
            .open_channel("follow_databases", Value::Null)
            .await
            .unwrap()
    }

    async fn start(&self, text: &str) -> (ConversationId, mannie_plugin_harness::types::RunId) {
        let started = self
            .plugin
            .core
            .runtime
            .start_conversation(StartConversation::new(text))
            .await
            .unwrap();
        (started.conversation_id, started.run_id)
    }
}

/// The names of the databases in the next list of the channel.
async fn next_names(channel: &mut PluginStream<ChannelMessage<Value>>) -> Vec<String> {
    match channel.next().await {
        Some(ChannelMessage::Item { item }) => {
            assert_eq!(item["type"], "databases", "{item}");
            item["databases"]
                .as_array()
                .unwrap()
                .iter()
                .map(|database| database["name"].as_str().unwrap().to_owned())
                .collect()
        }
        other => panic!("not a list of databases: {other:?}"),
    }
}

/// The code of a command's failure, or of the service's error inside it.
fn failure_code(failure: &CallFailure) -> String {
    match failure {
        CallFailure::Plugin(value) => {
            let error = &value.error;
            let code = error["code"].as_str().unwrap_or_default();
            match code {
                "files" => format!(
                    "files.{}",
                    error["error"]["code"].as_str().unwrap_or_default()
                ),
                other => other.to_owned(),
            }
        }
        other @ CallFailure::Host(_) => panic!("not the plugin's failure: {other:?}"),
    }
}

/// The tool call of the turn that waits for the owner, if one does.
fn waiting_call(transcript: &Transcript) -> Option<(PartId, ApprovalRequest)> {
    transcript.parts(0).into_iter().find_map(|part| match part {
        SeenPart {
            id,
            content: PartContent::ToolCall { call },
            ..
        } if call.status == ToolCallStatus::AwaitingApproval => {
            call.request.clone().map(|request| (id, request))
        }
        _ => None,
    })
}

#[tokio::test]
async fn databases_starts_off_and_its_commands_say_so_until_the_owner_turns_it_on() {
    let secrets: Arc<dyn ServiceProvider> = Arc::new(FakeSecrets::new());
    let plugin = HostedPlugin::new(
        mannie_workers::mannie_plugin(),
        HostOptions::new().with_provider(secrets),
    )
    .await;
    plugin.turn_on().await;

    let off = plugin.feature_view(DATABASES);
    let refused = plugin
        .invoke("refresh_databases", Value::Null)
        .await
        .unwrap_err();

    assert_eq!(off.state, PluginState::Off);
    assert!(!off.enabled);
    assert_eq!(
        refused,
        CallFailure::Host(HostRefusal::feature_off(WORKERS_KIND, DATABASES))
    );
}

#[tokio::test]
async fn the_owner_registers_files_through_the_dialog_with_names_that_no_other_database_has() {
    let setup = Setup::new().await;
    let first = setup.database("inventory.db");
    let other = tempfile::tempdir().unwrap();
    let second = other.path().join("inventory.db");
    create_for_tests(&second, SCHEMA).unwrap();
    let mut channel = setup.follow().await;
    let empty = next_names(&mut channel).await;

    let added = setup.add(&first, None).await.unwrap();
    let after_first = next_names(&mut channel).await;
    let again = setup.add(&first, None).await.unwrap_err();
    let numbered = setup.add(&second, None).await.unwrap();
    let taken = setup
        .invoke(
            "rename_database",
            json!({ "key": numbered["key"], "name": "INVENTORY.DB" }),
        )
        .await
        .unwrap_err();
    // A name that another database has is refused before the dialog shows,
    // so no file is set for it.
    let typed_taken = setup
        .invoke(
            "add_database",
            json!({
                "name": "Inventory.db",
                "readOnly": false,
                "databasesFilter": "SQLite databases",
                "allFilesFilter": "All files",
            }),
        )
        .await
        .unwrap_err();
    let closed = setup
        .invoke(
            "add_database",
            json!({
                "readOnly": false,
                "databasesFilter": "SQLite databases",
                "allFilesFilter": "All files",
            }),
        )
        .await
        .unwrap();

    assert_eq!(empty, Vec::<String>::new());
    assert_eq!(added["name"], "inventory.db");
    assert_eq!(added["fileName"], "inventory.db");
    assert_eq!(added["state"], "available");
    assert_eq!(added["machine"], json!({ "kind": "this_computer" }));
    assert_eq!(after_first, ["inventory.db"]);
    assert_eq!(failure_code(&again), "files.already_registered");
    assert_eq!(numbered["name"], "inventory.db (2)");
    assert_eq!(failure_code(&taken), "name_taken");
    assert_eq!(failure_code(&typed_taken), "name_taken");
    assert_eq!(closed, Value::Null, "the owner closed the dialog");
    let filters = setup.plugin.file_dialog_filters().unwrap();
    assert_eq!(filters.databases, "SQLite databases");
    assert_eq!(filters.all_files, "All files");
}

#[tokio::test]
async fn the_view_reads_a_database_and_runs_the_owners_change_once() {
    let setup = Setup::new().await;
    let key = setup.added(&setup.database("inventory.db")).await;
    let operation = "0199e0f2-0000-7000-8000-000000000501";
    let update = json!({
        "key": key,
        "change": {
            "id": operation,
            "change": {
                "kind": "update_row",
                "table": "parts",
                "row": {
                    "key": [{ "type": "integer", "value": "1" }],
                    "expected": [{ "column": "stock", "value": { "type": "integer", "value": "12" } }]
                },
                "set": [{ "column": "stock", "value": { "type": "integer", "value": "11" } }]
            }
        }
    });

    let structure = setup
        .invoke("database_structure", json!({ "key": key }))
        .await
        .unwrap();
    let count = setup
        .invoke(
            "count_database_rows",
            json!({ "key": key, "table": "parts" }),
        )
        .await
        .unwrap();
    let page = setup
        .invoke(
            "database_page",
            json!({ "key": key, "request": { "table": "parts", "order": null, "after": null, "rows": 100 } }),
        )
        .await
        .unwrap();
    let class = setup
        .invoke(
            "classify_database_statement",
            json!({ "key": key, "text": "DELETE FROM parts" }),
        )
        .await
        .unwrap();
    let changed = setup
        .invoke("run_database_change", update.clone())
        .await
        .unwrap();
    let again = setup
        .invoke("run_database_change", update)
        .await
        .unwrap_err();
    let read = setup
        .invoke(
            "read_database",
            json!({
                "key": key,
                "statement": { "text": "SELECT stock FROM parts WHERE name = ?1", "parameters": [{ "type": "text", "value": "bolt" }] }
            }),
        )
        .await
        .unwrap();
    let write_as_read = setup
        .invoke(
            "read_database",
            json!({ "key": key, "statement": { "text": "DELETE FROM parts" } }),
        )
        .await
        .unwrap_err();

    assert_eq!(structure["tables"][0]["name"], "parts");
    assert_eq!(count, "2");
    assert_eq!(page["rows"].as_array().unwrap().len(), 2);
    assert_eq!(class, "write");
    assert_eq!(changed["changes"], "1");
    assert_eq!(failure_code(&again), "files.already_run");
    assert_eq!(
        read["rows"],
        json!([[{ "type": "integer", "value": "11" }]])
    );
    assert_eq!(failure_code(&write_as_read), "files.not_a_read");
    assert_eq!(
        setup
            .plugin
            .core
            .store
            .db_operation_state(&DatabaseOperationId::parse(operation).unwrap())
            .await
            .unwrap(),
        Some(OperationState::Completed)
    );
}

#[tokio::test]
async fn the_owner_stops_a_read_of_the_view() {
    let setup = Setup::new().await;
    let key = setup.added(&setup.database("inventory.db")).await;
    let endless = json!({
        "key": key,
        "statement": { "text": "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n) SELECT max(x) FROM n" },
        "run": "read-1",
    });
    let (core, host) = setup.plugin.into_parts();
    let reading_host = host.clone();
    let reading = tokio::spawn(async move {
        reading_host
            .invoke(WORKERS_KIND, "read_database", endless)
            .await
    });

    // The read runs until a stop finds it; a stop of an ID that runs nothing
    // changes nothing.
    let stopped = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            host.invoke(
                WORKERS_KIND,
                "stop_database_read",
                json!({ "run": "read-1" }),
            )
            .await
            .unwrap();
            if reading.is_finished() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;

    stopped.unwrap();
    let failure = reading.await.unwrap().unwrap_err();
    assert_eq!(failure_code(&failure), "stopped");
    drop(core);
}

#[tokio::test]
async fn an_agent_reads_without_asking_and_changes_after_the_owners_approval() {
    let setup = Setup::new().await;
    setup.added(&setup.database("inventory.db")).await;
    let model = &setup.plugin.core.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        QUERY_DATABASE,
        &json!({
            "activity": "Reading",
            "database": "inventory",
            "statement": "SELECT stock FROM parts WHERE name = ?1",
            "parameters": ["bolt"],
        })
        .to_string(),
    )]));
    model.push_reply(tool_calls(&[(
        "call_2",
        CHANGE_DATABASE,
        &json!({
            "activity": "Counting",
            "database": "Inventory",
            "statements": [{ "statement": "UPDATE parts SET stock = ?1 WHERE name = ?2", "parameters": [11, "bolt"] }],
        })
        .to_string(),
    )]));
    model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("OWNER: how many bolts, then one fewer").await;
    let harness = &setup.plugin.core;
    let transcript = harness
        .wait_until(&conversation, |transcript| {
            waiting_call(transcript).is_some()
        })
        .await;
    let (part, request) = waiting_call(&transcript).unwrap();
    harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id: part,
            digest: request.digest.clone(),
            choice: ApprovalChoice::AllowOnce,
        })
        .await
        .unwrap();
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let calls = transcript.tool_calls(0);
    assert_eq!(calls[0].decision, Some(Decision::RegisteredDatabase));
    assert_eq!(
        calls[0]
            .result
            .as_ref()
            .map(|result| result.output.as_str()),
        Some("1 row:\nstock\n12\n")
    );
    assert_eq!(request.reasons, vec![ApprovalReason::DatabaseWrite]);
    assert_eq!(calls[1].decision, Some(Decision::AllowedOnce));
    assert_eq!(
        calls[1]
            .result
            .as_ref()
            .map(|result| result.output.as_str()),
        Some("Statement 1: changed 1 row.\n")
    );
    // The section lists the database for the model.
    let instructions = &model.requests_for("OWNER: how many bolts")[0].instructions;
    assert!(
        instructions.contains("- Inventory (file inventory.db on this computer)"),
        "{instructions}"
    );
}

#[tokio::test]
async fn turning_databases_off_removes_its_tools_and_ends_its_channel_and_keeps_the_files() {
    let setup = Setup::new().await;
    setup.added(&setup.database("inventory.db")).await;
    let mut channel = setup.follow().await;
    let listed = next_names(&mut channel).await;
    let harness = &setup.plugin.core;
    harness.model.push_reply(reply("Hello."));
    let (conversation, run) = setup.start("ON: hello").await;
    harness.wait_for_run(&conversation, &run).await;

    let off = setup.plugin.turn_feature_off(DATABASES).await;
    let ended = channel.next().await;
    let refused = setup
        .invoke("refresh_databases", Value::Null)
        .await
        .unwrap_err();
    harness.model.push_reply(reply("Hello."));
    let (conversation, run) = setup.start("OFF: hello").await;
    harness.wait_for_run(&conversation, &run).await;
    let on_again = setup.plugin.turn_feature_on(DATABASES).await;
    let kept = next_names(&mut setup.follow().await).await;

    let tools = |marker: &str| -> Vec<String> {
        harness.model.requests_for(marker)[0]
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .filter(|name| name.contains("database"))
            .collect()
    };
    assert_eq!(listed, ["Inventory"]);
    assert_eq!(
        tools("ON: hello"),
        [DESCRIBE_DATABASE, QUERY_DATABASE, CHANGE_DATABASE]
    );
    assert_eq!(off.state, PluginState::Off);
    assert_eq!(
        ended,
        Some(ChannelMessage::Ended {
            reason: SubscriptionEnd::FeatureOff
        })
    );
    assert_eq!(
        refused,
        CallFailure::Host(HostRefusal::feature_off(WORKERS_KIND, DATABASES))
    );
    assert_eq!(tools("OFF: hello"), Vec::<String>::new());
    assert_eq!(on_again.state, PluginState::On);
    assert_eq!(kept, ["Inventory"], "the registrations stay");
}
