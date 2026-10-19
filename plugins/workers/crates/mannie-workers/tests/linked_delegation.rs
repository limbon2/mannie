//! Delegation over the link (V6; plugins/workers/docs/delegation.md,
//! credential-leases.md): a task delegated through the master's runtime runs
//! on the worker and reports; its lease never reaches the worker's files and
//! the redactor forgets it; the owner answers the worker's approval through
//! the link, and the worker refuses a choice that it did not offer; a
//! conversation in full access answers the worker's request (ADR-0029); the
//! task's conversation is read through the link; the worker stops while the
//! master follows a task; a link that drops during a task comes back, and the
//! master learns the task's end once; an approval waits while the master is
//! offline; a worker one minor version behind works and others do not.

#![cfg(test)]

use std::time::Duration;

use mannie_plugin_harness::provider::Step;
use mannie_plugin_harness::provider::{ProviderEvent, Role};
use mannie_plugin_harness::runtime::texts;
use mannie_plugin_harness::runtime::{WorkerOptions, fast_config, reply, tool_calls};
use mannie_plugin_harness::types::FinishReason;
use mannie_plugin_harness::types::SubscribeConversation;
use mannie_plugin_harness::types::TaskRefusal;
use mannie_plugin_harness::types::{Decision, UpdateToolsSettings};
use mannie_plugin_sdk::contracts::CeilingMode;
use mannie_plugin_sdk::contracts::ConversationUpdate;
use mannie_plugin_sdk::contracts::TaskState;
use mannie_plugin_sdk::contracts::{ApprovalChoice, ToolsError};
use mannie_workers_testing::master::{
    LinkedMaster, PLANTED_CREDENTIAL, delegation, files_contain, paired,
};
use mannie_workers_testing::worker::{LinkedWorker, LinkedWorkerOptions, Relay};

use mannie_workers::view::WorkerStatus;
use serde_json::json;

fn last_tool_result(master: &LinkedMaster) -> String {
    let requests = master.model.reply_requests();
    let last = requests.last().unwrap();
    last.messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .flat_map(|message| &message.content)
        .find_map(|content| match content {
            mannie_plugin_harness::provider::Content::ToolResult { output, .. } => {
                Some(output.clone())
            }
            _ => None,
        })
        .unwrap()
}

#[tokio::test]
async fn a_task_delegated_over_the_link_reports_and_its_lease_leaves_no_trace() {
    let (worker, master) = paired(LinkedWorkerOptions::default()).await;
    let marker = worker.harness.allowed.join("marker.txt");
    let write =
        json!({ "activity": "Writing", "path": marker.to_str().unwrap(), "content": "built\n" })
            .to_string();
    master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegation("Write the marker, then report."),
    )]));
    master.model.push_reply(reply("The worker wrote it."));
    worker
        .harness
        .model
        .push_reply(tool_calls(&[("call_w", "write_file", &write)]));
    worker.harness.model.push_reply(reply("Written."));
    worker
        .harness
        .model
        .push_reply(reply("Report: the marker is written."));

    let conversation = master
        .start_conversation("Write the marker on the server")
        .await;
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| task.approval.is_some())
        })
        .await;
    let forged = master.decide(&tasks[0], ApprovalChoice::AllowAlways).await;
    master.allow_once(&tasks[0]).await.unwrap();
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| !task.state.is_active())
        })
        .await;
    master.wait_for_idle(&conversation).await;

    assert_eq!(
        forged,
        Err(ToolsError::GrantNotPossible),
        "the worker refuses a choice that its request does not offer"
    );
    assert_eq!(tasks[0].state, TaskState::Reported);
    assert_eq!(
        tasks[0].report.as_deref(),
        Some("Report: the marker is written.")
    );
    assert_eq!(
        last_tool_result(&master),
        texts::task_reported(1, "Report: the marker is written.")
    );
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "built\n");
    assert_eq!(
        worker
            .harness
            .model
            .credentials()
            .first()
            .cloned()
            .flatten()
            .as_deref(),
        Some(PLANTED_CREDENTIAL),
        "the worker's run used the lease"
    );
    assert!(worker.leases.is_empty(), "the lease went with the task");
    assert_eq!(
        worker.redactor.registered_count(),
        0,
        "the redactor forgot it"
    );
    assert!(
        !files_contain(&worker.data(), PLANTED_CREDENTIAL),
        "no file of the worker's data folder holds the credential"
    );
}

