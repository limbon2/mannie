//! The tasks of a plugin's executor (ADR-0031, decision 4), through a fake
//! executor: a registered executor's tool and section, its plans and
//! refusals, reports, background tasks and waits, follow-ups, Stop, the
//! plugin stopping before a call, during a submission and while a task runs,
//! waits while it is off, the reads of its task conversations and the
//! outbox of deletions.

#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use mannie_provider::testing::Step;
use mannie_provider::{Content, ModelRequest, Role};
use mannie_runtime::testing::{
    FAKE_EXECUTOR_AGENT, FAKE_EXECUTOR_KIND, FAKE_EXECUTOR_NOUN, FAKE_EXECUTOR_SECTION,
    FAKE_EXECUTOR_TOOL, FakeExecutor, Harness, HarnessOptions, fast_config, reply, tool_calls,
};
use mannie_runtime::{ExecutorRefusal, RuntimeConfig, TaskSubmission, texts};
use mannie_store::OutboxEntry;
use mannie_types::chat::{
    ChatError, LoadEarlierTurns, StartConversation, StopOutcome, SubscribeConversation,
};
use mannie_types::ids::{ConversationId, RunId};
use mannie_types::run::RunState;
use mannie_types::tasks::{TaskRefusal, TaskState};
use mannie_types::tools::{NotRunReason, ToolCallStatus};
use serde_json::json;

/// How long a test waits for the executor to be asked something.
const WAIT: Duration = Duration::from_secs(60);

struct Setup {
    harness: Harness,
    executor: Arc<FakeExecutor>,
}

impl Setup {
    async fn new() -> Self {
        Self::with_config(fast_config()).await
    }

    async fn with_config(config: RuntimeConfig) -> Self {
        let harness = Harness::with_options(HarnessOptions {
            use_tools: true,
            config,
            ..HarnessOptions::default()
        })
        .await;
        let executor = FakeExecutor::new();
        harness
            .runtime
            .add_executor(Arc::<FakeExecutor>::clone(&executor))
            .unwrap();
        Self { harness, executor }
    }

