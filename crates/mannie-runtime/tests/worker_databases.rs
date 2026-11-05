//! Plugins' tools on the databases of a worker's host, as the master's core
//! sees them (ADR-0057, decisions 6 to 8): a statement names what the worker
//! reported of its database, the core classifies it against the copy of the
//! schema, a read runs without asking, and the master's policy narrows a
//! write to the host: a read-only host's write ends before anything asks,
//! an ask host's asks with only Allow once, an autonomous host's takes a
//! grant, and full access counts only where the host takes it. The tool
//! gets the evidence of exactly what was allowed, once, with what allowed
//! it, and writes nothing on this computer with its ticket; a call whose
//! plugin lost the answer is uncertain. The owner's change on a worker's
//! database is checked as the worker checks it and journaled before the
//! plugin sends it, and one that the plugin drops becomes uncertain.

#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::HostProfile;
use mannie_plugin_sdk::contracts::{
    CeilingMode, DatabaseKey, DatabaseOperationId, WorkerId, WriteAuthority, WriteOrigin,
};
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::host::PluginBinding;
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, DatabaseAccess, PluginTool, Tool, ToolAccess, ToolCall, ToolExecution,
    ToolRefusal, ToolResult, ToolSpec, Tools,
};
use mannie_plugin_sdk::services::sqlite_files::{
    Change, ColumnValue, DatabaseSchema, EvidencedWrite, MAX_SCHEMA_STATEMENTS, OwnerChange,
    RemoteOutcome, RowMatch, SqlValue, SqliteFiles, SqliteFilesApi, SqliteFilesError, Statement,
    WorkerDatabase, WorkerHost, WriteEvidence,
};
use mannie_provider::{Content, ModelRequest, Role};
use mannie_runtime::testing::{Harness, HarnessOptions, Transcript, add_plugin, reply, tool_calls};
use mannie_store::dbfiles::OperationState;
use mannie_tools::{WrittenStatement, write_digest};
use mannie_types::chat::{SendMessage, StartConversation};
use mannie_types::ids::{ConversationId, PartId, RunId};
use mannie_types::tools::{
    Access, ApprovalChoice, ApprovalReason, DatabaseHome, DecideApproval, Decision, GrantScope,
    Interruption, SetFullAccess, StatementClass, ToolCallStatus, ToolCallView, ToolOutcome,
    ToolsError,
};
use mannie_types::transcript::PartContent;
use serde_json::{Value, json};

/// The fake plugin's ID.
const PLUGIN: &str = "fleet";

/// The fake plugin's tool that reads a worker's database.
const QUERY: &str = "fleet_query";

/// The fake plugin's tool that changes a worker's database.
const CHANGE: &str = "fleet_change";

/// The worker, its name and the owner's name for its database.
const WORKER: &str = "0199e0f2-0000-7000-8000-0000000000aa";
const WORKER_NAME: &str = "build-box";
const NAME: &str = "Parts";

/// The database's key, as the plugin keeps it, and the file's identity on
/// the worker's host.
const KEY: &str = "0199e0f2-0000-7000-8000-0000000000ee";
const IDENTITY: &str = "2049:131";

/// The tables of the worker's database, as the worker reported them.
const SCHEMA: &str = "CREATE TABLE parts (name TEXT PRIMARY KEY, stock INTEGER NOT NULL)";

const SELECT: &str = "SELECT stock FROM parts WHERE name = ?1";
const UPDATE: &str = "UPDATE parts SET stock = ?1 WHERE name = ?2";
const CREATE_INDEX: &str = "CREATE INDEX parts_stock ON parts (stock)";

/// What the fake change tool does with its call's ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Behavior {
    /// Asks for the evidence of its statements, and for it a second time.
    Evidence,
    /// Asks for the evidence of other values than its call was allowed.
    TamperedEvidence,
    /// Writes its statements on this computer.
    WriteHere,
    /// Asks for the evidence, then says that the worker's answer was lost.
    LoseAnswer,
}