#[tokio::test]
async fn a_conversation_in_full_access_answers_the_workers_request_over_the_link() {
    let (worker, master) = paired(LinkedWorkerOptions::default()).await;
    master
        .runtime
        .update_tools_settings(UpdateToolsSettings {
            allow_changes_in_workspace: false,
            command_environment: Vec::new(),
            full_access_for_new_conversations: true,
        })
        .await
        .unwrap();
    let marker = worker.harness.allowed.join("marker.txt");
    let write =
        json!({ "activity": "Writing", "path": marker.to_str().unwrap(), "content": "built\n" })
            .to_string();
    master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegation("Write the marker, then report."),
    )]));
    master.model.push_reply(reply("The worker wrote it."));
    worker
        .harness
        .model
        .push_reply(tool_calls(&[("call_w", "write_file", &write)]));
    worker.harness.model.push_reply(reply("Written."));
    worker
        .harness
        .model
        .push_reply(reply("Report: the marker is written."));

    let conversation = master
        .start_conversation("Write the marker on the server")
        .await;
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| !task.state.is_active())
        })
        .await;
    master.wait_for_idle(&conversation).await;

    assert_eq!(tasks[0].state, TaskState::Reported);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "built\n");
    let decisions: Vec<_> = worker
        .harness
        .store
        .model_turns(&tasks[0].conversation_id, None)
        .await
        .unwrap()
        .iter()
        .flat_map(|turn| &turn.parts)
        .filter_map(|part| part.tool_call.as_ref())
        .map(|call| call.decision)
        .collect();
    assert_eq!(
        decisions,
        [Some(Decision::FullAccess)],
        "the worker records the master's automatic answer"
    );
}

#[tokio::test]
async fn the_tasks_conversation_is_read_over_the_link() {
    let (worker, master) = paired(LinkedWorkerOptions::default()).await;
    master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegation("Count the files."),
    )]));
    master.model.push_reply(reply("Counted."));
    worker.harness.model.push_reply(reply("Three files."));
    worker
        .harness
        .model
        .push_reply(reply("Report: three files."));
    let conversation = master.start_conversation("Count").await;
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| !task.state.is_active())
        })
        .await;
    master.wait_for_idle(&conversation).await;

    let mut subscription = master
        .runtime
        .subscribe(SubscribeConversation {
            conversation_id: tasks[0].conversation_id.clone(),
            after_seq: None,
        })
        .await
        .unwrap();
    let first = subscription.next().await.unwrap();

    let ConversationUpdate::Snapshot { snapshot } = first else {
        panic!("a snapshot comes first: {first:?}");
    };
    assert_eq!(
        snapshot
            .conversation
            .task
            .and_then(|task| task.executor)
            .map(|executor| executor.target_name),
        Some("build-01".to_owned())
    );
    assert_eq!(snapshot.turns.len(), 1);
}

#[tokio::test]
async fn the_worker_stops_while_its_master_follows_a_task() {
    let (mut worker, master) = paired(LinkedWorkerOptions::default()).await;
    master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegation("Build it."),
    )]));
    master.model.push_reply(reply("Built."));
    worker.harness.model.push_reply(vec![Step::Hang]);
    let conversation = master.start_conversation("Build").await;
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks
                .first()
                .is_some_and(|task| task.state == TaskState::Running)
        })
        .await;
    // The master has watched the worker's tasks since it connected, and a
    // side panel follows the task's conversation: two streams that stay open.
    let mut subscription = master
        .runtime
        .subscribe(SubscribeConversation {
            conversation_id: tasks[0].conversation_id.clone(),
            after_seq: None,
        })
        .await
        .unwrap();
    let _snapshot = subscription.next().await.unwrap();

    tokio::time::timeout(Duration::from_secs(10), worker.stop())
        .await
        .expect("the worker's link stops within 10 seconds while the master follows a task");
}

#[tokio::test]
async fn a_link_that_drops_during_a_task_comes_back_and_the_end_arrives_once() {
    let worker = LinkedWorker::start().await;
    let relay = Relay::start(&worker.address).await;
    let master = LinkedMaster::start().await;
    master
        .pair(&relay.address, &worker.new_code())
        .await
        .unwrap();
    master.wait_for_status(WorkerStatus::Connected).await;
    master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegation("Build slowly."),
    )]));
    master.model.push_reply(reply("Built after the drop."));
    worker.harness.model.push_reply(vec![
        Step::Delay(Duration::from_millis(1_500)),
        Step::Event(ProviderEvent::TextDelta("Built.".to_owned())),
        Step::Event(ProviderEvent::Finished(FinishReason::Stop)),
    ]);
    worker
        .harness
        .model
        .push_reply(reply("Report: built once."));
    let conversation = master.start_conversation("Build").await;
    master
        .tasks_when(&conversation, |tasks| {
            tasks
                .first()
                .is_some_and(|task| task.state == TaskState::Running)
        })
        .await;

    relay.set_blocked(true);
    relay.cut();
    master.wait_for_status(WorkerStatus::Offline).await;
    relay.set_blocked(false);
    master.wait_for_status(WorkerStatus::Connected).await;
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| !task.state.is_active())
        })
        .await;
    master.wait_for_idle(&conversation).await;

    assert_eq!(tasks[0].state, TaskState::Reported);
    assert_eq!(tasks[0].report.as_deref(), Some("Report: built once."));
    assert_eq!(
        worker.harness.model.reply_requests().len(),
        2,
        "the task ran once, with its confirmation"
    );
}