    /// Starts a general conversation with the owner's message.
    async fn start(&self, text: &str) -> (ConversationId, RunId) {
        let started = self
            .harness
            .runtime
            .start_conversation(StartConversation {
                mentions: Vec::new(),
                text: text.to_owned(),
                profile_id: None,
                project_id: None,
            })
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

    /// Publishes a state of the executor's task, as its plugin does.
    async fn publish(&self, state: mannie_types::tasks::AssignedTaskState) {
        self.harness
            .runtime
            .executor_services()
            .publish_task_state(FAKE_EXECUTOR_KIND, state)
            .await
            .unwrap();
    }
}

fn give(target: &str, title: &str, instruction: &str, background: bool) -> String {
    json!({
        "activity": format!("Giving {title}"),
        "target": target,
        "title": title,
        "instruction": instruction,
        "background": background,
    })
    .to_string()
}

fn wait(tasks: &[u32]) -> String {
    json!({ "activity": "Waiting for reports", "tasks": tasks }).to_string()
}

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

fn tool_names(request: &ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

#[tokio::test]
async fn a_registered_executors_task_returns_its_report_as_the_result() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &give("box-01", "Build it", "Build the backend.", false),
    )]));
    setup.parent(reply("The box built it."));

    let (conversation, run) = setup.start("OWNER: build it on the box").await;
    let [submission] = setup.submissions(1).await.try_into().unwrap();
    setup
        .publish(FakeExecutor::state(&submission, 2, TaskState::Running))
        .await;
    setup
        .publish(FakeExecutor::reported(&submission, 3, "Built and tested."))
        .await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    assert_eq!(submission.number, 1);
    assert_eq!(submission.title, "Build it");
    assert_eq!(submission.instruction, "Build the backend.");
    assert!(!submission.follow_up);
    assert_eq!(submission.parent_id, conversation);
    assert_eq!(submission.target, FakeExecutor::target("box-01"));
    let parent = setup.harness.model.requests_for("OWNER:");
    let tools = tool_names(&parent[0]);
    assert!(tools.contains(&FAKE_EXECUTOR_TOOL.to_owned()), "{tools:?}");
    assert!(tools.contains(&"start_task".to_owned()), "{tools:?}");
    assert!(parent[0].instructions.contains(FAKE_EXECUTOR_SECTION));
    assert_eq!(
        result(&parent[1], "call_1"),
        texts::task_reported(1, "Built and tested.")
    );
    let [task] = transcript.tasks(0).try_into().unwrap();
    assert_eq!(task.state, TaskState::Reported);
    assert_eq!(task.agent, FAKE_EXECUTOR_AGENT);
    assert_eq!(task.executor, Some(FakeExecutor::target("box-01")));
    assert_eq!(task.report.as_deref(), Some("Built and tested."));
    let summary = setup
        .harness
        .store
        .conversation_summary(&task.conversation_id)
        .await
        .unwrap()
        .unwrap();
    let origin = summary.task.unwrap();
    assert_eq!(origin.executor, Some(FakeExecutor::target("box-01")));
    assert_eq!(origin.parent_id, conversation);
    let stored = setup
        .harness
        .store
        .task(&task.task_id)
        .await
        .unwrap()
        .unwrap();
    let execution = stored.executor.unwrap();
    assert_eq!(execution.kind, FAKE_EXECUTOR_KIND);
    assert_eq!(execution.version, 3);
    assert_eq!(execution.document.as_deref(), Some(r#"{"version":1}"#));
    let listed = setup.harness.store.list_conversations().await.unwrap();
    assert_eq!(listed.len(), 1, "the task conversation is not listed");
}

#[tokio::test]
async fn mannie_without_executors_offers_their_tools_to_no_run() {
    let harness = Harness::with_options(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await;
    harness.model.push_reply(reply("Nothing to give."));

    let started = harness
        .runtime
        .start_conversation(StartConversation {
            mentions: Vec::new(),
            text: "OWNER: anything?".to_owned(),
            profile_id: None,
            project_id: None,
        })
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let request = &harness.model.requests_for("OWNER:")[0];
    let tools = tool_names(request);
    assert!(!tools.contains(&FAKE_EXECUTOR_TOOL.to_owned()), "{tools:?}");
    assert!(tools.contains(&"start_task".to_owned()), "{tools:?}");
    assert!(!request.instructions.contains(FAKE_EXECUTOR_SECTION));
}

#[tokio::test]
async fn a_refused_call_keeps_the_executors_text_for_the_model_and_its_code_for_the_owner() {
    let setup = Setup::new().await;
    setup.executor.refuse_plans(ExecutorRefusal {
        code: "offline".to_owned(),
        target: Some("box-01".to_owned()),
        text: "Not run: box-01 is offline.".to_owned(),
    });
    setup.parent(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &give("box-01", "Build it", "Build the backend.", false),
    )]));
    setup.parent(reply("The box is offline."));

    let (conversation, run) = setup.start("OWNER: build it").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(result(&parent[1], "call_1"), "Not run: box-01 is offline.");
    let [call] = transcript.task_calls(0).try_into().unwrap();
    assert_eq!(
        call.refusal,
        Some(TaskRefusal::Executor {
            kind: FAKE_EXECUTOR_KIND.to_owned(),
            code: "offline".to_owned(),
            target: Some("box-01".to_owned()),
        })
    );
    assert_eq!(call.task, None);
    assert_eq!(setup.executor.submissions().len(), 0);
}

#[tokio::test]
async fn a_background_task_reports_through_wait_for_tasks() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &give("box-01", "Build it", "Build the backend.", true),
    )]));
    setup.parent(tool_calls(&[("call_2", "wait_for_tasks", &wait(&[]))]));
    setup.parent(reply("It reported."));

    let (conversation, run) = setup.start("OWNER: in the background").await;
    let [submission] = setup.submissions(1).await.try_into().unwrap();
    setup
        .publish(FakeExecutor::reported(&submission, 2, "Built."))
        .await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(
        result(&parent[1], "call_1"),
        texts::task_started_in_background_on(1, FAKE_EXECUTOR_NOUN, "box-01")
    );
    assert_eq!(
        result(&parent[2], "call_2"),
        texts::task_reported(1, "Built.")
    );
}

