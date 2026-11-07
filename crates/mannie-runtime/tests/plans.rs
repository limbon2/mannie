//! Plans (docs/features/plans.md; ADR-0060): `save_plan` saves a conversation's
//! plan in one transaction with its call, an owner's edit is never overwritten
//! unread, and a plan reaches a later reply only as a mention, which never
//! stands in for the owner's words.

#![cfg(test)]

use std::fs;
use std::path::Path;

use mannie_provider::testing::Step;
use mannie_provider::{Content, ModelRequest, ProviderEvent, Role};
use mannie_runtime::plans::SAVE_PLAN;
use mannie_runtime::testing::{Harness, HarnessOptions, mentions_with_plans, reply, tool_calls};
use mannie_runtime::texts;
use mannie_types::agents::{AgentDraft, AgentKind, AgentView};
use mannie_types::chat::{SendMessage, StartConversation};
use mannie_types::ids::{ConversationId, RunId};
use mannie_types::mentions::{MentionPlace, MentionRef, SuggestMentions};
use mannie_types::plans::{MAX_PLAN_BODY_BYTES, PlanError, PlanField, UpdatePlan};
use mannie_types::run::RunState;
use mannie_types::tools::{
    ApprovalChoice, DecideApproval, NotRunReason, ToolCallStatus, ToolOutcome,
};
use mannie_types::transcript::PartContent;
use serde_json::json;

struct Setup {
    harness: Harness,
}

fn save_plan_tools() -> Vec<String> {
    ["read_file", "ask_owner", SAVE_PLAN]
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect()
}

fn planner() -> AgentDraft {
    AgentDraft {
        name: "Planner".to_owned(),
        description: String::new(),
        kind: AgentKind::Conversation,
        read_only: true,
        profile_id: None,
        instructions: "Plan things.".to_owned(),
        steps: Vec::new(),
        tools: save_plan_tools(),
        plugins: None,
        children: None,
        max_tasks: 0,
        max_tool_rounds: 100,
        max_tool_calls: 250,
    }
}

impl Setup {
    async fn new() -> Self {
        let harness = Harness::with_options(HarnessOptions {
            use_tools: true,
            mentions: mentions_with_plans,
            ..HarnessOptions::default()
        })
        .await;
        Self { harness }
    }

    async fn planner(&self) -> AgentView {
        self.harness.runtime.create_agent(&planner()).await.unwrap()
    }

    async fn start(&self, text: &str, agent: Option<&str>) -> (ConversationId, RunId) {
        let started = self
            .harness
            .runtime
            .start_conversation(StartConversation::new(text).with_agent(agent.map(str::to_owned)))
            .await
            .unwrap();
        (started.conversation_id, started.run_id)
    }

    async fn send(&self, conversation: &ConversationId, text: &str) -> RunId {
        self.harness
            .runtime
            .send_message(SendMessage::new(conversation.clone(), text.to_owned()))
            .await
            .unwrap()
            .run_id
    }

    /// Runs a reply of the planner that saves this plan.
    async fn saved(&self, title: &str, body: &str) -> ConversationId {
        let agent = self.planner().await;
        let model = &self.harness.model;
        model.push_reply(tool_calls(&[("call_1", SAVE_PLAN, &save(title, body))]));
        model.push_reply(reply("I saved the plan."));
        let (conversation, run) = self.start("Make a plan", Some(&agent.key)).await;
        let transcript = self.harness.wait_for_run(&conversation, &run).await;
        assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
        conversation
    }
}

fn save(title: &str, body: &str) -> String {
    json!({ "activity": "Saving the plan", "title": title, "body": body }).to_string()
}

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
                ..
            } if id == call_id => Some(output.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no result for {call_id}"))
}

