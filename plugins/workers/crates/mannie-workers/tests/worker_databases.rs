//! The feature Databases with a worker over the link (plugins/workers/docs/
//! databases.md; ADR-0057): the owner registers a file on a paired worker's
//! host by its path, which the worker checks under its ceiling; the view
//! reads its structure, counts, pages and statements through the link and
//! runs the owner's change once, journaled on both machines; agents read it
//! without asking, and a write asks with only Allow once on an ask host and
//! runs under a grant on an autonomous one; a worker whose configuration
//! does not allow databases refuses them; a worker whose version lacks the
//! package stays connected and needs an update; an offline worker's
//! databases stay listed and its requests fail; and removing the worker
//! forgets them.

#![cfg(test)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use mannie_plugin_harness::host::{CallFailure, HostOptions, HostedPlugin};
use mannie_plugin_harness::runtime::{HarnessOptions, SeenPart, Transcript, reply, tool_calls};
use mannie_plugin_harness::store::dbfiles::{OperationState, create_for_tests};
use mannie_plugin_harness::types::{
    ApprovalChoice, ApprovalRequest, DecideApproval, Decision, PartContent, PartId, PluginState,
    StartConversation, ToolCallStatus,
};
use mannie_plugin_sdk::contracts::{CeilingMode, ConversationId, DatabaseOperationId};
use mannie_plugin_sdk::host::ServiceProvider;
use mannie_plugin_sdk::ipc::{ChannelMessage, PluginStream};
use mannie_plugin_sdk::testing::FakeSecrets;
use mannie_workers::databases::DATABASES;
use mannie_workers::databases::tools::{CHANGE_DATABASE, QUERY_DATABASE};
use mannie_workers_testing::worker::{LinkedWorker, LinkedWorkerOptions, WorkerDatabases};
use serde_json::{Value, json};

/// The tables and rows of a test's database.
const SCHEMA: &str = "CREATE TABLE parts (name TEXT PRIMARY KEY, stock INTEGER NOT NULL);
                      INSERT INTO parts (name, stock) VALUES ('bolt', 12), ('nut', 40);";

const UPDATE: &str = "UPDATE parts SET stock = ?1 WHERE name = ?2";

/// Workers with its feature Databases on, paired with a worker on loopback.
struct Setup {
    plugin: HostedPlugin,
    worker: LinkedWorker,
    worker_id: String,
}

impl Setup {
    /// A worker whose configuration takes databases as `databases` says.
    async fn new(databases: Option<WorkerDatabases>) -> Self {
        Self::with_options(LinkedWorkerOptions {
            databases,
            ..LinkedWorkerOptions::default()
        })
        .await
    }

    /// A worker with these options.
    async fn with_options(options: LinkedWorkerOptions) -> Self {
        let worker = LinkedWorker::with_options(options).await;
        let secrets: Arc<dyn ServiceProvider> = Arc::new(FakeSecrets::new());
        let options = HostOptions::new()
            .with_core(HarnessOptions {
                use_tools: true,
                ..HarnessOptions::default()
            })
            .with_provider(secrets);
        let plugin = HostedPlugin::new(mannie_workers::mannie_plugin(), options).await;
        assert_eq!(plugin.turn_on().await.state, PluginState::On);
        let paired = plugin
            .invoke(
                "pair_worker",
                json!({ "address": worker.address, "code": worker.new_code() }),
            )
            .await
            .unwrap();
        let worker_id = paired["id"].as_str().unwrap().to_owned();
        let feature = plugin.turn_feature_on(DATABASES).await;
        assert_eq!(feature.state, PluginState::On, "{feature:?}");
        let setup = Self {
            plugin,
            worker,
            worker_id,
        };
        setup.wait_for_worker("connected").await;
        setup
    }

    /// A worker in `mode` that allows databases.
    async fn allowing(mode: CeilingMode) -> Self {
        Self::new(Some(WorkerDatabases {
            mode,
            accepts_master_full_access: true,
        }))
        .await
    }