#[tokio::test]
async fn a_follow_up_goes_to_the_target_of_the_earlier_task() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &give("box-01", "Build it", "Build the backend.", false),
    )]));
    setup.parent(tool_calls(&[(
        "call_2",
        "follow_up_task",
        &follow_up(1, "Now run the tests."),
    )]));
    setup.parent(reply("Built and tested."));

    let (conversation, run) = setup.start("OWNER: build, then test").await;
    let [first] = setup.submissions(1).await.try_into().unwrap();
    setup
        .publish(FakeExecutor::reported(&first, 2, "Built."))
        .await;
    let submissions = setup.submissions(2).await;
    let second = submissions[1].clone();
    setup
        .publish(FakeExecutor::reported(&second, 3, "Tested."))
        .await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    assert_eq!(
        setup.executor.follow_ups(),
        [FakeExecutor::target("box-01")]
    );
    assert!(second.follow_up);
    assert_eq!(second.number, 1);
    assert_eq!(second.conversation_id, first.conversation_id);
    assert_eq!(second.instruction, "Now run the tests.");
    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(
        result(&parent[2], "call_2"),
        texts::task_reported(1, "Tested.")
    );
}

#[tokio::test]
async fn stopping_the_reply_cancels_the_task_and_lets_it_go_until_a_wait_takes_it_up() {
    let setup = Setup::with_config(RuntimeConfig {
        executor_cancel_timeout: Duration::from_millis(200),
        ..fast_config()
    })
    .await;
    setup.parent(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &give("box-01", "Build it", "Build the backend.", false),
    )]));

    let (conversation, run) = setup.start("OWNER: build it").await;
    let [submission] = setup.submissions(1).await.try_into().unwrap();
    let stopped = setup.harness.runtime.stop_reply(&conversation).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(stopped, StopOutcome::Stopped);
    assert_eq!(transcript.run_state(&run), Some(RunState::Stopped));
    assert_eq!(
        setup.executor.cancels(),
        std::slice::from_ref(&submission.task_id)
    );
    let [task] = transcript.tasks(0).try_into().unwrap();
    assert!(task.detached, "the part lets the task go");
    let stored = setup
        .harness
        .store
        .task(&submission.task_id)
        .await
        .unwrap()
        .unwrap();
    assert!(stored.state.is_active(), "the task runs on");

    // The next reply's wait takes the task up again and reads its report.
    setup.parent(tool_calls(&[("call_2", "wait_for_tasks", &wait(&[1]))]));
    setup.parent(reply("It reported after all."));
    let run = setup
        .harness
        .runtime
        .send_message(mannie_types::chat::SendMessage {
            mentions: Vec::new(),
            conversation_id: conversation.clone(),
            text: "OWNER-2: did it finish?".to_owned(),
        })
        .await
        .unwrap()
        .run_id;
    setup
        .publish(FakeExecutor::reported(&submission, 2, "Built late."))
        .await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    // The second turn reads the first, so its requests start with it too.
    let requests = setup.harness.model.requests_for("OWNER:");
    assert_eq!(
        result(requests.last().unwrap(), "call_2"),
        texts::task_reported(1, "Built late.")
    );
}

#[tokio::test]
async fn a_call_of_a_stopped_plugins_tool_does_not_run() {
    let setup = Setup::new().await;
    let mut held = vec![Step::Delay(Duration::from_millis(300))];
    held.extend(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &give("box-01", "Build it", "Build the backend.", false),
    )]));
    setup.parent(held);
    setup.parent(reply("The plugin is off."));

    let (conversation, run) = setup.start("OWNER: build it").await;
    let model = setup.harness.model.clone();
    tokio::time::timeout(WAIT, async move {
        while model.requests_for("OWNER:").is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    // The run started with the tool, and the plugin stops before the call.
    assert!(
        setup
            .harness
            .runtime
            .remove_executor(FAKE_EXECUTOR_KIND)
            .is_some()
    );
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let parent = setup.harness.model.requests_for("OWNER:");
    assert!(tool_names(&parent[1]).contains(&FAKE_EXECUTOR_TOOL.to_owned()));
    assert_eq!(
        result(&parent[1], "call_1"),
        texts::tool_plugin_off(FAKE_EXECUTOR_KIND)
    );
    let [call] = transcript.task_calls(0).try_into().unwrap();
    assert!(matches!(
        call.refusal,
        Some(TaskRefusal::Executor { ref code, .. }) if code == "plugin_off"
    ));
    assert_eq!(setup.executor.submissions().len(), 0);
}

#[tokio::test]
async fn arguments_that_are_not_valid_do_not_run_as_for_any_tool() {
    let setup = Setup::new().await;
    let missing_target = json!({
        "activity": "Giving",
        "title": "Build it",
        "instruction": "Build the backend.",
        "background": false,
    })
    .to_string();
    setup.parent(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &missing_target,
    )]));
    setup.parent(reply("The call was not valid."));

    let (conversation, run) = setup.start("OWNER: build it").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.task_calls(0).try_into().unwrap();
    assert!(
        matches!(call.not_run, Some(NotRunReason::InvalidArguments { ref detail }) if detail.contains("target")),
        "{:?}",
        call.not_run
    );
    assert_eq!(call.refusal, None);
    assert_eq!(setup.executor.submissions().len(), 0);
}