/// The fake plugin's tools on its worker's database. A test sets what the
/// worker reported and what the change tool does, and reads the evidence
/// that the core gave.
#[derive(Debug)]
struct FleetTools {
    files: Arc<dyn SqliteFilesApi>,
    database: Mutex<WorkerDatabase>,
    behavior: Mutex<Behavior>,
    /// Whether the query names the database on this computer too.
    named_here: Mutex<bool>,
    answers: Mutex<Vec<Result<WriteEvidence, SqliteFilesError>>>,
}

#[derive(Debug)]
struct QueryTool(Arc<FleetTools>);

#[derive(Debug)]
struct ChangeTool(Arc<FleetTools>);

fn key() -> DatabaseKey {
    DatabaseKey::parse(KEY).unwrap()
}

fn worker() -> WorkerId {
    WorkerId::parse(WORKER).unwrap()
}

/// The database as a worker in `mode` reports it.
fn reported(mode: CeilingMode, accepts_full_access: bool) -> WorkerDatabase {
    WorkerDatabase::new(
        WorkerHost::new(worker(), WORKER_NAME, mode, accepts_full_access),
        NAME,
        IDENTITY,
        false,
        DatabaseSchema::new(vec![SCHEMA.to_owned()]),
    )
}

/// The home that the core's access names for a worker in `mode`.
fn home(mode: CeilingMode, accepts_full_access: bool) -> DatabaseHome {
    DatabaseHome::Worker {
        worker: worker(),
        name: WORKER_NAME.to_owned(),
        mode,
        accepts_full_access,
    }
}

impl FleetTools {
    fn contributions(tools: &Arc<Self>) -> Contributions {
        let query: Arc<dyn Tool> = Arc::new(QueryTool(Arc::clone(tools)));
        let change: Arc<dyn Tool> = Arc::new(ChangeTool(Arc::clone(tools)));
        Contributions::new()
            .add::<Tools>(
                PluginTool::new(query, AgentSelector::Conversations).with_database_reads(),
            )
            .add::<Tools>(PluginTool::new(change, AgentSelector::Conversations).with_side_effects())
    }

    fn accesses(&self, arguments: &str) -> Result<Vec<ToolAccess>, ToolRefusal> {
        let database = self.database.lock().unwrap().clone();
        let mut accesses: Vec<ToolAccess> = statements_of(arguments)?
            .into_iter()
            .map(|statement| {
                ToolAccess::Database(
                    DatabaseAccess::new(key(), statement).on_worker(database.clone()),
                )
            })
            .collect();
        if *self.named_here.lock().unwrap() {
            accesses.push(ToolAccess::Database(DatabaseAccess::new(
                key(),
                Statement::new(SELECT, vec![SqlValue::text("nut")]),
            )));
        }
        Ok(accesses)
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
        spec(QUERY, "Reads a worker's database.")
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let accesses = self.0.accesses(&call.arguments);
        async move { accesses }.boxed()
    }

    fn execute(&self, _execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        // The worker runs a read; the master's core takes no part in it.
        let lost = *self.0.behavior.lock().unwrap() == Behavior::LoseAnswer;
        async move {
            if lost {
                ToolResult::new("The link to the worker dropped before its answer.")
                    .with_lost_answer()
            } else {
                ToolResult::new("12")
            }
        }
        .boxed()
    }
}

impl Tool for ChangeTool {
    fn spec(&self) -> ToolSpec {
        spec(CHANGE, "Changes a worker's database.")
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let accesses = self.0.accesses(&call.arguments);
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
            let behavior = *tools.behavior.lock().unwrap();
            if behavior == Behavior::WriteHere {
                let written = tools.files.write(ticket, key(), statements).await;
                let answer = written.map_or_else(|error| error.code(), |_written| "written");
                return ToolResult::new(answer);
            }
            if behavior == Behavior::TamperedEvidence
                && let Some(first) = statements.first_mut()
            {
                first.parameters = vec![SqlValue::integer(0), SqlValue::text("bolt")];
            }
            let first = tools
                .files
                .evidence(ticket.clone(), key(), statements.clone())
                .await;
            tools.answers.lock().unwrap().push(first);
            match behavior {
                Behavior::Evidence => {
                    let again = tools.files.evidence(ticket, key(), statements).await;
                    tools.answers.lock().unwrap().push(again);
                    ToolResult::new("Sent to the worker.")
                }
                Behavior::LoseAnswer => ToolResult::new(
                    "The link to the worker dropped before its answer, so the write may have run.",
                )
                .with_failure("worker_answer_lost")
                .with_lost_answer(),
                Behavior::TamperedEvidence | Behavior::WriteHere => ToolResult::new("Done."),
            }
        }
        .boxed()
    }
}

