use super::*;

use mannie_runtime::testing::{Harness, reply};
use mannie_types::chat::StartConversation;
use mannie_types::export::RecordArchive;
use mannie_types::ids::ArchiveId;
use mannie_types::tasks::TaskConversation;

#[tokio::test]
async fn persisted_records_export_as_complete_relocatable_folders_with_explicit_missing_children() {
    let harness = Harness::start().await;
    harness
        .model
        .push_reply(reply("Saved answer with é and ``` code."));
    let started = harness
        .runtime
        .start_conversation(StartConversation::new("Owner's question"))
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let mut root = harness
        .runtime
        .export_record(&started.conversation_id)
        .await
        .unwrap();
    let archive = ArchiveId::parse("0199e0f2-0000-7000-8000-000000000082").unwrap();
    root.archives.push(RecordArchive {
        id: archive.clone(),
        part_id: root.turns[0].parts[0].id.clone(),
        name: "stdout".to_owned(),
        content: "complete\r\narchive\n".to_owned(),
    });
    let mut missing = root.conversation.clone();
    missing.id = ConversationId::parse("0199e0f2-0000-7000-8000-000000000081").unwrap();
    missing.title = "Unavailable child".to_owned();
    missing.task = Some(TaskConversation {
        parent_id: started.conversation_id.clone(),
        number: 1,
        agent: "general".to_owned(),
        worker: None,
        executor: None,
    });
    let child_id = missing.id.clone();
    root.children.push(missing);
    let parent = tempfile::tempdir().unwrap();
    let destination = parent.path().join("export");
    let operation = harness.runtime.new_export_id();
    harness
        .store
        .begin_export(
            &operation,
            &started.conversation_id,
            &destination.to_string_lossy(),
        )
        .await
        .unwrap();
    let result = write_export(
        &harness.runtime,
        &harness.store,
        &operation,
        destination.clone(),
        root,
    )
    .await
    .unwrap();
    let ExportOutcome::Saved { warnings, .. } = result else {
        panic!("an export was expected");
    };
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].reason, ExportError::ConversationNotFound);
    let main = std::fs::read_to_string(destination.join("conversation.md")).unwrap();
    assert!(main.contains("Owner's question"));
    assert!(main.contains("Saved answer with é"));
    assert!(main.contains("Unavailable content"));
    assert!(main.contains(&format!("tasks/{child_id}.md")));
    assert_eq!(
        std::fs::read(destination.join(format!("outputs/{archive}.txt"))).unwrap(),
        b"complete\r\narchive\n"
    );
    assert!(
        std::fs::read_to_string(destination.join(format!("tasks/{child_id}.md")))
            .unwrap()
            .contains("Unavailable content")
    );
    assert!(!destination.join(".incomplete").exists());
    harness.runtime.shutdown().await;
}

#[tokio::test]
async fn an_executor_without_complete_record_support_has_an_explicit_default() {
    use mannie_plugin_sdk::points::executors::TaskExecutor;
    use mannie_runtime::testing::FakeExecutor;
    use mannie_types::tasks::TaskExecutorRef;
    let executor = FakeExecutor::new();
    let target = TaskExecutorRef {
        kind: "test-executor".to_owned(),
        target_id: "host".to_owned(),
        target_name: "Host".to_owned(),
        noun: "host".to_owned(),
    };
    let conversation = ConversationId::parse("0199e0f2-0000-7000-8000-000000000083").unwrap();
    assert_eq!(
        executor.export_record(&target, &conversation).await,
        Err(ExportError::Unsupported)
    );
}
