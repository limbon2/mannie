//! Plugins' tools on the owner's SQLite files (ADR-0056, decisions 5 to 7):
//! a read runs in every conversation without asking, a write asks with its
//! statement and offers a grant of the same statement on the same database
//! with any values, a change of the schema asks despite a grant and offers
//! none, the tool writes through its call's ticket exactly what was allowed
//! and only for its own contributor, a database that the owner marked
//! read-only refuses writes before anything asks, a tool that reads cannot
//! declare a write, and a file that a program replaced after the request
//! ends the call. The owner's changes from the plugin's view are journaled
//! before they run and never run twice, a change of the schema runs only
//! once the owner confirmed it, an edit checks the values that the view
//! showed, and a change whose caller stops waiting stops and records how it
//! ended (decision 8).

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::HostProfile;
use mannie_plugin_sdk::contracts::DatabaseKey;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::host::PluginBinding;
use mannie_plugin_sdk::ids::{FeatureId, PluginId};
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, DatabaseAccess, PluginTool, Tool, ToolAccess, ToolCall, ToolExecution,
    ToolRefusal, ToolResult, ToolSpec, Tools,
};
use mannie_plugin_sdk::services::sqlite_files::{
    Change, ChooseRequest, ColumnValue, OwnerChange, ReadLimits, RowMatch, SqlValue, SqliteFiles,
    SqliteFilesApi, SqliteFilesError, Statement, StatementRows,
};
use mannie_runtime::testing::{
    Harness, HarnessOptions, SeenPart, Transcript, add_plugin, reply, tool_calls,
};
use mannie_runtime::{DatabaseChooser, DatabaseFilters};
use mannie_store::dbfiles::{self, OperationState};
use mannie_types::chat::StartConversation;
use mannie_types::ids::{ConversationId, DatabaseOperationId, PartId, RunId};
use mannie_types::tools::{
    Access, ApprovalChoice, ApprovalReason, ApprovalRequest, DatabaseHome, DecideApproval,
    Decision, GrantScope, StatementClass, ToolCallStatus, ToolOutcome,
};
use mannie_types::transcript::PartContent;
use serde_json::{Value, json};
use tempfile::TempDir;

/// The fake plugin's ID.
const PLUGIN: &str = "inventory";

/// The fake plugin's tool that reads its database.
const QUERY: &str = "inventory_query";

/// The fake plugin's tool that changes its database.
const CHANGE: &str = "inventory_change";

/// The owner's name for the database.
const NAME: &str = "Inventory";

/// The database's tables and rows when a test starts.
const SCHEMA: &str = "CREATE TABLE parts (name TEXT PRIMARY KEY, stock INTEGER NOT NULL);
                      INSERT INTO parts (name, stock) VALUES ('bolt', 12), ('nut', 40);";

const SELECT: &str = "SELECT stock FROM parts WHERE name = ?1";
const UPDATE: &str = "UPDATE parts SET stock = ?1 WHERE name = ?2";
const DELETE: &str = "DELETE FROM parts WHERE name = ?1";
const CREATE_INDEX: &str = "CREATE INDEX parts_stock ON parts (stock)";

/// The system's file dialog as a test answers it: the owner chooses one
/// file.
#[derive(Debug)]
struct ChosenFile(PathBuf);

impl DatabaseChooser for ChosenFile {
    fn choose(&self, _filters: DatabaseFilters) -> BoxFuture<'static, Option<PathBuf>> {
        let path = self.0.clone();
        async move { Some(path) }.boxed()
    }
}

/// The fake plugin's tools on its one database: a query, which reads, and a
/// change, which writes its statements through the call's ticket. A change
/// may write other values than its call was allowed, as a plugin that broke
/// the SDK's rules would.
#[derive(Debug)]
struct DatabaseTools {
    files: Arc<dyn SqliteFilesApi>,
    key: DatabaseKey,
    tamper: Mutex<bool>,
}

/// The fake plugin's two tools, as the registry holds them.
#[derive(Debug)]
struct QueryTool(Arc<DatabaseTools>);

#[derive(Debug)]
struct ChangeTool(Arc<DatabaseTools>);