/// The manifest of the fake plugin, which declares the service.
fn manifest() -> Manifest {
    serde_json::from_value(json!({
        "id": PLUGIN,
        "sdk": 1,
        "order": 40,
        "rust": { "entry": "mannie-fleet" },
        "services": { "sqlite-files": 1 },
    }))
    .unwrap()
}

/// A harness with the fake plugin, whose worker reported its database as
/// a worker in `mode` reports it.
struct Setup {
    harness: Harness,
    tools: Arc<FleetTools>,
}

impl Setup {
    async fn new(mode: CeilingMode, accepts_full_access: bool) -> Self {
        let harness = Harness::with_options(HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        })
        .await;
        let id = PluginId::parse(PLUGIN).unwrap();
        let manifest = manifest();
        let binding = PluginBinding::new(&id, &manifest, HostProfile::Development);
        let files = harness
            .runtime
            .sqlite_files_provider(None)
            .bind(&binding)
            .unwrap()
            .get::<SqliteFiles>()
            .expect("the bound service is sqlite-files");
        let tools = Arc::new(FleetTools {
            files,
            database: Mutex::new(reported(mode, accepts_full_access)),
            behavior: Mutex::new(Behavior::Evidence),
            named_here: Mutex::new(false),
            answers: Mutex::new(Vec::new()),
        });
        add_plugin(&harness.runtime, PLUGIN, FleetTools::contributions(&tools)).unwrap();
        Self { harness, tools }
    }

    fn behave(&self, behavior: Behavior) {
        *self.tools.behavior.lock().unwrap() = behavior;
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

    /// Starts a conversation whose first reply runs nothing, and switches
    /// it to full access.
    async fn start_in_full_access(&self) -> ConversationId {
        self.harness.model.push_reply(reply("Ready."));
        let (conversation, run) = self.start("OWNER: get ready").await;
        self.harness.wait_for_run(&conversation, &run).await;
        self.harness
            .runtime
            .set_full_access(SetFullAccess {
                conversation_id: conversation.clone(),
                enabled: true,
            })
            .await
            .unwrap();
        conversation
    }

    async fn send(&self, conversation: &ConversationId, text: &str) -> RunId {
        self.harness
            .runtime
            .send_message(SendMessage::new(conversation.clone(), text))
            .await
            .unwrap()
            .run_id
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

    /// The evidence that the core gave the change tool, in order.
    fn answers(&self) -> Vec<Result<WriteEvidence, SqliteFilesError>> {
        self.tools.answers.lock().unwrap().clone()
    }

    fn grant(text: &str) -> GrantScope {
        GrantScope::Database {
            plugin: PLUGIN.to_owned(),
            key: key(),
            name: NAME.to_owned(),
            statement: text.to_owned(),
        }
    }
}

fn update(stock: i64) -> Vec<SqlValue> {
    vec![SqlValue::integer(stock), SqlValue::text("bolt")]
}

/// The tool call of the first two turns that waits for the owner, if one
/// does.
fn waiting_call(transcript: &Transcript) -> Option<(PartId, Box<ToolCallView>)> {
    (0..2)
        .flat_map(|position| transcript.parts(position))
        .find_map(|part| match part.content {
            PartContent::ToolCall { call } if call.status == ToolCallStatus::AwaitingApproval => {
                Some((part.id, call))
            }
            _ => None,
        })
}

/// Waits until a call waits for the owner, and returns its part and its
/// document.
async fn waiting(harness: &Harness, conversation: &ConversationId) -> (PartId, Box<ToolCallView>) {
    let transcript = harness
        .wait_until(conversation, |transcript| {
            waiting_call(transcript).is_some()
        })
        .await;
    waiting_call(&transcript).unwrap()
}

/// Answers the call that waits with `choice`.
async fn decide(
    harness: &Harness,
    conversation: &ConversationId,
    choice: ApprovalChoice,
) -> Result<(), ToolsError> {
    let (part_id, call) = waiting(harness, conversation).await;
    let request = call.request.expect("a waiting call has a request");
    harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id,
            digest: request.digest,
            choice,
        })
        .await
}