fn user_text(request: &ModelRequest) -> String {
    request
        .messages
        .iter()
        .filter(|message| message.role == Role::User)
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn tool_names(request: &ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

#[tokio::test]
async fn only_an_agent_that_names_the_tool_is_offered_it() {
    let setup = Setup::new().await;
    let agent = setup.planner().await;
    let model = &setup.harness.model;
    model.push_reply(reply("Default."));
    model.push_reply(reply("Planner."));

    let (conversation, run) = setup.start("Hi", None).await;
    setup.harness.wait_for_run(&conversation, &run).await;
    let (conversation, run) = setup.start("Hi", Some(&agent.key)).await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let requests = setup.harness.model.reply_requests();
    assert!(!tool_names(&requests[0]).contains(&SAVE_PLAN.to_owned()));
    assert!(!requests[0].instructions.contains(texts::PLAN_GUIDANCE));
    assert!(tool_names(&requests[1]).contains(&SAVE_PLAN.to_owned()));
    assert!(requests[1].instructions.contains(texts::PLAN_GUIDANCE));
}

#[tokio::test]
async fn a_call_from_an_agent_without_the_tool_does_not_run() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        SAVE_PLAN,
        &save("Mine", "Do it."),
    )]));
    model.push_reply(reply("I could not."));

    let (conversation, run) = setup.start("Make a plan", None).await;
    setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(
        setup.harness.runtime.list_plans().await.unwrap(),
        Vec::new()
    );
}

#[tokio::test]
async fn saving_stores_the_plan_and_records_the_call_and_the_model_reads_that_it_saved() {
    let setup = Setup::new().await;

    let conversation = setup.saved("Add dark mode", "# Steps\n\n1. Do it.\n").await;

    let plans = setup.harness.runtime.list_plans().await.unwrap();
    let [plan] = plans.as_slice() else {
        panic!("one plan: {plans:?}");
    };
    assert_eq!(plan.title, "Add dark mode");
    assert_eq!(plan.conversation_id.as_ref(), Some(&conversation));
    let whole = setup.harness.runtime.get_plan(plan.number).await.unwrap();
    assert_eq!(whole.body, "# Steps\n\n1. Do it.\n");
    let requests = setup.harness.model.reply_requests();
    assert_eq!(result(&requests[1], "call_1"), texts::PLAN_SAVED);
    let transcript = setup.harness.snapshot(&conversation).await;
    let call = transcript
        .parts(0)
        .into_iter()
        .find_map(|part| match part.content {
            PartContent::ToolCall { call } => Some(call),
            _ => None,
        })
        .unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(
        call.result.unwrap().outcome,
        ToolOutcome::PlanSaved {
            title: "Add dark mode".to_owned()
        }
    );
}

#[tokio::test]
async fn saving_again_replaces_the_plan_of_the_conversation() {
    let setup = Setup::new().await;
    let conversation = setup.saved("First", "One.").await;
    let number = setup.harness.runtime.list_plans().await.unwrap()[0].number;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_2",
        SAVE_PLAN,
        &save("Second", "Two."),
    )]));
    model.push_reply(reply("Saved again."));

    let second = setup.send(&conversation, "Change it").await;
    setup.harness.wait_for_run(&conversation, &second).await;

    let plans = setup.harness.runtime.list_plans().await.unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].number, number);
    assert_eq!(plans[0].title, "Second");
    let whole = setup.harness.runtime.get_plan(number).await.unwrap();
    assert_eq!(whole.body, "Two.");
}

#[tokio::test]
async fn a_plan_over_its_limit_is_refused_and_nothing_changes() {
    let setup = Setup::new().await;
    let agent = setup.planner().await;
    let model = &setup.harness.model;
    let long = "x".repeat(MAX_PLAN_BODY_BYTES + 1);
    model.push_reply(tool_calls(&[("call_1", SAVE_PLAN, &save("Long", &long))]));
    model.push_reply(reply("It was too long."));

    let (conversation, run) = setup.start("Make a plan", Some(&agent.key)).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(
        setup.harness.runtime.list_plans().await.unwrap(),
        Vec::new()
    );
    let call = transcript
        .parts(0)
        .into_iter()
        .find_map(|part| match part.content {
            PartContent::ToolCall { call } => Some(call),
            _ => None,
        })
        .unwrap();
    assert!(
        matches!(call.not_run, Some(NotRunReason::InvalidArguments { .. })),
        "{:?}",
        call.not_run
    );
}

