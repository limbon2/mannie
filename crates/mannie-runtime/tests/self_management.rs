//! Self-awareness, read-only inspection, redacted log reading, and
//! chat-driven self-configuration through `mannie_inspect`, `mannie_read_logs`,
//! and `mannie_configure` (ADR-0081).

#![cfg(test)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::points::management::{
    ActionResult, ExecuteActionRequest, Management, ManagementActionSpec, ManagementError,
    ManagementGuide, ManagementProvider, ManagementSnapshot, PluginManagement,
    PrepareActionRequest, PreparedAction, SecretFieldSpec,
};
use mannie_provider::{Content, ModelRequest, Role};
use mannie_runtime::testing::{
    Harness, HarnessOptions, add_plugin, remove_plugin, reply, tool_calls,
};
use mannie_runtime::{
    AppHost, AppRuntimeInfo, MANNIE_CONFIGURE, MANNIE_INSPECT, MANNIE_READ_LOGS, texts,
};
use mannie_types::chat::{SendMessage, StartConversation};
use mannie_types::ids::{ConversationId, PartId};
use mannie_types::interface::{InterfaceSettings, InterfaceSettingsError, UpdateInterfaceSettings};
use mannie_types::notifications::{
    NotificationError, NotificationSettings, UpdateNotificationSettings,
};
use mannie_types::plugins::{PluginState, PluginUses, PluginView, PluginWork, PluginsError};
use mannie_types::secret::Secret;
use mannie_types::tools::{
    Access, ApprovalChoice, ApprovalReason, ConfigFieldPreview, ConfigSecretField, DecideApproval,
    DecideConfigApproval, NotRunReason, SetFullAccess, ToolCallStatus, ToolCallView, ToolOutcome,
    ToolsError,
};
use mannie_types::transcript::PartContent;
use mannie_types::updates::{
    UpdateState, UpdateStatus, UpdateUpdatesSettings, UpdatesError, UpdatesSettings,
};
use mannie_types::window::{UpdateWindowSettings, WindowError, WindowSettings};
use serde_json::{Value, json};

/// A recorded execution on [`DemoManagementProvider`], keeping the secret
/// value only in test RAM so assertions can verify it reached the target.
#[derive(Debug, Clone)]
struct RecordedAction {
    action: String,
    token: Option<Secret>,
}

/// A fake plugin management provider that accepts a `save_item` action with a
/// required secret `token` and a `ping` action with no secrets.
#[derive(Debug, Default)]
struct DemoManagementProvider {
    items: Mutex<Vec<String>>,
    recorded: Mutex<Vec<RecordedAction>>,
}

impl DemoManagementProvider {
    fn recorded(&self) -> Vec<RecordedAction> {
        self.recorded.lock().expect("recorded mutex").clone()
    }
}

impl ManagementProvider for DemoManagementProvider {
    fn guide(&self) -> ManagementGuide {
        ManagementGuide {
            summary: "Demo plugin for self-management integration tests.".to_owned(),
            storage: "In-memory test list.".to_owned(),
            troubleshooting: "Inspect items with mannie_inspect.".to_owned(),
            actions: vec![
                ManagementActionSpec {
                    name: "save_item".to_owned(),
                    description: "Adds an item authenticated with a secret token.".to_owned(),
                    security_sensitive: true,
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" }
                        },
                        "required": ["name"],
                        "additionalProperties": false
                    }),
                    secret_fields: vec![SecretFieldSpec {
                        key: "token".to_owned(),
                        label: "Item token".to_owned(),
                        description: "Secret token for the demo item.".to_owned(),
                        required: true,
                    }],
                },
                ManagementActionSpec {
                    name: "ping".to_owned(),
                    description: "Non-sensitive ping action.".to_owned(),
                    security_sensitive: false,
                    parameters: json!({
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false
                    }),
                    secret_fields: Vec::new(),
                },
            ],
        }
    }

    fn inspect(
        &self,
        _query: Option<String>,
    ) -> BoxFuture<'static, Result<ManagementSnapshot, ManagementError>> {
        let items = self.items.lock().expect("items mutex").clone();
        futures_util::future::ready(Ok(ManagementSnapshot::new(
            format!("{} demo item(s)", items.len()),
            json!({ "items": items }),
        )))
        .boxed()
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> BoxFuture<'static, Result<PreparedAction, ManagementError>> {
        let provided_in_chat = request.has_provided_secret("token");
        let result = match request.action.as_str() {
            "save_item" => {
                let name = request
                    .arguments
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                match name {
                    Some(name) => Ok(PreparedAction {
                        title: format!("Save demo item {name}"),
                        fields: vec![ConfigFieldPreview {
                            label: "Item name".to_owned(),
                            value: name.clone(),
                        }],
                        secret_fields: vec![ConfigSecretField {
                            key: "token".to_owned(),
                            label: "Item token".to_owned(),
                            required: true,
                            provided_in_chat,
                        }],
                        security_sensitive: true,
                        canonical_arguments: json!({ "name": name }),
                    }),
                    None => Err(ManagementError::invalid_field(
                        "name",
                        "name must be a non-empty string",
                    )),
                }
            }
            "ping" => Ok(PreparedAction {
                title: "Ping demo plugin".to_owned(),
                fields: Vec::new(),
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments: json!({}),
            }),
            other => Err(ManagementError::unknown_action(other)),
        };
        futures_util::future::ready(result).boxed()
    }

    fn execute_action(
        &self,
        request: ExecuteActionRequest,
    ) -> BoxFuture<'static, Result<ActionResult, ManagementError>> {
        let result = match request.action.as_str() {
            "save_item" => {
                let name = request
                    .arguments
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unnamed")
                    .to_owned();
                let token = request.secrets.get("token").cloned();
                self.items.lock().expect("items mutex").push(name.clone());
                self.recorded
                    .lock()
                    .expect("recorded mutex")
                    .push(RecordedAction {
                        action: request.action,
                        token,
                    });
                Ok(ActionResult::new(format!("Saved demo item {name}."))
                    .with_data(json!({ "name": name })))
            }
            "ping" => {
                self.recorded
                    .lock()
                    .expect("recorded mutex")
                    .push(RecordedAction {
                        action: request.action,
                        token: None,
                    });
                Ok(ActionResult::new("Pong.").with_data(json!({ "ok": true })))
            }
            other => Err(ManagementError::unknown_action(other)),
        };
        futures_util::future::ready(result).boxed()
    }
}