#[tokio::test]
async fn a_worker_one_minor_version_behind_works_and_an_older_one_does_not() {
    let master_version = "0.5.1";
    for (worker_version, expected) in [
        ("0.5.0", WorkerStatus::Connected),
        ("0.4.9", WorkerStatus::Connected),
        ("0.3.2", WorkerStatus::Incompatible),
        ("0.6.0", WorkerStatus::Incompatible),
    ] {
        let worker = LinkedWorker::with_options(LinkedWorkerOptions {
            version: worker_version.to_owned(),
            ..LinkedWorkerOptions::default()
        })
        .await;
        let master = LinkedMaster::with_version(master_version, fast_config()).await;

        master
            .pair(&worker.address, &worker.new_code())
            .await
            .unwrap();
        let view = master.wait_for_status(expected).await;

        assert_eq!(view.status, expected, "{worker_version}");
        if expected == WorkerStatus::Incompatible {
            master.model.push_reply(tool_calls(&[(
                "call_1",
                "delegate_task",
                &delegation("Build it."),
            )]));
            master.model.push_reply(reply("It needs an update."));
            let conversation = master.start_conversation("Build").await;
            master.wait_for_idle(&conversation).await;
            let snapshot = master
                .store
                .snapshot(&conversation, 10, 1_000)
                .await
                .unwrap()
                .unwrap();
            let refusal = snapshot.turns[0]
                .parts
                .iter()
                .find_map(|part| match &part.content {
                    mannie_plugin_harness::types::PartContent::Task { call } => {
                        call.refusal.clone()
                    }
                    _ => None,
                });
            assert_eq!(
                refusal,
                Some(TaskRefusal::Executor {
                    kind: mannie_workers::WORKERS_KIND.to_owned(),
                    code: "worker_incompatible".to_owned(),
                    target: Some("build-01".to_owned()),
                }),
                "{worker_version}"
            );
        }
    }
}

#[tokio::test]
async fn a_read_only_host_refuses_a_write_of_a_delegated_task() {
    let (worker, master) = paired(LinkedWorkerOptions {
        worker: WorkerOptions {
            mode: CeilingMode::ReadOnly,
            ..WorkerOptions::default()
        },
        ..LinkedWorkerOptions::default()
    })
    .await;
    let target = worker.harness.allowed.join("never.txt");
    let write =
        json!({ "activity": "Writing", "path": target.to_str().unwrap(), "content": "x\n" })
            .to_string();
    master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegation("Write it, even though you may not."),
    )]));
    master.model.push_reply(reply("The host only reads."));
    worker
        .harness
        .model
        .push_reply(tool_calls(&[("call_w", "write_file", &write)]));
    worker.harness.model.push_reply(reply("I could not write."));
    worker
        .harness
        .model
        .push_reply(reply("Report: the host only reads."));

    let conversation = master.start_conversation("Write it").await;
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| !task.state.is_active())
        })
        .await;
    master.wait_for_idle(&conversation).await;

    assert_eq!(tasks[0].state, TaskState::Reported);
    assert!(!target.exists());
    let worker_requests = worker.harness.model.reply_requests();
    let refused = worker_requests[1]
        .messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .flat_map(|message| &message.content)
        .find_map(|content| match content {
            mannie_plugin_harness::provider::Content::ToolResult { output, .. } => {
                Some(output.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(refused, texts::READ_ONLY_HOST);
}

#[tokio::test]
async fn an_approval_waits_while_the_master_is_offline() {
    let worker = LinkedWorker::start().await;
    let relay = Relay::start(&worker.address).await;
    let master = LinkedMaster::start().await;
    master
        .pair(&relay.address, &worker.new_code())
        .await
        .unwrap();
    master.wait_for_status(WorkerStatus::Connected).await;
    let marker = worker.harness.allowed.join("marker.txt");
    let write =
        json!({ "activity": "Writing", "path": marker.to_str().unwrap(), "content": "built\n" })
            .to_string();
    master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegation("Write the marker, then report."),
    )]));
    master.model.push_reply(reply("The worker wrote it."));
    worker
        .harness
        .model
        .push_reply(tool_calls(&[("call_w", "write_file", &write)]));
    worker.harness.model.push_reply(reply("Written."));
    worker
        .harness
        .model
        .push_reply(reply("Report: the marker is written."));
    let conversation = master
        .start_conversation("Write the marker on the server")
        .await;
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| task.approval.is_some())
        })
        .await;
    let task = tasks[0].task_id.clone();

    relay.set_blocked(true);
    relay.cut();
    master.wait_for_status(WorkerStatus::Offline).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let while_offline = worker
        .harness
        .runtime
        .assigned_task(&task)
        .await
        .unwrap()
        .unwrap();
    relay.set_blocked(false);
    master.wait_for_status(WorkerStatus::Connected).await;
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| task.approval.is_some())
        })
        .await;
    master.allow_once(&tasks[0]).await.unwrap();
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| !task.state.is_active())
        })
        .await;
    master.wait_for_idle(&conversation).await;

    assert_eq!(while_offline.state, TaskState::Running);
    assert!(
        while_offline.approval.is_some(),
        "the request waits on the worker"
    );
    assert_eq!(tasks[0].state, TaskState::Reported);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "built\n");
}