/// The failure code of a call that the core ended for its plugin.
fn failure_of(outcome: &ToolOutcome) -> Option<&str> {
    match outcome {
        ToolOutcome::Plugin { failure, .. } => failure.as_deref(),
        _other => None,
    }
}

/// The digest of the parameters that an access covers.
fn parameters_digest_of(access: &Access) -> String {
    let Access::Database {
        parameters_digest, ..
    } = access
    else {
        panic!("a database access: {access:?}");
    };
    parameters_digest.clone()
}

/// The results that a request gives the model, by their calls.
fn result(request: &ModelRequest, call_id: &str) -> String {
    request
        .messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .flat_map(|message| &message.content)
        .find_map(|content| match content {
            Content::ToolResult {
                call_id: id,
                output,
                ..
            } if id == call_id => Some(output.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no result for {call_id}"))
}

#[tokio::test]
async fn a_read_of_a_workers_database_runs_without_asking_and_names_its_host() {
    let setup = Setup::new(CeilingMode::Ask, false).await;
    setup.push_call("call_1", QUERY, &[(SELECT, vec![SqlValue::text("bolt")])]);
    setup.harness.model.push_reply(reply("12 bolts."));

    let (conversation, run) = setup.start("OWNER: how many bolts on the build box?").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.decision, Some(Decision::RegisteredDatabase));
    let [
        Access::Database {
            plugin,
            key: named,
            name,
            home: named_home,
            identity,
            class,
            ..
        },
    ] = &call.accesses[..]
    else {
        panic!("one database access: {:?}", call.accesses);
    };
    assert_eq!(plugin, PLUGIN);
    assert_eq!(*named, key());
    assert_eq!(name, NAME);
    assert_eq!(*named_home, home(CeilingMode::Ask, false));
    assert_eq!(identity, IDENTITY);
    assert_eq!(*class, StatementClass::Read);
}

#[tokio::test]
async fn an_ask_hosts_write_offers_only_allow_once_and_its_evidence_names_the_approved_write() {
    let setup = Setup::new(CeilingMode::Ask, true).await;
    setup.push_call("call_1", CHANGE, &[(UPDATE, update(11))]);
    setup.harness.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("OWNER: one bolt is gone").await;
    let (part, call) = waiting(&setup.harness, &conversation).await;
    let request = call.request.clone().unwrap();
    assert_eq!(request.reasons, vec![ApprovalReason::DatabaseWrite]);
    assert_eq!(request.grants, Vec::new());
    assert_eq!(request.rule, None);
    assert_eq!(
        decide(
            &setup.harness,
            &conversation,
            ApprovalChoice::AllowForConversation
        )
        .await,
        Err(ToolsError::GrantNotPossible)
    );
    decide(&setup.harness, &conversation, ApprovalChoice::AllowOnce)
        .await
        .unwrap();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [done] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(done.decision, Some(Decision::AllowedOnce));
    let parameters_digest = parameters_digest_of(&done.accesses[0]);
    let answers = setup.answers();
    let [Ok(evidence), again] = &answers[..] else {
        panic!("two answers: {answers:?}");
    };
    assert_eq!(evidence.key, key());
    assert_eq!(evidence.identity, IDENTITY);
    assert_eq!(
        evidence.write,
        EvidencedWrite::Statements {
            statements: vec![Statement::new(UPDATE, update(11))]
        }
    );
    assert_eq!(
        evidence.origin,
        WriteOrigin::Call {
            conversation: conversation.clone(),
            part
        }
    );
    assert_eq!(
        evidence.authority,
        WriteAuthority::AllowedOnce {
            digest: write_digest(
                &key(),
                IDENTITY,
                &[WrittenStatement {
                    text: UPDATE,
                    parameters_digest: &parameters_digest,
                    class: StatementClass::Write,
                }]
            )
        }
    );
    // The ticket gives the evidence of a database's writes once.
    assert_eq!(*again, Err(SqliteFilesError::NotAllowed));
}

#[tokio::test]
async fn an_autonomous_hosts_write_runs_under_a_grant_whose_evidence_names_it() {
    let setup = Setup::new(CeilingMode::Autonomous, false).await;
    setup.push_call("call_1", CHANGE, &[(UPDATE, update(11))]);
    setup.push_call("call_2", CHANGE, &[(UPDATE, update(10))]);
    setup.harness.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("OWNER: count the bolts again").await;
    let (_part, call) = waiting(&setup.harness, &conversation).await;
    assert_eq!(
        call.request.unwrap().grants,
        vec![Setup::grant(UPDATE)],
        "an autonomous host takes a grant"
    );
    decide(
        &setup.harness,
        &conversation,
        ApprovalChoice::AllowForConversation,
    )
    .await
    .unwrap();
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
            Some(Decision::Grant)
        ]
    );
    let authorities: Vec<WriteAuthority> = setup
        .answers()
        .into_iter()
        .filter_map(Result::ok)
        .map(|evidence| evidence.authority)
        .collect();
    assert!(
        matches!(authorities[0], WriteAuthority::AllowedOnce { .. }),
        "the owner approved the first call: {authorities:?}"
    );
    assert_eq!(
        authorities[1],
        WriteAuthority::Grants {
            scopes: vec![Setup::grant(UPDATE)]
        }
    );
}