/// Fake [`AppHost`] supporting plugin toggles (wired to runtime contributions),
/// canonical test paths, and core host settings.
#[derive(Debug)]
struct FakeAppHost {
    runtime: mannie_runtime::Runtime,
    info: AppRuntimeInfo,
    plugins: Mutex<Vec<PluginView>>,
    demo_provider: Arc<DemoManagementProvider>,
    window: Mutex<WindowSettings>,
    notifications: Mutex<NotificationSettings>,
    interface: Mutex<InterfaceSettings>,
    updates: Mutex<UpdatesSettings>,
}

impl FakeAppHost {
    async fn new(
        harness: &Harness,
        log_dir: PathBuf,
        demo_initially_on: bool,
    ) -> (Arc<Self>, Arc<DemoManagementProvider>) {
        let diag = harness
            .store
            .diagnostics()
            .await
            .expect("store diagnostics");
        let data_dir = diag
            .database_path
            .parent()
            .expect("db parent")
            .to_path_buf();
        let demo_provider = Arc::new(DemoManagementProvider::default());
        let demo_state = if demo_initially_on {
            PluginState::On
        } else {
            PluginState::Off
        };
        if demo_initially_on {
            let provider: Arc<dyn ManagementProvider> =
                Arc::<DemoManagementProvider>::clone(&demo_provider);
            add_plugin(
                &harness.runtime,
                "demo",
                Contributions::new().add::<Management>(PluginManagement::new("demo", provider)),
            )
            .expect("register demo plugin");
        }
        let host = Arc::new(Self {
            runtime: harness.runtime.clone(),
            info: AppRuntimeInfo {
                version: "0.9.0-test".to_owned(),
                build_profile: "development".to_owned(),
                bundle_identifier: "dev.mannie.desktop.test".to_owned(),
                os: std::env::consts::OS.to_owned(),
                arch: std::env::consts::ARCH.to_owned(),
                data_dir: data_dir.clone(),
                database_path: diag.database_path,
                backups_dir: data_dir.join("backups"),
                log_dir,
                skills_dir: Some(data_dir.join("skills")),
                browser_data_dir: None,
                credential_store: "in-memory test vault".to_owned(),
            },
            plugins: Mutex::new(vec![
                PluginView {
                    id: "demo".to_owned(),
                    enabled: demo_initially_on,
                    state: demo_state,
                    failure: None,
                    uses: PluginUses::default(),
                    features: Vec::new(),
                },
                PluginView {
                    id: "broken".to_owned(),
                    enabled: true,
                    state: PluginState::Failed,
                    failure: Some("start_failed".to_owned()),
                    uses: PluginUses::default(),
                    features: Vec::new(),
                },
            ]),
            demo_provider: Arc::clone(&demo_provider),
            window: Mutex::new(WindowSettings::default()),
            notifications: Mutex::new(NotificationSettings::default()),
            interface: Mutex::new(InterfaceSettings::default()),
            updates: Mutex::new(UpdatesSettings::default()),
        });
        (host, demo_provider)
    }
}

impl AppHost for FakeAppHost {
    fn app_info(&self) -> AppRuntimeInfo {
        self.info.clone()
    }

