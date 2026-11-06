//! Questions to the owner (docs/features/questions.md; ADR-0058): a call of
//! `ask_owner` waits for the owner's reply, the model reads the reply as a
//! result that only the core writes, the reply allows nothing, and Stop, a
//! restart and a retry behave as they do for an approval.

#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::points::executors::{Executors, TaskExecutor};
use mannie_provider::{Content, ModelRequest, Role};
use mannie_runtime::testing::{
    FAKE_EXECUTOR_KIND, FakeExecutor, Harness, HarnessOptions, SeenPart, Transcript, WorkerHarness,
    add_plugin, fast_config, reply, tool_calls,
};
use mannie_runtime::{RuntimeConfig, texts};
use mannie_types::chat::{SendMessage, StartConversation};
use mannie_types::ids::{ConversationId, PartId, ProjectId, RunId};
use mannie_types::run::{RunActivity, RunState};
use mannie_types::tools::{
    AnswerQuestion, ApprovalReason, NotRunReason, Question, QuestionAnswer, QuestionOption,
    QuestionReply, ToolCallStatus, ToolCallView, ToolOutcome, ToolsError, UpdateToolsSettings,
};
use mannie_types::transcript::{ConversationSummary, PartContent};
use serde_json::json;
use tempfile::TempDir;

const ASK_OWNER: &str = "ask_owner";

/// A runtime whose profile uses tools, and a project whose folder is the
/// workspace of the conversations that start in it.
struct Setup {
    harness: Harness,
    _workspace: TempDir,
    root: PathBuf,
    project: ProjectId,
}

impl Setup {
    async fn new() -> Self {
        Self::with_options(HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        })
        .await
    }

    async fn with_options(options: HarnessOptions) -> Self {
        let harness = Harness::with_options(options).await;
        let workspace = tempfile::tempdir().unwrap();
        let folder = workspace.path().join("mannie");
        fs::create_dir_all(&folder).unwrap();
        let project = harness
            .runtime
            .add_project(folder.to_str().unwrap().to_owned())
            .await
            .unwrap();
        Self {
            harness,
            _workspace: workspace,
            root: PathBuf::from(&project.folder),
            project: project.id,
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Starts a conversation in the project.
    async fn start(&self, text: &str) -> (ConversationId, RunId) {
        let started = self
            .harness
            .runtime
            .start_conversation(
                StartConversation::new(text).with_project(Some(self.project.clone())),
            )
            .await
            .unwrap();
        (started.conversation_id, started.run_id)
    }

    /// Waits until a call waits for an answer, and returns its part and its
    /// questions.
    async fn waiting(&self, conversation: &ConversationId) -> (PartId, Vec<Question>) {
        wait_for_question(&self.harness, conversation).await
    }

    async fn summary(&self, conversation: &ConversationId) -> ConversationSummary {
        self.harness
            .store
            .conversation_summary(conversation)
            .await
            .unwrap()
            .unwrap()
    }
}

async fn wait_for_question(
    harness: &Harness,
    conversation: &ConversationId,
) -> (PartId, Vec<Question>) {
    let transcript = harness
        .wait_until(conversation, |transcript| {
            waiting_question(transcript).is_some()
        })
        .await;
    waiting_question(&transcript).unwrap()
}

/// The call that waits for an answer, if one does.
fn waiting_question(transcript: &Transcript) -> Option<(PartId, Vec<Question>)> {
    transcript.parts(0).into_iter().find_map(|part| match part {
        SeenPart {
            id,
            content: PartContent::ToolCall { call },
            ..
        } if call.status == ToolCallStatus::AwaitingAnswer => Some((id, call.questions.clone())),
        _ => None,
    })
}

fn reply_of(
    harness: &Harness,
    conversation: &ConversationId,
    part: &PartId,
    reply: QuestionReply,
) -> Result<(), ToolsError> {
    harness.runtime.answer_question(AnswerQuestion {
        conversation_id: conversation.clone(),
        part_id: part.clone(),
        reply,
    })
}

fn answered(answers: &[(&[u32], Option<&str>)]) -> QuestionReply {
    QuestionReply::Answered {
        answers: answers
            .iter()
            .map(|(selected, own_words)| QuestionAnswer {
                selected: selected.to_vec(),
                own_words: own_words.map(str::to_owned),
            })
            .collect(),
    }
}

/// The arguments of a call that asks which database to use.
fn which_database() -> String {
    json!({
        "activity": "Asking which database to use",
        "questions": [{
            "question": "Which database should the service use?",
            "multiple": false,
            "options": [
                { "label": "PostgreSQL", "description": "It has the JSON queries." },
                { "label": "SQLite", "description": "" },
            ],
        }],
    })
    .to_string()
}

fn the_call(transcript: &Transcript, index: usize) -> ToolCallView {
    transcript.tool_calls(0)[index].clone()
}

/// The tool results of a request, by call ID.
fn results(request: &ModelRequest) -> Vec<(String, String)> {
    request
        .messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::ToolResult {
                call_id, output, ..
            } => Some((call_id.clone(), output.clone())),
            _ => None,
        })
        .collect()
}