#[tokio::test]
async fn a_save_after_the_owners_edit_is_refused_once_and_the_model_reads_the_edit() {
    let setup = Setup::new().await;
    let conversation = setup.saved("Mine", "The model's text.").await;
    let number = setup.harness.runtime.list_plans().await.unwrap()[0].number;
    setup
        .harness
        .runtime
        .update_plan(UpdatePlan {
            number,
            title: "Edited".to_owned(),
            body: "The owner's text.".to_owned(),
        })
        .await
        .unwrap();
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_2",
        SAVE_PLAN,
        &save("Mine", "Stale."),
    )]));
    model.push_reply(tool_calls(&[(
        "call_3",
        SAVE_PLAN,
        &save("Edited", "The owner's text, and more."),
    )]));
    model.push_reply(reply("Saved on top of your edit."));

    let second = setup.send(&conversation, "Add a step").await;
    setup.harness.wait_for_run(&conversation, &second).await;

    let requests = setup.harness.model.reply_requests();
    let first_result = result(requests.last().unwrap(), "call_2");
    assert_eq!(
        first_result,
        texts::plan_kept("Edited", "The owner's text.")
    );
    let whole = setup.harness.runtime.get_plan(number).await.unwrap();
    assert_eq!(whole.summary.title, "Edited");
    assert_eq!(whole.body, "The owner's text, and more.");
}

#[tokio::test]
async fn a_plan_outlives_its_conversation_and_its_project() {
    let setup = Setup::new().await;
    let folder = tempfile::tempdir().unwrap();
    let project = setup
        .harness
        .runtime
        .add_project(folder.path().to_str().unwrap().to_owned())
        .await
        .unwrap();
    let agent = setup.planner().await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        SAVE_PLAN,
        &save("Mine", "Do it."),
    )]));
    model.push_reply(reply("Saved."));
    let started = setup
        .harness
        .runtime
        .start_conversation(
            StartConversation::new("Plan")
                .with_project(Some(project.id.clone()))
                .with_agent(Some(agent.key.clone())),
        )
        .await
        .unwrap();
    setup
        .harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let plan = setup.harness.runtime.list_plans().await.unwrap().remove(0);
    assert_eq!(plan.project_id, Some(project.id.clone()));

    setup
        .harness
        .runtime
        .delete_conversation(&started.conversation_id)
        .await
        .unwrap();
    let plan = setup.harness.runtime.get_plan(plan.number).await.unwrap();
    assert_eq!(plan.summary.conversation_id, None);
    assert_eq!(plan.summary.project_id, Some(project.id.clone()));

    setup
        .harness
        .runtime
        .remove_project(mannie_types::projects::RemoveProject {
            project_id: project.id.clone(),
            conversations: 0,
        })
        .await
        .unwrap();
    let plan = setup
        .harness
        .runtime
        .get_plan(plan.summary.number)
        .await
        .unwrap();
    assert_eq!(plan.summary.project_id, None);
}

#[tokio::test]
async fn the_commands_refuse_what_is_wrong() {
    let setup = Setup::new().await;
    let runtime = &setup.harness.runtime;

    assert_eq!(runtime.get_plan(99).await, Err(PlanError::NotFound));
    assert_eq!(runtime.delete_plan(99).await, Err(PlanError::NotFound));
    assert_eq!(
        runtime
            .update_plan(UpdatePlan {
                number: 99,
                title: "Title".to_owned(),
                body: "Body".to_owned(),
            })
            .await,
        Err(PlanError::NotFound)
    );
    assert_eq!(
        runtime
            .update_plan(UpdatePlan {
                number: 99,
                title: " ".to_owned(),
                body: "Body".to_owned(),
            })
            .await,
        Err(PlanError::FieldNotValid {
            field: PlanField::Title
        })
    );

    setup.saved("Mine", "Do it.").await;
    let number = setup.harness.runtime.list_plans().await.unwrap()[0].number;
    setup.harness.runtime.delete_plan(number).await.unwrap();
    assert_eq!(
        setup.harness.runtime.list_plans().await.unwrap(),
        Vec::new()
    );
}

fn mention(number: u32) -> MentionRef {
    MentionRef {
        provider: "plans".to_owned(),
        key: number.to_string(),
    }
}