    fn list_plugins(&self) -> Vec<PluginView> {
        self.plugins.lock().expect("plugins mutex").clone()
    }

    fn plugin_work(
        &self,
        _plugin_id: String,
    ) -> BoxFuture<'static, Result<PluginWork, PluginsError>> {
        futures_util::future::ready(Ok(PluginWork { items: Vec::new() })).boxed()
    }

    fn plugin_feature_work(
        &self,
        _plugin_id: String,
        _feature_id: String,
    ) -> BoxFuture<'static, Result<PluginWork, PluginsError>> {
        futures_util::future::ready(Ok(PluginWork { items: Vec::new() })).boxed()
    }

    fn set_plugin_enabled(
        &self,
        plugin_id: String,
        enabled: bool,
    ) -> BoxFuture<'static, Result<PluginView, PluginsError>> {
        let mut plugins = self.plugins.lock().expect("plugins mutex");
        let Some(plugin) = plugins.iter_mut().find(|entry| entry.id == plugin_id) else {
            return futures_util::future::ready(Err(PluginsError::UnknownPlugin)).boxed();
        };
        if plugin_id == "demo" {
            if enabled && plugin.state != PluginState::On {
                let provider: Arc<dyn ManagementProvider> =
                    Arc::<DemoManagementProvider>::clone(&self.demo_provider);
                add_plugin(
                    &self.runtime,
                    "demo",
                    Contributions::new().add::<Management>(PluginManagement::new("demo", provider)),
                )
                .expect("add demo management");
            } else if !enabled && plugin.state == PluginState::On {
                remove_plugin(&self.runtime, "demo");
            }
        }
        plugin.enabled = enabled;
        plugin.state = if enabled {
            PluginState::On
        } else {
            PluginState::Off
        };
        plugin.failure = None;
        futures_util::future::ready(Ok(plugin.clone())).boxed()
    }

    fn set_plugin_feature_enabled(
        &self,
        _plugin_id: String,
        _feature_id: String,
        _enabled: bool,
    ) -> BoxFuture<'static, Result<PluginView, PluginsError>> {
        futures_util::future::ready(Err(PluginsError::UnknownFeature)).boxed()
    }

    fn window_settings(&self) -> WindowSettings {
        self.window.lock().expect("window mutex").clone()
    }

    fn update_window_settings(
        &self,
        update: UpdateWindowSettings,
    ) -> BoxFuture<'static, Result<WindowSettings, WindowError>> {
        let mut guard = self.window.lock().expect("window mutex");
        guard.close_action = update.close_action;
        futures_util::future::ready(Ok(guard.clone())).boxed()
    }

    fn notification_settings(&self) -> NotificationSettings {
        *self.notifications.lock().expect("notifications mutex")
    }

    fn update_notification_settings(
        &self,
        update: UpdateNotificationSettings,
    ) -> BoxFuture<'static, Result<NotificationSettings, NotificationError>> {
        let mut guard = self.notifications.lock().expect("notifications mutex");
        *guard = NotificationSettings::from(update);
        futures_util::future::ready(Ok(*guard)).boxed()
    }

    fn update_status(&self) -> UpdateStatus {
        UpdateStatus {
            current_version: "0.9.0-test".to_owned(),
            state: UpdateState::Idle { checked_at: None },
            last_outcome: None,
            needs_system_password: false,
        }
    }

    fn updates_settings(&self) -> BoxFuture<'static, Result<UpdatesSettings, UpdatesError>> {
        let settings = self.updates.lock().expect("updates mutex").clone();
        futures_util::future::ready(Ok(settings)).boxed()
    }

    fn update_updates_settings(
        &self,
        update: UpdateUpdatesSettings,
    ) -> BoxFuture<'static, Result<UpdatesSettings, UpdatesError>> {
        let mut guard = self.updates.lock().expect("updates mutex");
        guard.check_automatically = update.check_automatically;
        futures_util::future::ready(Ok(guard.clone())).boxed()
    }

    fn check_for_updates(&self) -> Result<(), UpdatesError> {
        Ok(())
    }

    fn interface_settings(
        &self,
    ) -> BoxFuture<'static, Result<InterfaceSettings, InterfaceSettingsError>> {
        let settings = self.interface.lock().expect("interface mutex").clone();
        futures_util::future::ready(Ok(settings)).boxed()
    }

    fn update_interface_settings(
        &self,
        update: UpdateInterfaceSettings,
    ) -> BoxFuture<'static, Result<InterfaceSettings, InterfaceSettingsError>> {
        let settings = match update.into_settings() {
            Ok(settings) => settings,
            Err(error) => return futures_util::future::ready(Err(error)).boxed(),
        };
        let mut guard = self.interface.lock().expect("interface mutex");
        *guard = settings.clone();
        futures_util::future::ready(Ok(settings)).boxed()
    }

    fn conversations_deleted(
        &self,
        _conversation_ids: Vec<ConversationId>,
    ) -> BoxFuture<'static, ()> {
        futures_util::future::ready(()).boxed()
    }
}

