//! Manifest, SDK conformance, settings CRUD, live channels, work reporting,
//! conversation-deletion teardown, and idle VNC session reaper tests.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::StreamExt as _;
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::HostOptions;
use mannie_plugin_sdk::ipc::ChannelMessage;
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::testing::FakeSecrets;
use mannie_proxmox::commands::interface;
use mannie_proxmox::{MODULE, PLUGIN_MANIFEST, PROXMOX, WORK_VNC_SESSIONS, mannie_plugin};
use serde_json::{Value, json};

use super::support::{
    FakeProxmoxServer, PASSWORD, USERNAME, hosted_plugin, hosted_plugin_with_idle_timeout,
    new_conv_id, register_pinned_server, valid_self_signed_cert,
};

#[test]
fn manifest_declares_proxmox_store_secrets_and_interface() {
    let manifest = Manifest::parse(PLUGIN_MANIFEST).unwrap();

    assert_eq!(manifest.id.as_str(), PROXMOX);
    assert!(manifest.declares("store"));
    assert!(manifest.declares("secrets"));
    assert_eq!(manifest.secrets.keyed, ["server"]);
    assert_eq!(MODULE.name, manifest.id.prefix());
    for migration in MODULE.migrations {
        assert!(migration.sql().contains("proxmox_servers"));
        assert!(migration.sql().contains("proxmox_allowed_vms"));
    }

    let declared = interface();
    let exported = mannie_plugin().plugin.interface();
    assert_eq!(declared.command_names(), exported.command_names());
    assert_eq!(
        declared.channel_names(),
        ["follow_servers", "follow_vnc_session"]
    );
}

#[tokio::test]
async fn sdk_conformance_passes_without_findings() {
    let secrets = FakeSecrets::new();
    let options = HostOptions::new().with_provider(Arc::new(secrets));

    let findings = conformance(mannie_plugin(), options).await;

    assert_eq!(findings, Vec::<String>::new());
}

#[tokio::test]
async fn adding_a_server_keeps_password_in_keyring_exposes_zero_vms_and_forbids_duplicate_names() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, false).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(1)).await;

    let added = hosted
        .invoke(
            "add_server",
            json!({
                "name": "HomeLab",
                "host": "127.0.0.1",
                "port": fake.port(),
                "username": USERNAME,
                "password": PASSWORD,
                "note": "Primary homelab cluster",
            }),
        )
        .await
        .expect("add_server");

    let id = added["id"].as_str().unwrap();
    assert_eq!(added["name"], "HomeLab");
    assert_eq!(added["status"], "no_vms_allowed");
    assert_eq!(added["allowedVms"].as_array().unwrap().len(), 0);
    assert_eq!(added["passwordSet"], true);
    assert_eq!(
        secrets
            .stored(&format!("proxmox/server/{id}"))
            .unwrap()
            .expose(),
        PASSWORD
    );
    assert!(!added.to_string().contains(PASSWORD));
    assert_eq!(fake.auth_attempts(), 0);

    // Case-insensitive duplicate name is rejected.
    let dup_err = hosted
        .invoke(
            "add_server",
            json!({
                "name": "homelab",
                "host": "127.0.0.1",
                "port": fake.port(),
                "username": USERNAME,
                "password": PASSWORD,
            }),
        )
        .await
        .expect_err("duplicate name fails");
    assert_eq!(dup_err.code(), "proxmox.name_taken");

    // Removing deletes stored password.
    hosted
        .invoke("remove_server", json!({ "serverId": id }))
        .await
        .expect("remove_server");
    assert!(secrets.stored(&format!("proxmox/server/{id}")).is_none());
}