    async fn invoke(&self, command: &str, input: Value) -> Result<Value, CallFailure> {
        self.plugin.invoke(command, input).await
    }

    /// Waits until the worker shows `status`.
    async fn wait_for_worker(&self, status: &str) {
        let mut workers = self
            .plugin
            .open_channel("subscribe_workers", Value::Null)
            .await
            .unwrap();
        let wait = async {
            while let Some(message) = workers.next().await {
                let ChannelMessage::Item { item } = message else {
                    continue;
                };
                let shown = item["workers"].as_array().is_some_and(|workers| {
                    workers.iter().any(|worker| {
                        worker["id"] == self.worker_id.as_str() && worker["status"] == status
                    })
                });
                if shown {
                    return;
                }
            }
            panic!("the workers' channel ended before the worker was {status}");
        };
        tokio::time::timeout(Duration::from_secs(30), wait)
            .await
            .unwrap_or_else(|_elapsed| panic!("the worker was not {status} within 30 seconds"));
    }

    /// A SQLite file with the test's tables in the worker's allowed folder.
    fn database(&self, name: &str) -> PathBuf {
        let path = self.worker.harness.allowed.join(name);
        create_for_tests(&path, SCHEMA).unwrap();
        path
    }

    /// Registers the file at `path` on the worker.
    async fn add(&self, path: &std::path::Path, name: &str) -> Result<Value, CallFailure> {
        self.invoke(
            "add_worker_database",
            json!({
                "worker": self.worker_id,
                "path": path.to_str().unwrap(),
                "name": name,
                "readOnly": false,
            }),
        )
        .await
    }

    /// Registers the test's database on the worker and returns its key.
    async fn added(&self) -> String {
        let path = self.database("parts.db");
        let added = self.add(&path, "Parts").await.unwrap();
        added["key"].as_str().unwrap().to_owned()
    }