impl DatabaseTools {
    fn contributions(tools: &Arc<Self>) -> Contributions {
        let query: Arc<dyn Tool> = Arc::new(QueryTool(Arc::clone(tools)));
        let change: Arc<dyn Tool> = Arc::new(ChangeTool(Arc::clone(tools)));
        Contributions::new()
            .add::<Tools>(
                PluginTool::new(query, AgentSelector::Conversations).with_database_reads(),
            )
            .add::<Tools>(PluginTool::new(change, AgentSelector::Conversations).with_side_effects())
    }

    fn access(&self, statement: Statement) -> ToolAccess {
        ToolAccess::Database(DatabaseAccess::new(self.key.clone(), statement))
    }
}

/// The statements of a call's arguments: `{ "statements": [{ "text",
/// "parameters" }] }`.
fn statements_of(arguments: &str) -> Result<Vec<Statement>, ToolRefusal> {
    let arguments: Value =
        serde_json::from_str(arguments).map_err(|error| ToolRefusal::new(error.to_string()))?;
    serde_json::from_value(arguments["statements"].clone())
        .map_err(|error| ToolRefusal::new(error.to_string()))
}

fn spec(name: &str, description: &str) -> ToolSpec {
    ToolSpec::new(
        name,
        description,
        json!({
            "additionalProperties": false,
            "properties": {
                "statements": { "type": "array", "items": { "type": "object" } },
            },
            "required": ["statements"],
            "type": "object",
        }),
    )
}

impl Tool for QueryTool {
    fn spec(&self) -> ToolSpec {
        spec(QUERY, "Reads the inventory.")
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let accesses = statements_of(&call.arguments).map(|statements| {
            statements
                .into_iter()
                .map(|statement| self.0.access(statement))
                .collect()
        });
        async move { accesses }.boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let tools = Arc::clone(&self.0);
        async move {
            let Ok(statements) = statements_of(&execution.arguments) else {
                return ToolResult::new("No statements.");
            };
            let mut outputs = Vec::new();
            for statement in statements {
                let rows = tools
                    .files
                    .read(tools.key.clone(), statement, ReadLimits::VIEW)
                    .await;
                outputs.push(match rows {
                    Ok(rows) => shown(&rows),
                    Err(error) => error.code().to_owned(),
                });
            }
            ToolResult::new(outputs.join("\n"))
        }
        .boxed()
    }
}

impl Tool for ChangeTool {
    fn spec(&self) -> ToolSpec {
        spec(CHANGE, "Changes the inventory.")
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let accesses = statements_of(&call.arguments).map(|statements| {
            statements
                .into_iter()
                .map(|statement| self.0.access(statement))
                .collect()
        });
        async move { accesses }.boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let tools = Arc::clone(&self.0);
        async move {
            let Ok(mut statements) = statements_of(&execution.arguments) else {
                return ToolResult::new("No statements.");
            };
            let Some(ticket) = execution.database else {
                return ToolResult::new("No ticket.");
            };
            if *tools.tamper.lock().unwrap()
                && let Some(first) = statements.first_mut()
            {
                first.parameters = vec![SqlValue::integer(0), SqlValue::text("bolt")];
            }
            match tools
                .files
                .write(ticket, tools.key.clone(), statements)
                .await
            {
                Ok(outcomes) => {
                    let changes: u64 = outcomes.iter().map(|outcome| outcome.changes).sum();
                    ToolResult::new(format!("Changed {changes} rows."))
                }
                Err(error) => ToolResult::new(error.code()),
            }
        }
        .boxed()
    }
}

/// Rows as the fake tool shows them: one line of values for each row.
fn shown(rows: &StatementRows) -> String {
    rows.rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|value| value.display(40))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The manifest of the fake plugin, which declares the service.
fn manifest() -> Manifest {
    serde_json::from_value(json!({
        "id": PLUGIN,
        "sdk": 1,
        "order": 40,
        "rust": { "entry": "mannie-inventory" },
        "services": { "sqlite-files": 1 },
    }))
    .unwrap()
}

