//! End-to-end tool loop tests through `mannie-plugin-harness`:
//! - Full OS-install workflow (list allowed VMs -> get config -> list ISOs ->
//!   attach ISO -> set boot order -> power reboot -> VNC screenshot PNG ->
//!   VNC pointer & keyboard -> close VNC) with scoped approvals and grant reuse.
//! - Zlib and `CopyRect` RFB encodings.
//! - Owner takeover blocking ALL agent VNC calls (including screenshots & close)
//!   even under conversation full access (ADR-0029 + ADR-0080).
//! - Owner input refused while agent has control, accepted after takeover.
//! - Planted-secret audit proving passwords, `PVEAuthCookie`, `CSRFPreventionToken`,
//!   and vncticket never appear in transcripts, approvals, outputs, or IPC views.

use futures_util::StreamExt as _;
use mannie_plugin_harness::host::HostedPlugin;
use mannie_plugin_harness::runtime::{SeenPart, Transcript, reply, tool_calls};
use mannie_plugin_harness::types::{
    Access, ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision, PartContent,
    PartId, ProxmoxVmScope, SendMessage, SetFullAccess, StartConversation, ToolCallStatus,
    ToolOutcome,
};
use mannie_plugin_sdk::contracts::ConversationId;
use mannie_plugin_sdk::ipc::ChannelMessage;
use mannie_plugin_sdk::testing::FakeSecrets;
use mannie_proxmox::{
    TOOL_ATTACH_ISO, TOOL_EJECT_ISO, TOOL_GET_VM_CONFIG, TOOL_LIST_ISOS, TOOL_LIST_VMS,
    TOOL_SET_BOOT_ORDER, TOOL_VM_POWER, TOOL_VNC_CLOSE, TOOL_VNC_KEYBOARD, TOOL_VNC_POINTER,
    TOOL_VNC_SCREENSHOT,
};
use serde_json::json;

use super::support::{
    CSRF, FakeProxmoxServer, PASSWORD, RfbMode, TICKET, VNC_TICKET, hosted_plugin,
    register_pinned_server, valid_self_signed_cert,
};