fn result_of(request: &ModelRequest, call_id: &str) -> String {
    let (_id, output) = results(request)
        .into_iter()
        .find(|(id, _output)| id == call_id)
        .expect("the request holds the call's result");
    output
}

fn offers(request: &ModelRequest, tool: &str) -> bool {
    request.tools.iter().any(|offered| offered.name == tool)
}

#[tokio::test]
async fn an_answer_reaches_the_model_and_the_reply_continues() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[("call_1", ASK_OWNER, &which_database())]));
    model.push_reply(reply("PostgreSQL it is."));

    let (conversation, run) = setup.start("Set up the service.").await;
    let (part, questions) = setup.waiting(&conversation).await;

    assert_eq!(
        questions,
        [Question {
            text: "Which database should the service use?".to_owned(),
            multiple: false,
            options: vec![
                QuestionOption {
                    label: "PostgreSQL".to_owned(),
                    description: Some("It has the JSON queries.".to_owned()),
                },
                QuestionOption {
                    label: "SQLite".to_owned(),
                    description: None,
                },
            ],
        }]
    );
    let summary = setup.summary(&conversation).await;
    assert!(summary.awaiting_answer);
    assert!(!summary.awaiting_approval);
    let snapshot = setup.harness.snapshot(&conversation).await;
    assert_eq!(
        snapshot.run(&run).and_then(|run| run.activity.clone()),
        Some(RunActivity::AwaitingAnswer {
            part_id: part.clone(),
        })
    );
    let first = &setup.harness.model.reply_requests()[0];
    assert!(offers(first, ASK_OWNER));
    assert!(first.instructions.contains(texts::QUESTION_GUIDANCE));

    reply_of(
        &setup.harness,
        &conversation,
        &part,
        answered(&[(&[0], None)]),
    )
    .unwrap();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let call = the_call(&transcript, 0);
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(
        call.activity.as_deref(),
        Some("Asking which database to use")
    );
    assert_eq!(call.decision, None);
    assert_eq!(call.accesses, Vec::new());
    assert_eq!(call.questions, questions);
    assert_eq!(
        call.result.map(|result| result.outcome),
        Some(ToolOutcome::QuestionAnswered {
            answers: vec![QuestionAnswer {
                selected: vec![0],
                own_words: None,
            }],
        })
    );
    let requests = setup.harness.model.reply_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        result_of(&requests[1], "call_1"),
        "The owner answered.\n\n\
         Question: Which database should the service use?\nAnswer: PostgreSQL"
    );
    assert_eq!(transcript.reply_text(0), "PostgreSQL it is.");
    assert!(!setup.summary(&conversation).await.awaiting_answer);
    assert_eq!(
        reply_of(
            &setup.harness,
            &conversation,
            &part,
            answered(&[(&[0], None)])
        ),
        Err(ToolsError::QuestionNotPending),
        "an answered question no longer waits"
    );
}