/// Binds the runtime's service `sqlite-files` to the plugin, or to one of
/// its features, as the host does when it starts them, with a dialog in
/// which the owner chooses `path`.
fn bind(harness: &Harness, path: &Path, feature: Option<&str>) -> Arc<dyn SqliteFilesApi> {
    let id = PluginId::parse(PLUGIN).unwrap();
    let manifest = manifest();
    let feature = feature.map(|feature| FeatureId::parse(feature).unwrap());
    let mut binding = PluginBinding::new(&id, &manifest, HostProfile::Development);
    if let Some(feature) = &feature {
        binding = binding.for_feature(feature);
    }
    harness
        .runtime
        .sqlite_files_provider(Some(Arc::new(ChosenFile(path.to_path_buf()))))
        .bind(&binding)
        .unwrap()
        .get::<SqliteFiles>()
        .expect("the bound service is sqlite-files")
}

/// A harness with the fake plugin and its registered database.
struct Setup {
    harness: Harness,
    _folder: TempDir,
    path: PathBuf,
    tools: Arc<DatabaseTools>,
}

impl Setup {
    async fn new() -> Self {
        Self::with_writer(None).await
    }

    /// The setup, with the change tool writing through the service bound to
    /// `feature` of the plugin, or to the plugin itself.
    async fn with_writer(feature: Option<&str>) -> Self {
        let harness = Harness::with_options(HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        })
        .await;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("inventory.db");
        dbfiles::create_for_tests(&path, SCHEMA).unwrap();
        let files = bind(&harness, &path, feature);
        let file = files
            .choose(ChooseRequest {
                databases_filter: "SQLite databases".to_owned(),
                all_files_filter: "All files".to_owned(),
                name: Some(NAME.to_owned()),
                read_only: false,
            })
            .await
            .unwrap()
            .expect("the owner chose the file");
        let tools = Arc::new(DatabaseTools {
            files,
            key: file.key,
            tamper: Mutex::new(false),
        });
        add_plugin(
            &harness.runtime,
            PLUGIN,
            DatabaseTools::contributions(&tools),
        )
        .unwrap();
        Self {
            harness,
            _folder: folder,
            path,
            tools,
        }
    }

    async fn start(&self, text: &str) -> (ConversationId, RunId) {
        let started = self
            .harness
            .runtime
            .start_conversation(StartConversation::new(text))
            .await
            .unwrap();
        (started.conversation_id, started.run_id)
    }

    /// Queues a call of the fake plugin's `tool` with these statements.
    fn push_call(&self, id: &str, tool: &str, statements: &[(&str, Vec<SqlValue>)]) {
        let statements: Vec<Statement> = statements
            .iter()
            .map(|(text, parameters)| Statement::new(*text, parameters.clone()))
            .collect();
        let arguments = json!({ "activity": "Working", "statements": statements }).to_string();
        self.harness
            .model
            .push_reply(tool_calls(&[(id, tool, &arguments)]));
    }

    /// The stock of each part, read through the service.
    async fn stock(&self) -> String {
        let rows = self
            .tools
            .files
            .read(
                self.tools.key.clone(),
                Statement::new("SELECT name, stock FROM parts ORDER BY name", Vec::new()),
                ReadLimits::VIEW,
            )
            .await
            .unwrap();
        shown(&rows)
    }

    /// The core's access of a statement on the database.
    async fn access(&self, text: &str, parameters: &[&str], class: StatementClass) -> Access {
        let files = self.tools.files.files().await.unwrap();
        let [file] = &files[..] else {
            panic!("one file is registered: {files:?}");
        };
        assert_eq!(file.key, self.tools.key);
        Access::Database {
            plugin: PLUGIN.to_owned(),
            key: self.tools.key.clone(),
            name: NAME.to_owned(),
            home: DatabaseHome::ThisComputer,
            identity: identity_of(self),
            statement: text.to_owned(),
            statement_cut: false,
            parameters: parameters.iter().map(|value| (*value).to_owned()).collect(),
            parameter_count: u32::try_from(parameters.len()).unwrap(),
            parameters_digest: String::new(),
            class,
        }
    }

    fn grant(&self, text: &str) -> GrantScope {
        GrantScope::Database {
            plugin: PLUGIN.to_owned(),
            key: self.tools.key.clone(),
            name: NAME.to_owned(),
            statement: text.to_owned(),
        }
    }
}

/// The identity of the registered file, as the core checks it.
fn identity_of(setup: &Setup) -> String {
    dbfiles::check_new(&setup.path, &[]).unwrap().identity
}