fn waiting_call(transcript: &Transcript, turn: u32) -> Option<(PartId, ApprovalRequest)> {
    transcript
        .parts(turn)
        .into_iter()
        .find_map(|part| match part {
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

async fn approve(
    hosted: &HostedPlugin,
    conversation_id: &ConversationId,
    turn: u32,
    choice: ApprovalChoice,
) -> ApprovalRequest {
    let transcript = hosted
        .core
        .wait_until(conversation_id, |t| waiting_call(t, turn).is_some())
        .await;
    let (part_id, request) = waiting_call(&transcript, turn).unwrap();
    hosted
        .core
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation_id.clone(),
            part_id: part_id.clone(),
            digest: request.digest.clone(),
            choice,
        })
        .await
        .expect("decide_approval");
    hosted
        .core
        .wait_until(conversation_id, |t| {
            waiting_call(t, turn).is_none_or(|(waiting, _)| waiting != part_id)
        })
        .await;
    request
}

#[expect(
    clippy::too_many_lines,
    reason = "end-to-end OS-install integration test exercises all 11 tools, scoped approvals, grant reuse, and secret audit"
)]
#[tokio::test]
async fn full_os_install_workflow_with_scoped_grants_vnc_and_planted_secret_audit() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, true).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(30)).await;
    let server = register_pinned_server(&hosted, "InstallLab", &fake, &[100]).await;

    let a1 = json!({ "activity": "List VMs", "server": "InstallLab" }).to_string();
    let a2 =
        json!({ "activity": "Get VM 100 config", "server": "InstallLab", "vmid": 100 }).to_string();
    let a3 = json!({ "activity": "List ISOs for VM 100", "server": "InstallLab", "vmid": 100 })
        .to_string();
    let a4 = json!({
        "activity": "Attach Debian ISO",
        "server": "InstallLab",
        "vmid": 100,
        "volid": "local:iso/debian-13-netinst.iso",
        "device": "ide2"
    })
    .to_string();
    let a5 = json!({
        "activity": "Set boot order",
        "server": "InstallLab",
        "vmid": 100,
        "order": ["ide2", "scsi0"]
    })
    .to_string();
    let a6 = json!({
        "activity": "Eject ISO",
        "server": "InstallLab",
        "vmid": 100,
        "device": "ide2"
    })
    .to_string();
    let a7 = json!({
        "activity": "Reboot VM 100",
        "server": "InstallLab",
        "vmid": 100,
        "action": "reboot"
    })
    .to_string();
    let a8 = json!({
        "activity": "Inspect installer screen",
        "server": "InstallLab",
        "vmid": 100
    })
    .to_string();
    let a9 = json!({
        "activity": "Click Continue",
        "server": "InstallLab",
        "vmid": 100,
        "action": "click",
        "x": 20,
        "y": 15,
        "wait_for_change_ms": 1500
    })
    .to_string();
    let a10 = json!({
        "activity": "Type hostname and press Enter",
        "server": "InstallLab",
        "vmid": 100,
        "text": "deb",
        "keys": ["Return"],
        "wait_for_change_ms": 1500
    })
    .to_string();
    let a11 = json!({
        "activity": "Close console",
        "server": "InstallLab",
        "vmid": 100
    })
    .to_string();

    hosted.core.model.push_reply(tool_calls(&[
        ("t1", TOOL_LIST_VMS, &a1),
        ("t2", TOOL_GET_VM_CONFIG, &a2),
        ("t3", TOOL_LIST_ISOS, &a3),
    ]));
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("t4", TOOL_ATTACH_ISO, &a4)]));
    hosted.core.model.push_reply(tool_calls(&[
        ("t5", TOOL_SET_BOOT_ORDER, &a5),
        ("t6", TOOL_EJECT_ISO, &a6),
    ]));
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("t7", TOOL_VM_POWER, &a7)]));
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("t8", TOOL_VNC_SCREENSHOT, &a8)]));
    hosted.core.model.push_reply(tool_calls(&[
        ("t9", TOOL_VNC_POINTER, &a9),
        ("t10", TOOL_VNC_KEYBOARD, &a10),
    ]));
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("t11", TOOL_VNC_CLOSE, &a11)]));
    hosted
        .core
        .model
        .push_reply(reply("OS install steps completed."));

    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Install Debian on VM 100"))
        .await
        .expect("start_conversation");
    let conv_id = started.conversation_id;

    // 1. Config approval for attach_iso
    let config_req = approve(&hosted, &conv_id, 0, ApprovalChoice::AllowForConversation).await;
    assert_eq!(config_req.reasons, vec![ApprovalReason::ProxmoxVm]);

    // 2. Power approval for vm_power (t5 & t6 reused the Config grant without prompting!)
    let power_req = approve(&hosted, &conv_id, 0, ApprovalChoice::AllowForConversation).await;
    assert_eq!(power_req.reasons, vec![ApprovalReason::ProxmoxVm]);

    // 3. VNC approval for vnc_screenshot (t9, t10, t11 reuse the Vnc grant!)
    let vnc_req = approve(&hosted, &conv_id, 0, ApprovalChoice::AllowForConversation).await;
    assert_eq!(vnc_req.reasons, vec![ApprovalReason::ProxmoxVnc]);

    let transcript = hosted.core.wait_for_run(&conv_id, &started.run_id).await;
    let calls = transcript.tool_calls(0);
    assert_eq!(calls.len(), 11);
    for call in &calls {
        assert_eq!(
            call.status,
            ToolCallStatus::Completed,
            "tool {} status: {:?}",
            call.tool,
            call.result
        );
        assert!(
            matches!(
                call.result.as_ref().map(|r| &r.outcome),
                Some(ToolOutcome::Plugin { failure: None, .. })
            ),
            "tool {} failed: {:?}",
            call.tool,
            call.result
        );
    }

    // Verify exact access scopes & grant reuse decisions.
    assert!(matches!(
        &calls[3].accesses[0],
        Access::ProxmoxVm {
            server_label,
            vmid: 100,
            scope: ProxmoxVmScope::Config,
            ..
        } if server_label == "InstallLab"
    ));
    assert_eq!(calls[4].decision, Some(Decision::Grant));
    assert_eq!(calls[5].decision, Some(Decision::Grant));
    assert!(matches!(
        &calls[6].accesses[0],
        Access::ProxmoxVm {
            vmid: 100,
            scope: ProxmoxVmScope::Power,
            ..
        }
    ));
    assert!(matches!(
        &calls[7].accesses[0],
        Access::ProxmoxVm {
            vmid: 100,
            scope: ProxmoxVmScope::Vnc,
            ..
        }
    ));
    assert_eq!(calls[8].decision, Some(Decision::Grant));
    assert_eq!(calls[9].decision, Some(Decision::Grant));
    assert_eq!(calls[10].decision, Some(Decision::Grant));

    // Verify all 3 VNC calls attached real PNG screenshots via ADR-0044.
    for idx in [7usize, 8, 9] {
        let images = &calls[idx].result.as_ref().unwrap().images;
        assert_eq!(images.len(), 1, "call {idx} produced 1 PNG screenshot");
        assert_eq!(images[0].width, 64);
        assert_eq!(images[0].height, 48);
        assert!(images[0].bytes > 0);
    }

    // Verify backend Proxmox state mutations & RFB input events actually occurred.
    {
        let st = fake.state.lock().unwrap();
        assert_eq!(st.config_updates.len(), 3);
        assert_eq!(st.power_actions, vec![(100, "reboot".to_owned())]);
        assert!(
            st.vnc_pointer_events
                .iter()
                .any(|&(v, x, y, mask)| v == 100 && x == 20 && y == 15 && mask == 1),
            "click down recorded: {:?}",
            st.vnc_pointer_events
        );
        assert!(
            st.vnc_key_events
                .iter()
                .any(|&(v, ks, down)| v == 100 && ks == 0xff0d && down),
            "Return key recorded: {:?}",
            st.vnc_key_events
        );
    }

    // Planted-secret audit: NONE of PASSWORD, TICKET, CSRF, VNC_TICKET may leak anywhere!
    let all_text = format!("{calls:?} {config_req:?} {power_req:?} {vnc_req:?} {server:?}");
    for secret in [PASSWORD, TICKET, CSRF, VNC_TICKET] {
        assert!(
            !all_text.contains(secret),
            "planted secret leaked into transcript/approval/IPC output!"
        );
    }

    // Verify VNC session was closed by t11.
    let open_after = hosted
        .invoke(
            "list_vnc_sessions",
            json!({ "conversationId": conv_id.to_string() }),
        )
        .await
        .expect("list_vnc_sessions");
    assert_eq!(open_after.as_array().unwrap().len(), 0);
}