async fn setup_with_host(
    demo_initially_on: bool,
) -> (
    Harness,
    tempfile::TempDir,
    Arc<DemoManagementProvider>,
    PathBuf,
) {
    let harness = Harness::with_options(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await;
    let logs = tempfile::tempdir().expect("temp log dir");
    let (host, demo) =
        FakeAppHost::new(&harness, logs.path().to_path_buf(), demo_initially_on).await;
    let db_path = host.app_info().database_path;
    harness.runtime.attach_app_host(host);
    (harness, logs, demo, db_path)
}

fn tool_names(request: &ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

fn tool_result_for(request: &ModelRequest, call_id: &str) -> String {
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
        .unwrap_or_else(|| panic!("no tool result for {call_id}"))
}

async fn wait_for_awaiting_in_turn(
    harness: &Harness,
    conversation: &ConversationId,
    turn: u32,
) -> (PartId, ToolCallView) {
    let transcript = harness
        .wait_until(conversation, |transcript| {
            transcript
                .tool_calls(turn)
                .iter()
                .any(|call| call.status == ToolCallStatus::AwaitingApproval)
        })
        .await;
    for part in transcript.parts(turn) {
        if let PartContent::ToolCall { call } = part.content
            && call.status == ToolCallStatus::AwaitingApproval
        {
            return (part.id, *call);
        }
    }
    panic!("awaiting approval part disappeared")
}

fn assert_absent_from_sqlite_files(db_path: &Path, needle: &str) {
    let bytes = needle.as_bytes();
    let paths = [
        db_path.to_path_buf(),
        PathBuf::from(format!("{}-wal", db_path.display())),
        PathBuf::from(format!("{}-shm", db_path.display())),
    ];
    for path in paths {
        if let Ok(content) = std::fs::read(&path) {
            assert!(
                !content.windows(bytes.len()).any(|window| window == bytes),
                "planted secret leaked into {}",
                path.display()
            );
        }
    }
}

async fn wait_for_nth_awaiting_in_turn(
    harness: &Harness,
    conversation: &ConversationId,
    turn: u32,
    index: usize,
) -> (PartId, ToolCallView) {
    let transcript = harness
        .wait_until(conversation, |transcript| {
            transcript
                .tool_calls(turn)
                .get(index)
                .is_some_and(|call| call.status == ToolCallStatus::AwaitingApproval)
        })
        .await;
    transcript
        .parts(turn)
        .into_iter()
        .find_map(|part| match part.content {
            PartContent::ToolCall { call } if call.status == ToolCallStatus::AwaitingApproval => {
                Some((part.id, *call))
            }
            _ => None,
        })
        .expect("awaiting approval part")
}

fn push_inspect_and_log_calls(harness: &Harness) {
    let overview = json!({ "activity": "Inspecting overview", "topic": "overview", "target": null, "query": null }).to_string();
    let plugins = json!({ "activity": "Inspecting plugins", "topic": "plugins", "target": null, "query": null }).to_string();
    let demo = json!({ "activity": "Inspecting demo plugin", "topic": "plugin", "target": "demo", "query": null }).to_string();
    let logs = json!({ "activity": "Reading error logs", "file": null, "level": "error", "targetPrefix": "mannie::ssh", "contains": "handshake", "tailLines": 20 }).to_string();
    let traversal = json!({ "activity": "Trying path traversal", "file": "../mannie.sqlite", "level": null, "targetPrefix": null, "contains": null, "tailLines": 10 }).to_string();
    harness.model.push_reply(tool_calls(&[
        ("c1", MANNIE_INSPECT, &overview),
        ("c2", MANNIE_INSPECT, &plugins),
        ("c3", MANNIE_INSPECT, &demo),
        ("c4", MANNIE_READ_LOGS, &logs),
        ("c5", MANNIE_READ_LOGS, &traversal),
    ]));
}

#[tokio::test]
async fn inspect_and_read_logs_run_without_approval_and_reject_traversal() {
    let (harness, logs, _demo, _db_path) = setup_with_host(true).await;
    std::fs::write(
        logs.path().join("mannie.2026-11-26.log"),
        concat!(
            r#"{"timestamp":"2026-11-26T00:00:01Z","level":"INFO","target":"mannie::boot","fields":{"message":"booted"}}"#,
            "\n",
            r#"{"timestamp":"2026-11-26T00:00:02Z","level":"ERROR","target":"mannie::ssh","fields":{"message":"handshake failed"}}"#,
            "\n",
        ),
    )
    .expect("write daily log");

    push_inspect_and_log_calls(&harness);
    harness.model.push_reply(reply("Diagnosed."));

    let started = harness
        .runtime
        .start_conversation(StartConversation::new("INSPECT: check Mannie"))
        .await
        .expect("start");
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let requests = harness.model.requests_for("INSPECT:");
    assert!(
        requests[0]
            .instructions
            .contains(texts::MANNIE_SELF_MANAGEMENT_GUIDANCE)
    );
    assert!(
        requests[0]
            .instructions
            .contains("Installed plugins: demo (on), broken (failed).")
    );

    let overview = tool_result_for(&requests[1], "c1");
    assert!(overview.contains("0.9.0-test"), "{overview}");
    assert!(overview.contains("in-memory test vault"), "{overview}");

    let plugins = tool_result_for(&requests[1], "c2");
    assert!(plugins.contains("broken"), "{plugins}");
    assert!(plugins.contains("start_failed"), "{plugins}");

    let demo = tool_result_for(&requests[1], "c3");
    assert!(demo.contains("save_item"), "{demo}");
    assert!(demo.contains("Item token"), "{demo}");

    let log_out = tool_result_for(&requests[1], "c4");
    assert!(log_out.contains("handshake failed"), "{log_out}");
    assert!(!log_out.contains("booted"), "{log_out}");

    let traversal = tool_result_for(&requests[1], "c5");
    assert!(
        traversal.to_lowercase().contains("path")
            || traversal.to_lowercase().contains("invalid")
            || traversal.to_lowercase().contains("name")
            || traversal.to_lowercase().contains("not a valid"),
        "{traversal}"
    );

    let calls = transcript.tool_calls(0);
    assert_eq!(calls.len(), 5);
    assert!(matches!(
        calls[0].result.as_ref().expect("c1 result").outcome,
        ToolOutcome::MannieInspect { .. }
    ));
    assert!(matches!(
        calls[3].result.as_ref().expect("c4 result").outcome,
        ToolOutcome::MannieLogs { .. }
    ));
    assert!(calls.iter().all(|call| call.request.is_none()));
}

async fn assert_config_approval_refusals(
    harness: &Harness,
    conversation_id: &ConversationId,
    part_id: &PartId,
    digest: &str,
    card_secret: &str,
) {
    let grant_err = harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation_id.clone(),
            part_id: part_id.clone(),
            digest: digest.to_owned(),
            choice: ApprovalChoice::AllowForConversation,
        })
        .await
        .unwrap_err();
    assert_eq!(grant_err, ToolsError::GrantNotPossible);

    let missing_err = harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation_id.clone(),
            part_id: part_id.clone(),
            digest: digest.to_owned(),
            choice: ApprovalChoice::AllowOnce,
        })
        .await
        .unwrap_err();
    assert_eq!(
        missing_err,
        ToolsError::SecretRequired {
            field: "token".to_owned()
        }
    );

    let digest_err = harness
        .runtime
        .decide_config_approval(DecideConfigApproval {
            conversation_id: conversation_id.clone(),
            part_id: part_id.clone(),
            digest: "wrong-digest".to_owned(),
            allow: true,
            secrets: BTreeMap::from([("token".to_owned(), Secret::new(card_secret.to_owned()))]),
        })
        .unwrap_err();
    assert_eq!(digest_err, ToolsError::ApprovalChanged);

    let unknown_err = harness
        .runtime
        .decide_config_approval(DecideConfigApproval {
            conversation_id: conversation_id.clone(),
            part_id: part_id.clone(),
            digest: digest.to_owned(),
            allow: true,
            secrets: BTreeMap::from([("bogus".to_owned(), Secret::new("x".to_owned()))]),
        })
        .unwrap_err();
    assert_eq!(
        unknown_err,
        ToolsError::SecretInvalid {
            field: "bogus".to_owned()
        }
    );
}