#[tokio::test]
async fn full_access_counts_for_a_workers_write_only_where_its_host_takes_it() {
    let accepting = Setup::new(CeilingMode::Ask, true).await;
    let conversation = accepting.start_in_full_access().await;
    accepting.push_call("call_1", CHANGE, &[(UPDATE, update(11))]);
    accepting.harness.model.push_reply(reply("Done."));
    let run = accepting
        .send(&conversation, "OWNER: one bolt is gone")
        .await;
    let transcript = accepting.harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(1).try_into().unwrap();
    assert_eq!(call.decision, Some(Decision::FullAccess));
    assert_eq!(
        accepting.answers()[0]
            .as_ref()
            .map(|evidence| &evidence.authority),
        Ok(&WriteAuthority::FullAccess)
    );

    let refusing = Setup::new(CeilingMode::Autonomous, false).await;
    let conversation = refusing.start_in_full_access().await;
    refusing.push_call("call_1", CHANGE, &[(CREATE_INDEX, Vec::new())]);
    refusing.harness.model.push_reply(reply("Indexed."));
    let run = refusing.send(&conversation, "OWNER: index the stock").await;
    let (_part, waiting_call) = waiting(&refusing.harness, &conversation).await;
    let request = waiting_call.request.unwrap();
    assert_eq!(request.reasons, vec![ApprovalReason::DatabaseSchema]);
    assert_eq!(request.grants, Vec::new());
    decide(&refusing.harness, &conversation, ApprovalChoice::AllowOnce)
        .await
        .unwrap();
    refusing.harness.wait_for_run(&conversation, &run).await;
    assert!(matches!(
        refusing.answers()[0]
            .as_ref()
            .map(|evidence| &evidence.authority),
        Ok(WriteAuthority::AllowedOnce { .. })
    ));
}

#[tokio::test]
async fn a_read_only_hosts_write_and_a_read_only_database_end_before_anything_asks() {
    let host = Setup::new(CeilingMode::ReadOnly, true).await;
    host.push_call("call_1", CHANGE, &[(UPDATE, update(11))]);
    host.harness.model.push_reply(reply("I cannot."));
    let (conversation, run) = host.start("OWNER: one bolt is gone").await;
    let transcript = host.harness.wait_for_run(&conversation, &run).await;
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(
        failure_of(&call.result.unwrap().outcome),
        Some("database_read_only_host")
    );

    let marked = Setup::new(CeilingMode::Autonomous, true).await;
    marked.tools.database.lock().unwrap().read_only = true;
    marked.push_call("call_1", CHANGE, &[(UPDATE, update(11))]);
    marked.harness.model.push_reply(reply("I cannot."));
    let (conversation, run) = marked.start("OWNER: one bolt is gone").await;
    let transcript = marked.harness.wait_for_run(&conversation, &run).await;
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(
        failure_of(&call.result.unwrap().outcome),
        Some("database_read_only")
    );
    assert!(host.answers().is_empty() && marked.answers().is_empty());
}