#[expect(
    clippy::too_many_lines,
    reason = "end-to-end takeover integration test verifies full access, agent->owner->agent handoff, and input isolation"
)]
#[tokio::test]
async fn owner_takeover_blocks_all_agent_vnc_tools_even_under_conversation_full_access() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, true).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(31)).await;
    let server = register_pinned_server(&hosted, "TakeoverLab", &fake, &[100]).await;
    let server_id = server["id"].as_str().unwrap().to_owned();

    // Turn 0: start conversation, then enable Conversation Full Access (ADR-0029).
    hosted.core.model.push_reply(reply("Ready"));
    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Hello"))
        .await
        .expect("start_conversation");
    let conv_id = started.conversation_id;
    hosted.core.wait_for_run(&conv_id, &started.run_id).await;

    hosted
        .core
        .runtime
        .set_full_access(SetFullAccess {
            conversation_id: conv_id.clone(),
            enabled: true,
        })
        .await
        .expect("enable full access");

    // Turn 1: Under full access, agent opens VNC screenshot automatically without prompting.
    let open_arg =
        json!({ "activity": "Open VNC", "server": "TakeoverLab", "vmid": 100 }).to_string();
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("v1", TOOL_VNC_SCREENSHOT, &open_arg)]));
    hosted.core.model.push_reply(reply("Opened console"));
    let run1 = hosted
        .core
        .runtime
        .send_message(SendMessage::new(conv_id.clone(), "Open VNC"))
        .await
        .expect("send turn 1")
        .run_id;
    let t1 = hosted.core.wait_for_run(&conv_id, &run1).await;
    let [t1_call] = t1.tool_calls(1).try_into().unwrap();
    assert_eq!(t1_call.status, ToolCallStatus::Completed);
    assert!(matches!(
        t1_call.result.as_ref().map(|r| &r.outcome),
        Some(ToolOutcome::Plugin { failure: None, .. })
    ));

    // While agent holds control, owner pointer/keyboard/Ctrl+Alt+Del MUST be refused with agent_in_control.
    let err_ptr = hosted
        .invoke(
            "vnc_pointer",
            json!({
                "conversationId": conv_id.to_string(),
                "serverId": server_id,
                "vmid": 100,
                "x": 10,
                "y": 10,
                "buttonMask": 1,
            }),
        )
        .await
        .expect_err("owner pointer blocked while agent controls");
    assert_eq!(err_ptr.code(), "proxmox.agent_in_control");

    let err_cad = hosted
        .invoke(
            "vnc_send_ctrl_alt_del",
            json!({
                "conversationId": conv_id.to_string(),
                "serverId": server_id,
                "vmid": 100,
            }),
        )
        .await
        .expect_err("owner CAD blocked while agent controls");
    assert_eq!(err_cad.code(), "proxmox.agent_in_control");

    // Subscribe to the live VNC channel and verify initial Snapshot arrives.
    let mut vnc_stream = hosted
        .open_channel(
            "follow_vnc_session",
            json!({
                "conversationId": conv_id.to_string(),
                "serverId": server_id,
                "vmid": 100,
            }),
        )
        .await
        .expect("follow_vnc_session");
    let snap = vnc_stream.next().await.unwrap();
    match snap {
        ChannelMessage::Item { item } => {
            assert_eq!(item["kind"], "snapshot");
            assert_eq!(item["session"]["control"], "agent");
            assert_ne!(item["pngBase64"].as_str().unwrap(), "");
        }
        other @ ChannelMessage::Ended { .. } => panic!("expected snapshot, got {other:?}"),
    }

    // Owner clicks "Take over" -> control becomes "owner".
    let summary = hosted
        .invoke(
            "set_vnc_control",
            json!({
                "conversationId": conv_id.to_string(),
                "serverId": server_id,
                "vmid": 100,
                "control": "owner",
            }),
        )
        .await
        .expect("owner take over");
    assert_eq!(summary["control"], "owner");

    // Owner input now succeeds!
    hosted
        .invoke(
            "vnc_keyboard",
            json!({
                "conversationId": conv_id.to_string(),
                "serverId": server_id,
                "vmid": 100,
                "keysym": 0x0061,
                "down": true,
            }),
        )
        .await
        .expect("owner key allowed");
    hosted
        .invoke(
            "vnc_send_ctrl_alt_del",
            json!({
                "conversationId": conv_id.to_string(),
                "serverId": server_id,
                "vmid": 100,
            }),
        )
        .await
        .expect("owner Ctrl+Alt+Del allowed");

    // Turn 2: EVEN UNDER FULL ACCESS, ALL agent VNC tools (screenshot, pointer, keyboard, close)
    // MUST fail with `owner_in_control`!
    let b1 =
        json!({ "activity": "Sneak screenshot", "server": "TakeoverLab", "vmid": 100 }).to_string();
    let b2 = json!({
        "activity": "Sneak click",
        "server": "TakeoverLab",
        "vmid": 100,
        "action": "click",
        "x": 5,
        "y": 5
    })
    .to_string();
    let b3 = json!({
        "activity": "Sneak keys",
        "server": "TakeoverLab",
        "vmid": 100,
        "text": "x"
    })
    .to_string();
    let b4 = json!({ "activity": "Sneak close", "server": "TakeoverLab", "vmid": 100 }).to_string();
    hosted.core.model.push_reply(tool_calls(&[
        ("b1", TOOL_VNC_SCREENSHOT, &b1),
        ("b2", TOOL_VNC_POINTER, &b2),
        ("b3", TOOL_VNC_KEYBOARD, &b3),
        ("b4", TOOL_VNC_CLOSE, &b4),
    ]));
    hosted
        .core
        .model
        .push_reply(reply("Waiting for owner to resume"));
    let run2 = hosted
        .core
        .runtime
        .send_message(SendMessage::new(
            conv_id.clone(),
            "Try VNC while owner types",
        ))
        .await
        .expect("send turn 2")
        .run_id;
    let t2 = hosted.core.wait_for_run(&conv_id, &run2).await;
    let t2_calls = t2.tool_calls(2);
    assert_eq!(t2_calls.len(), 4);
    for call in &t2_calls {
        let failure_code = match call.result.as_ref().map(|r| &r.outcome) {
            Some(ToolOutcome::Plugin { failure, .. }) => failure.as_deref(),
            _ => None,
        };
        assert_eq!(
            failure_code,
            Some("owner_in_control"),
            "tool {} expected owner_in_control, got {:?}",
            call.tool,
            call.result
        );
    }

    // Owner clicks "Resume agent" -> agent VNC screenshot works again!
    let resumed = hosted
        .invoke(
            "set_vnc_control",
            json!({
                "conversationId": conv_id.to_string(),
                "serverId": server_id,
                "vmid": 100,
                "control": "agent",
            }),
        )
        .await
        .expect("resume agent");
    assert_eq!(resumed["control"], "agent");

    let r1_arg = json!({
        "activity": "Post-resume screenshot",
        "server": "TakeoverLab",
        "vmid": 100
    })
    .to_string();
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("r1", TOOL_VNC_SCREENSHOT, &r1_arg)]));
    hosted.core.model.push_reply(reply("Resumed cleanly"));
    let run3 = hosted
        .core
        .runtime
        .send_message(SendMessage::new(conv_id.clone(), "Continue"))
        .await
        .expect("send turn 3")
        .run_id;
    let t3 = hosted.core.wait_for_run(&conv_id, &run3).await;
    let [r1] = t3.tool_calls(3).try_into().unwrap();
    assert_eq!(r1.status, ToolCallStatus::Completed);
    assert!(matches!(
        r1.result.as_ref().map(|r| &r.outcome),
        Some(ToolOutcome::Plugin { failure: None, .. })
    ));
}