#[tokio::test]
async fn the_list_offers_plans_whose_token_holds_only_the_number() {
    let setup = Setup::new().await;
    setup.saved("Add dark mode", "Do it.").await;
    let number = setup.harness.runtime.list_plans().await.unwrap()[0].number;

    let suggestions = setup
        .harness
        .runtime
        .suggest_mentions(SuggestMentions::new(
            MentionPlace::NewConversation { project_id: None },
            "dark",
        ))
        .await
        .unwrap();

    let group = suggestions
        .groups
        .iter()
        .find(|group| group.provider == "plans")
        .expect("the plans' group");
    let [entry] = group.entries.as_slice() else {
        panic!("one entry: {:?}", group.entries);
    };
    assert_eq!(entry.label, "Add dark mode");
    assert_eq!(entry.key, number.to_string());
    assert_eq!(entry.token, format!("@plan-{number}"));
}

#[tokio::test]
async fn a_mentioned_plan_reaches_the_model_under_its_heading_and_a_retry_sends_the_stored_text() {
    let setup = Setup::new().await;
    setup.saved("Add dark mode", "THE PLAN'S TEXT.").await;
    let number = setup.harness.runtime.list_plans().await.unwrap()[0].number;
    let model = &setup.harness.model;
    model.push_reply(vec![
        Step::Event(ProviderEvent::TextDelta("Working".to_owned())),
        Step::Hang,
    ]);
    model.push_reply(reply("Carried out."));

    let started = setup
        .harness
        .runtime
        .start_conversation(
            StartConversation::new(format!("Carry out this plan. @plan-{number}"))
                .with_mentions(vec![mention(number)]),
        )
        .await
        .unwrap();
    let conversation = started.conversation_id;
    // The model took this reply, so a stop does not leave it for the retry.
    setup
        .harness
        .wait_until(&conversation, |transcript| {
            transcript.reply_text(0) == "Working"
        })
        .await;
    setup.harness.runtime.stop_reply(&conversation).await;
    setup
        .harness
        .wait_for_run(&conversation, &started.run_id)
        .await;
    // The plan changes after the message was sent, and a retry reads what the
    // message held.
    setup
        .harness
        .runtime
        .update_plan(UpdatePlan {
            number,
            title: "Add dark mode".to_owned(),
            body: "AN EDIT AFTERWARDS.".to_owned(),
        })
        .await
        .unwrap();
    let retried = setup
        .harness
        .runtime
        .retry_reply(&conversation)
        .await
        .unwrap();
    setup
        .harness
        .wait_for_run(&conversation, &retried.run_id)
        .await;

    let request = setup.harness.model.reply_requests().pop().unwrap();
    let said = user_text(&request);
    assert!(
        said.contains(&texts::plan_mention_heading(number, "Add dark mode")),
        "{said}"
    );
    assert!(said.contains("THE PLAN'S TEXT."), "{said}");
    assert!(!said.contains("AN EDIT AFTERWARDS."), "{said}");
}

#[tokio::test]
async fn a_mention_of_a_plan_that_does_not_exist_is_refused() {
    let setup = Setup::new().await;

    let refused = setup
        .harness
        .runtime
        .start_conversation(
            StartConversation::new("Carry out this plan. @plan-99")
                .with_mentions(vec![mention(99)]),
        )
        .await;

    assert_eq!(
        refused.err(),
        Some(mannie_types::chat::ChatError::MentionRefused {
            provider: "plans".to_owned(),
            reason: "plan_not_found".to_owned(),
        })
    );
}