#[tokio::test]
async fn a_statement_is_classified_against_the_copy_of_the_workers_schema() {
    let setup = Setup::new(CeilingMode::Autonomous, false).await;
    setup.push_call("call_1", QUERY, &[("SELECT stock FROM bins", Vec::new())]);
    setup.push_call("call_2", QUERY, &[(UPDATE, update(11))]);
    setup.harness.model.push_reply(reply("Neither ran."));

    let (conversation, run) = setup.start("OWNER: look around").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let failures: Vec<Option<String>> = transcript
        .tool_calls(0)
        .into_iter()
        .map(|call| failure_of(&call.result.unwrap().outcome).map(str::to_owned))
        .collect();
    assert_eq!(
        failures,
        vec![
            Some("database_statement_refused".to_owned()),
            Some("database_not_a_read".to_owned()),
        ]
    );
}

#[tokio::test]
async fn what_breaks_the_limits_or_names_a_database_twice_ends_the_call() {
    let large = Setup::new(CeilingMode::Ask, false).await;
    large.tools.database.lock().unwrap().schema =
        DatabaseSchema::new(vec![SCHEMA.to_owned(); MAX_SCHEMA_STATEMENTS + 1]);
    large.push_call("call_1", QUERY, &[(SELECT, vec![SqlValue::text("bolt")])]);
    let twice = Setup::new(CeilingMode::Ask, false).await;
    *twice.tools.named_here.lock().unwrap() = true;
    twice.push_call("call_1", QUERY, &[(SELECT, vec![SqlValue::text("bolt")])]);
    let unnamed = Setup::new(CeilingMode::Ask, false).await;
    unnamed.tools.database.lock().unwrap().host.name = "build\nbox".to_owned();
    unnamed.push_call("call_1", QUERY, &[(SELECT, vec![SqlValue::text("bolt")])]);

    for (setup, code) in [
        (large, "database_schema_too_large"),
        (twice, "database_worker_facts"),
        (unnamed, "database_worker_facts"),
    ] {
        setup.harness.model.push_reply(reply("It did not run."));
        let (conversation, run) = setup.start("OWNER: how many bolts?").await;
        let transcript = setup.harness.wait_for_run(&conversation, &run).await;
        let [call] = transcript.tool_calls(0).try_into().unwrap();
        assert_eq!(failure_of(&call.result.unwrap().outcome), Some(code));
    }
}

#[tokio::test]
async fn a_workers_ticket_gives_evidence_only_of_what_was_allowed_and_writes_nothing_here() {
    for behavior in [Behavior::TamperedEvidence, Behavior::WriteHere] {
        let setup = Setup::new(CeilingMode::Autonomous, false).await;
        setup.behave(behavior);
        setup.push_call("call_1", CHANGE, &[(UPDATE, update(11))]);
        setup.harness.model.push_reply(reply("Done."));

        let (conversation, run) = setup.start("OWNER: one bolt is gone").await;
        decide(&setup.harness, &conversation, ApprovalChoice::AllowOnce)
            .await
            .unwrap();
        let transcript = setup.harness.wait_for_run(&conversation, &run).await;

        let [call] = transcript.tool_calls(0).try_into().unwrap();
        match behavior {
            Behavior::TamperedEvidence => {
                assert_eq!(setup.answers(), vec![Err(SqliteFilesError::NotAllowed)]);
            }
            _write_here => assert_eq!(
                call.result.unwrap().output,
                SqliteFilesError::NotAllowed.code()
            ),
        }
    }
}