    /// The bolts' stock, read through the view's command.
    async fn bolts(&self, key: &str) -> String {
        let rows = self
            .invoke(
                "read_database",
                json!({
                    "key": key,
                    "statement": {
                        "text": "SELECT stock FROM parts WHERE name = ?1",
                        "parameters": [{ "type": "text", "value": "bolt" }],
                    },
                }),
            )
            .await
            .unwrap();
        rows["rows"][0][0]["value"].as_str().unwrap().to_owned()
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

    /// Queues a call of `tool` with these arguments.
    fn push_call(&self, id: &str, tool: &str, arguments: &Value) {
        let arguments = arguments.to_string();
        self.plugin
            .core
            .model
            .push_reply(tool_calls(&[(id, tool, &arguments)]));
    }
}

/// The arguments of `change_database` that set the bolts' stock.
fn set_bolts(stock: i64) -> Value {
    json!({
        "activity": "Counting",
        "database": "Parts",
        "statements": [{ "statement": UPDATE, "parameters": [stock, "bolt"] }],
    })
}

/// The code of a command's failure, or of the service's error inside it.
fn failure_code(failure: &CallFailure) -> String {
    match failure {
        CallFailure::Plugin(value) => {
            let error = &value.error;
            match error["code"].as_str().unwrap_or_default() {
                "files" => format!(
                    "files.{}",
                    error["error"]["code"].as_str().unwrap_or_default()
                ),
                "host_refused" => format!(
                    "host_refused.{}",
                    error["refusal"].as_str().unwrap_or_default()
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

/// Waits until a call waits for the owner, and answers it with `choice`.
async fn answer(
    setup: &Setup,
    conversation: &ConversationId,
    choice: ApprovalChoice,
) -> ApprovalRequest {
    let transcript = setup
        .plugin
        .core
        .wait_until(conversation, |transcript| {
            waiting_call(transcript).is_some()
        })
        .await;
    let (part_id, request) = waiting_call(&transcript).unwrap();
    setup
        .plugin
        .core
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id,
            digest: request.digest.clone(),
            choice,
        })
        .await
        .unwrap();
    request
}

/// The databases of the next list of the channel.
async fn next_databases(channel: &mut PluginStream<ChannelMessage<Value>>) -> Vec<Value> {
    match channel.next().await {
        Some(ChannelMessage::Item { item }) => item["databases"].as_array().cloned().unwrap(),
        other => panic!("not a list of databases: {other:?}"),
    }
}

#[tokio::test]
async fn a_file_on_a_worker_registers_by_its_path_and_the_view_reads_it_through_the_link() {
    let setup = Setup::allowing(CeilingMode::Autonomous).await;
    let path = setup.database("parts.db");

    let added = setup.add(&path, "Parts").await.unwrap();

    assert_eq!(added["name"], "Parts");
    assert_eq!(added["machine"]["kind"], "worker");
    assert_eq!(added["machine"]["worker"], setup.worker_id.as_str());
    assert_eq!(added["reach"], "reachable");
    let key = added["key"].as_str().unwrap();
    let structure = setup
        .invoke("database_structure", json!({ "key": key }))
        .await
        .unwrap();
    assert_eq!(structure["tables"][0]["name"], "parts");
    let rows = setup
        .invoke(
            "count_database_rows",
            json!({ "key": key, "table": "parts" }),
        )
        .await
        .unwrap();
    assert_eq!(rows, "2");
    let page = setup
        .invoke(
            "database_page",
            json!({ "key": key, "request": { "table": "parts", "order": null, "after": null, "rows": 10 } }),
        )
        .await
        .unwrap();
    assert_eq!(page["rows"].as_array().unwrap().len(), 2);
    assert_eq!(setup.bolts(key).await, "12");
    let class = setup
        .invoke(
            "classify_database_statement",
            json!({ "key": key, "text": "CREATE INDEX parts_stock ON parts (stock)" }),
        )
        .await
        .unwrap();
    assert_eq!(class, "schema");
}

#[tokio::test]
async fn the_worker_refuses_a_path_outside_its_allowed_folders_and_a_host_without_databases() {
    let setup = Setup::allowing(CeilingMode::Autonomous).await;
    let elsewhere = setup.worker.harness.outside.join("parts.db");
    create_for_tests(&elsewhere, SCHEMA).unwrap();

    let outside = setup.add(&elsewhere, "Parts").await.unwrap_err();

    assert_eq!(
        failure_code(&outside),
        "host_refused.outside_allowed_folders"
    );

    let without = Setup::new(None).await;
    let path = without.database("parts.db");
    let refused = without.add(&path, "Parts").await.unwrap_err();
    assert_eq!(failure_code(&refused), "host_refused.no_databases_on_host");
}

#[tokio::test]
async fn a_worker_whose_version_lacks_the_package_stays_connected_and_needs_an_update() {
    let mut levels = mannie_link::levels::levels();
    levels.remove(mannie_link::levels::DATABASES);
    let setup = Setup::with_options(LinkedWorkerOptions {
        levels,
        databases: Some(WorkerDatabases {
            mode: CeilingMode::Autonomous,
            accepts_master_full_access: true,
        }),
        ..LinkedWorkerOptions::default()
    })
    .await;
    let path = setup.database("parts.db");

    let refused = setup.add(&path, "Parts").await.unwrap_err();

    assert_eq!(failure_code(&refused), "worker_needs_update");
}

#[tokio::test]
async fn the_owners_change_on_a_workers_database_runs_once_and_both_machines_journal_it() {
    let setup = Setup::allowing(CeilingMode::Ask).await;
    let key = setup.added().await;
    let id = "0199e0f2-0000-7000-8000-000000000401";
    let change = json!({
        "key": key,
        "change": {
            "id": id,
            "change": {
                "kind": "update_row",
                "table": "parts",
                "row": {
                    "key": [{ "type": "integer", "value": "1" }],
                    "expected": [{ "column": "stock", "value": { "type": "integer", "value": "12" } }],
                },
                "set": [{ "column": "stock", "value": { "type": "integer", "value": "11" } }],
            },
        },
    });

    let done = setup
        .invoke("run_database_change", change.clone())
        .await
        .unwrap();
    let again = setup
        .invoke("run_database_change", change)
        .await
        .unwrap_err();

    assert_eq!(done["changes"], "1");
    assert_eq!(failure_code(&again), "files.already_run");
    assert_eq!(setup.bolts(&key).await, "11");
    let operation = DatabaseOperationId::parse(id).unwrap();
    assert_eq!(
        setup
            .plugin
            .core
            .store
            .db_operation_state(&operation)
            .await
            .unwrap(),
        Some(OperationState::Completed),
        "the master's journal"
    );
    assert_eq!(
        setup
            .worker
            .harness
            .store
            .db_operation_state(&operation)
            .await
            .unwrap(),
        Some(OperationState::Completed),
        "the worker's journal"
    );
}

#[tokio::test]
async fn an_agent_reads_without_asking_and_an_ask_hosts_write_asks_with_only_allow_once() {
    let setup = Setup::allowing(CeilingMode::Ask).await;
    let key = setup.added().await;
    setup.push_call(
        "call_1",
        QUERY_DATABASE,
        &json!({
            "activity": "Reading",
            "database": "Parts",
            "statement": "SELECT stock FROM parts WHERE name = ?1",
            "parameters": ["bolt"],
        }),
    );
    setup.push_call("call_2", CHANGE_DATABASE, &set_bolts(10));
    setup.plugin.core.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("OWNER: count the bolts on the build box").await;
    let request = answer(&setup, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = setup.plugin.core.wait_for_run(&conversation, &run).await;

    assert_eq!(
        request.grants,
        Vec::new(),
        "an ask host takes only Allow once"
    );
    let calls = transcript.tool_calls(0);
    assert_eq!(calls[0].decision, Some(Decision::RegisteredDatabase));
    assert_eq!(calls[0].status, ToolCallStatus::Completed);
    assert_eq!(calls[1].decision, Some(Decision::AllowedOnce));
    assert_eq!(calls[1].status, ToolCallStatus::Completed);
    assert_eq!(setup.bolts(&key).await, "10");
}

#[tokio::test]
async fn an_autonomous_hosts_write_runs_under_a_grant_of_the_master() {
    let setup = Setup::allowing(CeilingMode::Autonomous).await;
    let key = setup.added().await;
    setup.push_call("call_1", CHANGE_DATABASE, &set_bolts(11));
    setup.push_call("call_2", CHANGE_DATABASE, &set_bolts(9));
    setup.plugin.core.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("OWNER: count the bolts twice").await;
    answer(&setup, &conversation, ApprovalChoice::AllowForConversation).await;
    let transcript = setup.plugin.core.wait_for_run(&conversation, &run).await;

    let decisions: Vec<_> = transcript
        .tool_calls(0)
        .iter()
        .map(|call| call.decision)
        .collect();
    assert_eq!(
        decisions,
        vec![
            Some(Decision::AllowedForConversation),
            Some(Decision::Grant)
        ]
    );
    assert_eq!(setup.bolts(&key).await, "9");
}

#[tokio::test]
async fn an_offline_workers_databases_stay_listed_and_removing_the_worker_forgets_them() {
    let mut setup = Setup::allowing(CeilingMode::Autonomous).await;
    let key = setup.added().await;
    let mut list = setup
        .plugin
        .open_channel("follow_databases", Value::Null)
        .await
        .unwrap();
    assert_eq!(next_databases(&mut list).await[0]["reach"], "reachable");

    setup.worker.stop().await;
    setup.wait_for_worker("offline").await;

    let offline = setup
        .invoke("database_structure", json!({ "key": key }))
        .await
        .unwrap_err();
    assert_eq!(failure_code(&offline), "worker_offline");
    let mut shown = next_databases(&mut list).await;
    while shown[0]["reach"] != "worker_offline" {
        shown = next_databases(&mut list).await;
    }
    assert_eq!(shown[0]["name"], "Parts");

    setup
        .invoke("remove_worker", json!({ "workerId": setup.worker_id }))
        .await
        .unwrap();
    let mut remaining = next_databases(&mut list).await;
    while !remaining.is_empty() {
        remaining = next_databases(&mut list).await;
    }
}
