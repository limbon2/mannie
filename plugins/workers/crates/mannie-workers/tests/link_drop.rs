//! Calls whose answers are lost (V6; plugins/workers/docs/delegation.md,
//! acceptance): the link goes down after a call reached the worker and
//! before its answer reached the master. The relay loses what the worker
//! sends until the call took effect on the worker, then drops the link. The
//! master connects again, and nothing happens twice: a submission whose
//! answer was lost finds the task that the worker already runs, a decision
//! runs its call once, and a cancel ends the task once.

#![cfg(test)]

use std::time::Duration;

use mannie_plugin_harness::provider::ProviderEvent;
use mannie_plugin_harness::provider::Step;
use mannie_plugin_harness::runtime::{reply, tool_calls};
use mannie_plugin_harness::types::FinishReason;
use mannie_plugin_sdk::contracts::TaskId;
use mannie_plugin_sdk::contracts::{AssignedTaskState, TaskState};
use mannie_workers::view::WorkerStatus;
use mannie_workers_testing::master::{LinkedMaster, delegation};
use mannie_workers_testing::worker::{LinkedWorker, Relay};
use serde_json::json;

/// A worker, a relay in front of it, and a master paired through the relay.
async fn relayed() -> (LinkedWorker, Relay, LinkedMaster) {
    let worker = LinkedWorker::start().await;
    let relay = Relay::start(&worker.address).await;
    let master = LinkedMaster::start().await;
    master
        .pair(&relay.address, &worker.new_code())
        .await
        .unwrap();
    master.wait_for_status(WorkerStatus::Connected).await;
    (worker, relay, master)
}

/// Waits until `done` holds for the worker.
async fn until_worker(done: impl AsyncFn() -> bool) {
    let wait = async {
        while !done().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(30), wait)
        .await
        .expect("the worker gets there within 30 seconds");
}

/// The state of a task on the worker.
async fn on_worker(worker: &LinkedWorker, task: &TaskId) -> Option<AssignedTaskState> {
    worker.harness.runtime.assigned_task(task).await.unwrap()
}

#[tokio::test]
async fn a_submission_whose_answer_was_lost_runs_its_task_once() {
    let (worker, relay, master) = relayed().await;
    master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegation("Build it once."),
    )]));
    master.model.push_reply(reply("Built."));
    worker.harness.model.push_reply(vec![
        Step::Delay(Duration::from_millis(1_500)),
        Step::Event(ProviderEvent::TextDelta("Built.".to_owned())),
        Step::Event(ProviderEvent::Finished(FinishReason::Stop)),
    ]);
    worker
        .harness
        .model
        .push_reply(reply("Report: built once."));

    // The task waits for its lease on the worker, which the master learns
    // of only through the worker's answers, so the watch shows when the
    // worker stored it.
    let mut assigned = worker.harness.runtime.watch_assigned_tasks();
    relay.set_answers_lost(true);
    let conversation = master.start_conversation("Build").await;
    let stored = tokio::time::timeout(Duration::from_secs(30), assigned.next())
        .await
        .expect("the worker stores the task within 30 seconds")
        .expect("the watch goes on");
    relay.drop_link();
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| !task.state.is_active())
        })
        .await;
    master.wait_for_idle(&conversation).await;

    assert_eq!(stored.task_id, tasks[0].task_id);
    assert_eq!(tasks[0].state, TaskState::Reported);
    assert_eq!(tasks[0].report.as_deref(), Some("Report: built once."));
    assert_eq!(
        worker.harness.model.reply_requests().len(),
        2,
        "the task ran once, with its report"
    );
}

#[tokio::test]
async fn a_decision_whose_answer_was_lost_runs_its_call_once() {
    let (worker, relay, master) = relayed().await;
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
    let conversation = master.start_conversation("Write the marker").await;
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| task.approval.is_some())
        })
        .await;
    let task = tasks[0].task_id.clone();

    relay.set_answers_lost(true);
    let deciding = tokio::spawn({
        let master_task = tasks[0].clone();
        let runtime = master.runtime.clone();
        async move {
            let approval = master_task.approval.unwrap();
            runtime
                .decide_approval(mannie_plugin_sdk::contracts::DecideApproval {
                    conversation_id: master_task.conversation_id,
                    part_id: approval.part_id,
                    digest: approval.call.request.unwrap().digest,
                    choice: mannie_plugin_sdk::contracts::ApprovalChoice::AllowOnce,
                })
                .await
        }
    });
    until_worker(async || {
        on_worker(&worker, &task)
            .await
            .is_some_and(|state| state.approval.is_none())
    })
    .await;
    relay.drop_link();
    let decided = deciding.await.unwrap();
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| !task.state.is_active())
        })
        .await;
    master.wait_for_idle(&conversation).await;
    let calls = worker
        .harness
        .snapshot(&tasks[0].conversation_id)
        .await
        .tool_calls(0);

    assert_eq!(
        decided.unwrap_err(),
        mannie_plugin_sdk::contracts::ToolsError::TaskTargetUnreachable {
            kind: mannie_workers::WORKERS_KIND.to_owned(),
            target: "build-01".to_owned(),
        },
        "the owner learns that the answer did not come"
    );
    assert_eq!(tasks[0].state, TaskState::Reported);
    assert_eq!(calls.len(), 1, "one call, decided once: {calls:?}");
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "built\n");
}

#[tokio::test]
async fn a_cancel_whose_answer_was_lost_ends_its_task_once() {
    let (worker, relay, master) = relayed().await;
    master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegation("Build until stopped."),
    )]));
    worker.harness.model.push_reply(vec![Step::Hang]);
    let conversation = master.start_conversation("Build").await;
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks
                .first()
                .is_some_and(|task| task.state == TaskState::Running)
        })
        .await;
    let task = tasks[0].task_id.clone();
    until_worker(async || !worker.harness.model.reply_requests().is_empty()).await;

    relay.set_answers_lost(true);
    let stopping = tokio::spawn({
        let runtime = master.runtime.clone();
        let conversation = conversation.clone();
        async move { runtime.stop_reply(&conversation).await }
    });
    until_worker(async || {
        on_worker(&worker, &task)
            .await
            .is_some_and(|state| state.state == TaskState::Canceled)
    })
    .await;
    relay.drop_link();
    stopping.await.unwrap();
    master.wait_for_idle(&conversation).await;
    let row = async {
        loop {
            let row = master.store.task(&task).await.unwrap().unwrap();
            if !row.state.is_active() {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let row = tokio::time::timeout(Duration::from_secs(30), row)
        .await
        .expect("the master learns the end");

    assert_eq!(row.state, TaskState::Canceled);
    assert_eq!(
        on_worker(&worker, &task).await.unwrap().state,
        TaskState::Canceled
    );
    assert_eq!(
        worker.harness.model.reply_requests().len(),
        1,
        "the task never ran again"
    );
}