/// The access with the digest of its parameters left out, which the tests
/// compare apart.
fn without_digest(access: &Access) -> Access {
    let mut access = access.clone();
    if let Access::Database {
        parameters_digest, ..
    } = &mut access
    {
        parameters_digest.clear();
    }
    access
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

/// Waits until a call waits for the owner and returns its request.
async fn request_of(harness: &Harness, conversation: &ConversationId) -> (PartId, ApprovalRequest) {
    let transcript = harness
        .wait_until(conversation, |transcript| {
            waiting_call(transcript).is_some()
        })
        .await;
    waiting_call(&transcript).unwrap()
}

/// Answers the request of `part_id` with `choice`, and waits until that
/// request no longer waits, so the next answer finds the next request.
async fn decide(
    harness: &Harness,
    conversation: &ConversationId,
    (part_id, request): (PartId, ApprovalRequest),
    choice: ApprovalChoice,
) {
    harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id: part_id.clone(),
            digest: request.digest.clone(),
            choice,
        })
        .await
        .unwrap();
    harness
        .wait_until(conversation, |transcript| {
            waiting_call(transcript).is_none_or(|waiting| waiting.1.digest != request.digest)
        })
        .await;
}

/// Waits until a call waits for the owner and answers it with `choice`.
async fn answer(harness: &Harness, conversation: &ConversationId, choice: ApprovalChoice) {
    let waiting = request_of(harness, conversation).await;
    decide(harness, conversation, waiting, choice).await;
}

/// The failure code of a call that the core ended for its plugin.
fn failure_of(outcome: &ToolOutcome) -> Option<&str> {
    match outcome {
        ToolOutcome::Plugin { failure, .. } => failure.as_deref(),
        _other => None,
    }
}

#[tokio::test]
async fn a_read_runs_in_every_conversation_without_asking() {
    let setup = Setup::new().await;
    setup.push_call("call_1", QUERY, &[(SELECT, vec![SqlValue::text("bolt")])]);
    setup.harness.model.push_reply(reply("12 bolts."));

    let (conversation, run) = setup.start("OWNER: how many bolts?").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.decision, Some(Decision::RegisteredDatabase));
    let [access] = &call.accesses[..] else {
        panic!("one access: {:?}", call.accesses);
    };
    assert_eq!(
        without_digest(access),
        setup
            .access(SELECT, &["'bolt'"], StatementClass::Read)
            .await
    );
    assert_eq!(
        call.result.map(|result| result.output),
        Some("12".to_owned())
    );
}

#[tokio::test]
async fn a_write_asks_with_its_statement_and_its_values_and_runs_once_allowed() {
    let setup = Setup::new().await;
    setup.push_call(
        "call_1",
        CHANGE,
        &[(UPDATE, vec![SqlValue::integer(11), SqlValue::text("bolt")])],
    );
    setup.harness.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("OWNER: one bolt is gone").await;
    let (part, request) = request_of(&setup.harness, &conversation).await;
    decide(
        &setup.harness,
        &conversation,
        (part, request.clone()),
        ApprovalChoice::AllowOnce,
    )
    .await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(request.reasons, vec![ApprovalReason::DatabaseWrite]);
    assert_eq!(request.grants, vec![setup.grant(UPDATE)]);
    assert_eq!(request.rule, None);
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.decision, Some(Decision::AllowedOnce));
    let [access] = &call.accesses[..] else {
        panic!("one access: {:?}", call.accesses);
    };
    assert_eq!(
        without_digest(access),
        setup
            .access(UPDATE, &["11", "'bolt'"], StatementClass::Write)
            .await
    );
    assert_eq!(
        call.result.map(|result| result.output),
        Some("Changed 1 rows.".to_owned())
    );
    assert_eq!(setup.stock().await, "'bolt', 11; 'nut', 40");
}

