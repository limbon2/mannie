//! The services that the runtime binds to a plugin (ADR-0034, decision 5),
//! through a fake executor: the executor host answers only for the tasks of
//! the plugin's executor kind, and credentials lend only for an active task
//! of that kind whose run uses the profile, as the store records it.
//! `lend_log.rs` checks what a lend logs.

#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use mannie_plugin_sdk::context::HostProfile;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::host::PluginBinding;
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::points::executors::{Executors, TaskExecutor, TaskSubmission};
use mannie_plugin_sdk::services::background_commands::BackgroundCommands;
use mannie_plugin_sdk::services::credentials::{Credentials, LendError};
use mannie_plugin_sdk::services::executor_host::{ExecutorHost, ExecutorTask, OutboxItem};
use mannie_plugin_sdk::services::processes::Processes;
use mannie_plugin_sdk::services::service_info;
use mannie_provider::testing::Step;
use mannie_provider::{Content, ModelRequest, Role};
use mannie_runtime::testing::{
    FAKE_EXECUTOR_KIND, FAKE_EXECUTOR_TOOL, FakeExecutor, Harness, HarnessOptions, add_plugin,
    credentials, executor_host, reply, tool_calls,
};
use mannie_runtime::texts;
use mannie_types::chat::{SendMessage, SetConversationProfile, StartConversation};
use mannie_types::ids::{ConversationId, ProfileId, RunId, TaskId};
use mannie_types::provider::{
    ApiInterface, CreateProviderProfile, CredentialChange, CredentialStoreProblem, ProfileDraft,
    ProviderProfile, ReasoningEffort,
};
use mannie_types::secret::Secret;
use mannie_types::tasks::{AssignedTaskState, TaskState};
use mannie_types::tools::SetFullAccess;
use serde_json::json;

/// How long a test waits for the executor to be asked something.
const WAIT: Duration = Duration::from_secs(60);

/// The credential of the harness's profile, which a permitted lend hands
/// out.
const PLANTED: &str = "sk-planted-lent-credential-0123456789";

/// The kind of an executor that is not the fake one, whose plugin binds the
/// services too.
const OTHER_KIND: &str = "other";

/// A runtime with the fake plugin on, whose executor is `executor`.
struct Setup {
    harness: Harness,
    executor: Arc<FakeExecutor>,
}

impl Setup {
    async fn new() -> Self {
        let harness = Harness::with_options(HarnessOptions {
            use_tools: true,
            credential: Some(PLANTED.to_owned()),
            ..HarnessOptions::default()
        })
        .await;
        let executor = FakeExecutor::new();
        let contribution: Arc<dyn TaskExecutor> = Arc::<FakeExecutor>::clone(&executor);
        add_plugin(
            &harness.runtime,
            FAKE_EXECUTOR_KIND,
            Contributions::new().add::<Executors>(contribution),
        )
        .unwrap();
        Self { harness, executor }
    }

    /// Starts a conversation with the owner's message.
    async fn start(&self, text: &str) -> (ConversationId, RunId) {
        let started = self
            .harness
            .runtime
            .start_conversation(StartConversation::new(text))
            .await
            .unwrap();
        (started.conversation_id, started.run_id)
    }

    /// The parent's model scripts, in order.
    fn parent(&self, steps: Vec<Step>) {
        self.harness.model.push_reply(steps);
    }