#[tokio::test]
async fn turning_the_plugin_off_lets_a_running_task_go_and_a_later_wait_takes_it_up() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &give("box-01", "Build it", "Build the backend.", false),
    )]));
    setup.parent(reply("The plugin went off."));

    let (conversation, run) = setup.start("OWNER: build it").await;
    let [submission] = setup.submissions(1).await.try_into().unwrap();
    setup.harness.runtime.remove_executor(FAKE_EXECUTOR_KIND);
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(
        result(&parent[1], "call_1"),
        texts::task_let_go(1, FAKE_EXECUTOR_NOUN, "box-01")
    );
    let [task] = transcript.tasks(0).try_into().unwrap();
    assert!(task.detached);
    assert_eq!(setup.executor.cancels(), Vec::new(), "nothing is canceled");
    let stored = setup
        .harness
        .store
        .task(&submission.task_id)
        .await
        .unwrap()
        .unwrap();
    assert!(stored.state.is_active(), "the task runs on");

    // The plugin starts again, and a wait of the next reply reads the report.
    let executor: Arc<FakeExecutor> = Arc::clone(&setup.executor);
    setup.harness.runtime.add_executor(executor).unwrap();
    setup.parent(tool_calls(&[("call_2", "wait_for_tasks", &wait(&[1]))]));
    setup.parent(reply("It reported."));
    let run = setup
        .harness
        .runtime
        .send_message(mannie_types::chat::SendMessage {
            mentions: Vec::new(),
            conversation_id: conversation.clone(),
            text: "OWNER-2: did it finish?".to_owned(),
        })
        .await
        .unwrap()
        .run_id;
    setup
        .publish(FakeExecutor::reported(&submission, 2, "Built."))
        .await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let requests = setup.harness.model.requests_for("OWNER:");
    assert_eq!(
        result(requests.last().unwrap(), "call_2"),
        texts::task_reported(1, "Built.")
    );
}

#[tokio::test]
async fn a_background_task_that_was_let_go_is_named_by_the_wait_and_a_wait_while_off_does_not_run()
{
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &give("box-01", "Build it", "Build the backend.", true),
    )]));
    let mut held = vec![Step::Delay(Duration::from_millis(300))];
    held.extend(tool_calls(&[("call_2", "wait_for_tasks", &wait(&[]))]));
    setup.parent(held);
    setup.parent(tool_calls(&[("call_3", "wait_for_tasks", &wait(&[1]))]));
    setup.parent(reply("Workers are off."));

    let (conversation, run) = setup.start("OWNER: in the background").await;
    setup.submissions(1).await;
    setup.harness.runtime.remove_executor(FAKE_EXECUTOR_KIND);
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(
        result(&parent[1], "call_1"),
        texts::task_started_in_background_on(1, FAKE_EXECUTOR_NOUN, "box-01")
    );
    assert_eq!(
        result(&parent[2], "call_2"),
        texts::task_let_go(1, FAKE_EXECUTOR_NOUN, "box-01")
    );
    assert_eq!(
        result(&parent[3], "call_3"),
        texts::task_plugin_off(1, FAKE_EXECUTOR_KIND)
    );
    let calls = transcript.task_calls(0);
    assert_eq!(
        calls[2].refusal,
        Some(TaskRefusal::Executor {
            kind: FAKE_EXECUTOR_KIND.to_owned(),
            code: "plugin_off".to_owned(),
            target: Some("box-01".to_owned()),
        })
    );
}

#[tokio::test]
async fn a_submission_that_the_plugins_stop_cut_short_lets_the_task_go() {
    let setup = Setup::new().await;
    setup.executor.stop_submissions();
    setup.parent(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &give("box-01", "Build it", "Build the backend.", true),
    )]));
    setup.parent(reply("It may run there."));

    let (conversation, run) = setup.start("OWNER: build it").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(
        result(&parent[1], "call_1"),
        texts::task_let_go(1, FAKE_EXECUTOR_NOUN, "box-01")
    );
    let [call] = transcript.task_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    let task = call.task.unwrap();
    assert!(task.detached);
    let [submission] = setup.executor.submissions().try_into().unwrap();
    let stored = setup
        .harness
        .store
        .task(&submission.task_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        stored.state.is_active(),
        "the target may have stored the task"
    );
}