async fn verify_card_secret_path(
    harness: &Harness,
    conversation_id: &ConversationId,
    demo: &DemoManagementProvider,
    db_path: &Path,
) {
    let card_secret = "CARD-SECRET-999111";
    let args = json!({ "activity": "Saving demo item via card", "target": "demo", "action": "save_item", "arguments": { "name": "alpha" } }).to_string();
    harness
        .model
        .push_reply(tool_calls(&[("card_call", MANNIE_CONFIGURE, &args)]));
    harness.model.push_reply(reply("Alpha saved."));

    let run = harness
        .runtime
        .send_message(SendMessage::new(conversation_id.clone(), "CARD: add alpha"))
        .await
        .expect("send card turn");

    let (part_id, awaiting) = wait_for_awaiting_in_turn(harness, conversation_id, 1).await;
    let request = awaiting.request.as_ref().expect("approval request");
    assert!(request.reasons.contains(&ApprovalReason::AppConfigSecurity));
    assert_eq!(request.grants, Vec::new());
    assert!(request.rule.is_none());

    assert_config_approval_refusals(
        harness,
        conversation_id,
        &part_id,
        &request.digest,
        card_secret,
    )
    .await;

    harness
        .runtime
        .decide_config_approval(DecideConfigApproval {
            conversation_id: conversation_id.clone(),
            part_id,
            digest: request.digest.clone(),
            allow: true,
            secrets: BTreeMap::from([("token".to_owned(), Secret::new(card_secret.to_owned()))]),
        })
        .expect("approve with card secret");

    let transcript = harness.wait_for_run(conversation_id, &run.run_id).await;
    let [call] = transcript.tool_calls(1).try_into().expect("one call");
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert!(matches!(
        call.result.as_ref().expect("result").outcome,
        ToolOutcome::MannieConfigure {
            ref owner,
            ref domain,
            ref action,
        } if owner == "demo" && domain == "demo" && action == "save_item"
    ));

    let recorded = demo.recorded();
    assert_eq!(recorded.len(), 1);
    assert!(
        recorded[0].token.as_ref().map(Secret::expose) == Some(card_secret),
        "provider must receive the card secret"
    );
    assert_absent_from_sqlite_files(db_path, card_secret);
    let call_json = serde_json::to_string(&call).expect("serialize call view");
    assert!(
        !call_json.contains(card_secret),
        "card secret leaked into the call view"
    );
    for req in harness.model.requests_for("CARD:") {
        assert!(
            !format!("{req:?}").contains(card_secret),
            "card secret leaked into a model request"
        );
    }
}