    /// Waits until the executor got `count` submissions, and returns them.
    async fn submissions(&self, count: usize) -> Vec<TaskSubmission> {
        let executor = Arc::clone(&self.executor);
        tokio::time::timeout(WAIT, async move {
            loop {
                let submissions = executor.submissions();
                if submissions.len() >= count {
                    return submissions;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the executor got its submissions in time")
    }

    /// Starts a reply that gives the fake executor one task, and returns the
    /// conversation, the run and the task's submission. The task runs in the
    /// foreground, so it holds the run until it ends.
    async fn give_task(&self) -> (ConversationId, RunId, TaskSubmission) {
        self.parent(tool_calls(&[(
            "call_1",
            FAKE_EXECUTOR_TOOL,
            &give("box-01", "Build it", "Build the backend."),
        )]));
        self.parent(reply("Done."));
        let (conversation, run) = self.start("OWNER: build it on the box").await;
        let [submission] = self.submissions(1).await.try_into().unwrap();
        (conversation, run, submission)
    }

    /// Publishes a state of the fake executor's task, as its plugin does.
    async fn publish(&self, state: AssignedTaskState) {
        executor_host(&self.harness.runtime, FAKE_EXECUTOR_KIND)
            .publish_task_state(state)
            .await
            .unwrap();
    }

    /// Adds a second profile that uses tools, with its own credential.
    async fn second_profile(&self, credential: &str) -> ProviderProfile {
        self.harness
            .runtime
            .create_profile(CreateProviderProfile {
                draft: ProfileDraft {
                    name: "Second".to_owned(),
                    interface: ApiInterface::ChatCompletions,
                    base_url: "http://127.0.0.1:9/v1".to_owned(),
                    model: "test-model".to_owned(),
                    context_window: 128_000,
                    output_limit: 16_384,
                    reasoning_effort: ReasoningEffort::ServerDefault,
                    use_tools: true,
                },
                credential: Some(Secret::new(credential.to_owned())),
            })
            .await
            .unwrap()
    }

    /// The binding of a stored profile, as a lend names it.
    async fn binding(&self, profile: &ProfileId) -> String {
        self.harness
            .store
            .profile(profile)
            .await
            .unwrap()
            .unwrap()
            .binding()
    }
}

/// A call of the fake executor's tool in the foreground.
fn give(target: &str, title: &str, instruction: &str) -> String {
    json!({
        "activity": format!("Giving {title}"),
        "target": target,
        "title": title,
        "instruction": instruction,
        "background": false,
    })
    .to_string()
}

/// A call of `follow_up_task` in the foreground.
fn follow_up(task: u32, instruction: &str) -> String {
    json!({
        "activity": "Following up",
        "task": task,
        "instruction": instruction,
        "background": false,
    })
    .to_string()
}

/// The result of the call `call_id` in a request.
fn result(request: &ModelRequest, call_id: &str) -> String {
    request
        .messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .flat_map(|message| &message.content)
        .find_map(|content| match content {
            Content::ToolResult {
                call_id: id,
                output,
            } if id == call_id => Some(output.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no result for {call_id}"))
}

/// A task that no executor runs.
fn unknown_task() -> TaskId {
    TaskId::parse("0199e0f2-7a1b-7c3d-8e4f-0000000009ff").unwrap()
}

/// A manifest of the fake plugin that declares the executor host and
/// credentials.
fn fake_manifest() -> Manifest {
    serde_json::from_value(json!({
        "id": FAKE_EXECUTOR_KIND,
        "sdk": 1,
        "order": 10,
        "rust": { "entry": "mannie-fake" },
        "services": { "credentials": 1, "executor-host": 1 },
    }))
    .unwrap()
}

#[tokio::test]
async fn the_runtimes_providers_bind_the_executor_host_credentials_processes_and_background_commands_to_the_plugin()
 {
    let setup = Setup::new().await;
    let (_conversation, _run, submission) = setup.give_task().await;
    let providers = setup.harness.runtime.service_providers();
    let id = PluginId::parse(FAKE_EXECUTOR_KIND).unwrap();
    let manifest = fake_manifest();
    let binding = PluginBinding::new(&id, &manifest, HostProfile::Development);

    let provided: Vec<(&str, u16)> = providers
        .iter()
        .map(|provider| (provider.name(), provider.major()))
        .collect();
    let [
        host_provider,
        credentials_provider,
        processes_provider,
        commands_provider,
    ] = providers.as_slice()
    else {
        panic!("the runtime provides four services: {provided:?}");
    };
    let host = host_provider.bind(&binding).unwrap();
    let lender = credentials_provider.bind(&binding).unwrap();
    let programs = processes_provider.bind(&binding).unwrap();
    let commands = commands_provider.bind(&binding).unwrap();

    assert_eq!(
        provided,
        [
            ("executor-host", 1),
            ("credentials", 1),
            ("processes", 1),
            ("background-commands", 1)
        ]
    );
    for (name, major) in provided {
        let listed = service_info(name).expect("the SDK lists the service");
        assert_eq!(listed.major, major, "{name}");
    }
    assert!(host.get::<Credentials>().is_none(), "{host:?}");
    assert!(lender.get::<ExecutorHost>().is_none(), "{lender:?}");
    assert!(programs.get::<ExecutorHost>().is_none(), "{programs:?}");
    assert!(programs.get::<Processes>().is_some(), "{programs:?}");
    assert!(
        commands.get::<BackgroundCommands>().is_some(),
        "{commands:?}"
    );
    let active = host
        .get::<ExecutorHost>()
        .unwrap()
        .active_tasks()
        .await
        .unwrap();
    assert_eq!(
        active.iter().map(|task| &task.id).collect::<Vec<_>>(),
        [&submission.task_id],
        "the host is bound to the plugin's kind"
    );
    let lent = lender
        .get::<Credentials>()
        .unwrap()
        .lend(submission.task_id.clone(), setup.harness.profile.id.clone())
        .await
        .unwrap()
        .expect("the profile has a credential");
    assert_eq!(lent.credential.expose(), PLANTED);
}

#[tokio::test]
async fn the_executor_host_reads_and_changes_only_tasks_of_its_kind() {
    let setup = Setup::new().await;
    let (conversation, run, submission) = setup.give_task().await;
    setup
        .publish(FakeExecutor::state(&submission, 2, TaskState::Running))
        .await;
    let runtime = &setup.harness.runtime;
    let store = &setup.harness.store;
    let own = executor_host(runtime, FAKE_EXECUTOR_KIND);
    let other = executor_host(runtime, OTHER_KIND);
    let task = submission.task_id.clone();
    let own_before_full_access = own
        .parent_has_full_access(submission.conversation_id.clone())
        .await;
    runtime
        .set_full_access(SetFullAccess {
            conversation_id: conversation.clone(),
            enabled: true,
        })
        .await
        .unwrap();

    let seen_by_other = other.task(task.clone()).await.unwrap();
    let seen_by_own = own.task(task.clone()).await.unwrap();
    let active_of_other = other.active_tasks().await.unwrap();
    let active_of_own = own.active_tasks().await.unwrap();
    let other_document = other
        .set_document(task.clone(), Some(r#"{"forged":true}"#.to_owned()))
        .await
        .unwrap();
    let after_other_document = store.task(&task).await.unwrap().unwrap();
    let own_document = own
        .set_document(task.clone(), Some(r#"{"version":2}"#.to_owned()))
        .await
        .unwrap();
    let other_end = other
        .end_task(task.clone(), TaskState::Canceled)
        .await
        .unwrap_err();
    let after_other_end = store.task(&task).await.unwrap().unwrap();
    let unknown_end = own
        .end_task(unknown_task(), TaskState::Canceled)
        .await
        .unwrap_err();
    let other_full_access = other
        .parent_has_full_access(submission.conversation_id.clone())
        .await;
    let own_full_access = own
        .parent_has_full_access(submission.conversation_id.clone())
        .await;
    let parent_full_access = own.parent_has_full_access(conversation.clone()).await;
    setup
        .publish(FakeExecutor::reported(&submission, 3, "Built."))
        .await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let expected = ExecutorTask::new(
        task.clone(),
        submission.conversation_id.clone(),
        TaskState::Running,
        FakeExecutor::target("box-01").target_id,
        Some(r#"{"version":1}"#.to_owned()),
    );
    assert_eq!(seen_by_other, None);
    assert_eq!(seen_by_own, Some(expected.clone()));
    assert_eq!(active_of_other, Vec::new());
    assert_eq!(active_of_own, [expected]);
    assert!(!other_document, "another kind's task keeps its document");
    assert_eq!(
        after_other_document
            .executor
            .and_then(|execution| execution.document)
            .as_deref(),
        Some(r#"{"version":1}"#)
    );
    assert!(own_document);
    assert_eq!(other_end.code(), "not_permitted");
    assert_eq!(after_other_end.state, TaskState::Running, "the row stays");
    assert_eq!(after_other_end.ending, None);
    assert_eq!(unknown_end.code(), "not_permitted");
    assert!(!own_before_full_access, "the parent is not in full access");
    assert!(!other_full_access, "another kind's task conversation");
    assert!(own_full_access);
    assert!(!parent_full_access, "the parent is no task conversation");
    // The other kind's end never reached the board: the run that followed
    // the task got the executor's report.
    let requests = setup.harness.model.requests_for("OWNER:");
    assert_eq!(
        result(&requests[1], "call_1"),
        texts::task_reported(1, "Built.")
    );
    let [shown] = transcript.tasks(0).try_into().unwrap();
    assert_eq!(shown.state, TaskState::Reported);
    let stored = store.task(&task).await.unwrap().unwrap();
    assert_eq!(
        stored
            .executor
            .and_then(|execution| execution.document)
            .as_deref(),
        Some(r#"{"version":2}"#)
    );
}

#[tokio::test]
async fn ending_a_task_of_the_kind_that_ended_keeps_its_first_ending() {
    let setup = Setup::new().await;
    let (conversation, run, submission) = setup.give_task().await;
    setup
        .publish(FakeExecutor::reported(&submission, 2, "Built."))
        .await;
    setup.harness.wait_for_run(&conversation, &run).await;
    let own = executor_host(&setup.harness.runtime, FAKE_EXECUTOR_KIND);

    own.end_task(submission.task_id.clone(), TaskState::Canceled)
        .await
        .unwrap();

    let stored = setup
        .harness
        .store
        .task(&submission.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.state, TaskState::Reported);
    assert_eq!(
        stored.ending.and_then(|ending| ending.report).as_deref(),
        Some("Built.")
    );
}

#[tokio::test]
async fn a_plugin_cannot_publish_another_kinds_task_state() {
    let setup = Setup::new().await;
    let (conversation, run, submission) = setup.give_task().await;
    let other = executor_host(&setup.harness.runtime, OTHER_KIND);
    let own = executor_host(&setup.harness.runtime, FAKE_EXECUTOR_KIND);

    let forged = other
        .publish_task_state(FakeExecutor::reported(&submission, 9, "Forged."))
        .await
        .unwrap();
    let after_forged = setup
        .harness
        .store
        .task(&submission.task_id)
        .await
        .unwrap()
        .unwrap();
    let taken = own
        .publish_task_state(FakeExecutor::reported(&submission, 2, "Built."))
        .await
        .unwrap();
    setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(forged, None);
    assert!(after_forged.state.is_active(), "{after_forged:?}");
    assert_eq!(after_forged.ending, None);
    assert_eq!(
        taken.map(|before| (before.id, before.state)),
        Some((submission.task_id.clone(), TaskState::Accepted)),
        "the task as it was before the state"
    );
    let requests = setup.harness.model.requests_for("OWNER:");
    assert_eq!(
        result(&requests[1], "call_1"),
        texts::task_reported(1, "Built.")
    );
    let stored = setup
        .harness
        .store
        .task(&submission.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.ending.and_then(|ending| ending.report).as_deref(),
        Some("Built.")
    );
}

#[tokio::test]
async fn the_executor_hosts_outbox_holds_only_the_deletions_of_its_kind() {
    let setup = Setup::new().await;
    let (conversation, run, submission) = setup.give_task().await;
    setup
        .publish(FakeExecutor::reported(&submission, 2, "Built."))
        .await;
    setup.harness.wait_for_run(&conversation, &run).await;
    let own = executor_host(&setup.harness.runtime, FAKE_EXECUTOR_KIND);
    let other = executor_host(&setup.harness.runtime, OTHER_KIND);
    let mut changes = own.outbox_changes();
    changes.borrow_and_update();
    let target = FakeExecutor::target("box-01").target_id;
    let deletion = OutboxItem::new(target.clone(), submission.conversation_id.clone());

    setup
        .harness
        .runtime
        .delete_conversation(&conversation)
        .await
        .unwrap();
    let outbox_of_other = other.outbox().await.unwrap();
    other.drop_outbox(vec![deletion.clone()]).await.unwrap();
    let dropped_by_other = other.drop_target_outbox(target.clone()).await.unwrap();
    let outbox_of_own = own.outbox().await.unwrap();
    let dropped_by_own = own.drop_target_outbox(target).await.unwrap();

    assert!(changes.has_changed().unwrap(), "the executors are told");
    assert_eq!(outbox_of_other, Vec::new());
    assert_eq!(dropped_by_other, 0);
    assert_eq!(
        outbox_of_own,
        [deletion],
        "another kind's drops keep the deletion"
    );
    assert_eq!(dropped_by_own, 1);
    assert_eq!(own.outbox().await.unwrap(), Vec::new());
}

#[tokio::test]
async fn a_lend_for_an_active_task_of_the_kind_and_its_runs_profile_is_lent() {
    let setup = Setup::new().await;
    let (conversation, run, submission) = setup.give_task().await;
    let profile = setup.harness.profile.id.clone();

    let lent = credentials(&setup.harness.runtime, FAKE_EXECUTOR_KIND)
        .lend(submission.task_id.clone(), profile.clone())
        .await
        .unwrap()
        .expect("the profile has a credential");
    setup
        .publish(FakeExecutor::reported(&submission, 2, "Built."))
        .await;
    setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(submission.profile.id, profile);
    assert_eq!(lent.credential.expose(), PLANTED);
    assert_eq!(lent.binding, setup.binding(&profile).await);
}

#[tokio::test]
async fn a_permitted_lend_of_a_profile_without_a_credential_or_a_deleted_one_lends_nothing() {
    let setup = Setup::new().await;
    let (_conversation, _run, submission) = setup.give_task().await;
    let profile = setup.harness.profile.clone();
    let task = submission.task_id.clone();
    setup
        .harness
        .runtime
        .update_profile(
            &profile.id,
            ProfileDraft {
                name: profile.name.clone(),
                interface: profile.interface,
                base_url: profile.base_url.clone(),
                model: profile.model.clone(),
                context_window: profile.context_window,
                output_limit: profile.output_limit,
                reasoning_effort: profile.reasoning_effort,
                use_tools: profile.use_tools,
            },
            CredentialChange::Remove,
        )
        .await
        .unwrap();
    let without_credential = credentials(&setup.harness.runtime, FAKE_EXECUTOR_KIND)
        .lend(task.clone(), profile.id.clone())
        .await;
    // The reply ends with Mannie and the task runs on, so the profile that
    // the reply used can go.
    let harness = setup.harness.restart().await;
    harness.runtime.delete_profile(&profile.id).await.unwrap();

    let deleted = credentials(&harness.runtime, FAKE_EXECUTOR_KIND)
        .lend(task.clone(), profile.id.clone())
        .await;

    assert!(
        matches!(without_credential, Ok(None)),
        "{without_credential:?}"
    );
    assert!(
        matches!(deleted, Err(LendError::ProfileMissing)),
        "{deleted:?}"
    );
    assert!(
        harness
            .store
            .task(&task)
            .await
            .unwrap()
            .unwrap()
            .state
            .is_active(),
        "the task runs on"
    );
}

#[tokio::test]
async fn a_permitted_lend_whose_credential_cannot_be_read_fails_and_lends_once_it_can() {
    let setup = Setup::new().await;
    let (conversation, run, submission) = setup.give_task().await;
    let profile = setup.harness.profile.id.clone();
    let lender = credentials(&setup.harness.runtime, FAKE_EXECUTOR_KIND);
    setup
        .harness
        .vault
        .fail_reads(Some(CredentialStoreProblem::AccessDenied));

    let unreadable = lender
        .lend(submission.task_id.clone(), profile.clone())
        .await;
    // The parent's next model call reads the credential too, so the vault
    // reads again before the task reports.
    setup.harness.vault.fail_reads(None);
    let readable = lender.lend(submission.task_id.clone(), profile).await;
    setup
        .publish(FakeExecutor::reported(&submission, 2, "Built."))
        .await;
    setup.harness.wait_for_run(&conversation, &run).await;

    assert!(
        matches!(unreadable, Err(LendError::CredentialUnavailable)),
        "{unreadable:?}"
    );
    let lent = readable.unwrap().expect("the profile has a credential");
    assert_eq!(lent.credential.expose(), PLANTED);
}

#[tokio::test]
async fn a_lend_is_refused_for_another_kind_another_profile_an_ended_task_and_an_unknown_task() {
    let setup = Setup::new().await;
    let (conversation, run, submission) = setup.give_task().await;
    let runtime = &setup.harness.runtime;
    let profile = setup.harness.profile.id.clone();
    let second = setup.second_profile("sk-second-credential").await;
    let own = credentials(runtime, FAKE_EXECUTOR_KIND);
    let task = submission.task_id.clone();

    let other_kind = credentials(runtime, OTHER_KIND)
        .lend(task.clone(), profile.clone())
        .await;
    let other_profile = own.lend(task.clone(), second.id.clone()).await;
    let unknown = own.lend(unknown_task(), profile.clone()).await;
    setup
        .publish(FakeExecutor::reported(&submission, 2, "Built."))
        .await;
    setup.harness.wait_for_run(&conversation, &run).await;
    let ended = own.lend(task, profile).await;

    for (case, refused) in [
        ("another kind", other_kind),
        ("another profile", other_profile),
        ("an unknown task", unknown),
        ("an ended task", ended),
    ] {
        assert!(
            matches!(refused, Err(LendError::NotPermitted)),
            "{case}: {refused:?}"
        );
    }
}

#[tokio::test]
async fn a_follow_up_lends_the_profile_of_the_run_that_gave_it() {
    let setup = Setup::new().await;
    let (conversation, run, first) = setup.give_task().await;
    setup
        .publish(FakeExecutor::reported(&first, 2, "Built."))
        .await;
    setup.harness.wait_for_run(&conversation, &run).await;
    let first_profile = setup.harness.profile.id.clone();
    let second = setup.second_profile("sk-second-credential").await;
    // The owner chooses another profile for the parent's next replies; the
    // task conversation keeps the profile that it started with.
    setup
        .harness
        .runtime
        .set_conversation_profile(SetConversationProfile {
            conversation_id: conversation.clone(),
            profile_id: Some(second.id.clone()),
        })
        .await
        .unwrap();
    setup.parent(tool_calls(&[(
        "call_2",
        "follow_up_task",
        &follow_up(1, "Now run the tests."),
    )]));
    setup
        .harness
        .runtime
        .send_message(SendMessage::new(
            conversation.clone(),
            "OWNER-2: now test it",
        ))
        .await
        .unwrap();
    let [_, follow_up_task] = setup.submissions(2).await.try_into().unwrap();
    // The reply ends with Mannie while the follow-up runs on, and the lend
    // reads the store, so it holds after the restart.
    let harness = setup.harness.restart().await;
    let lender = credentials(&harness.runtime, FAKE_EXECUTOR_KIND);

    let lent = lender
        .lend(follow_up_task.task_id.clone(), second.id.clone())
        .await
        .unwrap()
        .expect("the second profile has a credential");
    let earlier_profile = lender
        .lend(follow_up_task.task_id.clone(), first_profile.clone())
        .await;

    assert!(follow_up_task.follow_up);
    assert_eq!(follow_up_task.conversation_id, first.conversation_id);
    assert_eq!(follow_up_task.profile.id, second.id);
    let summary = harness
        .store
        .conversation_summary(&first.conversation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        summary.profile_id,
        Some(first_profile),
        "the task conversation keeps its profile"
    );
    assert_eq!(lent.credential.expose(), "sk-second-credential");
    assert!(
        matches!(earlier_profile, Err(LendError::NotPermitted)),
        "{earlier_profile:?}"
    );
}
