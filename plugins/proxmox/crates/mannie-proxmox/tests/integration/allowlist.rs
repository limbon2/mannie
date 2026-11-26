//! Fail-closed per-server VM allowlist enforcement (D74 / ADR-0080):
//! - Newly registered servers expose zero VMs.
//! - LXC containers are never listed or allowed.
//! - Unselected and newly created VMs are hidden from discovery, mentions, and
//!   refused with `vm_not_allowed` across every config/power/VNC tool & IPC.
//! - Unchecking a VM immediately closes its active VNC sessions.

use std::sync::atomic::Ordering;
use std::time::Duration;

use mannie_plugin_harness::runtime::{reply, tool_calls};
use mannie_plugin_harness::types::{
    MentionPlace, StartConversation, SuggestMentions, ToolCallStatus, ToolOutcome,
};
use mannie_plugin_sdk::testing::FakeSecrets;
use mannie_proxmox::{
    TOOL_ATTACH_ISO, TOOL_GET_VM_CONFIG, TOOL_LIST_ISOS, TOOL_LIST_VMS, TOOL_SET_BOOT_ORDER,
    TOOL_VM_POWER, TOOL_VNC_SCREENSHOT,
};
use serde_json::json;

use super::support::{
    FakeProxmoxServer, FakeVm, hosted_plugin, new_conv_id, register_pinned_server,
    valid_self_signed_cert,
};

#[expect(
    clippy::too_many_lines,
    reason = "end-to-end allowlist test exercises discovery, mentions, all 7 tools, and IPC refusal in one scenario"
)]
#[tokio::test]
async fn unselected_and_newly_created_vms_and_lxc_are_hidden_and_refused_everywhere() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, true).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(20)).await;

    // Step 1: Newly pinned server with 0 allowed VMs exposes zero VMs.
    let server = register_pinned_server(&hosted, "AllowLab", &fake, &[]).await;
    let server_id = server["id"].as_str().unwrap();
    assert_eq!(server["status"], "no_vms_allowed");

    // discover_server_vms returns the 3 QEMU VMs and filters out LXC container 999.
    let discoverable = hosted
        .invoke("discover_server_vms", json!({ "serverId": server_id }))
        .await
        .expect("discover_server_vms");
    let vmids: Vec<u64> = discoverable
        .as_array()
        .unwrap()
        .iter()
        .map(|vm| vm["vmid"].as_u64().unwrap())
        .collect();
    assert_eq!(vmids, vec![100, 101, 200]);

    // Step 2: Explicitly allow ONLY VM 100 (`debian-lab` on `pve1`).
    // VM 101 (same node), VM 200 (`pve2`), and newly created VM 300 stay disallowed.
    hosted
        .invoke(
            "set_allowed_vms",
            json!({
                "serverId": server_id,
                "vms": [{ "vmid": 100, "node": "pve1", "name": "debian-lab" }],
            }),
        )
        .await
        .expect("allow VM 100 only");

    // Add a brand-new VM 300 on the Proxmox server after allowlisting.
    fake.state
        .lock()
        .unwrap()
        .vms
        .push(FakeVm::new(300, "pve1", "fresh-vm", "running"));

    // Mentions list only the server and explicitly allowed VM 100 — never 101, 200, or 300.
    let suggestions = hosted
        .core
        .runtime
        .suggest_mentions(
            SuggestMentions::new(MentionPlace::NewConversation { project_id: None }, "")
                .with_provider("proxmox"),
        )
        .await
        .expect("suggest_mentions");
    let rendered_suggestions = format!("{suggestions:?}");
    assert!(rendered_suggestions.contains("AllowLab"));
    assert!(rendered_suggestions.contains("debian-lab"));
    assert!(!rendered_suggestions.contains("windows-sandbox"));
    assert!(!rendered_suggestions.contains("prod-secret-db"));
    assert!(!rendered_suggestions.contains("fresh-vm"));

    // Agent calls proxmox_list_vms -> sees ONLY VM 100; calls tools on VM 101, 200, 300 -> ALL refused!
    let a1 = json!({ "activity": "List VMs", "server": "AllowLab" }).to_string();
    let a2 =
        json!({ "activity": "Read disallowed 101", "server": "AllowLab", "vmid": 101 }).to_string();
    let a3 = json!({
        "activity": "ISOs on disallowed node pve2",
        "server": "AllowLab",
        "node": "pve2"
    })
    .to_string();
    let a4 = json!({
        "activity": "Attach ISO to 200",
        "server": "AllowLab",
        "vmid": 200,
        "volid": "local:iso/debian-13-netinst.iso"
    })
    .to_string();
    let a5 = json!({
        "activity": "Boot order on fresh 300",
        "server": "AllowLab",
        "vmid": 300,
        "order": ["ide2", "scsi0"]
    })
    .to_string();
    let a6 = json!({
        "activity": "Stop disallowed 101",
        "server": "AllowLab",
        "vmid": 101,
        "action": "stop"
    })
    .to_string();
    let a7 = json!({ "activity": "VNC on disallowed 200", "server": "AllowLab", "vmid": 200 })
        .to_string();

    hosted.core.model.push_reply(tool_calls(&[
        ("c1", TOOL_LIST_VMS, &a1),
        ("c2", TOOL_GET_VM_CONFIG, &a2),
        ("c3", TOOL_LIST_ISOS, &a3),
        ("c4", TOOL_ATTACH_ISO, &a4),
        ("c5", TOOL_SET_BOOT_ORDER, &a5),
        ("c6", TOOL_VM_POWER, &a6),
        ("c7", TOOL_VNC_SCREENSHOT, &a7),
    ]));
    hosted.core.model.push_reply(reply("Checked allowlist"));

    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Inspect Proxmox VMs"))
        .await
        .expect("start_conversation");
    let transcript = hosted
        .core
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let calls = transcript.tool_calls(0);
    assert_eq!(calls.len(), 7);

    // c1 (proxmox_list_vms): succeeds and lists ONLY VM 100.
    assert_eq!(calls[0].status, ToolCallStatus::Completed);
    let list_res = calls[0].result.as_ref().unwrap();
    assert!(matches!(
        &list_res.outcome,
        ToolOutcome::Plugin { failure: None, .. }
    ));
    assert!(list_res.output.contains("100"));
    assert!(list_res.output.contains("debian-lab"));
    assert!(!list_res.output.contains("windows-sandbox"));
    assert!(!list_res.output.contains("prod-secret-db"));
    assert!(!list_res.output.contains("fresh-vm"));

    // c2..c7: every disallowed VM / node tool call is refused before execution!
    for call in &calls[1..] {
        let refused_or_failed = call.status == ToolCallStatus::NotRun
            || matches!(
                call.result.as_ref().map(|r| &r.outcome),
                Some(
                    ToolOutcome::Plugin {
                        failure: Some(_),
                        ..
                    } | ToolOutcome::Failed { .. }
                )
            );
        assert!(
            refused_or_failed,
            "expected refusal or plugin failure for tool {}: status={:?} result={:?}",
            call.tool, call.status, call.result
        );
    }

    // Also verify IPC open_vnc_session refuses disallowed VM 200.
    let vnc_err = hosted
        .invoke(
            "open_vnc_session",
            json!({
                "conversationId": started.conversation_id.to_string(),
                "serverId": server_id,
                "vmid": 200,
            }),
        )
        .await
        .expect_err("open_vnc_session refuses disallowed VM");
    assert_eq!(vnc_err.code(), "proxmox.vm_not_allowed");
}