#[tokio::test]
async fn a_grant_covers_the_same_statement_with_other_values_and_no_other_statement() {
    let setup = Setup::new().await;
    setup.push_call(
        "call_1",
        CHANGE,
        &[(UPDATE, vec![SqlValue::integer(11), SqlValue::text("bolt")])],
    );
    setup.push_call(
        "call_2",
        CHANGE,
        &[(UPDATE, vec![SqlValue::integer(39), SqlValue::text("nut")])],
    );
    setup.push_call("call_3", CHANGE, &[(DELETE, vec![SqlValue::text("nut")])]);
    setup.harness.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("OWNER: count again").await;
    answer(
        &setup.harness,
        &conversation,
        ApprovalChoice::AllowForConversation,
    )
    .await;
    answer(&setup.harness, &conversation, ApprovalChoice::Deny).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let decisions: Vec<_> = transcript
        .tool_calls(0)
        .iter()
        .map(|call| call.decision)
        .collect();
    assert_eq!(
        decisions,
        vec![
            Some(Decision::AllowedForConversation),
            Some(Decision::Grant),
            Some(Decision::Denied),
        ]
    );
    assert_eq!(setup.stock().await, "'bolt', 11; 'nut', 39");
}

#[tokio::test]
async fn a_change_of_the_schema_asks_despite_a_grant_and_offers_none() {
    let setup = Setup::new().await;
    setup.push_call(
        "call_1",
        CHANGE,
        &[(UPDATE, vec![SqlValue::integer(11), SqlValue::text("bolt")])],
    );
    setup.push_call("call_2", CHANGE, &[(CREATE_INDEX, Vec::new())]);
    setup.harness.model.push_reply(reply("Indexed."));

    let (conversation, run) = setup.start("OWNER: count and index").await;
    answer(
        &setup.harness,
        &conversation,
        ApprovalChoice::AllowForConversation,
    )
    .await;
    let (part, request) = request_of(&setup.harness, &conversation).await;
    decide(
        &setup.harness,
        &conversation,
        (part, request.clone()),
        ApprovalChoice::AllowOnce,
    )
    .await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(request.reasons, vec![ApprovalReason::DatabaseSchema]);
    assert_eq!(request.grants, Vec::new());
    assert_eq!(request.rule, None);
    let calls = transcript.tool_calls(0);
    assert_eq!(calls[1].decision, Some(Decision::AllowedOnce));
    let structure = setup
        .tools
        .files
        .structure(setup.tools.key.clone())
        .await
        .unwrap();
    assert!(
        structure
            .indexes
            .iter()
            .any(|index| index.name == "parts_stock"),
        "{structure:?}"
    );
}