#[tokio::test]
async fn rfb_zlib_and_copyrect_encodings_render_clean_png_screenshots() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, true).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(32)).await;
    {
        let mut st = fake.state.lock().unwrap();
        st.vms[0].rfb_mode = RfbMode::Zlib;
        st.vms[1].rfb_mode = RfbMode::CopyRect;
    }
    let _server = register_pinned_server(&hosted, "EncLab", &fake, &[100, 101]).await;

    let z1 = json!({ "activity": "Zlib shot", "server": "EncLab", "vmid": 100 }).to_string();
    let c1 = json!({ "activity": "CopyRect shot", "server": "EncLab", "vmid": 101 }).to_string();
    hosted.core.model.push_reply(tool_calls(&[
        ("z1", TOOL_VNC_SCREENSHOT, &z1),
        ("c1", TOOL_VNC_SCREENSHOT, &c1),
    ]));
    hosted
        .core
        .model
        .push_reply(reply("Both encodings decoded"));

    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Check VNC encodings"))
        .await
        .expect("start");
    let conv_id = started.conversation_id;

    // Approve both VM 100 and VM 101 VNC access.
    approve(&hosted, &conv_id, 0, ApprovalChoice::AllowOnce).await;
    approve(&hosted, &conv_id, 0, ApprovalChoice::AllowOnce).await;

    let transcript = hosted.core.wait_for_run(&conv_id, &started.run_id).await;
    let calls = transcript.tool_calls(0);
    assert_eq!(calls.len(), 2);
    for call in &calls {
        assert_eq!(call.status, ToolCallStatus::Completed);
        assert!(matches!(
            call.result.as_ref().map(|r| &r.outcome),
            Some(ToolOutcome::Plugin { failure: None, .. })
        ));
        assert_eq!(call.result.as_ref().unwrap().images.len(), 1);
    }
}

