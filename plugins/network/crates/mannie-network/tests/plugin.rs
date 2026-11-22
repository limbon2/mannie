//! Integration tests for the Network Client plugin through the real host and
//! core runtime (ADR-0075; V15):
//!
//! - manifest, store tables and SDK conformance;
//! - OS credential-store persistence with zero secrets or cookies in SQLite;
//! - owner as-is visibility in the workbench alongside `[redacted]` agent tools,
//!   mentions, instruction section, approvals and transcripts;
//! - `OpenAPI` v2 JSON and v3.1 YAML import through file dialog and pasted text;
//! - real loopback HTTP execution, cookie jar attachment and `Set-Cookie`
//!   updates, no-redirect safety, per-endpoint grants and environment-switch
//!   re-approval.

use std::sync::Arc;

use futures_util::StreamExt as _;
use mannie_network::commands::interface;
use mannie_network::{MODULE, NETWORK, PLUGIN_MANIFEST, mannie_plugin};
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::{HostOptions, HostedPlugin};
use mannie_plugin_harness::runtime::{HarnessOptions, SeenPart, Transcript, reply, tool_calls};
use mannie_plugin_harness::types::{
    Access, ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision, GrantScope,
    MentionPlace, PartContent, PartId, StartConversation, SuggestMentions, ToolCallStatus,
    ToolCallView,
};
use mannie_plugin_sdk::ipc::ChannelMessage;
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::testing::{FakeFile, FakeFileDialog, FakeSecrets};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

const PLANTED_TOKEN: &str = "sk_live_planted_secret_token_98765";
const PLANTED_COOKIE: &str = "sess_planted_cookie_value_54321";
const PLANTED_RESPONSE_COOKIE: &str = "fresh_session_cookie_from_server_11223";

async fn hosted_with(secrets: &Arc<FakeSecrets>, dialog: &Arc<FakeFileDialog>) -> HostedPlugin {
    let core = HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    };
    let options = HostOptions::new()
        .with_core(core)
        .with_provider(Arc::<FakeSecrets>::clone(secrets))
        .with_provider(Arc::<FakeFileDialog>::clone(dialog));
    let hosted = HostedPlugin::new(mannie_plugin(), options).await;
    hosted.turn_on().await;
    hosted
}

fn call_args(mut value: Value) -> String {
    value["activity"] = json!("Running API request");
    value.to_string()
}

fn waiting_call(transcript: &Transcript) -> Option<(PartId, ApprovalRequest, ToolCallView)> {
    transcript
        .turns
        .values()
        .flat_map(|turn| turn.parts.values())
        .find_map(|part: &SeenPart| {
            let PartContent::ToolCall { call } = &part.content else {
                return None;
            };
            if call.status == ToolCallStatus::AwaitingApproval {
                Some((
                    part.id.clone(),
                    call.request.clone()?,
                    call.as_ref().clone(),
                ))
            } else {
                None
            }
        })
}

fn assert_no_secret_bytes_on_disk(folder: &std::path::Path, forbidden: &[&str]) {
    let mut stack = vec![folder.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                stack.push(path);
                continue;
            }

            if let Ok(bytes) = std::fs::read(&path) {
                for needle in forbidden {
                    let found = bytes
                        .windows(needle.len())
                        .any(|window| window == needle.as_bytes());
                    assert!(
                        !found,
                        "secret {needle:?} leaked into disk file {}",
                        path.display()
                    );
                }
            }
        }
    }
}

/// Starts a real loopback HTTP/1.1 test server that echoes request path,
/// headers (including Authorization and Cookie) and body as JSON, sets a
/// `Set-Cookie` header, and returns 302 on `/redirect`.
async fn start_echo_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback test server");
    let addr = listener.local_addr().expect("read loopback local addr");

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _peer)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16 * 1024];
                let Ok(read) = stream.read(&mut buf).await else {
                    return;
                };
                if read == 0 {
                    return;
                }
                let raw = String::from_utf8_lossy(&buf[..read]);
                let mut lines = raw.split("\r\n");
                let request_line = lines.next().unwrap_or_default();
                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap_or("");
                let target = parts.next().unwrap_or("/");
                let mut headers_map = serde_json::Map::new();
                for line in lines.by_ref() {
                    if line.is_empty() {
                        break;
                    }
                    if let Some((name, val)) = line.split_once(':') {
                        headers_map.insert(
                            name.trim().to_ascii_lowercase(),
                            Value::String(val.trim().to_owned()),
                        );
                    }
                }
                let body = raw.split_once("\r\n\r\n").map_or("", |(_head, body)| body);

                if target.starts_with("/redirect") {
                    let response = "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    drop(stream.write_all(response.as_bytes()).await);
                    return;
                }

                if target.starts_with("/drop-mid-body") {
                    let partial = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5000\r\nConnection: close\r\n\r\npartial-before-drop";
                    drop(stream.write_all(partial.as_bytes()).await);
                    drop(stream.shutdown().await);
                    return;
                }

                if target.starts_with("/huge-response") {
                    let payload = "A".repeat(2 * 1024 * 1024);
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        payload.len()
                    );
                    drop(stream.write_all(header.as_bytes()).await);
                    drop(stream.write_all(payload.as_bytes()).await);
                    return;
                }

                let payload = json!({
                    "method": method,
                    "target": target,
                    "headers": headers_map,
                    "body": body,
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: application/json\r\n\
                     X-Echo-Auth: {PLANTED_TOKEN}\r\n\
                     Set-Cookie: srv_sid={PLANTED_RESPONSE_COOKIE}; Path=/; HttpOnly\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\r\n\
                     {payload}",
                    payload.len()
                );
                drop(stream.write_all(response.as_bytes()).await);
            });
        }
    });

    format!("http://127.0.0.1:{}", addr.port())
}