#[tokio::test]
async fn a_plans_title_that_names_a_project_does_not_allow_a_move_into_it() {
    let setup = Setup::new().await;
    let folder = tempfile::tempdir().unwrap();
    let project = setup
        .harness
        .runtime
        .add_project(folder.path().to_str().unwrap().to_owned())
        .await
        .unwrap();
    setup
        .saved(&project.name, &format!("Move into {}.", project.name))
        .await;
    let number = setup.harness.runtime.list_plans().await.unwrap()[0].number;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        "move_to_project",
        &json!({ "activity": "Moving", "project": project.name }).to_string(),
    )]));

    let started = setup
        .harness
        .runtime
        .start_conversation(
            StartConversation::new(format!("Carry out this plan. @plan-{number}"))
                .with_mentions(vec![mention(number)]),
        )
        .await
        .unwrap();
    let conversation = started.conversation_id;

    // The move asks for approval, because the plan's words are not the owner's.
    let transcript = setup
        .harness
        .wait_until(&conversation, |transcript| {
            transcript
                .parts(0)
                .into_iter()
                .any(|part| match part.content {
                    PartContent::ToolCall { call } => {
                        call.status == ToolCallStatus::AwaitingApproval
                    }
                    _ => false,
                })
        })
        .await;
    let part = transcript
        .parts(0)
        .into_iter()
        .find(|part| matches!(part.content, PartContent::ToolCall { .. }))
        .unwrap();
    assert!(
        setup
            .harness
            .store
            .conversation_summary(&conversation)
            .await
            .unwrap()
            .unwrap()
            .project_id
            .is_none()
    );
    setup
        .harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id: part.id,
            digest: match part.content {
                PartContent::ToolCall { call } => call.request.unwrap().digest,
                _ => unreachable!("the part is a tool call"),
            },
            choice: ApprovalChoice::Deny,
        })
        .await
        .unwrap();
    setup.harness.runtime.stop_reply(&conversation).await;
}

/// `save_plan` as a model receives it, compared with a reviewed copy in
/// `tests/fixtures/plan_definition.json`: a change to a description or to the
/// schema changes how models save plans, so it shows in review.
/// `MANNIE_UPDATE_EXPECTED=1` writes the file instead of comparing; review the
/// result before committing it.
#[tokio::test]
async fn the_tool_definition_matches_its_reviewed_copy() {
    let setup = Setup::new().await;
    let agent = setup.planner().await;
    setup.harness.model.push_reply(reply("Hello."));
    let (conversation, run) = setup.start("Hello", Some(&agent.key)).await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let request = &setup.harness.model.reply_requests()[0];
    let tool = request
        .tools
        .iter()
        .find(|tool| tool.name == SAVE_PLAN)
        .unwrap();
    let actual = json!({
        "description": tool.description,
        "name": tool.name,
        "parameters": tool.parameters,
        "strict": tool.strict,
    });
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("plan_definition.json");
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

#[tokio::test]
async fn the_newest_plan_stays_whole_through_compaction() {
    let setup = Setup::new().await;
    let long = format!("START {} END", "x".repeat(2_000));
    let conversation = setup.saved("Long", &long).await;
    for text in ["Second", "Third", "Fourth"] {
        setup.harness.model.push_reply(reply("Noted."));
        let run = setup.send(&conversation, text).await;
        setup.harness.wait_for_run(&conversation, &run).await;
    }
    setup
        .harness
        .model
        .push_summary(reply("Summary of the plan."));

    setup
        .harness
        .runtime
        .compact_conversation(&conversation)
        .await
        .unwrap();

    let summary = setup.harness.model.summary_requests().pop().unwrap();
    let read = user_text(&summary);
    assert!(read.contains("START"), "{read}");
    assert!(read.contains("END"), "{read}");
}

#[tokio::test]
async fn an_agent_that_only_reads_saves_a_plan_and_changes_nothing_else() {
    let setup = Setup::new().await;
    let agent = setup.planner().await;
    let folder = tempfile::tempdir().unwrap();
    let model = &setup.harness.model;
    let target = folder.path().join("changed.txt");
    model.push_reply(tool_calls(&[(
        "call_1",
        "write_file",
        &json!({
            "activity": "Writing",
            "path": target.to_str().unwrap(),
            "content": "changed\n",
        })
        .to_string(),
    )]));
    model.push_reply(tool_calls(&[(
        "call_2",
        SAVE_PLAN,
        &save("Mine", "Do it."),
    )]));
    model.push_reply(reply("Saved."));

    let (conversation, run) = setup.start("Plan", Some(&agent.key)).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    assert!(!target.exists());
    let requests = setup.harness.model.reply_requests();
    assert!(
        !tool_names(&requests[0]).contains(&"write_file".to_owned()),
        "{:?}",
        tool_names(&requests[0])
    );
    assert_eq!(setup.harness.runtime.list_plans().await.unwrap().len(), 1);
}