#[tokio::test]
async fn the_owner_can_choose_several_options_and_add_their_own_words() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    let arguments = json!({
        "questions": [
            {
                "question": "Which extras?",
                "multiple": true,
                "options": [
                    { "label": "Caching", "description": "" },
                    { "label": "Metrics", "description": "" },
                ],
            },
            {
                "question": "Where does it run?",
                "multiple": false,
                "options": [
                    { "label": "Here", "description": "" },
                    { "label": "There", "description": "" },
                ],
            },
        ],
    })
    .to_string();
    model.push_reply(tool_calls(&[("call_1", ASK_OWNER, &arguments)]));
    model.push_reply(reply("Noted."));

    let (conversation, run) = setup.start("Set it up.").await;
    let (part, _questions) = setup.waiting(&conversation).await;
    reply_of(
        &setup.harness,
        &conversation,
        &part,
        answered(&[
            (&[1, 0], Some("  and tracing\n")),
            (&[], Some("On the build machine")),
        ]),
    )
    .unwrap();
    setup.harness.wait_for_run(&conversation, &run).await;

    let requests = setup.harness.model.reply_requests();
    assert_eq!(
        result_of(&requests[1], "call_1"),
        "The owner answered.\n\n\
         Question: Which extras?\nAnswer: Caching; Metrics\nIn the owner's own words: and tracing\n\n\
         Question: Where does it run?\nIn the owner's own words: On the build machine"
    );
}

#[tokio::test]
async fn declining_tells_the_model_and_the_reply_goes_on() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[("call_1", ASK_OWNER, &which_database())]));
    model.push_reply(reply("I will use SQLite."));

    let (conversation, run) = setup.start("Set up the service.").await;
    let (part, _questions) = setup.waiting(&conversation).await;
    reply_of(
        &setup.harness,
        &conversation,
        &part,
        QuestionReply::Declined,
    )
    .unwrap();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    assert_eq!(
        the_call(&transcript, 0).result.map(|result| result.outcome),
        Some(ToolOutcome::QuestionDeclined)
    );
    assert_eq!(
        result_of(&setup.harness.model.reply_requests()[1], "call_1"),
        texts::QUESTION_DECLINED
    );
}

#[tokio::test]
async fn a_refused_reply_leaves_the_question_waiting() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[("call_1", ASK_OWNER, &which_database())]));
    model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("Set up the service.").await;
    let (part, _questions) = setup.waiting(&conversation).await;
    let other_part = PartId::parse("0199e0f2-0000-7000-8000-000000000999").unwrap();
    let other_conversation = ConversationId::parse("0199e0f2-0000-7000-8000-000000000998").unwrap();
    let good = answered(&[(&[1], None)]);

    let harness = &setup.harness;
    for refused in [
        answered(&[(&[2], None)]),
        answered(&[(&[0, 1], None)]),
        answered(&[(&[], None)]),
        answered(&[(&[], Some("   "))]),
        answered(&[(&[0], Some("and this"))]),
        answered(&[(&[0], None), (&[1], None)]),
        answered(&[]),
    ] {
        assert_eq!(
            reply_of(harness, &conversation, &part, refused.clone()),
            Err(ToolsError::AnswerNotValid),
            "{refused:?}"
        );
    }
    assert_eq!(
        reply_of(harness, &conversation, &other_part, good.clone()),
        Err(ToolsError::QuestionNotPending)
    );
    assert_eq!(
        reply_of(harness, &other_conversation, &part, good.clone()),
        Err(ToolsError::QuestionNotPending)
    );
    assert_eq!(
        the_call(&harness.snapshot(&conversation).await, 0).status,
        ToolCallStatus::AwaitingAnswer
    );

    reply_of(harness, &conversation, &part, good).unwrap();
    let transcript = harness.wait_for_run(&conversation, &run).await;
    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
}