#[tokio::test]
async fn a_tool_that_writes_other_values_than_were_allowed_is_refused() {
    let setup = Setup::new().await;
    *setup.tools.tamper.lock().unwrap() = true;
    setup.push_call(
        "call_1",
        CHANGE,
        &[(UPDATE, vec![SqlValue::integer(11), SqlValue::text("bolt")])],
    );
    setup.harness.model.push_reply(reply("It failed."));

    let (conversation, run) = setup.start("OWNER: one bolt is gone").await;
    answer(&setup.harness, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(
        call.result.map(|result| result.output),
        Some("sqlite_files.not_allowed".to_owned())
    );
    assert_eq!(setup.stock().await, "'bolt', 12; 'nut', 40");
}

#[tokio::test]
async fn a_ticket_writes_only_for_the_contributor_whose_tool_made_the_call() {
    let setup = Setup::with_writer(Some("stock")).await;
    setup.push_call(
        "call_1",
        CHANGE,
        &[(UPDATE, vec![SqlValue::integer(11), SqlValue::text("bolt")])],
    );
    setup.harness.model.push_reply(reply("It failed."));

    let (conversation, run) = setup.start("OWNER: one bolt is gone").await;
    answer(&setup.harness, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(
        call.result.map(|result| result.output),
        Some("sqlite_files.not_allowed".to_owned())
    );
    assert_eq!(setup.stock().await, "'bolt', 12; 'nut', 40");
}

#[tokio::test]
async fn a_database_marked_read_only_refuses_a_write_before_anything_asks() {
    let setup = Setup::new().await;
    setup
        .tools
        .files
        .mark(setup.tools.key.clone(), true)
        .await
        .unwrap();
    setup.push_call(
        "call_1",
        CHANGE,
        &[(UPDATE, vec![SqlValue::integer(11), SqlValue::text("bolt")])],
    );
    setup.harness.model.push_reply(reply("It is read-only."));

    let (conversation, run) = setup.start("OWNER: one bolt is gone").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.request, None);
    let result = call.result.expect("the call ended with a result");
    assert_eq!(failure_of(&result.outcome), Some("database_read_only"));
    assert_eq!(setup.stock().await, "'bolt', 12; 'nut', 40");
}

#[tokio::test]
async fn a_tool_that_reads_cannot_declare_a_write() {
    let setup = Setup::new().await;
    setup.push_call("call_1", QUERY, &[(DELETE, vec![SqlValue::text("bolt")])]);
    setup.harness.model.push_reply(reply("It failed."));

    let (conversation, run) = setup.start("OWNER: drop the bolts").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    let result = call.result.expect("the call ended with a result");
    assert_eq!(failure_of(&result.outcome), Some("database_not_a_read"));
    assert_eq!(setup.stock().await, "'bolt', 12; 'nut', 40");
}

#[tokio::test]
async fn a_file_that_a_program_replaced_after_the_request_ends_the_call() {
    let setup = Setup::new().await;
    setup.push_call(
        "call_1",
        CHANGE,
        &[(UPDATE, vec![SqlValue::integer(11), SqlValue::text("bolt")])],
    );
    setup.harness.model.push_reply(reply("It changed."));

    let (conversation, run) = setup.start("OWNER: one bolt is gone").await;
    let waiting = request_of(&setup.harness, &conversation).await;
    // A program saves by writing a copy and renaming it over the file, so
    // the copy exists beside the file and is another file on every system.
    let copy = setup.path.with_file_name("inventory-copy.db");
    dbfiles::create_for_tests(&copy, SCHEMA).unwrap();
    std::fs::rename(&copy, &setup.path).unwrap();
    decide(
        &setup.harness,
        &conversation,
        waiting,
        ApprovalChoice::AllowOnce,
    )
    .await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    let result = call.result.expect("the call ended with a result");
    assert_eq!(
        failure_of(&result.outcome),
        Some("database_statement_refused")
    );
    let files = setup.tools.files.files().await.unwrap();
    assert_eq!(
        files.first().map(|file| file.state),
        Some(mannie_plugin_sdk::services::sqlite_files::FileState::Changed)
    );
}

/// An owner's change from the plugin's view, with the ID of its operation.
fn owner_change(last: u8, change: Change) -> OwnerChange {
    OwnerChange {
        id: DatabaseOperationId::parse(&format!("0199e0f2-0000-7000-8000-0000000003{last:02x}"))
            .unwrap(),
        change,
    }
}

/// The statements of an owner's change, without a confirmed change of the
/// schema unless `confirmed`.
fn statements(statements: Vec<Statement>, confirmed: bool) -> Change {
    Change::Statements {
        statements,
        schema_confirmed: confirmed,
    }
}

/// The stock of a part as a column's value.
fn stock(value: i64) -> ColumnValue {
    ColumnValue {
        column: "stock".to_owned(),
        value: SqlValue::integer(value),
    }
}

#[tokio::test]
async fn an_owner_change_is_journaled_runs_once_and_changes_the_schema_once_confirmed() {
    let setup = Setup::new().await;
    let (files, key) = (&setup.tools.files, setup.tools.key.clone());
    let update = owner_change(
        1,
        statements(
            vec![Statement::new(
                UPDATE,
                vec![SqlValue::integer(11), SqlValue::text("bolt")],
            )],
            false,
        ),
    );
    let index = vec![Statement::new(CREATE_INDEX, Vec::new())];
    let unconfirmed = owner_change(2, statements(index.clone(), false));
    let query = owner_change(
        4,
        statements(
            vec![Statement::new(SELECT, vec![SqlValue::text("nut")])],
            false,
        ),
    );

    let changed = files.change(key.clone(), update.clone()).await;
    let again = files.change(key.clone(), update.clone()).await;
    let refused = files.change(key.clone(), unconfirmed.clone()).await;
    let confirmed = files
        .change(key.clone(), owner_change(3, statements(index, true)))
        .await;
    let read = files.change(key.clone(), query).await;
    files.mark(key.clone(), true).await.unwrap();
    let marked = files
        .change(
            key.clone(),
            owner_change(
                5,
                statements(
                    vec![Statement::new(DELETE, vec![SqlValue::text("nut")])],
                    false,
                ),
            ),
        )
        .await;

    assert_eq!(changed.map(|outcome| outcome.changes), Ok(1));
    assert_eq!(again, Err(SqliteFilesError::AlreadyRun));
    assert_eq!(refused, Err(SqliteFilesError::SchemaUnconfirmed));
    assert!(confirmed.is_ok(), "{confirmed:?}");
    assert_eq!(
        read,
        Err(SqliteFilesError::NotARead {
            class: StatementClass::Read
        })
    );
    assert_eq!(marked, Err(SqliteFilesError::MarkedReadOnly));
    let store = &setup.harness.store;
    assert_eq!(
        store.db_operation_state(&update.id).await.unwrap(),
        Some(OperationState::Completed)
    );
    // A change that its plan refused never reached the journal.
    assert_eq!(
        store.db_operation_state(&unconfirmed.id).await.unwrap(),
        None
    );
    assert_eq!(setup.stock().await, "'bolt', 11; 'nut', 40");
    let structure = files.structure(key).await.unwrap();
    assert!(
        structure
            .indexes
            .iter()
            .any(|index| index.name == "parts_stock"),
        "{structure:?}"
    );
}

#[tokio::test]
async fn an_owner_edit_checks_the_values_that_the_view_showed() {
    let setup = Setup::new().await;
    let (files, key) = (&setup.tools.files, setup.tools.key.clone());
    // The parts table has rowids, which name its rows: the bolt's is 1.
    let bolt = |expected: i64| RowMatch {
        key: vec![SqlValue::integer(1)],
        expected: vec![stock(expected)],
    };
    let stale = owner_change(
        7,
        Change::UpdateRow {
            table: "parts".to_owned(),
            row: bolt(12),
            set: vec![stock(10)],
        },
    );

    let edited = files
        .change(
            key.clone(),
            owner_change(
                6,
                Change::UpdateRow {
                    table: "parts".to_owned(),
                    row: bolt(12),
                    set: vec![stock(11)],
                },
            ),
        )
        .await;
    let changed_since = files.change(key.clone(), stale.clone()).await;
    let inserted = files
        .change(
            key.clone(),
            owner_change(
                8,
                Change::InsertRow {
                    table: "parts".to_owned(),
                    values: vec![
                        ColumnValue {
                            column: "name".to_owned(),
                            value: SqlValue::text("gear"),
                        },
                        stock(3),
                    ],
                },
            ),
        )
        .await;

    assert_eq!(edited.map(|outcome| outcome.changes), Ok(1));
    assert_eq!(changed_since, Err(SqliteFilesError::RowChanged));
    assert_eq!(inserted.map(|outcome| outcome.rowid), Ok(Some(3)));
    assert_eq!(
        setup
            .harness
            .store
            .db_operation_state(&stale.id)
            .await
            .unwrap(),
        Some(OperationState::Failed)
    );
    assert_eq!(setup.stock().await, "'bolt', 11; 'gear', 3; 'nut', 40");
}

#[tokio::test]
async fn an_owner_change_whose_caller_stops_waiting_stops_and_records_how_it_ended() {
    let setup = Setup::new().await;
    // Inserts millions of rows, which runs for far longer than the test
    // waits: only a stop ends it within the waits below.
    let slow = owner_change(
        9,
        statements(
            vec![Statement::new(
                "INSERT INTO parts (name, stock) SELECT 'part ' || x, x FROM \
                 (WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n \
                 WHERE x < 20000000) SELECT x FROM n)",
                Vec::new(),
            )],
            false,
        ),
    );
    let store = setup.harness.store.clone();
    let id = slow.id.clone();

    let running = tokio::spawn(setup.tools.files.change(setup.tools.key.clone(), slow));
    let started = tokio::time::timeout(Duration::from_secs(10), async {
        while store.db_operation_state(&id).await.unwrap() != Some(OperationState::Executing) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    running.abort();
    let ended = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = store.db_operation_state(&id).await.unwrap();
            if state != Some(OperationState::Executing) {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;

    started.unwrap();
    assert_eq!(ended.unwrap(), Some(OperationState::Failed));
    assert_eq!(setup.stock().await, "'bolt', 12; 'nut', 40");
}
