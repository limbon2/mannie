//! Task executors (ADR-0031, decision 4): what a plugin contributes to run
//! the tasks of a reply somewhere else, such as on a worker.
//!
//! Child agents are the core's own executor. An executor's task is a task of
//! ADR-0024 all the same: a `task` part with a number among the
//! conversation's children, on the run's board and in its lane, with
//! background tasks, follow-ups and waits. The core offers the executor's
//! tool among the task tools, stores each task before the executor submits
//! it, and keeps the executor's document on the task's row. The executor
//! submits, follows and cancels its tasks, and publishes each state to the
//! core, which stores it in the task's row and shows it on the board of the
//! run that follows the task.
//!
//! An executor's kind follows the rule `PluginKind`: it is exactly the ID of
//! the plugin that contributes it (ADR-0034, decision 6).
//!
//! SDK 1 moves the contract in its current shape (ADR-0034, decision 3), so
//! `tests/protocol.rs` lists this module as a known exception: `kind` and
//! `agent` lend `&'static str`, `plan` takes a [`PlanCall`] that borrows the
//! call's arguments and profile, `kind`, `agent`, `tool`, `section`,
//! `outlives_reply`, `plan`, `plan_follow_up` and `document` answer at once
//! instead of returning futures, and `plan_follow_up`, `document` and every
//! method that returns a future apart from `submit` borrow some of their
//! arguments, such as the target, the profile, the task or the submission.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::contracts::{
    AssignedTaskState, ChatError, Checkpoint, CheckpointRef, ConversationId, ConversationUpdate,
    DecideApproval, EarlierTurns, LoadEarlierTurns, ProfileSnapshot, ReadToolOutput,
    TaskExecutorRef, TaskId, ToolOutputPage, ToolsError,
};
use crate::contributions::{ExtensionPoint, IdRule};
use crate::points::tools::ToolSpec;
use crate::{BoxFuture, BoxStream};

/// The point of plugins' task executors, whose entries are named by their
/// kinds, so a plugin has at most one.
#[derive(Debug)]
pub enum Executors {}

impl ExtensionPoint for Executors {
    const NAME: &'static str = "executors";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::PluginKind;
    type Entry = Arc<dyn TaskExecutor>;

    fn id(entry: &Arc<dyn TaskExecutor>) -> &str {
        entry.kind()
    }
}

/// A call of an executor's tool, as the executor plans it.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct PlanCall<'a> {
    /// The call's arguments as the model wrote them, without the activity.
    pub arguments: &'a str,
    /// The provider profile of the reply that makes the call, without its
    /// credential.
    pub profile: &'a ProfileSnapshot,
}

#[cfg(feature = "host")]
impl<'a> PlanCall<'a> {
    /// A call with its arguments and the profile of its reply. Only the core
    /// builds one.
    pub fn new(arguments: &'a str, profile: &'a ProfileSnapshot) -> Self {
        Self { arguments, profile }
    }
}

/// A task that an executor takes on, as it planned a call of its tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct TaskPlan {
    /// The target that runs the task, such as a worker.
    pub target: TaskExecutorRef,
    /// The task's title, cleaned for the owner.
    pub title: String,
    /// The instruction.
    pub instruction: String,
    /// Whether the reply continues while the target works.
    pub background: bool,
    /// What else the executor needs to submit the task, which the core
    /// passes on as it is.
    pub options: serde_json::Value,
}

impl TaskPlan {
    /// A task for `target`. The title and the instruction are easy to swap,
    /// so callers pass named locals.
    pub fn new(
        target: TaskExecutorRef,
        title: String,
        instruction: String,
        background: bool,
        options: serde_json::Value,
    ) -> Self {
        Self {
            target,
            title,
            instruction,
            background,
            options,
        }
    }
}

/// The target of a follow-up, as an executor takes it on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct FollowUpPlan {
    /// The target that runs the earlier task, as it is named now.
    pub target: TaskExecutorRef,
    /// What else the executor needs to submit the task.
    pub options: serde_json::Value,
}

impl FollowUpPlan {
    /// A follow-up on `target`.
    pub fn new(target: TaskExecutorRef, options: serde_json::Value) -> Self {
        Self { target, options }
    }
}

