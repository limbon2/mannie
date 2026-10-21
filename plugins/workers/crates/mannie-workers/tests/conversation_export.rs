//! Full saved worker records over the authenticated link (ADR-0040).

use mannie_plugin_harness::runtime::{reply, tool_calls};
use mannie_plugin_sdk::contracts::{ExportError, MAX_EXPORT_CHUNK_BYTES, TaskState};
use mannie_workers_testing::master::{delegation, paired};
use mannie_workers_testing::worker::LinkedWorkerOptions;

#[tokio::test(flavor = "multi_thread")]
async fn complete_worker_records_cross_multiple_chunks_and_keep_the_master_parent() {
    let (mut worker, master) = paired(LinkedWorkerOptions::default()).await;
    let long_reply = "worker content é\n".repeat(50_000);
    worker.harness.model.push_reply(reply(&long_reply));
    worker.harness.model.push_reply(reply("worker report"));
    let arguments = delegation("export the complete saved worker conversation");
    master
        .model
        .push_reply(tool_calls(&[("delegate", "delegate_task", &arguments)]));
    master.model.push_reply(reply("done"));
    let parent = master.start_conversation("export a task").await;
    let tasks = master
        .tasks_when(&parent, |tasks| {
            tasks.first().is_some_and(|task| !task.state.is_active())
        })
        .await;
    let child = &tasks[0].conversation_id;
    assert_eq!(tasks[0].state, TaskState::Reported);
    let record = master.runtime.export_record(child).await.unwrap();
    assert_eq!(record.conversation.id, *child);
    assert_eq!(record.conversation.task.as_ref().unwrap().parent_id, parent);
    assert!(serde_json::to_vec(&record).unwrap().len() > MAX_EXPORT_CHUNK_BYTES * 2);
    assert!(
        record
            .turns
            .iter()
            .flat_map(|turn| &turn.parts)
            .any(|part| part.text == long_reply)
    );
    let local = worker
        .harness
        .runtime
        .assigned_export_record(child)
        .await
        .unwrap();
    assert_eq!(record.through_seq, local.through_seq);
    assert_eq!(record.turns, local.turns);
    assert_eq!(
        worker.harness.runtime.assigned_export_record(&parent).await,
        Err(ExportError::ConversationNotFound)
    );
    worker.stop().await;
    assert!(matches!(
        master.runtime.export_record(child).await,
        Err(ExportError::HostUnavailable)
    ));
    master.executor.stop();
    assert_eq!(
        master.runtime.export_record(child).await,
        Err(ExportError::PluginOff)
    );
}