#[test]
fn manifest_and_interface_match_adr_0075() {
    let manifest = Manifest::parse(PLUGIN_MANIFEST).unwrap();

    assert_eq!(manifest.id.as_str(), NETWORK);
    for service in ["file-dialog", "secrets", "store"] {
        assert!(manifest.declares(service), "{service}");
    }
    assert_eq!(
        manifest.secrets.keyed,
        ["collection", "environment", "cookie", "request", "global"]
    );
    assert_eq!(manifest.crates.exclusive, ["saphyr-parser"]);
    assert_eq!(MODULE.name, manifest.id.prefix());
    let sql = MODULE.migrations[0].sql();
    for table in [
        "network_collections",
        "network_folders",
        "network_requests",
        "network_environments",
        "network_cookies",
        "network_global_variables",
    ] {
        assert!(sql.contains(table), "missing table {table}");
    }

    let declared = interface();
    assert_eq!(declared.channel_names(), ["follow_state"]);
    assert!(declared.command_names().contains(&"create_collection"));
    assert!(declared.command_names().contains(&"send_owner_request"));
    assert!(declared.command_names().contains(&"confirm_openapi_import"));
}

#[tokio::test]
async fn plugin_passes_sdk_conformance() {
    let options = HostOptions::new()
        .with_provider(Arc::new(FakeSecrets::new()))
        .with_provider(Arc::new(FakeFileDialog::default()));

    assert_eq!(
        conformance(mannie_plugin(), options).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn secrets_and_cookies_stay_in_credential_store_and_never_hit_sqlite_while_owner_sees_as_is()
{
    let secrets = Arc::new(FakeSecrets::new());
    let dialog = Arc::new(FakeFileDialog::default());
    let hosted = hosted_with(&secrets, &dialog).await;

    let mut channel = hosted
        .open_channel("follow_state", Value::Null)
        .await
        .unwrap();
    let initial = channel.next().await.unwrap();
    assert!(matches!(initial, ChannelMessage::Item { .. }));

    let created = hosted
        .invoke(
            "create_collection",
            json!({
                "name": "Billing API",
                "baseUrl": "https://api.example.com",
                "enabled": true,
                "autoCookies": true,
                "auth": { "kind": "bearer", "token": PLANTED_TOKEN },
                "variables": [
                    { "key": "api_secret", "value": PLANTED_TOKEN, "enabled": true, "secret": true }
                ]
            }),
        )
        .await
        .unwrap();
    let col_id = created["id"].as_str().unwrap().to_owned();

    let cookie = hosted
        .invoke(
            "save_cookie",
            json!({
                "name": "sid",
                "value": PLANTED_COOKIE,
                "domain": "127.0.0.1",
                "path": "/",
                "secure": false,
                "httpOnly": true,
                "enabled": true
            }),
        )
        .await
        .unwrap();
    let cookie_id = cookie["id"].as_str().unwrap().to_owned();

    // Owner workbench sees secrets and cookies as-is.
    let state = hosted.invoke("get_state", Value::Null).await.unwrap();
    assert_eq!(state["collections"][0]["auth"]["token"], PLANTED_TOKEN);
    assert_eq!(
        state["collections"][0]["variables"][0]["value"],
        PLANTED_TOKEN
    );
    assert_eq!(state["cookies"][0]["value"], PLANTED_COOKIE);

    // Credential store holds the real values.
    assert!(
        secrets
            .stored(&format!("network/collection/{col_id}-0"))
            .unwrap()
            .expose()
            .contains(PLANTED_TOKEN)
    );
    assert_eq!(
        secrets
            .stored(&format!("network/cookie/{cookie_id}-0"))
            .unwrap()
            .expose(),
        PLANTED_COOKIE
    );

    // Zero planted secrets or cookies ever appear in SQLite, WAL, SHM or any
    // other file inside Mannie's data folder.
    assert_no_secret_bytes_on_disk(hosted.core.data_folder(), &[PLANTED_TOKEN, PLANTED_COOKIE]);

    // Deleting collection and cookie removes credential store entries.
    hosted
        .invoke("delete_collection", json!({ "collectionId": col_id }))
        .await
        .unwrap();
    hosted
        .invoke("delete_cookie", json!({ "cookieId": cookie_id }))
        .await
        .unwrap();
    assert!(
        secrets
            .stored(&format!("network/collection/{col_id}-0"))
            .is_none()
    );
    assert!(
        secrets
            .stored(&format!("network/cookie/{cookie_id}-0"))
            .is_none()
    );
}

#[tokio::test]
async fn openapi_file_dialog_and_text_import_work_for_v2_and_v3() {
    let secrets = Arc::new(FakeSecrets::new());
    let dialog = Arc::new(FakeFileDialog::default());
    let yaml_v3 = "
openapi: 3.0.3
info:
  title: Inventory YAML
servers:
  - url: https://inv.example.com/v1
paths:
  /items/{itemId}:
    get:
      tags: [Items]
      summary: Fetch item
      parameters:
        - name: itemId
          in: path
          required: true
          schema:
            type: string
            example: sku-1
";
    dialog.answer(vec![FakeFile::new(
        "inventory.yaml",
        yaml_v3.as_bytes().to_vec(),
    )]);

    let hosted = hosted_with(&secrets, &dialog).await;
    let preview = hosted
        .invoke("pick_openapi_file", json!({ "filterName": "OpenAPI" }))
        .await
        .unwrap();
    let preview_id = preview["previewId"].as_str().unwrap().to_owned();
    assert_eq!(preview["title"], "Inventory YAML");
    assert_eq!(preview["specVersion"], "3.0.3");
    assert_eq!(preview["endpoints"].as_array().unwrap().len(), 1);

    let imported = hosted
        .invoke(
            "confirm_openapi_import",
            json!({ "previewId": preview_id, "enabled": true }),
        )
        .await
        .unwrap();
    assert_eq!(imported["name"], "Inventory YAML");
    assert_eq!(imported["folders"][0]["name"], "Items");
    assert_eq!(imported["requests"][0]["name"], "Fetch item");

    // Also verify Swagger 2.0 pasted JSON import.
    let v2_json = json!({
        "swagger": "2.0",
        "info": { "title": "Legacy V2" },
        "host": "legacy.example.com",
        "basePath": "/api",
        "schemes": ["https"],
        "paths": {
            "/ping": { "get": { "summary": "Ping" } }
        }
    })
    .to_string();
    let v2_preview = hosted
        .invoke("preview_openapi_text", json!({ "text": v2_json }))
        .await
        .unwrap();
    assert_eq!(v2_preview["specVersion"], "2.0");
    let v2_col = hosted
        .invoke(
            "confirm_openapi_import",
            json!({ "previewId": v2_preview["previewId"] }),
        )
        .await
        .unwrap();
    assert_eq!(v2_col["baseUrl"], "https://legacy.example.com/api");
}

async fn seed_loopback_cookie(hosted: &HostedPlugin) {
    hosted
        .invoke(
            "save_cookie",
            json!({
                "name": "sid",
                "value": PLANTED_COOKIE,
                "domain": "127.0.0.1",
                "path": "/",
                "secure": false,
                "httpOnly": true,
                "enabled": true
            }),
        )
        .await
        .expect("seed cookie");
}

async fn seed_payments_api(
    hosted: &HostedPlugin,
    base_url_1: &str,
    base_url_2: &str,
) -> (String, String, String) {
    let col = hosted
        .invoke(
            "create_collection",
            json!({
                "name": "Payments API",
                "baseUrl": "{{api_base}}",
                "enabled": true,
                "autoCookies": true,
                "auth": { "kind": "bearer", "token": PLANTED_TOKEN },
                "variables": [
                    { "key": "api_base", "value": base_url_1, "enabled": true, "secret": false }
                ]
            }),
        )
        .await
        .expect("create Payments API");
    let col_id = col["id"].as_str().expect("collection id").to_owned();

    let env_prod = hosted
        .invoke(
            "save_environment",
            json!({
                "collectionId": col_id,
                "name": "Server Two",
                "variables": [
                    { "key": "api_base", "value": base_url_2, "enabled": true, "secret": false }
                ]
            }),
        )
        .await
        .expect("create Server Two environment");
    let env_prod_id = env_prod["id"].as_str().expect("env id").to_owned();

    seed_loopback_cookie(hosted).await;

    let req = hosted
        .invoke(
            "create_request",
            json!({
                "collectionId": col_id,
                "input": {
                    "name": "Charge",
                    "method": "POST",
                    "urlTemplate": "/charges/{id}",
                    "pathParams": [{ "key": "id", "value": "ch_1", "enabled": true }]
                }
            }),
        )
        .await
        .expect("create Charge request");
    let req_id = req["id"].as_str().expect("request id").to_owned();

    (col_id, env_prod_id, req_id)
}

#[tokio::test]
async fn owner_workbench_executes_unredacted_updates_cookies_and_blocks_redirects_and_insecure_http()
 {
    let base_url = start_echo_server().await;
    let secrets = Arc::new(FakeSecrets::new());
    let dialog = Arc::new(FakeFileDialog::default());
    let hosted = hosted_with(&secrets, &dialog).await;

    let col = hosted
        .invoke(
            "create_collection",
            json!({
                "name": "Local Echo",
                "baseUrl": base_url,
                "enabled": true,
                "autoCookies": true,
                "auth": { "kind": "bearer", "token": PLANTED_TOKEN }
            }),
        )
        .await
        .unwrap();
    let col_id = col["id"].as_str().unwrap().to_owned();
    seed_loopback_cookie(&hosted).await;

    let req = hosted
        .invoke(
            "create_request",
            json!({
                "collectionId": col_id,
                "input": {
                    "name": "Echo",
                    "method": "POST",
                    "urlTemplate": "/echo/{id}",
                    "pathParams": [{ "key": "id", "value": "42", "enabled": true }],
                    "queryParams": [{ "key": "tag", "value": "v1", "enabled": true }],
                    "body": { "kind": "json", "content": "{\"hello\":\"world\"}" }
                }
            }),
        )
        .await
        .unwrap();
    let req_id = req["id"].as_str().unwrap().to_owned();

    // Owner workbench receives full unredacted response and updates Cookie Jar.
    let response = hosted
        .invoke(
            "send_owner_request",
            json!({
                "runId": "run-1",
                "collectionId": col_id,
                "requestId": req_id
            }),
        )
        .await
        .unwrap();
    assert_eq!(response["status"], 200);
    let body_text = response["body"].as_str().unwrap();
    assert!(body_text.contains(PLANTED_TOKEN));
    assert!(body_text.contains(PLANTED_COOKIE));
    assert_eq!(response["cookies"][0]["name"], "srv_sid");
    assert_eq!(response["cookies"][0]["value"], PLANTED_RESPONSE_COOKIE);

    // Cookie Jar now holds the new Set-Cookie from the server.
    let state = hosted.invoke("get_state", Value::Null).await.unwrap();
    assert!(
        state["cookies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "srv_sid" && c["value"] == PLANTED_RESPONSE_COOKIE)
    );

    // Redirects are never followed automatically.
    let redir_resp = hosted
        .invoke(
            "send_owner_request",
            json!({
                "runId": "run-2",
                "collectionId": col_id,
                "draft": {
                    "name": "Redirect",
                    "method": "GET",
                    "urlTemplate": "/redirect"
                }
            }),
        )
        .await
        .unwrap();
    assert_eq!(redir_resp["status"], 302);

    // Plain HTTP to a public host with credentials is refused.
    let insecure = hosted
        .invoke(
            "send_owner_request",
            json!({
                "runId": "run-3",
                "collectionId": col_id,
                "draft": {
                    "name": "Public HTTP",
                    "method": "GET",
                    "urlTemplate": "http://api.example.com/secret"
                }
            }),
        )
        .await;
    assert!(insecure.is_err(), "plain http with credentials must fail");
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end approval, conversation grant, origin-switch re-approval and redaction sequence"
)]
async fn agent_tools_enforce_approvals_endpoint_grants_origin_changes_and_full_redaction() {
    let base_url_1 = start_echo_server().await;
    let base_url_2 = start_echo_server().await;
    let secrets = Arc::new(FakeSecrets::new());
    let dialog = Arc::new(FakeFileDialog::default());
    let hosted = hosted_with(&secrets, &dialog).await;
    let (col_id, env_prod_id, req_id) = seed_payments_api(&hosted, &base_url_1, &base_url_2).await;

    // Mentions list and resolve collections and requests without secrets.
    let suggestions = hosted
        .core
        .runtime
        .suggest_mentions(SuggestMentions::new(
            MentionPlace::NewConversation { project_id: None },
            "Payments",
        ))
        .await
        .unwrap();
    assert!(
        suggestions
            .groups
            .iter()
            .any(|g| g.provider == NETWORK && !g.entries.is_empty())
    );

    // Scripted model calls:
    // 1. network_get_request (read-only, runs without approval)
    // 2. network_send_request #1 (asks -> owner grants for conversation)
    // 3. network_send_request #2 with different path param (same origin -> covered by grant!)
    // 4. network_send_request #3 on Server Two environment (different origin -> asks again!)
    hosted.core.model.push_reply(tool_calls(&[(
        "call_get",
        "network_get_request",
        &call_args(json!({ "request_id": req_id })),
    )]));
    hosted.core.model.push_reply(tool_calls(&[(
        "call_send_1",
        "network_send_request",
        &call_args(json!({ "request_id": req_id, "path_params": { "id": "ch_1" } })),
    )]));
    hosted.core.model.push_reply(tool_calls(&[(
        "call_send_2",
        "network_send_request",
        &call_args(json!({ "request_id": req_id, "path_params": { "id": "ch_2" } })),
    )]));
    hosted.core.model.push_reply(tool_calls(&[(
        "call_send_3",
        "network_send_request",
        &call_args(json!({
            "request_id": req_id,
            "environment_id": env_prod_id,
            "path_params": { "id": "ch_3" }
        })),
    )]));
    hosted.core.model.push_reply(reply("All charges checked."));

    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Run charges on Payments API"))
        .await
        .unwrap();
    let conversation_id = started.conversation_id;

    // Wait for first approval on call_send_1.
    let transcript_1 = hosted
        .core
        .wait_until(&conversation_id, |transcript| {
            waiting_call(transcript).is_some()
        })
        .await;
    let (part_1, request_1, call_1) =
        waiting_call(&transcript_1).expect("first network_send_request asks for approval");

    assert_eq!(request_1.reasons, vec![ApprovalReason::NetworkRequest]);
    assert!(request_1.rule.is_none());
    assert!(matches!(
        request_1.grants.first(),
        Some(GrantScope::NetworkRequest {
            plugin,
            collection_id,
            request_id,
            protocol,
            method,
            origin,
            ..
        }) if plugin == NETWORK
            && collection_id == &col_id
            && request_id == &req_id
            && protocol == "http"
            && method == "POST"
            && origin == &base_url_1
    ));
    assert!(matches!(
        call_1.accesses.as_slice(),
        [Access::NetworkRequest { origin, .. }] if origin == &base_url_1
    ));
    let approval_json = serde_json::to_string(&call_1).unwrap();
    assert!(!approval_json.contains(PLANTED_TOKEN));
    assert!(!approval_json.contains(PLANTED_COOKIE));

    // Allow for conversation -> call_send_1 AND call_send_2 run; call_send_3
    // hits base_url_2 (different origin) and MUST ask again!
    hosted
        .core
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation_id.clone(),
            part_id: part_1.clone(),
            digest: request_1.digest,
            choice: ApprovalChoice::AllowForConversation,
        })
        .await
        .unwrap();

    let transcript_3 = hosted
        .core
        .wait_until(&conversation_id, |transcript| {
            waiting_call(transcript).is_some_and(|(id, ..)| id != part_1)
        })
        .await;
    let (part_3, request_3, call_3) =
        waiting_call(&transcript_3).expect("switching environment to a new origin must ask again");

    assert!(matches!(
        call_3.accesses.as_slice(),
        [Access::NetworkRequest { origin, .. }] if origin == &base_url_2
    ));

    hosted
        .core
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation_id.clone(),
            part_id: part_3,
            digest: request_3.digest,
            choice: ApprovalChoice::AllowOnce,
        })
        .await
        .unwrap();

    let final_transcript = hosted
        .core
        .wait_for_run(&conversation_id, &started.run_id)
        .await;

    // Inspect the entire transcript and all model requests:
    // 1. System prompt includes the network-collections preference instruction.
    let sent_requests = hosted.core.model.requests();
    assert!(
        sent_requests
            .iter()
            .any(|req| req.instructions.contains("Payments API")
                && req.instructions.contains("prefer network_list_collections")),
        "system prompt should advertise enabled collections and prefer network_* over curl"
    );

    // 2. call_send_2 was allowed automatically by the conversation grant.
    let calls: Vec<&ToolCallView> = final_transcript
        .turns
        .values()
        .flat_map(|t| t.parts.values())
        .filter_map(|p| match &p.content {
            PartContent::ToolCall { call } => Some(call.as_ref()),
            _other => None,
        })
        .collect();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[2].decision, Some(Decision::Grant));

    // 3. ZERO planted secrets or cookies appear anywhere in the transcript,
    //    tool results, or model prompt messages — they appear as `[redacted]`.
    let transcript_dump = format!("{final_transcript:?}");
    assert!(!transcript_dump.contains(PLANTED_TOKEN));
    assert!(!transcript_dump.contains(PLANTED_COOKIE));
    assert!(!transcript_dump.contains(PLANTED_RESPONSE_COOKIE));
    assert!(transcript_dump.contains("[redacted]"));

    let model_dump = format!("{sent_requests:?}");
    assert!(!model_dump.contains(PLANTED_TOKEN));
    assert!(!model_dump.contains(PLANTED_COOKIE));
    assert!(!model_dump.contains(PLANTED_RESPONSE_COOKIE));
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "integration test seeds two collections and verifies ambiguous names, origin override refusal and URL secret extraction"
)]
async fn agent_refuses_origin_redirect_ambiguous_names_and_extracts_secret_url_params() {
    let base_url = start_echo_server().await;
    let secrets = Arc::new(FakeSecrets::new());
    let dialog = Arc::new(FakeFileDialog::default());
    let hosted = hosted_with(&secrets, &dialog).await;

    let alpha = hosted
        .invoke(
            "create_collection",
            json!({
                "name": "Alpha API",
                "baseUrl": "{{api_base}}",
                "enabled": true,
                "variables": [
                    { "key": "api_base", "value": base_url, "enabled": true, "secret": false }
                ]
            }),
        )
        .await
        .unwrap();
    let alpha_id = alpha["id"].as_str().unwrap().to_owned();

    let beta = hosted
        .invoke(
            "create_collection",
            json!({
                "name": "Beta API",
                "baseUrl": base_url,
                "enabled": true
            }),
        )
        .await
        .unwrap();
    let beta_id = beta["id"].as_str().unwrap().to_owned();

    let url_secret = "sk_secret_in_pasted_url_44444";
    let alpha_req = hosted
        .invoke(
            "create_request",
            json!({
                "collectionId": alpha_id,
                "input": {
                    "name": "Lookup",
                    "method": "GET",
                    "urlTemplate": format!("/items?limit=5&api_key={url_secret}")
                }
            }),
        )
        .await
        .unwrap();
    let alpha_req_id = alpha_req["id"].as_str().unwrap().to_owned();
    assert_eq!(alpha_req["urlTemplate"], "/items?limit=5");
    assert!(
        alpha_req["queryParams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["key"] == "api_key" && p["secret"] == true && p["value"] == url_secret)
    );
    assert_no_secret_bytes_on_disk(hosted.core.data_folder(), &[url_secret]);

    hosted
        .invoke(
            "create_request",
            json!({
                "collectionId": beta_id,
                "input": {
                    "name": "Lookup",
                    "method": "GET",
                    "urlTemplate": "/other"
                }
            }),
        )
        .await
        .unwrap();

    // 1. Ambiguous request name across collections is refused.
    // 2. Variable override attempting to redirect origin is refused.
    // 3. Get request never exposes the extracted url secret.
    hosted.core.model.push_reply(tool_calls(&[
        (
            "call_ambig",
            "network_send_request",
            &call_args(json!({ "request_id": "Lookup" })),
        ),
        (
            "call_redir",
            "network_send_request",
            &call_args(json!({
                "request_id": alpha_req_id,
                "variables": { "api_base": "https://attacker.example" }
            })),
        ),
        (
            "call_get",
            "network_get_request",
            &call_args(json!({ "request_id": alpha_req_id })),
        ),
    ]));
    hosted.core.model.push_reply(reply("Checked."));

    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Check safety"))
        .await
        .unwrap();
    let transcript = hosted
        .core
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let calls: Vec<&ToolCallView> = transcript
        .turns
        .values()
        .flat_map(|t| t.parts.values())
        .filter_map(|p| match &p.content {
            PartContent::ToolCall { call } => Some(call.as_ref()),
            _other => None,
        })
        .collect();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].status, ToolCallStatus::Completed);
    assert!(calls[0].decision.is_none());
    assert_eq!(calls[1].status, ToolCallStatus::Completed);
    assert!(calls[1].decision.is_none());
    assert_eq!(calls[2].status, ToolCallStatus::Completed);

    let dump = format!("{transcript:?}");
    assert!(!dump.contains(url_secret));
    assert!(dump.contains("Multiple saved requests match"));
    assert!(dump.contains("cannot change the target host"));
}