/// Why an executor did not plan a call of its tool. The enum stays
/// exhaustive, because plugins produce its values and the core matches them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanError {
    /// The arguments are not valid. The call does not run, as a call of any
    /// tool with arguments that are not valid does.
    InvalidArguments {
        /// What is wrong, for the model.
        detail: String,
    },
    /// The executor refused the task.
    Refused(ExecutorRefusal),
}

/// Why a submission did not return a first state. The enum stays exhaustive,
/// because plugins produce its values and the core matches them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmitError {
    /// The executor refused the task, which its target never stored.
    Refused(ExecutorRefusal),
    /// The executor's plugin stopped before the executor knew whether the
    /// target stored the task. The run lets the task go, as quitting does,
    /// and the executor learns what became of it when it starts again.
    Stopped,
}

/// Why an executor did not take a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ExecutorRefusal {
    /// The executor's code for the refusal, which the UI shows a message of
    /// the plugin's for.
    pub code: String,
    /// The name of the target that the call named, if it named one.
    pub target: Option<String>,
    /// The result that the model reads.
    pub text: String,
}

impl ExecutorRefusal {
    /// A refusal with the plugin's code, the target that the call named, if
    /// it named one, and the text that the model reads.
    pub fn new(code: impl Into<String>, target: Option<String>, text: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            target,
            text: text.into(),
        }
    }
}

/// A task that the core stored for an executor, which the executor submits
/// to its target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct TaskSubmission {
    /// The task, which the core chose and stored first, so a submission
    /// that is sent again finds the same task.
    pub task_id: TaskId,
    /// The task's conversation.
    pub conversation_id: ConversationId,
    /// The parent conversation.
    pub parent_id: ConversationId,
    /// The task's number among the parent's children.
    pub number: u32,
    /// Whether the task continues an earlier task of its conversation.
    pub follow_up: bool,
    /// The task's title.
    pub title: String,
    /// The instruction.
    pub instruction: String,
    /// The target.
    pub target: TaskExecutorRef,
    /// What the executor's plan added.
    pub options: serde_json::Value,
    /// The provider profile of the reply that gave the task, without its
    /// credential.
    pub profile: ProfileSnapshot,
}

#[cfg(feature = "host")]
impl TaskSubmission {
    /// A new task for `target`, with an empty title and instruction, no
    /// options, and not a follow-up; the `with_` methods set the rest. Only
    /// the core builds one. The two conversations are easy to swap, so
    /// callers pass named locals.
    pub fn new(
        task_id: TaskId,
        conversation_id: ConversationId,
        parent_id: ConversationId,
        number: u32,
        target: TaskExecutorRef,
        profile: ProfileSnapshot,
    ) -> Self {
        Self {
            task_id,
            conversation_id,
            parent_id,
            number,
            follow_up: false,
            title: String::new(),
            instruction: String::new(),
            target,
            options: serde_json::Value::Null,
            profile,
        }
    }

    /// The submission with the task's title.
    #[must_use]
    pub fn with_title(mut self, title: String) -> Self {
        self.title = title;
        self
    }

    /// The submission with the task's instruction.
    #[must_use]
    pub fn with_instruction(mut self, instruction: String) -> Self {
        self.instruction = instruction;
        self
    }

    /// The submission with what the executor's plan added.
    #[must_use]
    pub fn with_options(mut self, options: serde_json::Value) -> Self {
        self.options = options;
        self
    }

    /// The submission of a task that continues an earlier task of its
    /// conversation, or of one that does not.
    #[must_use]
    pub fn with_follow_up(mut self, follow_up: bool) -> Self {
        self.follow_up = follow_up;
        self
    }
}

/// The instruction section that an executor adds for the runs that are
/// offered its tool, as it is when a run starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ExecutorSection {
    /// Where the section goes among the sections.
    pub position: u32,
    /// The section's text.
    pub text: String,
}

impl ExecutorSection {
    /// A section at a position, with its text.
    pub fn new(position: u32, text: impl Into<String>) -> Self {
        Self {
            position,
            text: text.into(),
        }
    }
}