#[tokio::test]
async fn typing_shifted_shell_punctuation_holds_shift_around_each_symbol() {
    const SHIFT_L: u32 = 0xffe1;

    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, true).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(33)).await;
    let _server = register_pinned_server(&hosted, "KeysLab", &fake, &[100]).await;

    let k1 = json!({
        "activity": "Type shifted command",
        "server": "KeysLab",
        "vmid": 100,
        "text": "a:%{b_c}|d",
        "screenshot": false
    })
    .to_string();
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("k1", TOOL_VNC_KEYBOARD, &k1)]));
    hosted.core.model.push_reply(reply("Typed command"));

    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Type shell symbols"))
        .await
        .expect("start");
    let conv_id = started.conversation_id;

    approve(&hosted, &conv_id, 0, ApprovalChoice::AllowOnce).await;

    let transcript = hosted.core.wait_for_run(&conv_id, &started.run_id).await;
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);

    let expected = vec![
        (u32::from(b'a'), true),
        (u32::from(b'a'), false),
        (SHIFT_L, true),
        (u32::from(b':'), true),
        (u32::from(b':'), false),
        (SHIFT_L, false),
        (SHIFT_L, true),
        (u32::from(b'%'), true),
        (u32::from(b'%'), false),
        (SHIFT_L, false),
        (SHIFT_L, true),
        (u32::from(b'{'), true),
        (u32::from(b'{'), false),
        (SHIFT_L, false),
        (u32::from(b'b'), true),
        (u32::from(b'b'), false),
        (SHIFT_L, true),
        (u32::from(b'_'), true),
        (u32::from(b'_'), false),
        (SHIFT_L, false),
        (u32::from(b'c'), true),
        (u32::from(b'c'), false),
        (SHIFT_L, true),
        (u32::from(b'}'), true),
        (u32::from(b'}'), false),
        (SHIFT_L, false),
        (SHIFT_L, true),
        (u32::from(b'|'), true),
        (u32::from(b'|'), false),
        (SHIFT_L, false),
        (u32::from(b'd'), true),
        (u32::from(b'd'), false),
    ];
    let events = fake.wait_for_vnc_key_events(100, expected.len()).await;
    assert_eq!(events, expected);
}

#[tokio::test]
async fn list_isos_distinguishes_inaccessible_storage_from_empty_storage() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, true).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(34)).await;
    fake.state.lock().unwrap().isos.clear();
    let _server = register_pinned_server(&hosted, "IsoLab", &fake, &[100]).await;

    let list_args = json!({
        "activity": "List ISOs on pve",
        "server": "IsoLab",
        "vmid": 100
    })
    .to_string();

    // Case 1: Proxmox hides all storages from this user (0 ISO-capable storages returned).
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("i1", TOOL_LIST_ISOS, &list_args)]));
    hosted.core.model.push_reply(reply("Checked permissions"));
    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Check ISOs"))
        .await
        .expect("start");
    let conv_id = started.conversation_id;
    let t1 = hosted.core.wait_for_run(&conv_id, &started.run_id).await;
    let [call1] = t1.tool_calls(0).try_into().unwrap();
    let out1 = &call1.result.as_ref().unwrap().output;
    assert!(
        out1.contains("Datastore.Audit"),
        "expected Datastore.Audit hint when 0 storages are visible: {out1}"
    );

    // Case 2: Storage `local` is visible to the user, but holds 0 ISOs.
    fake.state
        .lock()
        .unwrap()
        .isos
        .insert("local".to_owned(), Vec::new());
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("i2", TOOL_LIST_ISOS, &list_args)]));
    hosted.core.model.push_reply(reply("Storage is empty"));
    let run2 = hosted
        .core
        .runtime
        .send_message(SendMessage::new(conv_id.clone(), "Check again"))
        .await
        .expect("send")
        .run_id;
    let t2 = hosted.core.wait_for_run(&conv_id, &run2).await;
    let [call2] = t2.tool_calls(1).try_into().unwrap();
    let out2 = &call2.result.as_ref().unwrap().output;
    assert!(
        out2.contains("checked: local"),
        "expected checked storage list when storage is accessible but empty: {out2}"
    );
}