#[tokio::test]
async fn failed_update_preserves_stored_credentials_and_host_only_cookies() {
    let secrets = Arc::new(FakeSecrets::new());
    let dialog = Arc::new(FakeFileDialog::default());
    let hosted = hosted_with(&secrets, &dialog).await;

    let col_a = hosted
        .invoke(
            "create_collection",
            json!({
                "name": "Billing",
                "baseUrl": "https://billing.example.com",
                "enabled": true,
                "auth": { "kind": "bearer", "token": PLANTED_TOKEN }
            }),
        )
        .await
        .unwrap();
    let col_a_id = col_a["id"].as_str().unwrap().to_owned();

    hosted
        .invoke(
            "create_collection",
            json!({
                "name": "Orders",
                "baseUrl": "https://orders.example.com",
                "enabled": true
            }),
        )
        .await
        .unwrap();

    // Attempt a duplicate-name update carrying a different token; SQLite
    // rejects it and the previously committed token must survive restart.
    let rejected_token = "sk_rejected_update_token_00000";
    let err = hosted
        .invoke(
            "update_collection",
            json!({
                "collectionId": col_a_id,
                "input": {
                    "name": "Orders",
                    "baseUrl": "https://billing.example.com",
                    "enabled": true,
                    "auth": { "kind": "bearer", "token": rejected_token }
                }
            }),
        )
        .await;
    assert!(err.is_err(), "duplicate collection name must fail");

    hosted.turn_off().await;
    hosted.turn_on().await;

    let reloaded = hosted.invoke("get_state", Value::Null).await.unwrap();
    let billing = reloaded["collections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == col_a_id)
        .unwrap();
    assert_eq!(billing["auth"]["token"], PLANTED_TOKEN);

    // Host-only cookies persist their hostOnly flag across restart.
    let saved_cookie = hosted
        .invoke(
            "save_cookie",
            json!({
                "name": "host_sid",
                "value": PLANTED_COOKIE,
                "domain": "api.example.com",
                "path": "/",
                "hostOnly": true,
                "secure": true,
                "httpOnly": true,
                "enabled": true
            }),
        )
        .await
        .unwrap();
    assert_eq!(saved_cookie["hostOnly"], true);

    hosted.turn_off().await;
    hosted.turn_on().await;

    let after_cookie_reload = hosted.invoke("get_state", Value::Null).await.unwrap();
    assert!(
        after_cookie_reload["cookies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "host_sid" && c["hostOnly"] == true)
    );
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "verifies large IPC payloads, body hash TOCTOU check, lost-response uncertainty and 2 MiB archive capping"
)]
async fn large_payloads_body_digest_toctou_lost_response_and_archive_cap_hold() {
    let base_url = start_echo_server().await;
    let secrets = Arc::new(FakeSecrets::new());
    let dialog = Arc::new(FakeFileDialog::default());
    let hosted = hosted_with(&secrets, &dialog).await;

    // 1. Large OpenAPI paste (>150 KiB) succeeds through host IPC.
    let big_desc = "d".repeat(160 * 1024);
    let big_spec = json!({
        "openapi": "3.0.3",
        "info": { "title": "Large Spec", "description": big_desc },
        "servers": [{ "url": base_url }],
        "paths": { "/ping": { "get": { "summary": "Ping" } } }
    })
    .to_string();
    let preview = hosted
        .invoke("preview_openapi_text", json!({ "text": big_spec }))
        .await
        .unwrap();
    assert_eq!(preview["title"], "Large Spec");

    let col = hosted
        .invoke(
            "create_collection",
            json!({
                "name": "Edge Server",
                "baseUrl": base_url,
                "enabled": true
            }),
        )
        .await
        .unwrap();
    let col_id = col["id"].as_str().unwrap().to_owned();

    // 2. Large request body (>150 KiB) saves through host IPC.
    let prefix = "z".repeat(600);
    let body_a = format!("{{\"payload\":\"{prefix}A\"}}");
    let body_b = format!("{{\"payload\":\"{prefix}B\"}}");
    let big_body = format!("{{\"blob\":\"{}\"}}", "b".repeat(160 * 1024));
    let _large_req = hosted
        .invoke(
            "create_request",
            json!({
                "collectionId": col_id,
                "input": {
                    "name": "Large Upload",
                    "method": "POST",
                    "urlTemplate": "/echo",
                    "body": { "kind": "json", "content": big_body }
                }
            }),
        )
        .await
        .unwrap();

    let req_toctou = hosted
        .invoke(
            "create_request",
            json!({
                "collectionId": col_id,
                "input": {
                    "name": "Mutated Body",
                    "method": "POST",
                    "urlTemplate": "/echo",
                    "body": { "kind": "json", "content": body_a }
                }
            }),
        )
        .await
        .unwrap();
    let req_toctou_id = req_toctou["id"].as_str().unwrap().to_owned();

    let req_drop = hosted
        .invoke(
            "create_request",
            json!({
                "collectionId": col_id,
                "input": {
                    "name": "Drop Mid Body",
                    "method": "POST",
                    "urlTemplate": "/drop-mid-body"
                }
            }),
        )
        .await
        .unwrap();
    let req_drop_id = req_drop["id"].as_str().unwrap().to_owned();

    let req_huge = hosted
        .invoke(
            "create_request",
            json!({
                "collectionId": col_id,
                "input": {
                    "name": "Huge Body",
                    "method": "GET",
                    "urlTemplate": "/huge-response"
                }
            }),
        )
        .await
        .unwrap();
    let req_huge_id = req_huge["id"].as_str().unwrap().to_owned();

    hosted.core.model.push_reply(tool_calls(&[(
        "call_toctou",
        "network_send_request",
        &call_args(json!({ "request_id": req_toctou_id })),
    )]));
    hosted.core.model.push_reply(tool_calls(&[(
        "call_drop",
        "network_send_request",
        &call_args(json!({ "request_id": req_drop_id })),
    )]));
    hosted.core.model.push_reply(tool_calls(&[(
        "call_huge",
        "network_send_request",
        &call_args(json!({ "request_id": req_huge_id })),
    )]));
    hosted.core.model.push_reply(reply("Done."));

    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Run edge cases"))
        .await
        .unwrap();
    let conv_id = started.conversation_id;

    // 3. Before approving call_toctou, edit the request body past byte 512
    //    keeping identical byte length. The core's approval digest includes
    //    the body SHA-256 so it must re-prompt with a new digest instead of
    //    sending the mutated body under the old one-time approval.
    let t1 = hosted
        .core
        .wait_until(&conv_id, |tr| waiting_call(tr).is_some())
        .await;
    let (p1, r1, _) = waiting_call(&t1).unwrap();
    hosted
        .invoke(
            "update_request",
            json!({
                "requestId": req_toctou_id,
                "input": {
                    "name": "Mutated Body",
                    "method": "POST",
                    "urlTemplate": "/echo",
                    "body": { "kind": "json", "content": body_b }
                }
            }),
        )
        .await
        .unwrap();
    hosted
        .core
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conv_id.clone(),
            part_id: p1.clone(),
            digest: r1.digest.clone(),
            choice: ApprovalChoice::AllowOnce,
        })
        .await
        .unwrap();

    let t1_reprompt = hosted
        .core
        .wait_until(&conv_id, |tr| {
            waiting_call(tr).is_some_and(|(id, req, _)| id == p1 && req.digest != r1.digest)
        })
        .await;
    let (_, r1_new, _) = waiting_call(&t1_reprompt).unwrap();
    assert_ne!(r1_new.digest, r1.digest);
    hosted
        .core
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conv_id.clone(),
            part_id: p1.clone(),
            digest: r1_new.digest,
            choice: ApprovalChoice::AllowOnce,
        })
        .await
        .unwrap();

    // 4. Approve call_drop -> server closes mid-body -> ToolCallStatus::Uncertain!
    let t2 = hosted
        .core
        .wait_until(&conv_id, |tr| {
            waiting_call(tr).is_some_and(|(id, ..)| id != p1)
        })
        .await;
    let (p2, r2, _) = waiting_call(&t2).unwrap();
    hosted
        .core
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conv_id.clone(),
            part_id: p2.clone(),
            digest: r2.digest,
            choice: ApprovalChoice::AllowOnce,
        })
        .await
        .unwrap();

    // 5. Approve call_huge -> 2 MiB body + headers capped cleanly into archive.
    let t3 = hosted
        .core
        .wait_until(&conv_id, |tr| {
            waiting_call(tr).is_some_and(|(id, ..)| id != p1 && id != p2)
        })
        .await;
    let (p3, r3, _) = waiting_call(&t3).unwrap();
    hosted
        .core
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conv_id.clone(),
            part_id: p3,
            digest: r3.digest,
            choice: ApprovalChoice::AllowOnce,
        })
        .await
        .unwrap();

    let final_tr = hosted.core.wait_for_run(&conv_id, &started.run_id).await;
    let calls: Vec<&ToolCallView> = final_tr
        .turns
        .values()
        .flat_map(|t| t.parts.values())
        .filter_map(|p| match &p.content {
            PartContent::ToolCall { call } => Some(call.as_ref()),
            _other => None,
        })
        .collect();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].status, ToolCallStatus::Completed);
    assert_eq!(calls[1].status, ToolCallStatus::Uncertain);
    assert!(calls[1].interruption.is_some());
    assert_eq!(calls[2].status, ToolCallStatus::Completed);
    let huge_result = calls[2].result.as_ref().expect("huge response result");
    assert_eq!(huge_result.archives.len(), 1);
}