/// What a plugin contributes to run tasks somewhere else (ADR-0031,
/// decision 4). Every call may fail with the target unreachable; the calls
/// that read a task conversation answer with that conversation's errors.
pub trait TaskExecutor: Send + Sync + fmt::Debug + 'static {
    /// The executor's kind: the ID of the plugin that contributes it.
    fn kind(&self) -> &'static str;

    /// The agent that the core records for the executor's tasks.
    fn agent(&self) -> &'static str;

    /// The executor's tool, as a run that starts now is offered it, or
    /// `None` while the executor offers none. The core reads it once for
    /// each run, and adds the activity argument that every call carries for
    /// the owner, so the spec has none.
    fn tool(&self) -> Option<ToolSpec>;

    /// The instruction section for the runs that are offered the tool, read
    /// once for each run, or `None`.
    fn section(&self) -> Option<ExecutorSection>;

    /// Whether the executor's tasks outlive the reply that gave them, as a
    /// worker's do (ADR-0027, decision 6): a stop cancels them without
    /// waiting longer than the core's cancel timeout, and quitting leaves
    /// them running.
    fn outlives_reply(&self) -> bool;

    /// Takes on a call of the executor's tool, or refuses it. The core
    /// checks the reply's task budget between arguments that are not valid
    /// and a refusal, as it does for `start_task`.
    fn plan(&self, call: PlanCall<'_>) -> Result<TaskPlan, PlanError>;

    /// Takes on a follow-up of an earlier task of the executor's, on the
    /// target that ran it, or refuses it.
    fn plan_follow_up(
        &self,
        earlier: &TaskExecutorRef,
        profile: &ProfileSnapshot,
    ) -> Result<FollowUpPlan, ExecutorRefusal>;

    /// The executor's document of a new task, which the core stores with the
    /// task before it submits it.
    fn document(&self, submission: &TaskSubmission) -> Option<String>;

    /// Submits a task that the core stored, which the target stores once for
    /// its ID, and returns its first state. A target that cannot be reached
    /// is tried again until the executor gives up; a stop of the run drops
    /// the future.
    fn submit(
        &self,
        submission: TaskSubmission,
    ) -> BoxFuture<'static, Result<AssignedTaskState, SubmitError>>;

    /// Asks the target to cancel a task. The executor keeps the cancel until
    /// the target takes it or the task ends.
    fn cancel(&self, task: &TaskId) -> BoxFuture<'static, ()>;

    /// Follows a task conversation on its target, from a cursor or from a
    /// snapshot. The stream ends when the target can no longer be reached.
    fn subscribe(
        &self,
        target: &TaskExecutorRef,
        conversation: &ConversationId,
        after_seq: Option<u32>,
    ) -> BoxFuture<'static, Result<BoxStream<'static, ConversationUpdate>, ChatError>>;

    /// A complete saved record, distinct from shortened UI views (ADR-0040).
    /// An executor without this capability is explicitly unsupported.
    fn export_record(
        &self,
        _target: &TaskExecutorRef,
        _conversation: &ConversationId,
    ) -> BoxFuture<
        'static,
        Result<crate::contracts::ConversationRecord, crate::contracts::ExportError>,
    > {
        Box::pin(async { Err(crate::contracts::ExportError::Unsupported) })
    }

    /// Earlier turns of a task conversation on its target.
    fn earlier_turns(
        &self,
        target: &TaskExecutorRef,
        input: LoadEarlierTurns,
    ) -> BoxFuture<'static, Result<EarlierTurns, ChatError>>;

    /// A checkpoint of a task conversation on its target.
    fn checkpoint(
        &self,
        target: &TaskExecutorRef,
        input: CheckpointRef,
    ) -> BoxFuture<'static, Result<Checkpoint, ChatError>>;

    /// A page of an archived output of a task conversation on its target.
    fn tool_output(
        &self,
        target: &TaskExecutorRef,
        input: ReadToolOutput,
    ) -> BoxFuture<'static, Result<ToolOutputPage, ToolsError>>;

    /// The owner's decision about a call of a task on its target.
    fn decide_approval(
        &self,
        target: &TaskExecutorRef,
        input: DecideApproval,
    ) -> BoxFuture<'static, Result<(), ToolsError>>;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ExecutorRefusal, ExecutorSection, FollowUpPlan, TaskPlan};
    use crate::contracts::TaskExecutorRef;

    fn target() -> TaskExecutorRef {
        TaskExecutorRef {
            kind: "workers".to_owned(),
            target_id: "0199e0f2-0000-7000-8000-0000000000aa".to_owned(),
            target_name: "build-01".to_owned(),
            noun: "the worker".to_owned(),
        }
    }

    #[test]
    fn the_constructors_keep_each_argument_in_its_field() {
        let title = "Build it".to_owned();
        let instruction = "Build the backend.".to_owned();

        let plan = TaskPlan::new(
            target(),
            title,
            instruction,
            true,
            json!({ "readOnly": true }),
        );
        let follow_up = FollowUpPlan::new(target(), json!({ "readOnly": false }));
        let refusal = ExecutorRefusal::new(
            "worker_offline",
            Some("build-01".to_owned()),
            "Not run: build-01 is offline.",
        );
        let section = ExecutorSection::new(310, "Workers you can give tasks to: build-01.");

        assert_eq!(plan.target, target());
        assert_eq!(plan.title, "Build it");
        assert_eq!(plan.instruction, "Build the backend.");
        assert!(plan.background);
        assert_eq!(plan.options, json!({ "readOnly": true }));
        assert_eq!(follow_up.target, target());
        assert_eq!(follow_up.options, json!({ "readOnly": false }));
        assert_eq!(refusal.code, "worker_offline");
        assert_eq!(refusal.target.as_deref(), Some("build-01"));
        assert_eq!(refusal.text, "Not run: build-01 is offline.");
        assert_eq!(section.position, 310);
        assert_eq!(section.text, "Workers you can give tasks to: build-01.");
    }

    #[test]
    fn a_refusal_may_name_no_target() {
        let refusal = ExecutorRefusal::new("plugin_off", None, "Not run: the plugin is off.");

        assert_eq!(refusal.target, None);
    }

    #[cfg(feature = "host")]
    mod host {
        use serde_json::json;

        use super::target;
        use crate::contracts::{ConversationId, ProfileSnapshot, TaskId};
        use crate::points::executors::{PlanCall, TaskSubmission};

        fn profile() -> ProfileSnapshot {
            serde_json::from_value(json!({
                "id": "0199e0f2-0000-7000-8000-0000000000bb",
                "configRevision": 3,
                "interface": "chat_completions",
                "baseUrl": "https://models.example.com/v1",
                "model": "model-1",
                "contextWindow": 128_000,
                "outputLimit": 8_000,
                "reasoningEffort": "server_default",
                "useTools": true,
                "hasCredential": true
            }))
            .unwrap()
        }

        #[test]
        fn a_plan_call_lends_the_arguments_and_the_profile() {
            let profile = profile();
            let arguments = r#"{"worker":"build-01"}"#.to_owned();

            let call = PlanCall::new(&arguments, &profile);

            assert_eq!(call.arguments, arguments);
            assert_eq!(*call.profile, profile);
        }

        #[test]
        fn a_new_submission_is_a_first_task_until_the_with_methods_set_the_rest() {
            let task_id = TaskId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap();
            let conversation_id =
                ConversationId::parse("0199e0f2-0000-7000-8000-000000000002").unwrap();
            let parent_id = ConversationId::parse("0199e0f2-0000-7000-8000-000000000003").unwrap();

            let new = TaskSubmission::new(
                task_id.clone(),
                conversation_id.clone(),
                parent_id.clone(),
                4,
                target(),
                profile(),
            );
            let set = new
                .clone()
                .with_title("Build it".to_owned())
                .with_instruction("Build the backend.".to_owned())
                .with_options(json!({ "readOnly": true }))
                .with_follow_up(true);

            assert_eq!(new.task_id, task_id);
            assert_eq!(new.conversation_id, conversation_id);
            assert_eq!(new.parent_id, parent_id);
            assert_eq!(new.number, 4);
            assert_eq!(new.target, target());
            assert_eq!(new.profile, profile());
            assert!(!new.follow_up);
            assert_eq!(new.title, "");
            assert_eq!(new.instruction, "");
            assert_eq!(new.options, serde_json::Value::Null);
            assert_eq!(set.title, "Build it");
            assert_eq!(set.instruction, "Build the backend.");
            assert_eq!(set.options, json!({ "readOnly": true }));
            assert!(set.follow_up);
            assert_eq!(set.task_id, new.task_id);
            assert_eq!(set.conversation_id, new.conversation_id);
            assert_eq!(set.parent_id, new.parent_id);
            assert_eq!(set.number, new.number);
        }
    }
}