#[tokio::test]
async fn stop_cancels_the_question_and_the_model_reads_that_the_owner_did_not_answer() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[("call_1", ASK_OWNER, &which_database())]));
    model.push_reply(reply("Understood."));

    let (conversation, run) = setup.start("Set up the service.").await;
    let (part, questions) = setup.waiting(&conversation).await;
    setup.harness.runtime.stop_reply(&conversation).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Stopped));
    let call = the_call(&transcript, 0);
    assert_eq!(call.status, ToolCallStatus::Canceled);
    assert_eq!(
        call.questions, questions,
        "the record of what the owner saw"
    );
    assert_eq!(
        reply_of(
            &setup.harness,
            &conversation,
            &part,
            answered(&[(&[0], None)])
        ),
        Err(ToolsError::QuestionNotPending)
    );
    assert!(!setup.summary(&conversation).await.awaiting_answer);

    let next = setup
        .harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "Use SQLite."))
        .await
        .unwrap();
    setup
        .harness
        .wait_for_run(&conversation, &next.run_id)
        .await;
    let requests = setup.harness.model.reply_requests();
    assert_eq!(
        result_of(requests.last().unwrap(), "call_1"),
        texts::QUESTION_CANCELED
    );
}

#[tokio::test]
async fn a_restart_cancels_the_question_and_a_retry_asks_again() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[("call_1", ASK_OWNER, &which_database())]));

    let (conversation, run) = setup.start("Set up the service.").await;
    let (part, _questions) = setup.waiting(&conversation).await;
    let harness = setup.harness.restart().await;

    let recovered = harness.snapshot(&conversation).await;
    assert_eq!(recovered.run_state(&run), Some(RunState::Interrupted));
    assert_eq!(the_call(&recovered, 0).status, ToolCallStatus::Canceled);
    assert_eq!(
        reply_of(&harness, &conversation, &part, answered(&[(&[0], None)])),
        Err(ToolsError::QuestionNotPending)
    );

    harness
        .model
        .push_reply(tool_calls(&[("call_2", ASK_OWNER, &which_database())]));
    harness.model.push_reply(reply("PostgreSQL it is."));
    let retried = harness.runtime.retry_reply(&conversation).await.unwrap();
    let (again, _questions) = wait_for_question(&harness, &conversation).await;
    assert_ne!(again, part);
    let asked_again = harness.model.reply_requests();
    assert_eq!(
        result_of(asked_again.last().unwrap(), "call_1"),
        texts::QUESTION_CANCELED,
        "the retry keeps the call and says that the owner did not answer"
    );
    reply_of(&harness, &conversation, &again, answered(&[(&[0], None)])).unwrap();
    let transcript = harness.wait_for_run(&conversation, &retried.run_id).await;

    assert_eq!(
        transcript.run_state(&retried.run_id),
        Some(RunState::Completed)
    );
}

#[tokio::test]
async fn calls_after_a_question_do_not_run_and_calls_before_it_do() {
    let setup = Setup::new().await;
    fs::write(setup.path("notes.txt"), "hello\n").unwrap();
    let model = &setup.harness.model;
    let write = json!({ "path": "new.txt", "content": "made\n" }).to_string();
    let read = json!({ "path": "notes.txt" }).to_string();
    model.push_reply(tool_calls(&[
        ("call_1", "read_file", &read),
        ("call_2", ASK_OWNER, &which_database()),
        ("call_3", "write_file", &write),
        ("call_4", ASK_OWNER, &which_database()),
    ]));
    model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("Set up the service.").await;
    let (part, _questions) = setup.waiting(&conversation).await;
    let waiting = setup.harness.snapshot(&conversation).await;
    assert_eq!(the_call(&waiting, 0).status, ToolCallStatus::Completed);
    reply_of(
        &setup.harness,
        &conversation,
        &part,
        answered(&[(&[0], None)]),
    )
    .unwrap();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    for index in [2, 3] {
        let call = the_call(&transcript, index);
        assert_eq!(call.status, ToolCallStatus::NotRun);
        assert_eq!(call.not_run, Some(NotRunReason::AfterQuestion));
    }
    assert!(!setup.path("new.txt").exists());
    let requests = setup.harness.model.reply_requests();
    assert_eq!(result_of(&requests[1], "call_3"), texts::AFTER_QUESTION);
    assert_eq!(result_of(&requests[1], "call_4"), texts::AFTER_QUESTION);
}