#[tokio::test]
async fn a_call_whose_plugin_lost_its_answer_is_uncertain_and_the_model_reads_the_plugins_words() {
    let setup = Setup::new(CeilingMode::Ask, false).await;
    setup.behave(Behavior::LoseAnswer);
    setup.push_call("call_1", CHANGE, &[(UPDATE, update(11))]);
    setup
        .harness
        .model
        .push_reply(reply("I will check the stock."));

    let (conversation, run) = setup.start("OWNER: one bolt is gone").await;
    decide(&setup.harness, &conversation, ApprovalChoice::AllowOnce)
        .await
        .unwrap();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Uncertain);
    assert_eq!(
        call.interruption,
        Some(Interruption::AnswerLost {
            plugin: PLUGIN.to_owned()
        })
    );
    let outcome = call.result.unwrap().outcome;
    assert_eq!(failure_of(&outcome), Some("worker_answer_lost"));
    assert!(matches!(
        outcome,
        ToolOutcome::Plugin {
            uncertain: true,
            ..
        }
    ));
    let requests = setup.harness.model.reply_requests();
    assert!(
        result(&requests[1], "call_1").contains("the write may have run"),
        "{}",
        result(&requests[1], "call_1")
    );
}

#[tokio::test]
async fn a_read_whose_plugin_lost_its_answer_only_completes() {
    let setup = Setup::new(CeilingMode::Ask, false).await;
    setup.behave(Behavior::LoseAnswer);
    setup.push_call("call_1", QUERY, &[(SELECT, vec![SqlValue::text("bolt")])]);
    setup.harness.model.push_reply(reply("I will ask again."));

    let (conversation, run) = setup.start("OWNER: how many bolts?").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    // A read changes nothing, so losing its answer leaves nothing uncertain.
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.interruption, None);
    assert!(matches!(
        call.result.unwrap().outcome,
        ToolOutcome::Plugin {
            uncertain: false,
            ..
        }
    ));
}

/// An owner's change of one row of the worker's database.
fn row_change(id: &str) -> OwnerChange {
    OwnerChange {
        id: DatabaseOperationId::parse(id).unwrap(),
        change: Change::UpdateRow {
            table: "parts".to_owned(),
            row: RowMatch {
                key: vec![SqlValue::integer(1)],
                expected: vec![ColumnValue {
                    column: "stock".to_owned(),
                    value: SqlValue::integer(12),
                }],
            },
            set: vec![ColumnValue {
                column: "stock".to_owned(),
                value: SqlValue::integer(11),
            }],
        },
    }
}

const OPERATION: &str = "0199e0f2-0000-7000-8000-000000000101";
const OTHER_OPERATION: &str = "0199e0f2-0000-7000-8000-000000000102";

#[tokio::test]
async fn the_owners_change_on_a_workers_database_is_journaled_before_the_plugin_sends_it() {
    let setup = Setup::new(CeilingMode::Ask, false).await;
    let change = row_change(OPERATION);
    let store = &setup.harness.store;

    let remote = setup
        .tools
        .files
        .begin_remote_change(key(), reported(CeilingMode::Ask, false), change.clone())
        .await
        .unwrap();

    let evidence = remote.evidence().clone();
    assert_eq!(evidence.operation, change.id);
    assert_eq!(evidence.key, key());
    assert_eq!(evidence.identity, IDENTITY);
    assert_eq!(evidence.origin, WriteOrigin::OwnerOperation);
    assert_eq!(evidence.authority, WriteAuthority::OwnerAction);
    assert_eq!(
        evidence.write,
        EvidencedWrite::Change {
            change: change.change.clone()
        }
    );
    assert_eq!(
        store.db_operation_state(&change.id).await.unwrap(),
        Some(OperationState::Executing)
    );
    remote.end(RemoteOutcome::Completed { changes: 1 }).await;
    assert_eq!(
        store.db_operation_state(&change.id).await.unwrap(),
        Some(OperationState::Completed)
    );
    // An operation that the journal has never begins again.
    assert_eq!(
        setup
            .tools
            .files
            .begin_remote_change(key(), reported(CeilingMode::Ask, false), change)
            .await
            .map(|_remote| ()),
        Err(SqliteFilesError::AlreadyRun)
    );
}