#[tokio::test]
async fn follow_servers_streams_initial_and_subsequent_updates() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, false).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(2)).await;

    let mut stream = hosted
        .open_channel("follow_servers", Value::Null)
        .await
        .expect("follow_servers");

    let first = stream.next().await.unwrap();
    match first {
        ChannelMessage::Item { item } => assert_eq!(item["servers"].as_array().unwrap().len(), 0),
        other @ ChannelMessage::Ended { .. } => {
            panic!("expected initial server snapshot, got {other:?}")
        }
    }

    let _server = register_pinned_server(&hosted, "Lab", &fake, &[100]).await;

    let mut saw_ready = false;
    for _ in 0..6 {
        if let Some(ChannelMessage::Item { item }) = stream.next().await {
            let servers = item["servers"].as_array().unwrap();
            if servers.len() == 1 && servers[0]["status"] == "ready" {
                assert_eq!(servers[0]["allowedVms"].as_array().unwrap().len(), 1);
                saw_ready = true;
                break;
            }
        }
    }
    assert!(saw_ready, "channel streamed updated ready server");
}

#[tokio::test]
async fn conversations_deleted_closes_active_vnc_sessions_and_clears_work_items() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, false).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(3)).await;
    let server = register_pinned_server(&hosted, "Lab", &fake, &[100]).await;
    let server_id = server["id"].as_str().unwrap();

    let conv_a = new_conv_id();
    let conv_b = new_conv_id();

    hosted
        .invoke(
            "open_vnc_session",
            json!({
                "conversationId": conv_a.to_string(),
                "serverId": server_id,
                "vmid": 100,
            }),
        )
        .await
        .expect("open conv_a vnc");
    hosted
        .invoke(
            "open_vnc_session",
            json!({
                "conversationId": conv_b.to_string(),
                "serverId": server_id,
                "vmid": 100,
            }),
        )
        .await
        .expect("open conv_b vnc");

    let work = hosted.host.work(PROXMOX).await.expect("work");
    assert_eq!(work.items.len(), 1);
    assert_eq!(work.items[0].code, WORK_VNC_SESSIONS);
    assert_eq!(work.items[0].count, 2);

    // Deleting conv_a closes only conv_a's session.
    hosted
        .host
        .conversations_deleted(std::slice::from_ref(&conv_a))
        .await;

    let remaining_a = hosted
        .invoke(
            "list_vnc_sessions",
            json!({ "conversationId": conv_a.to_string() }),
        )
        .await
        .expect("list conv_a");
    assert_eq!(remaining_a.as_array().unwrap().len(), 0);

    let remaining_b = hosted
        .invoke(
            "list_vnc_sessions",
            json!({ "conversationId": conv_b.to_string() }),
        )
        .await
        .expect("list conv_b");
    assert_eq!(remaining_b.as_array().unwrap().len(), 1);

    let work_after = hosted.host.work(PROXMOX).await.expect("work");
    assert_eq!(work_after.items[0].count, 1);

    // Deleting conv_b closes the last session.
    hosted.host.conversations_deleted(&[conv_b]).await;
    let work_empty = hosted.host.work(PROXMOX).await.expect("work");
    assert_eq!(work_empty.items, []);
}

#[tokio::test]
async fn idle_vnc_session_is_reaped_after_inactivity() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin_with_idle_timeout(&secrets, Duration::from_millis(120)).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(4)).await;
    let server = register_pinned_server(&hosted, "IdleLab", &fake, &[100]).await;
    let server_id = server["id"].as_str().unwrap();
    let conv_id = new_conv_id();

    hosted
        .invoke(
            "open_vnc_session",
            json!({
                "conversationId": conv_id.to_string(),
                "serverId": server_id,
                "vmid": 100,
            }),
        )
        .await
        .expect("open vnc");

    assert_eq!(hosted.host.work(PROXMOX).await.unwrap().items.len(), 1);

    // Wait for the reaper to close the idle session and drop its WebSocket.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let work = hosted.host.work(PROXMOX).await.unwrap();
            if work.items.is_empty() && fake.active_vnc_sockets.load(Ordering::SeqCst) == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("idle session reaped");
}