#[tokio::test]
async fn nothing_runs_after_a_question_with_bad_arguments() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    let write = json!({ "path": "new.txt", "content": "made\n" }).to_string();
    model.push_reply(tool_calls(&[
        ("call_1", ASK_OWNER, r#"{"questions":[]}"#),
        ("call_2", "write_file", &write),
    ]));
    model.push_reply(reply("Let me ask again another time."));

    let (conversation, run) = setup.start("Set up the service.").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let question = the_call(&transcript, 0);
    assert_eq!(question.status, ToolCallStatus::NotRun);
    assert!(matches!(
        question.not_run,
        Some(NotRunReason::InvalidArguments { .. })
    ));
    assert_eq!(
        the_call(&transcript, 1).not_run,
        Some(NotRunReason::AfterQuestion)
    );
    assert!(!setup.path("new.txt").exists());
    let requests = setup.harness.model.reply_requests();
    let refusal = result_of(&requests[1], "call_1");
    assert!(refusal.contains("1 to 4 questions"), "{refusal}");
}

#[tokio::test]
async fn a_question_waits_in_full_access_too() {
    let setup = Setup::new().await;
    setup
        .harness
        .runtime
        .update_tools_settings(UpdateToolsSettings {
            allow_changes_in_workspace: false,
            command_environment: Vec::new(),
            full_access_for_new_conversations: true,
            wake_on_command_end: true,
        })
        .await
        .unwrap();
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[("call_1", ASK_OWNER, &which_database())]));
    model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("Set up the service.").await;
    let (part, _questions) = setup.waiting(&conversation).await;

    assert!(setup.summary(&conversation).await.full_access);
    let first = &setup.harness.model.reply_requests()[0];
    assert!(first.instructions.contains(texts::QUESTION_GUIDANCE));
    reply_of(
        &setup.harness,
        &conversation,
        &part,
        QuestionReply::Declined,
    )
    .unwrap();
    setup.harness.wait_for_run(&conversation, &run).await;
}

#[tokio::test]
async fn an_answer_allows_nothing() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    let may_i = json!({
        "questions": [{
            "question": "May I write new.txt?",
            "multiple": false,
            "options": [
                { "label": "Allow once", "description": "" },
                { "label": "Allow always", "description": "" },
            ],
        }],
    })
    .to_string();
    let write = json!({ "path": "new.txt", "content": "made\n" }).to_string();
    model.push_reply(tool_calls(&[("call_1", ASK_OWNER, &may_i)]));
    model.push_reply(tool_calls(&[("call_2", "write_file", &write)]));

    let (conversation, _run) = setup.start("Write new.txt").await;
    let (part, _questions) = setup.waiting(&conversation).await;
    reply_of(
        &setup.harness,
        &conversation,
        &part,
        answered(&[(&[1], None)]),
    )
    .unwrap();

    let transcript = setup
        .harness
        .wait_until(&conversation, |transcript| {
            transcript
                .tool_calls(0)
                .iter()
                .any(|call| call.status == ToolCallStatus::AwaitingApproval)
        })
        .await;
    assert!(!setup.path("new.txt").exists());
    let reasons = the_call(&transcript, 1).request.unwrap().reasons;
    assert_eq!(reasons, [ApprovalReason::ChangeInWorkspace]);
    setup.harness.runtime.stop_reply(&conversation).await;
}

