//! Tasks that the master assigns to a worker (ADR-0028; docs/features/
//! workers/delegation.md): a task runs under the worker agent, confirms and
//! reports; assigning again returns the stored task; a follow-up continues
//! its conversation; a run waits for its lease, and a revoked lease ends it;
//! cancels, slots, restarts, unpairing and deletion; and the master's watch.
//!
//! The worker's model calls play in order, as the fake model's scripts.

#![cfg(test)]

use std::time::Duration;

use mannie_provider::testing::Step;
use mannie_provider::{Content, ModelRequest, Role};
use mannie_runtime::testing::{
    HARNESS_WORKER_BASE, Harness, LEASE_CREDENTIAL, WorkerHarness, WorkerOptions,
    ceiling_mode_line, reply,
};
use mannie_runtime::{AssignError, LeaseError, texts};
use mannie_types::ceiling::CeilingMode;
use mannie_types::run::{RunActivity, RunFailure};
use mannie_types::secret::Secret;
use mannie_types::tasks::TaskState;

/// A step of the model that never ends, so a run stays busy until it stops.
fn hang() -> Vec<Step> {
    vec![Step::Hang]
}

/// The text of a request's messages of one role, in order.
fn texts_of(request: &ModelRequest, role: Role) -> Vec<String> {
    request
        .messages
        .iter()
        .filter(|message| message.role == role)
        .map(|message| {
            message
                .content
                .iter()
                .filter_map(|content| match content {
                    Content::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

/// Waits until the model got `calls` reply requests, so the scripts that
/// follow go to the calls after them.
async fn until_model_called(worker: &WorkerHarness, calls: usize) {
    let wait = async {
        while worker.model.reply_requests().len() < calls {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(20), wait)
        .await
        .expect("the model is called");
}

/// A worker whose model answers each task once and confirms with a report.
fn script_report(worker: &WorkerHarness, answer: &str, report: &str) {
    worker.model.push_reply(reply(answer));
    worker.model.push_reply(reply(report));
}

#[tokio::test]
async fn an_assigned_task_runs_as_the_worker_agent_confirms_and_reports() {
    let worker = WorkerHarness::with_options(WorkerOptions {
        host_instructions: Some("Never restart nginx.".to_owned()),
        ..WorkerOptions::default()
    })
    .await;
    script_report(&worker, "The build passes.", "Report: the build passes.");
    let assignment = worker.assignment("Build the backend and run its tests.");
    let task = assignment.task_id.clone();

    let accepted = worker.assign(assignment.clone()).await;
    let ended = worker.wait_for_end(&task).await;

    assert_eq!(accepted.state, TaskState::Accepted);
    assert_eq!(accepted.conversation_id, assignment.conversation_id);
    assert_eq!(ended.state, TaskState::Reported);
    assert_eq!(ended.report.as_deref(), Some("Report: the build passes."));
    assert!(ended.version > accepted.version, "{ended:?}");
    let requests = worker.model.reply_requests();
    assert_eq!(
        requests.len(),
        2,
        "one answer and one report after the confirmation"
    );
    let instructions = &requests[0].instructions;
    assert!(
        instructions.starts_with(HARNESS_WORKER_BASE),
        "{instructions}"
    );
    assert!(
        instructions.contains(&ceiling_mode_line(CeilingMode::Ask)),
        "{instructions}"
    );
    assert!(
        instructions.contains("Never restart nginx."),
        "{instructions}"
    );
    assert!(!instructions.contains(texts::OWNER_INSTRUCTIONS_HEADING));
    assert_eq!(
        texts_of(&requests[0], Role::User),
        ["Build the backend and run its tests."]
    );
    let confirmed = texts_of(&requests[1], Role::User);
    assert_eq!(
        confirmed.last().map(String::as_str),
        Some(texts::CONFIRMATION_NOTE)
    );
    assert_eq!(
        worker.model.credentials(),
        [
            Some(LEASE_CREDENTIAL.to_owned()),
            Some(LEASE_CREDENTIAL.to_owned())
        ],
        "the runs used the lease"
    );
    assert_eq!(
        worker.vault.references(),
        Vec::<String>::new(),
        "the lease went with the task"
    );
}

#[tokio::test]
async fn assigning_a_task_again_returns_it_and_starts_nothing_new() {
    let worker = WorkerHarness::start().await;
    script_report(&worker, "Done.", "Report: done.");
    let assignment = worker.assignment("Count the files.");
    let task = assignment.task_id.clone();

    worker.assign(assignment.clone()).await;
    let again = worker
        .runtime
        .assign_task(assignment.clone())
        .await
        .unwrap();
    worker.wait_for_end(&task).await;
    let after_the_end = worker
        .runtime
        .assign_task(assignment.clone())
        .await
        .unwrap();
    let mut other = worker.assignment("Something else.");
    other.conversation_id = assignment.conversation_id.clone();
    let conflict = worker.runtime.assign_task(other).await;

    assert_eq!(again.task_id, task);
    assert_eq!(after_the_end.state, TaskState::Reported);
    assert_eq!(after_the_end.report.as_deref(), Some("Report: done."));
    assert_eq!(conflict, Err(AssignError::Conflict));
    assert_eq!(worker.model.reply_requests().len(), 2, "one task ran once");
}

#[tokio::test]
async fn a_follow_up_continues_the_conversation_after_the_task_ended() {
    let worker = WorkerHarness::start().await;
    let first = worker.assignment("Build the backend.");
    worker.model.push_reply(hang());
    worker.assign(first.clone()).await;
    until_model_called(&worker, 1).await;
    let while_running = worker
        .runtime
        .assign_task(WorkerHarness::follow_up(&first, "Now deploy it."))
        .await;
    worker
        .runtime
        .cancel_assigned_task(&first.task_id)
        .await
        .unwrap();
    worker.wait_for_end(&first.task_id).await;
    script_report(&worker, "Deployed.", "Report: deployed.");
    let follow_up = WorkerHarness::follow_up(&first, "Now deploy it.");

    worker.assign(follow_up.clone()).await;
    let ended = worker.wait_for_end(&follow_up.task_id).await;
    let mut unknown = WorkerHarness::follow_up(&first, "Elsewhere.");
    unknown.conversation_id = worker.assignment("x").conversation_id;
    let unknown = worker.runtime.assign_task(unknown).await;

    assert_eq!(while_running, Err(AssignError::StillRunning));
    assert_eq!(ended.state, TaskState::Reported);
    assert_eq!(ended.conversation_id, first.conversation_id);
    let requests = worker.model.reply_requests();
    let continued = requests.last().unwrap();
    let users = texts_of(continued, Role::User);
    assert_eq!(
        users.first().map(String::as_str),
        Some("Build the backend.")
    );
    assert!(users.contains(&"Now deploy it.".to_owned()), "{users:?}");
    assert_eq!(unknown, Err(AssignError::ConversationNotFound));
}

#[tokio::test]
async fn a_run_waits_for_its_lease_without_a_model_call() {
    let worker = WorkerHarness::start().await;
    script_report(&worker, "Done.", "Report: done.");
    let assignment = worker.assignment("Count the files.");
    let task = assignment.task_id.clone();

    worker
        .runtime
        .assign_task(assignment.clone())
        .await
        .unwrap();
    let waiting = worker
        .wait_for_task(&task, |state| state.waits_for_lease)
        .await;
    let transcript = worker.snapshot(&assignment.conversation_id).await;
    let wrong_binding = worker
        .runtime
        .deliver_lease(&task, "other:1", Secret::new("sk-other".to_owned()))
        .await;
    worker.deliver_lease(&task).await;
    let ended = worker.wait_for_end(&task).await;

    assert_eq!(waiting.state, TaskState::Accepted);
    assert_eq!(
        transcript.turns[&0]
            .run
            .as_ref()
            .and_then(|run| run.activity.clone()),
        Some(RunActivity::WaitingForLease)
    );
    assert_eq!(wrong_binding, Err(LeaseError::BindingMismatch));
    assert_eq!(ended.state, TaskState::Reported);
    assert!(!ended.waits_for_lease);
    assert_eq!(
        worker
            .model
            .credentials()
            .first()
            .cloned()
            .flatten()
            .as_deref(),
        Some(LEASE_CREDENTIAL)
    );
    assert_eq!(
        worker
            .runtime
            .deliver_lease(
                &task,
                &worker.profile().binding(),
                Secret::new("late".to_owned())
            )
            .await,
        Err(LeaseError::TaskNotActive),
        "an ended task takes no lease"
    );
    assert_eq!(worker.vault.references(), Vec::<String>::new());
}

#[tokio::test]
async fn revoking_the_lease_ends_the_run_and_drops_the_credential() {
    let worker = WorkerHarness::start().await;
    worker.model.push_reply(hang());
    let assignment = worker.assignment("Count the files.");
    let task = assignment.task_id.clone();
    worker.assign(assignment).await;
    worker
        .wait_for_task(&task, |state| state.state == TaskState::Running)
        .await;

    worker.runtime.revoke_lease(&task).await.unwrap();
    let ended = worker.wait_for_end(&task).await;

    assert_eq!(ended.state, TaskState::Failed);
    assert_eq!(ended.failure, Some(RunFailure::LeaseRevoked));
    assert_eq!(worker.vault.references(), Vec::<String>::new());
    worker
        .runtime
        .revoke_lease(&task)
        .await
        .expect("revoking a dropped lease changes nothing");
}

#[tokio::test]
async fn canceling_a_task_stops_its_run_and_ends_it_as_canceled() {
    let worker = WorkerHarness::start().await;
    worker.model.push_reply(hang());
    let assignment = worker.assignment("Count the files.");
    let task = assignment.task_id.clone();
    worker.assign(assignment).await;
    worker
        .wait_for_task(&task, |state| state.state == TaskState::Running)
        .await;

    let canceled = worker.runtime.cancel_assigned_task(&task).await.unwrap();
    let unknown = worker
        .runtime
        .cancel_assigned_task(&worker.assignment("x").task_id)
        .await
        .unwrap();

    assert_eq!(canceled.map(|state| state.state), Some(TaskState::Canceled));
    assert_eq!(unknown, None);
    assert_eq!(worker.vault.references(), Vec::<String>::new());
}

#[tokio::test]
async fn the_hosts_slots_limit_the_tasks_that_run_at_once() {
    let worker = WorkerHarness::with_options(WorkerOptions {
        max_parallel_tasks: 1,
        ..WorkerOptions::default()
    })
    .await;
    worker.model.push_reply(hang());
    script_report(&worker, "Second.", "Report: second.");
    let first = worker.assignment("First task.");
    let second = worker.assignment("Second task.");
    worker.assign(first.clone()).await;
    worker
        .wait_for_task(&first.task_id, |state| state.state == TaskState::Running)
        .await;
    until_model_called(&worker, 1).await;

    worker.assign(second.clone()).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let waiting = worker
        .runtime
        .assigned_task(&second.task_id)
        .await
        .unwrap()
        .unwrap();
    worker
        .runtime
        .cancel_assigned_task(&first.task_id)
        .await
        .unwrap();
    let ended = worker.wait_for_end(&second.task_id).await;

    assert_eq!(
        waiting.state,
        TaskState::Accepted,
        "the second waits for a slot"
    );
    assert_eq!(ended.state, TaskState::Reported);
}

#[tokio::test]
async fn a_restart_interrupts_the_tasks_that_ran() {
    let worker = WorkerHarness::start().await;
    worker.model.push_reply(hang());
    let assignment = worker.assignment("Count the files.");
    let task = assignment.task_id.clone();
    worker.assign(assignment).await;
    let running = worker
        .wait_for_task(&task, |state| state.state == TaskState::Running)
        .await;

    let worker = worker.restart().await;
    let after = worker.runtime.assigned_task(&task).await.unwrap().unwrap();

    assert_eq!(after.state, TaskState::Interrupted);
    assert!(after.version > running.version, "{after:?}");
    assert_eq!(
        worker.vault.references(),
        Vec::<String>::new(),
        "no lease survives"
    );
}

#[tokio::test]
async fn unpairing_lets_every_task_go_and_deleting_removes_its_conversation() {
    let worker = WorkerHarness::start().await;
    worker.model.push_reply(hang());
    worker.model.push_reply(hang());
    let first = worker.assignment("First task.");
    let second = worker.assignment("Second task.");
    worker.assign(first.clone()).await;
    worker.assign(second.clone()).await;
    for task in [&first.task_id, &second.task_id] {
        worker
            .wait_for_task(task, |state| state.state == TaskState::Running)
            .await;
    }

    worker.runtime.release_assignments().await;
    let ended = [
        worker.wait_for_end(&first.task_id).await,
        worker.wait_for_end(&second.task_id).await,
    ];
    worker
        .runtime
        .delete_assigned_conversations(std::slice::from_ref(&first.conversation_id))
        .await
        .unwrap();

    assert!(ended.iter().all(|state| state.state == TaskState::Canceled));
    assert_eq!(
        worker.runtime.assigned_task(&first.task_id).await.unwrap(),
        None
    );
    assert_eq!(
        worker
            .store
            .conversation_summary(&first.conversation_id)
            .await
            .unwrap(),
        None
    );
    assert!(
        worker
            .runtime
            .assigned_task(&second.task_id)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn the_watch_starts_with_the_active_tasks_and_follows_every_change() {
    let worker = WorkerHarness::start().await;
    worker.model.push_reply(hang());
    let running = worker.assignment("Runs.");
    worker.assign(running.clone()).await;
    worker
        .wait_for_task(&running.task_id, |state| state.state == TaskState::Running)
        .await;
    until_model_called(&worker, 1).await;

    let mut watch = worker.runtime.watch_assigned_tasks();
    let first = watch.next().await.unwrap();
    script_report(&worker, "Done.", "Report: done.");
    let later = worker.assignment("Later.");
    worker.assign(later.clone()).await;
    let mut seen = Vec::new();
    while let Some(state) = watch.next().await {
        let done = state.task_id == later.task_id && !state.state.is_active();
        seen.push(state);
        if done {
            break;
        }
    }

    assert_eq!(first.task_id, running.task_id);
    assert_eq!(first.state, TaskState::Running);
    let later_states: Vec<_> = seen
        .iter()
        .filter(|state| state.task_id == later.task_id)
        .collect();
    assert_eq!(
        later_states.first().map(|state| state.state),
        Some(TaskState::Accepted)
    );
    assert_eq!(
        later_states.last().map(|state| state.state),
        Some(TaskState::Reported)
    );
    let versions: Vec<u64> = later_states.iter().map(|state| state.version).collect();
    assert!(
        versions.windows(2).all(|pair| pair[0] <= pair[1]),
        "{versions:?}"
    );
}

#[tokio::test]
async fn an_assignment_that_is_not_valid_is_refused_before_anything_is_stored() {
    let worker = WorkerHarness::start().await;
    let mut empty = worker.assignment("  ");
    let mut public_plain_http = worker.assignment("Count the files.");
    public_plain_http.profile.base_url = "http://api.example.com/v1".to_owned();
    let mut untitled = worker.assignment("Count the files.");
    untitled.title = " \n ".to_owned();
    empty.title = "Empty".to_owned();

    for assignment in [empty, public_plain_http, untitled] {
        let conversation = assignment.conversation_id.clone();
        let refused = worker.runtime.assign_task(assignment).await;

        assert!(
            matches!(refused, Err(AssignError::Invalid { .. })),
            "{refused:?}"
        );
        assert_eq!(
            worker
                .store
                .conversation_summary(&conversation)
                .await
                .unwrap(),
            None
        );
    }
}

#[tokio::test]
async fn the_masters_runtime_takes_no_assignments() {
    let master = Harness::start().await;
    let worker = WorkerHarness::start().await;

    let refused = master
        .runtime
        .assign_task(worker.assignment("Count."))
        .await;

    assert_eq!(refused, Err(AssignError::NotAWorker));
}