#[tokio::test]
async fn the_conversation_of_an_executors_task_is_read_through_its_executor() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &give("box-01", "Build it", "Build the backend.", false),
    )]));
    setup.parent(reply("Done."));
    let (conversation, run) = setup.start("OWNER: build it").await;
    let [submission] = setup.submissions(1).await.try_into().unwrap();
    setup
        .publish(FakeExecutor::reported(&submission, 2, "Built."))
        .await;
    setup.harness.wait_for_run(&conversation, &run).await;
    let runtime = &setup.harness.runtime;
    let earlier = || LoadEarlierTurns {
        conversation_id: submission.conversation_id.clone(),
        before_position: 10,
        limit: 10,
    };

    let read = runtime.earlier_turns(earlier()).await;
    setup.executor.set_unreachable(true);
    let unreachable = runtime.earlier_turns(earlier()).await;
    runtime.remove_executor(FAKE_EXECUTOR_KIND);
    let off = runtime.earlier_turns(earlier()).await;
    let subscribed = runtime
        .subscribe(SubscribeConversation {
            conversation_id: submission.conversation_id.clone(),
            after_seq: None,
        })
        .await;

    assert_eq!(read.unwrap().turns, Vec::new());
    assert!(
        matches!(unreachable, Err(ChatError::TaskTargetUnreachable { ref kind, .. }) if kind == FAKE_EXECUTOR_KIND),
        "{unreachable:?}"
    );
    assert_eq!(
        off.unwrap_err(),
        ChatError::PluginOff {
            plugin: FAKE_EXECUTOR_KIND.to_owned()
        }
    );
    assert!(matches!(subscribed, Err(ChatError::PluginOff { .. })));
}

#[tokio::test]
async fn deleting_the_parent_leaves_the_deletion_in_the_outbox_for_the_executor() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &give("box-01", "Build it", "Build the backend.", false),
    )]));
    setup.parent(reply("Done."));
    let (conversation, run) = setup.start("OWNER: build it").await;
    let [submission] = setup.submissions(1).await.try_into().unwrap();
    setup
        .publish(FakeExecutor::reported(&submission, 2, "Built."))
        .await;
    setup.harness.wait_for_run(&conversation, &run).await;
    let mut changes = setup.harness.runtime.executor_services().outbox_changes();
    changes.borrow_and_update();

    setup
        .harness
        .runtime
        .delete_conversation(&conversation)
        .await
        .unwrap();

    assert!(changes.has_changed().unwrap(), "the executors are told");
    assert_eq!(
        setup
            .harness
            .runtime
            .executor_services()
            .executor_outbox(FAKE_EXECUTOR_KIND)
            .await
            .unwrap(),
        [OutboxEntry {
            target: FakeExecutor::target("box-01").target_id,
            conversation_id: submission.conversation_id.clone(),
        }]
    );
}

#[tokio::test]
async fn a_state_of_another_executor_or_an_older_one_changes_nothing() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &give("box-01", "Build it", "Build the backend.", true),
    )]));
    setup.parent(reply("Started."));
    let (conversation, run) = setup.start("OWNER: start it").await;
    let [submission] = setup.submissions(1).await.try_into().unwrap();
    setup
        .publish(FakeExecutor::state(&submission, 5, TaskState::Running))
        .await;
    let services = setup.harness.runtime.executor_services();

    let other_kind = services
        .publish_task_state(
            "other",
            FakeExecutor::state(&submission, 6, TaskState::Running),
        )
        .await
        .unwrap();
    let older = services
        .publish_task_state(
            FAKE_EXECUTOR_KIND,
            FakeExecutor::reported(&submission, 4, "Too late."),
        )
        .await
        .unwrap();
    let newer = services
        .publish_task_state(
            FAKE_EXECUTOR_KIND,
            FakeExecutor::reported(&submission, 6, "Built."),
        )
        .await
        .unwrap();
    setup.harness.wait_for_run(&conversation, &run).await;

    assert!(other_kind.is_none());
    assert!(older.is_none());
    assert!(newer.is_some());
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