#[tokio::test]
async fn an_answer_does_not_count_as_the_owner_naming_a_project() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    let move_there = json!({ "project": setup.root.to_str().unwrap() }).to_string();
    model.push_reply(tool_calls(&[("call_1", ASK_OWNER, &which_database())]));
    model.push_reply(tool_calls(&[("call_2", "move_to_project", &move_there)]));

    let started = setup
        .harness
        .runtime
        .start_conversation(StartConversation::new("What does a.txt in Mannie say?"))
        .await
        .unwrap();
    let conversation = started.conversation_id;
    let (part, _questions) = setup.waiting(&conversation).await;
    reply_of(
        &setup.harness,
        &conversation,
        &part,
        answered(&[(&[0], None)]),
    )
    .unwrap();

    let transcript = setup
        .harness
        .wait_until(&conversation, |transcript| {
            transcript
                .tool_calls(0)
                .iter()
                .any(|call| call.status == ToolCallStatus::AwaitingApproval)
        })
        .await;
    assert_eq!(
        the_call(&transcript, 1).status,
        ToolCallStatus::AwaitingApproval,
        "the same move without a question runs on the owner's words"
    );
    setup.harness.runtime.stop_reply(&conversation).await;
}

#[tokio::test]
async fn a_run_that_waits_for_an_answer_holds_no_model_slot() {
    let setup = Setup::with_options(HarnessOptions {
        use_tools: true,
        config: RuntimeConfig {
            max_concurrent_calls: 1,
            ..fast_config()
        },
        ..HarnessOptions::default()
    })
    .await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[("call_1", ASK_OWNER, &which_database())]));
    model.push_reply(reply("Meanwhile."));

    let (asking, _run) = setup.start("Set up the service.").await;
    setup.waiting(&asking).await;
    let (other, other_run) = setup.start("Say something.").await;
    let transcript = setup.harness.wait_for_run(&other, &other_run).await;

    assert_eq!(transcript.reply_text(0), "Meanwhile.");
    setup.harness.runtime.stop_reply(&asking).await;
}

#[tokio::test]
async fn a_child_agent_is_not_offered_the_tool_and_its_call_does_not_run() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    let task = json!({
        "title": "Look around",
        "agent": "general",
        "instruction": "CHILD-Q: look around.",
        "background": false,
    })
    .to_string();
    model.push_reply(tool_calls(&[("call_1", "start_task", &task)]));
    model.push_reply(reply("Done."));
    model.push_reply_for(
        "CHILD-Q",
        tool_calls(&[("child_1", ASK_OWNER, &which_database())]),
    );
    model.push_reply_for("CHILD-Q", reply("Looked."));
    model.push_reply_for("CHILD-Q", reply("Report: looked."));

    let (conversation, run) = setup.start("OWNER: look around").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let child_requests = setup.harness.model.requests_for("CHILD-Q");
    assert!(!offers(&child_requests[0], ASK_OWNER));
    assert!(
        !child_requests[0]
            .instructions
            .contains(texts::QUESTION_GUIDANCE)
    );
    assert_eq!(
        result_of(&child_requests[1], "child_1"),
        texts::unknown_tool(ASK_OWNER)
    );
    let child = &transcript.tasks(0)[0].conversation_id;
    let calls = setup.harness.snapshot(child).await.tool_calls(0);
    assert_eq!(calls[0].not_run, Some(NotRunReason::UnknownTool));
}

#[tokio::test]
async fn a_task_that_a_worker_runs_is_not_offered_the_tool() {
    let worker = WorkerHarness::start().await;
    worker
        .model
        .push_reply(tool_calls(&[("call_1", ASK_OWNER, &which_database())]));
    worker.model.push_reply(reply("Counted."));
    worker.model.push_reply(reply("Report: counted."));
    let assignment = worker.assignment("Count the files.");
    let task = assignment.task_id.clone();

    worker.assign(assignment.clone()).await;
    worker.wait_for_end(&task).await;

    let requests = worker.model.reply_requests();
    assert!(!offers(&requests[0], ASK_OWNER));
    assert_eq!(
        result_of(&requests[1], "call_1"),
        texts::unknown_tool(ASK_OWNER)
    );
    let calls = worker
        .snapshot(&assignment.conversation_id)
        .await
        .tool_calls(0);
    assert_eq!(calls[0].not_run, Some(NotRunReason::UnknownTool));
}

#[tokio::test]
async fn a_profile_without_tools_is_not_offered_the_tool() {
    let harness = Harness::start().await;
    harness.model.push_reply(reply("Hello."));

    let started = harness
        .runtime
        .start_conversation(StartConversation::new("Hi"))
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let request = &harness.model.reply_requests()[0];
    assert!(!offers(request, ASK_OWNER));
    assert!(!request.instructions.contains(texts::QUESTION_GUIDANCE));
}