async fn verify_chat_secret_and_override_paths(
    harness: &Harness,
    conversation_id: &ConversationId,
    demo: &DemoManagementProvider,
    db_path: &Path,
) {
    let chat_secret = "CHAT-SECRET-888222";
    let override_secret = "CARD-OVERRIDE-777333";
    let beta_args = json!({ "activity": "Saving beta with throwaway chat secret", "target": "demo", "action": "save_item", "arguments": { "name": "beta" }, "secrets": { "token": chat_secret } }).to_string();
    let gamma_args = json!({ "activity": "Saving gamma with override", "target": "demo", "action": "save_item", "arguments": { "name": "gamma" }, "secrets": { "token": "CHAT-REPLACED-666444" } }).to_string();
    harness.model.push_reply(tool_calls(&[
        ("chat_only", MANNIE_CONFIGURE, &beta_args),
        ("chat_overridden", MANNIE_CONFIGURE, &gamma_args),
    ]));
    harness.model.push_reply(reply("Beta and gamma saved."));

    let run2 = harness
        .runtime
        .send_message(SendMessage::new(
            conversation_id.clone(),
            "CHATSECRET: save beta and gamma",
        ))
        .await
        .expect("send chat secret turn");

    let (beta_part, beta_call) = wait_for_awaiting_in_turn(harness, conversation_id, 2).await;
    match &beta_call.accesses[0] {
        Access::AppConfig { secret_fields, .. } => {
            assert_eq!(secret_fields.len(), 1);
            assert!(secret_fields[0].provided_in_chat);
        }
        other => panic!("expected AppConfig, got {other:?}"),
    }
    harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation_id.clone(),
            part_id: beta_part,
            digest: beta_call.request.as_ref().expect("req").digest.clone(),
            choice: ApprovalChoice::AllowOnce,
        })
        .await
        .expect("approve beta using chat secret");

    let (gamma_part, gamma_call) =
        wait_for_nth_awaiting_in_turn(harness, conversation_id, 2, 1).await;
    harness
        .runtime
        .decide_config_approval(DecideConfigApproval {
            conversation_id: conversation_id.clone(),
            part_id: gamma_part,
            digest: gamma_call.request.as_ref().expect("req").digest.clone(),
            allow: true,
            secrets: BTreeMap::from([(
                "token".to_owned(),
                Secret::new(override_secret.to_owned()),
            )]),
        })
        .expect("approve gamma with override");

    harness.wait_for_run(conversation_id, &run2.run_id).await;

    let recorded = demo.recorded();
    assert_eq!(recorded.len(), 3);
    assert!(
        recorded[1].token.as_ref().map(Secret::expose) == Some(chat_secret),
        "provider must receive the chat secret"
    );
    assert!(
        recorded[2].token.as_ref().map(Secret::expose) == Some(override_secret),
        "provider must receive the card override"
    );
    for leaked in [chat_secret, override_secret, "CHAT-REPLACED-666444"] {
        assert_absent_from_sqlite_files(db_path, leaked);
        for req in harness.model.requests_for("CHATSECRET:") {
            assert!(
                !format!("{req:?}").contains(leaked),
                "planted secret leaked into a model request"
            );
        }
    }
}