#[tokio::test]
async fn screenshot_delay_ms_pauses_and_aborts_immediately_on_owner_takeover() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, true).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(35)).await;
    let server = register_pinned_server(&hosted, "DelayLab", &fake, &[100]).await;

    let short_delay = json!({
        "activity": "Brief delayed screenshot",
        "server": "DelayLab",
        "vmid": 100,
        "delay_ms": 200
    })
    .to_string();
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("d1", TOOL_VNC_SCREENSHOT, &short_delay)]));
    hosted.core.model.push_reply(reply("Delayed shot captured"));

    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Wait briefly then screenshot"))
        .await
        .expect("start");
    let conv_id = started.conversation_id;

    let t0 = std::time::Instant::now();
    approve(&hosted, &conv_id, 0, ApprovalChoice::AllowForConversation).await;
    let t1 = hosted.core.wait_for_run(&conv_id, &started.run_id).await;
    let elapsed = t0.elapsed();
    let [c1] = t1.tool_calls(0).try_into().unwrap();
    assert_eq!(c1.status, ToolCallStatus::Completed);
    assert!(
        elapsed >= std::time::Duration::from_millis(180),
        "delay_ms should wait unconditionally before capturing (elapsed={elapsed:?})"
    );

    // Start a 5-second delayed screenshot, then take over after 120 ms.
    // The delayed screenshot must wake immediately and fail with owner_in_control.
    let long_delay = json!({
        "activity": "Long delayed screenshot",
        "server": "DelayLab",
        "vmid": 100,
        "delay_ms": 5000
    })
    .to_string();
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("d2", TOOL_VNC_SCREENSHOT, &long_delay)]));
    hosted.core.model.push_reply(reply("Owner took over"));

    let t_takeover = std::time::Instant::now();
    let run2 = hosted
        .core
        .runtime
        .send_message(SendMessage::new(conv_id.clone(), "Wait 5 seconds"))
        .await
        .expect("send")
        .run_id;

    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    let _summary = hosted
        .invoke(
            "set_vnc_control",
            json!({
                "conversationId": conv_id.to_string(),
                "serverId": server["id"].as_str().unwrap(),
                "vmid": 100,
                "control": "owner"
            }),
        )
        .await
        .expect("set_vnc_control owner");

    let t2 = hosted.core.wait_for_run(&conv_id, &run2).await;
    let aborted_in = t_takeover.elapsed();
    let [c2] = t2.tool_calls(1).try_into().unwrap();
    assert!(
        matches!(
            c2.result.as_ref().map(|r| &r.outcome),
            Some(ToolOutcome::Plugin {
                failure: Some(code),
                ..
            }) if code == "owner_in_control"
        ),
        "expected owner_in_control during delay_ms takeover: {:?}",
        c2.result
    );
    assert!(
        aborted_in < std::time::Duration::from_secs(2),
        "delay_ms must wake promptly on takeover instead of sleeping 5s (took {aborted_in:?})"
    );
}