#[tokio::test]
async fn an_executors_tool_cannot_take_the_name_of_the_tool() {
    let setup = Setup::new().await;
    let executor = FakeExecutor::new();
    executor.name_tool(ASK_OWNER);
    let contributed: Arc<dyn TaskExecutor> = Arc::<FakeExecutor>::clone(&executor);
    add_plugin(
        &setup.harness.runtime,
        FAKE_EXECUTOR_KIND,
        Contributions::new().add::<Executors>(contributed),
    )
    .unwrap();
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[("call_1", ASK_OWNER, &which_database())]));
    model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("Set up the service.").await;
    let (part, _questions) = setup.waiting(&conversation).await;

    let first = &setup.harness.model.reply_requests()[0];
    let offered = first
        .tools
        .iter()
        .filter(|tool| tool.name == ASK_OWNER)
        .count();
    assert_eq!(offered, 1);
    assert_eq!(executor.submissions(), Vec::new());
    reply_of(
        &setup.harness,
        &conversation,
        &part,
        QuestionReply::Declined,
    )
    .unwrap();
    setup.harness.wait_for_run(&conversation, &run).await;
}

#[tokio::test]
async fn compaction_reads_a_long_answer_whole() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    let own_words = format!("KEEP-THIS {} END-OF-ANSWER", "word ".repeat(400));
    model.push_reply(tool_calls(&[("call_1", ASK_OWNER, &which_database())]));
    model.push_reply(reply("Noted."));
    model.push_reply(reply("Second."));

    let (conversation, run) = setup.start("Set up the service.").await;
    let (part, _questions) = setup.waiting(&conversation).await;
    reply_of(
        &setup.harness,
        &conversation,
        &part,
        answered(&[(&[], Some(own_words.as_str()))]),
    )
    .unwrap();
    setup.harness.wait_for_run(&conversation, &run).await;
    let next = setup
        .harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "And now?"))
        .await
        .unwrap();
    setup
        .harness
        .wait_for_run(&conversation, &next.run_id)
        .await;
    setup
        .harness
        .runtime
        .compact_conversation(&conversation)
        .await
        .unwrap();

    let summary_request = &setup.harness.model.summary_requests()[0];
    let summarized = match &summary_request.messages[0].content[0] {
        Content::Text(text) => text.clone(),
        _ => String::new(),
    };
    assert!(
        summarized.contains("Assistant asked the owner: Which database should the service use?"),
        "{summarized}"
    );
    assert!(
        summarized.contains(&format!("Owner: {}", own_words.trim())),
        "{summarized}"
    );
}

/// `ask_owner` as a model receives it, compared with a reviewed copy in
/// `tests/fixtures/question_definition.json`: a change to a description or to
/// the schema changes how models ask, so it shows in review.
/// `MANNIE_UPDATE_EXPECTED=1` writes the file instead of comparing; review the
/// result before committing it.
#[tokio::test]
async fn the_tool_definition_matches_its_reviewed_copy() {
    let setup = Setup::new().await;
    setup.harness.model.push_reply(reply("Hello."));
    let (conversation, run) = setup.start("Hello").await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let request = &setup.harness.model.reply_requests()[0];
    let tool = request
        .tools
        .iter()
        .find(|tool| tool.name == ASK_OWNER)
        .expect("the conversation agent is offered the tool");
    let actual = json!({
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.parameters,
        "strict": tool.strict,
    });
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("question_definition.json");
    if std::env::var_os("MANNIE_UPDATE_EXPECTED").is_some() {
        let text = serde_json::to_string_pretty(&actual).unwrap();
        fs::write(&fixture, format!("{text}\n")).unwrap();
        return;
    }
    let expected: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&fixture).unwrap()).unwrap();
    assert_eq!(
        actual,
        expected,
        "the tool's definition changed; review it and update {}",
        fixture.display()
    );
}