#[tokio::test]
async fn configure_always_asks_in_full_access_validates_secrets_and_supports_card_and_chat_paths() {
    let (harness, _logs, demo, db_path) = setup_with_host(true).await;

    harness.model.push_reply(reply("Ready."));
    let started = harness
        .runtime
        .start_conversation(StartConversation::new("SETUP: hello"))
        .await
        .expect("start");
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    harness
        .runtime
        .set_full_access(SetFullAccess {
            conversation_id: started.conversation_id.clone(),
            enabled: true,
        })
        .await
        .expect("enable full access");

    verify_card_secret_path(&harness, &started.conversation_id, &demo, &db_path).await;
    verify_chat_secret_and_override_paths(&harness, &started.conversation_id, &demo, &db_path)
        .await;
}

#[tokio::test]
async fn misplaced_secret_in_arguments_is_scrubbed_before_db_and_refused() {
    let (harness, _logs, demo, db_path) = setup_with_host(true).await;
    let misplaced = "MISPLACED-PASSWORD-555";
    harness.model.push_reply(tool_calls(&[(
        "bad_call",
        MANNIE_CONFIGURE,
        &json!({
            "activity": "Saving with misplaced password",
            "target": "demo",
            "action": "save_item",
            "arguments": { "name": "delta", "password": misplaced }
        })
        .to_string(),
    )]));
    harness
        .model
        .push_reply(reply("I will not put passwords in arguments."));

    let started = harness
        .runtime
        .start_conversation(StartConversation::new("MISPLACED: save delta"))
        .await
        .expect("start");
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let [call] = transcript.tool_calls(0).try_into().expect("one call");
    assert_eq!(call.status, ToolCallStatus::NotRun);
    assert!(matches!(
        call.not_run,
        Some(NotRunReason::InvalidArguments { .. })
    ));
    assert!(demo.recorded().is_empty());
    assert_absent_from_sqlite_files(&db_path, misplaced);
}

#[tokio::test]
async fn enabling_an_off_plugin_mid_reply_makes_it_inspectable_and_configurable_on_next_step() {
    let (harness, _logs, demo, _db_path) = setup_with_host(false).await;

    // Step 0: enable `demo` plugin (which starts OFF).
    harness.model.push_reply(tool_calls(&[(
        "enable_demo",
        MANNIE_CONFIGURE,
        &json!({
            "activity": "Enabling demo plugin",
            "target": "plugins",
            "action": "set_enabled",
            "arguments": { "pluginId": "demo", "enabled": true }
        })
        .to_string(),
    )]));
    // Step 1 (SAME reply): inspect `demo` and run its `ping` action.
    harness.model.push_reply(tool_calls(&[
        (
            "inspect_demo",
            MANNIE_INSPECT,
            &json!({ "activity": "Inspecting demo", "topic": "plugin", "target": "demo", "query": null })
                .to_string(),
        ),
        (
            "ping_demo",
            MANNIE_CONFIGURE,
            &json!({
                "activity": "Pinging demo",
                "target": "demo",
                "action": "ping",
                "arguments": {}
            })
            .to_string(),
        ),
    ]));
    harness.model.push_reply(reply("Enabled and pinged demo."));

    let started = harness
        .runtime
        .start_conversation(StartConversation::new("MIDREPLY: enable demo and ping it"))
        .await
        .expect("start");

    // Approve enabling the plugin.
    let (enable_part, enable_call) =
        wait_for_awaiting_in_turn(&harness, &started.conversation_id, 0).await;
    harness
        .runtime
        .decide_config_approval(DecideConfigApproval {
            conversation_id: started.conversation_id.clone(),
            part_id: enable_part,
            digest: enable_call.request.as_ref().expect("req").digest.clone(),
            allow: true,
            secrets: BTreeMap::new(),
        })
        .expect("approve enable");

    // Wait for the follow-up `ping` call on the newly enabled plugin (3rd call in turn 0).
    let (ping_part, ping_call) =
        wait_for_nth_awaiting_in_turn(&harness, &started.conversation_id, 0, 2).await;
    harness
        .runtime
        .decide_config_approval(DecideConfigApproval {
            conversation_id: started.conversation_id.clone(),
            part_id: ping_part,
            digest: ping_call.request.as_ref().expect("req").digest.clone(),
            allow: true,
            secrets: BTreeMap::new(),
        })
        .expect("approve ping");

    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let requests = harness.model.requests_for("MIDREPLY:");
    assert!(requests[0].instructions.contains("demo (off)"));
    assert!(requests[1].instructions.contains("demo (on)"));
    let inspected = tool_result_for(&requests[2], "inspect_demo");
    assert!(inspected.contains("save_item"), "{inspected}");
    assert_eq!(demo.recorded().len(), 1);
    assert_eq!(demo.recorded()[0].action, "ping");
}