#[tokio::test]
async fn an_owners_change_that_the_plugin_drops_or_loses_becomes_uncertain() {
    let setup = Setup::new(CeilingMode::Autonomous, false).await;
    let store = &setup.harness.store;
    let dropped = row_change(OPERATION);
    let lost = row_change(OTHER_OPERATION);

    let remote = setup
        .tools
        .files
        .begin_remote_change(
            key(),
            reported(CeilingMode::Autonomous, false),
            dropped.clone(),
        )
        .await
        .unwrap();
    drop(remote);
    let remote = setup
        .tools
        .files
        .begin_remote_change(
            key(),
            reported(CeilingMode::Autonomous, false),
            lost.clone(),
        )
        .await
        .unwrap();
    remote.end(RemoteOutcome::Uncertain).await;

    assert_eq!(
        store.db_operation_state(&lost.id).await.unwrap(),
        Some(OperationState::Uncertain)
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state = store.db_operation_state(&dropped.id).await.unwrap();
        if state == Some(OperationState::Uncertain) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the dropped change stays {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn the_owners_change_on_a_worker_is_checked_as_the_worker_checks_it() {
    let setup = Setup::new(CeilingMode::Ask, false).await;
    let files = &setup.tools.files;
    let statements = |text: &str, schema_confirmed: bool| OwnerChange {
        id: DatabaseOperationId::parse(OPERATION).unwrap(),
        change: Change::Statements {
            statements: vec![Statement::new(text, Vec::new())],
            schema_confirmed,
        },
    };
    let mut marked = reported(CeilingMode::Ask, false);
    marked.read_only = true;
    let mut large = reported(CeilingMode::Ask, false);
    large.schema = DatabaseSchema::new(vec![SCHEMA.to_owned(); MAX_SCHEMA_STATEMENTS + 1]);

    let cases = [
        (
            reported(CeilingMode::Ask, false),
            statements(CREATE_INDEX, false),
            SqliteFilesError::SchemaUnconfirmed,
        ),
        (
            reported(CeilingMode::Ask, false),
            statements("SELECT 1", false),
            SqliteFilesError::NotARead {
                class: StatementClass::Read,
            },
        ),
        (
            marked,
            row_change(OPERATION),
            SqliteFilesError::MarkedReadOnly,
        ),
        (
            reported(CeilingMode::ReadOnly, true),
            row_change(OPERATION),
            SqliteFilesError::ReadOnlyHost,
        ),
        (
            large,
            row_change(OPERATION),
            SqliteFilesError::SchemaTooLarge,
        ),
    ];
    for (database, change, refusal) in cases {
        assert_eq!(
            files
                .begin_remote_change(key(), database, change)
                .await
                .map(|_remote| ()),
            Err(refusal)
        );
    }
    // A refused change is never journaled, so the owner's next try begins.
    assert_eq!(
        setup
            .harness
            .store
            .db_operation_state(&DatabaseOperationId::parse(OPERATION).unwrap())
            .await
            .unwrap(),
        None
    );
    let confirmed = files
        .begin_remote_change(
            key(),
            reported(CeilingMode::Ask, false),
            statements(CREATE_INDEX, true),
        )
        .await
        .unwrap();
    confirmed
        .end(RemoteOutcome::Failed {
            code: "Not a code".to_owned(),
        })
        .await;
    assert_eq!(
        setup
            .harness
            .store
            .db_operation_state(&DatabaseOperationId::parse(OPERATION).unwrap())
            .await
            .unwrap(),
        Some(OperationState::Failed)
    );
}

#[tokio::test]
async fn a_statement_is_classified_against_a_copy_of_a_schema_within_its_limits() {
    let setup = Setup::new(CeilingMode::Ask, false).await;
    let files = &setup.tools.files;
    let schema = DatabaseSchema::new(vec![SCHEMA.to_owned()]);

    for (text, class) in [
        (SELECT, StatementClass::Read),
        (UPDATE, StatementClass::Write),
        (CREATE_INDEX, StatementClass::Schema),
    ] {
        assert_eq!(
            files.classify_copy(schema.clone(), text.to_owned()).await,
            Ok(class),
            "{text}"
        );
    }
    assert!(matches!(
        files
            .classify_copy(schema, "SELECT * FROM bins".to_owned())
            .await,
        Err(SqliteFilesError::InvalidStatement { .. })
    ));
    assert_eq!(
        files
            .classify_copy(
                DatabaseSchema::new(vec![SCHEMA.to_owned(); MAX_SCHEMA_STATEMENTS + 1]),
                SELECT.to_owned()
            )
            .await,
        Err(SqliteFilesError::SchemaTooLarge)
    );
}