#[tokio::test]
async fn keyboard_and_pointer_delay_ms_pause_before_capture() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, true).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(35)).await;
    let _server = register_pinned_server(&hosted, "ActionDelayLab", &fake, &[100]).await;

    let key_delay = json!({
        "activity": "Press Enter and wait briefly",
        "server": "ActionDelayLab",
        "vmid": 100,
        "key": "Enter",
        "delay_ms": 200
    })
    .to_string();
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("k1", TOOL_VNC_KEYBOARD, &key_delay)]));
    hosted.core.model.push_reply(reply("Key delayed shot"));

    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Press Enter with delay"))
        .await
        .expect("start");
    let conv_id = started.conversation_id;

    let t0 = std::time::Instant::now();
    approve(&hosted, &conv_id, 0, ApprovalChoice::AllowForConversation).await;

    let t1 = hosted.core.wait_for_run(&conv_id, &started.run_id).await;
    let key_elapsed = t0.elapsed();
    let [c1] = t1.tool_calls(0).try_into().unwrap();
    assert_eq!(c1.status, ToolCallStatus::Completed);
    assert_eq!(c1.result.as_ref().unwrap().images.len(), 1);
    assert!(
        key_elapsed >= std::time::Duration::from_millis(180),
        "keyboard delay_ms should wait before capturing (elapsed={key_elapsed:?})"
    );

    let click_delay = json!({
        "activity": "Click and wait briefly",
        "server": "ActionDelayLab",
        "vmid": 100,
        "action": "click",
        "x": 20,
        "y": 15,
        "delay_ms": 200
    })
    .to_string();
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("p1", TOOL_VNC_POINTER, &click_delay)]));
    hosted.core.model.push_reply(reply("Pointer delayed shot"));

    let t_click = std::time::Instant::now();
    let run2 = hosted
        .core
        .runtime
        .send_message(SendMessage::new(conv_id.clone(), "Click with delay"))
        .await
        .expect("send")
        .run_id;
    let t2 = hosted.core.wait_for_run(&conv_id, &run2).await;
    let click_elapsed = t_click.elapsed();
    let [c2] = t2.tool_calls(1).try_into().unwrap();
    assert_eq!(c2.status, ToolCallStatus::Completed);
    assert_eq!(c2.result.as_ref().unwrap().images.len(), 1);
    assert!(
        click_elapsed >= std::time::Duration::from_millis(180),
        "pointer delay_ms should wait before capturing (elapsed={click_elapsed:?})"
    );
}

#[tokio::test]
async fn keyboard_delay_ms_aborts_immediately_on_owner_takeover() {
    const RETURN: u32 = 0xff0d;

    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, true).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(35)).await;
    let server = register_pinned_server(&hosted, "TakeoverDelayLab", &fake, &[100]).await;

    let long_key_delay = json!({
        "activity": "Press Enter and wait 5 seconds",
        "server": "TakeoverDelayLab",
        "vmid": 100,
        "key": "Enter",
        "delay_ms": 5000
    })
    .to_string();
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("k2", TOOL_VNC_KEYBOARD, &long_key_delay)]));
    hosted.core.model.push_reply(reply("Owner took over"));

    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Press Enter and wait 5s"))
        .await
        .expect("start");
    let conv_id = started.conversation_id;

    approve(&hosted, &conv_id, 0, ApprovalChoice::AllowForConversation).await;

    let events = fake.wait_for_vnc_key_events(100, 2).await;
    assert_eq!(events, [(RETURN, true), (RETURN, false)]);

    let t_takeover = std::time::Instant::now();
    let _summary = hosted
        .invoke(
            "set_vnc_control",
            json!({
                "conversationId": conv_id.to_string(),
                "serverId": server["id"].as_str().unwrap(),
                "vmid": 100,
                "control": "owner"
            }),
        )
        .await
        .expect("set_vnc_control owner");

    let t1 = hosted.core.wait_for_run(&conv_id, &started.run_id).await;
    let aborted_in = t_takeover.elapsed();
    let [c1] = t1.tool_calls(0).try_into().unwrap();
    assert!(
        matches!(
            c1.result.as_ref().map(|r| &r.outcome),
            Some(ToolOutcome::Plugin {
                failure: Some(code),
                ..
            }) if code == "owner_in_control"
        ),
        "expected owner_in_control during keyboard delay_ms takeover: {:?}",
        c1.result
    );
    assert!(
        aborted_in < std::time::Duration::from_secs(2),
        "keyboard delay_ms must wake promptly on takeover (took {aborted_in:?})"
    );
}