#[tokio::test]
async fn denying_config_approval_skips_later_calls_in_same_step() {
    let (harness, _logs, demo, _db_path) = setup_with_host(true).await;

    harness.model.push_reply(tool_calls(&[
        (
            "deny_first",
            MANNIE_CONFIGURE,
            &json!({
                "activity": "First ping",
                "target": "demo",
                "action": "ping",
                "arguments": {}
            })
            .to_string(),
        ),
        (
            "skip_second",
            MANNIE_CONFIGURE,
            &json!({
                "activity": "Second ping",
                "target": "demo",
                "action": "ping",
                "arguments": {}
            })
            .to_string(),
        ),
    ]));
    harness.model.push_reply(reply("Understood."));

    let started = harness
        .runtime
        .start_conversation(StartConversation::new("DENY: ping twice"))
        .await
        .expect("start");
    let (part_id, call) = wait_for_awaiting_in_turn(&harness, &started.conversation_id, 0).await;
    harness
        .runtime
        .decide_config_approval(DecideConfigApproval {
            conversation_id: started.conversation_id.clone(),
            part_id,
            digest: call.request.as_ref().expect("req").digest.clone(),
            allow: false,
            secrets: BTreeMap::new(),
        })
        .expect("deny");

    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let calls = transcript.tool_calls(0);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].status, ToolCallStatus::Denied);
    assert_eq!(calls[1].status, ToolCallStatus::NotRun);
    assert_eq!(calls[1].not_run, Some(NotRunReason::EarlierCallDenied));
    assert!(demo.recorded().is_empty());
}

async fn verify_plan_agent_inspect_only(harness: &Harness) {
    let plan_draft = harness
        .runtime
        .agent_options()
        .templates
        .into_iter()
        .find(|template| template.id == "plan")
        .expect("plan template")
        .draft;
    let plan_agent = harness
        .runtime
        .create_agent(&plan_draft)
        .await
        .expect("create plan agent");
    harness.model.push_reply_for("PLAN:", reply("Planned."));
    let plan_started = harness
        .runtime
        .start_conversation(
            StartConversation::new("PLAN: inspect setup").with_agent(Some(plan_agent.key)),
        )
        .await
        .expect("start plan");
    harness
        .wait_for_run(&plan_started.conversation_id, &plan_started.run_id)
        .await;
    let plan_req = &harness.model.requests_for("PLAN:")[0];
    let plan_tools = tool_names(plan_req);
    assert!(
        plan_tools.contains(&MANNIE_INSPECT.to_owned()),
        "{plan_tools:?}"
    );
    assert!(
        plan_tools.contains(&MANNIE_READ_LOGS.to_owned()),
        "{plan_tools:?}"
    );
    assert!(
        !plan_tools.contains(&MANNIE_CONFIGURE.to_owned()),
        "{plan_tools:?}"
    );
    assert!(
        plan_req
            .instructions
            .contains(texts::MANNIE_INSPECT_ONLY_GUIDANCE)
    );
    assert!(
        !plan_req
            .instructions
            .contains(texts::MANNIE_SELF_MANAGEMENT_GUIDANCE)
    );
}

#[tokio::test]
async fn plan_agent_gets_inspect_only_and_child_agents_get_none() {
    let (harness, _logs, _demo, _db_path) = setup_with_host(true).await;
    verify_plan_agent_inspect_only(&harness).await;

    // 2. Child agent spawned via `start_task` gets none of the three tools.
    let spawn_args = json!({
        "activity": "Delegating exploration",
        "agent": "explore",
        "title": "Check stuff",
        "instruction": "CHILDCHECK: inspect nothing",
        "background": false
    })
    .to_string();
    harness.model.push_reply_for(
        "PARENT:",
        tool_calls(&[("spawn_child", "start_task", &spawn_args)]),
    );
    harness
        .model
        .push_reply_for("CHILDCHECK:", reply("Child report."));
    harness.model.push_reply_for("PARENT:", reply("Done."));

    let parent_started = harness
        .runtime
        .start_conversation(StartConversation::new("PARENT: delegate"))
        .await
        .expect("start parent");
    harness
        .wait_for_run(&parent_started.conversation_id, &parent_started.run_id)
        .await;
    let child_req = &harness.model.requests_for("CHILDCHECK:")[0];
    let child_tools = tool_names(child_req);
    assert!(
        !child_tools.contains(&MANNIE_INSPECT.to_owned()),
        "{child_tools:?}"
    );
    assert!(
        !child_tools.contains(&MANNIE_READ_LOGS.to_owned()),
        "{child_tools:?}"
    );
    assert!(
        !child_tools.contains(&MANNIE_CONFIGURE.to_owned()),
        "{child_tools:?}"
    );
    assert!(
        !child_req
            .instructions
            .contains(texts::MANNIE_SELF_MANAGEMENT_GUIDANCE)
    );
    assert!(
        !child_req
            .instructions
            .contains(texts::MANNIE_INSPECT_ONLY_GUIDANCE)
    );
    assert!(!child_req.instructions.contains("Installed plugins:"));
}