#[tokio::test]
async fn unchecking_a_vm_from_allowlist_immediately_closes_its_active_vnc_session() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, false).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(21)).await;
    let server = register_pinned_server(&hosted, "RevokeLab", &fake, &[100, 101]).await;
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
        .expect("open VM 100 VNC");
    hosted
        .invoke(
            "open_vnc_session",
            json!({
                "conversationId": conv_id.to_string(),
                "serverId": server_id,
                "vmid": 101,
            }),
        )
        .await
        .expect("open VM 101 VNC");
    assert_eq!(fake.active_vnc_sockets.load(Ordering::SeqCst), 2);

    // Uncheck VM 100, keeping only VM 101.
    hosted
        .invoke(
            "set_allowed_vms",
            json!({
                "serverId": server_id,
                "vms": [{ "vmid": 101, "node": "pve1", "name": "windows-sandbox" }],
            }),
        )
        .await
        .expect("revoke VM 100");

    let sessions = hosted
        .invoke(
            "list_vnc_sessions",
            json!({ "conversationId": conv_id.to_string() }),
        )
        .await
        .expect("list_vnc_sessions");
    let arr = sessions.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["vmid"], 101);

    // Wait for VM 100's WebSocket to close on the fake server.
    tokio::time::timeout(Duration::from_secs(3), async {
        while fake.active_vnc_sockets.load(Ordering::SeqCst) != 1 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("revoked VM VNC socket closed");
}