#[tokio::test]
async fn vnc_websocket_preserves_proxy_cookies_alpn_and_browser_upgrade_headers() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, true).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(36)).await;
    {
        let mut st = fake.state.lock().unwrap();
        st.ticket_set_cookies = vec![
            "SERVERID=PLANTED_PROXY_COOKIE_1122; Path=/; HttpOnly; Secure".to_owned(),
            "FOREIGN=ignored; Domain=evil.example; Path=/".to_owned(),
        ];
        st.vncproxy_set_cookies =
            vec!["VNCNODE=PLANTED_VNC_COOKIE_3344; Path=/api2/json/nodes/pve1; Secure".to_owned()];
        st.require_ws_cookies = vec![
            "SERVERID=PLANTED_PROXY_COOKIE_1122".to_owned(),
            "VNCNODE=PLANTED_VNC_COOKIE_3344".to_owned(),
        ];
    }
    let _server = register_pinned_server(&hosted, "ProxyLab", &fake, &[100]).await;

    let shot_args =
        json!({ "activity": "Capture behind proxy", "server": "ProxyLab", "vmid": 100 })
            .to_string();
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("p1", TOOL_VNC_SCREENSHOT, &shot_args)]));
    hosted.core.model.push_reply(reply("Captured behind proxy"));

    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Capture VM behind reverse proxy"))
        .await
        .expect("start");
    let conv_id = started.conversation_id;

    approve(&hosted, &conv_id, 0, ApprovalChoice::AllowOnce).await;

    let transcript = hosted.core.wait_for_run(&conv_id, &started.run_id).await;
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert!(
        matches!(
            call.result.as_ref().map(|r| &r.outcome),
            Some(ToolOutcome::Plugin { failure: None, .. })
        ),
        "VNC behind reverse proxy failed: {:?}",
        call.result
    );

    let st = fake.state.lock().unwrap();
    assert_eq!(
        st.last_ws_alpn.as_deref(),
        Some(b"http/1.1".as_slice()),
        "VNC TLS handshake must negotiate http/1.1 ALPN"
    );

    let headers = st
        .last_ws_headers
        .as_ref()
        .expect("recorded VNC WebSocket headers");
    let expected_host = format!("127.0.0.1:{}", fake.port());
    let expected_origin = format!("https://{expected_host}");
    assert_eq!(
        headers.get("host").map(String::as_str),
        Some(expected_host.as_str())
    );
    assert_eq!(
        headers.get("origin").map(String::as_str),
        Some(expected_origin.as_str())
    );
    assert!(
        headers
            .get("user-agent")
            .is_some_and(|ua| ua.starts_with("Mannie/")),
        "missing Mannie User-Agent: {headers:?}"
    );
    assert_eq!(headers.get("pragma").map(String::as_str), Some("no-cache"));
    assert_eq!(
        headers.get("cache-control").map(String::as_str),
        Some("no-cache")
    );
    let cookie = headers
        .get("cookie")
        .map(String::as_str)
        .unwrap_or_default();
    assert!(!cookie.contains("FOREIGN"), "foreign domain cookie sent");
}

#[tokio::test]
async fn vnc_non_101_gateway_error_includes_safe_diagnostics_and_scrubs_secrets() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, true).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(37)).await;
    let proxy_cookie = "PLANTED_GATEWAY_COOKIE_7788";
    {
        let mut st = fake.state.lock().unwrap();
        st.ticket_set_cookies = vec![format!("SERVERID={proxy_cookie}; Path=/; Secure")];
        st.ws_error_response = Some(format!(
            "HTTP/1.0 502 Bad Gateway\r\n\
             Server: haproxy\r\n\
             Via: 1.1 work-edge\r\n\
             Set-Cookie: LEAKED=SHOULD_NOT_SHOW\r\n\
             Content-Type: text/html\r\n\
             \r\n\
             <html><body><h1>502 Bad Gateway</h1>\n\
             The server returned an invalid or incomplete response while proxying {VNC_TICKET} {TICKET} {proxy_cookie}.\n\
             </body></html>"
        ));
    }
    let _server = register_pinned_server(&hosted, "BadGatewayLab", &fake, &[100]).await;

    let shot_args =
        json!({ "activity": "Trigger 502", "server": "BadGatewayLab", "vmid": 100 }).to_string();
    hosted
        .core
        .model
        .push_reply(tool_calls(&[("bg1", TOOL_VNC_SCREENSHOT, &shot_args)]));
    hosted.core.model.push_reply(reply("Handled 502"));

    let started = hosted
        .core
        .runtime
        .start_conversation(StartConversation::new("Capture VM with 502 gateway"))
        .await
        .expect("start");
    let conv_id = started.conversation_id;

    approve(&hosted, &conv_id, 0, ApprovalChoice::AllowOnce).await;

    let transcript = hosted.core.wait_for_run(&conv_id, &started.run_id).await;
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    let result = call.result.as_ref().expect("tool result");
    assert!(
        matches!(
            &result.outcome,
            ToolOutcome::Plugin {
                failure: Some(code),
                ..
            } if code == "vnc_failed"
        ),
        "expected vnc_failed, got {:?}",
        result.outcome
    );

    let output = &result.output;
    assert!(
        output.contains("HTTP/1.0 502 Bad Gateway"),
        "missing status line: {output}"
    );
    assert!(
        output.contains("server: haproxy"),
        "missing server header: {output}"
    );
    assert!(
        output.contains("via: 1.1 work-edge"),
        "missing via header: {output}"
    );
    assert!(
        output.contains("The server returned an invalid or incomplete response"),
        "missing body diagnostic: {output}"
    );
    for leaked in [
        PASSWORD,
        TICKET,
        CSRF,
        VNC_TICKET,
        proxy_cookie,
        "SHOULD_NOT_SHOW",
    ] {
        assert!(
            !output.contains(leaked),
            "secret {leaked} leaked into VNC error output: {output}"
        );
    }
}
