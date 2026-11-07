//! Skills (docs/features/skills.md; ADR-0061): the skills of this computer
//! are on until the owner turns one off, a project's are off until the owner
//! turns each on and off again when their listed text changes, agents see the
//! skills that are on and read them without asking, and a conversation
//! without skills sends what it sent before.

#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use mannie_provider::ModelRequest;
use mannie_runtime::testing::{Harness, HarnessOptions, reply, tool_calls};
use mannie_runtime::{SKILLS, skill_token, texts};
use mannie_types::agents::{AgentDraft, AgentKind};
use mannie_types::chat::StartConversation;
use mannie_types::mentions::{MentionPlace, MentionRef, SuggestMentions};
use mannie_types::projects::{ProjectView, RemoveProject};
use mannie_types::skills::{
    SetSkillOn, SkillError, SkillField, SkillPlace, SkillView, SkillsOverview,
};
use mannie_types::tools::{Decision, ToolCallStatus, ToolOutcome};
use serde_json::json;
use tempfile::TempDir;

async fn harness(skills: bool) -> Harness {
    Harness::with_options(HarnessOptions {
        use_tools: true,
        skills,
        ..HarnessOptions::default()
    })
    .await
}

/// Writes a skill into a place, with more lines of frontmatter in `extra`.
fn write_skill(place: &Path, name: &str, description: &str, extra: &str) -> PathBuf {
    let folder = place.join(name);
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("SKILL.md"),
        format!(
            "---\nname: {name}\ndescription: {description}\n{extra}---\n\n# {name}\nTHE {name} INSTRUCTIONS.\n"
        ),
    )
    .unwrap();
    folder
}

/// A project in a folder of its own.
async fn project(harness: &Harness) -> (TempDir, ProjectView) {
    let folder = tempfile::tempdir().unwrap();
    let project = harness
        .runtime
        .add_project(folder.path().display().to_string())
        .await
        .unwrap();
    (folder, project)
}

fn project_skills(project: &ProjectView) -> PathBuf {
    mannie_tools::skills_folder_in(Path::new(&project.folder))
}

fn tool_names(request: &ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

fn read_skill(name: &str) -> String {
    json!({ "name": name, "path": null, "offset": null, "limit": null }).to_string()
}

/// Runs one reply that answers at once, and returns its request.
async fn one_request(harness: &Harness, start: StartConversation) -> ModelRequest {
    harness.model.push_reply(reply("Done."));
    let started = harness.runtime.start_conversation(start).await.unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    harness.model.reply_requests().pop().unwrap()
}

fn skill<'a>(
    overview: &'a SkillsOverview,
    project: Option<&ProjectView>,
    name: &str,
) -> &'a SkillView {
    let place = match project {
        None => overview.computer.as_ref().unwrap(),
        Some(project) => overview
            .projects
            .iter()
            .find(|place| place.label.as_deref() == Some(project.name.as_str()))
            .unwrap(),
    };
    place
        .skills
        .iter()
        .find(|skill| skill.name == name)
        .unwrap()
}

#[tokio::test]
async fn a_skill_of_this_computer_is_listed_and_read_without_asking() {
    let harness = harness(true).await;
    write_skill(&harness.computer_skills(), "pdf", "Fills PDF forms.", "");
    harness
        .model
        .push_reply(tool_calls(&[("call_1", "read_skill", &read_skill("pdf"))]));
    harness.model.push_reply(reply("Done."));

    let started = harness
        .runtime
        .start_conversation(StartConversation::new("Fill the form."))
        .await
        .unwrap();
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let requests = harness.model.reply_requests();
    let first = &requests[0];
    assert!(first.instructions.contains(texts::SKILLS_HEADING));
    assert!(first.instructions.contains("\n- pdf: Fills PDF forms."));
    assert!(tool_names(first).contains(&"read_skill".to_owned()));
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.decision, Some(Decision::Skill));
    let result = call.result.unwrap();
    assert!(
        result.output.contains("THE pdf INSTRUCTIONS."),
        "{}",
        result.output
    );
    assert!(matches!(
        result.outcome,
        ToolOutcome::SkillRead { ref skill, ref path, .. } if skill == "pdf" && path == "SKILL.md"
    ));
}

#[tokio::test]
async fn a_conversation_without_skills_sends_what_it_sent_before() {
    let without_catalog = harness(false).await;
    let with_empty_folder = harness(true).await;
    fs::create_dir_all(with_empty_folder.computer_skills()).unwrap();
    let with_a_skill_turned_off = harness(true).await;
    write_skill(
        &with_a_skill_turned_off.computer_skills(),
        "pdf",
        "Fills PDF forms.",
        "",
    );
    with_a_skill_turned_off
        .runtime
        .set_skill_on(SetSkillOn {
            place: SkillPlace::Computer,
            name: "pdf".to_owned(),
            on: false,
        })
        .await
        .unwrap();

    let expected = one_request(&without_catalog, StartConversation::new("Hi")).await;
    for harness in [with_empty_folder, with_a_skill_turned_off] {
        let request = one_request(&harness, StartConversation::new("Hi")).await;
        assert_eq!(request.instructions, expected.instructions);
        assert_eq!(request.tools, expected.tools);
    }
    assert!(!tool_names(&expected).contains(&"read_skill".to_owned()));
}

#[tokio::test]
async fn a_project_s_skill_is_off_until_the_owner_turns_it_on_and_off_again_when_it_changes() {
    let harness = harness(true).await;
    let (_folder, project) = project(&harness).await;
    let docx = write_skill(
        &project_skills(&project),
        "docx",
        "Writes Word documents.",
        "",
    );
    let in_project = || StartConversation::new("Write it.").with_project(Some(project.id.clone()));

    let before = one_request(&harness, in_project()).await;
    let listed_before = harness.runtime.list_skills().await.unwrap();
    let turned_on = harness
        .runtime
        .set_skill_on(SetSkillOn {
            place: SkillPlace::Project {
                project_id: project.id.clone(),
            },
            name: "docx".to_owned(),
            on: true,
        })
        .await
        .unwrap();
    let after = one_request(&harness, in_project()).await;
    fs::write(
        docx.join("SKILL.md"),
        "---\nname: docx\ndescription: Something else entirely.\n---\nBody.\n",
    )
    .unwrap();
    let changed = harness.runtime.list_skills().await.unwrap();
    let after_change = one_request(&harness, in_project()).await;

    assert!(!before.instructions.contains("docx"));
    assert!(!tool_names(&before).contains(&"read_skill".to_owned()));
    let off = skill(&listed_before, Some(&project), "docx");
    assert!(!off.is_on && !off.changed, "{off:?}");
    assert!(skill(&turned_on, Some(&project), "docx").is_on);
    assert!(
        after
            .instructions
            .contains("\n- docx: Writes Word documents.")
    );
    let changed_skill = skill(&changed, Some(&project), "docx");
    assert!(
        !changed_skill.is_on && changed_skill.changed,
        "{changed_skill:?}"
    );
    assert!(!after_change.instructions.contains("docx"));
}

#[tokio::test]
async fn a_project_s_skill_hides_this_computer_s_and_authors_narrow_what_shows() {
    let harness = harness(true).await;
    let computer = harness.computer_skills();
    write_skill(&computer, "pdf", "THE COMPUTER'S PDF.", "");
    write_skill(
        &computer,
        "deploy",
        "Deploys the site.",
        "disable-model-invocation: true\n",
    );
    let (_folder, project) = project(&harness).await;
    write_skill(&project_skills(&project), "pdf", "THE PROJECT'S PDF.", "");
    harness
        .runtime
        .set_skill_on(SetSkillOn {
            place: SkillPlace::Project {
                project_id: project.id.clone(),
            },
            name: "pdf".to_owned(),
            on: true,
        })
        .await
        .unwrap();

    let general = one_request(&harness, StartConversation::new("Hi")).await;
    let in_project = one_request(
        &harness,
        StartConversation::new("Hi").with_project(Some(project.id.clone())),
    )
    .await;

    assert!(general.instructions.contains("- pdf: THE COMPUTER'S PDF."));
    assert!(
        in_project
            .instructions
            .contains("- pdf: THE PROJECT'S PDF.")
    );
    assert!(!in_project.instructions.contains("THE COMPUTER'S PDF."));
    for request in [&general, &in_project] {
        assert!(!request.instructions.contains("deploy"));
        assert!(tool_names(request).contains(&"read_skill".to_owned()));
    }
}

fn custom_agent(name: &str, read_only: bool, tools: &[&str]) -> AgentDraft {
    AgentDraft {
        name: name.to_owned(),
        description: String::new(),
        kind: AgentKind::Conversation,
        read_only,
        profile_id: None,
        instructions: String::new(),
        steps: Vec::new(),
        tools: tools.iter().map(|tool| (*tool).to_owned()).collect(),
        plugins: None,
        children: None,
        max_tasks: 0,
        max_tool_rounds: 100,
        max_tool_calls: 250,
    }
}

#[tokio::test]
async fn a_custom_agent_reads_skills_only_with_the_group_also_when_it_only_reads() {
    let harness = harness(true).await;
    write_skill(&harness.computer_skills(), "pdf", "Fills PDF forms.", "");
    let reader = harness
        .runtime
        .create_agent(&custom_agent("Reader", true, &["read_file", "read_skill"]))
        .await
        .unwrap();
    let without = harness
        .runtime
        .create_agent(&custom_agent("Without", false, &["read_file"]))
        .await
        .unwrap();

    let with_group = one_request(
        &harness,
        StartConversation::new("Hi").with_agent(Some(reader.key)),
    )
    .await;
    let without_group = one_request(
        &harness,
        StartConversation::new("Hi").with_agent(Some(without.key)),
    )
    .await;

    assert!(tool_names(&with_group).contains(&"read_skill".to_owned()));
    assert!(with_group.instructions.contains("- pdf: Fills PDF forms."));
    assert!(!tool_names(&without_group).contains(&"read_skill".to_owned()));
    assert!(!without_group.instructions.contains(texts::SKILLS_HEADING));
}

#[tokio::test]
async fn the_switches_survive_a_restart() {
    let harness = harness(true).await;
    write_skill(&harness.computer_skills(), "pdf", "Fills PDF forms.", "");
    harness
        .runtime
        .set_skill_on(SetSkillOn {
            place: SkillPlace::Computer,
            name: "pdf".to_owned(),
            on: false,
        })
        .await
        .unwrap();

    let harness = harness.restart().await;
    let overview = harness.runtime.list_skills().await.unwrap();

    assert!(!skill(&overview, None, "pdf").is_on);
}

#[tokio::test]
async fn a_switch_of_a_missing_skill_or_project_is_refused() {
    let harness = harness(true).await;
    write_skill(&harness.computer_skills(), "pdf", "Fills PDF forms.", "");
    let (_folder, project) = project(&harness).await;
    let set = |place: SkillPlace, name: &str| SetSkillOn {
        place,
        name: name.to_owned(),
        on: true,
    };

    let missing = harness
        .runtime
        .set_skill_on(set(SkillPlace::Computer, "docx"))
        .await;
    let forged = harness
        .runtime
        .set_skill_on(set(SkillPlace::Computer, "../pdf"))
        .await;
    let elsewhere = harness
        .runtime
        .set_skill_on(set(
            SkillPlace::Project {
                project_id: project.id.clone(),
            },
            "pdf",
        ))
        .await;
    harness
        .runtime
        .remove_project(RemoveProject {
            project_id: project.id.clone(),
            conversations: 0,
        })
        .await
        .unwrap();
    let gone = harness
        .runtime
        .set_skill_on(set(
            SkillPlace::Project {
                project_id: project.id.clone(),
            },
            "pdf",
        ))
        .await;

    assert_eq!(missing, Err(SkillError::NotFound));
    assert_eq!(
        forged,
        Err(SkillError::FieldNotValid {
            field: SkillField::Name
        })
    );
    assert_eq!(elsewhere, Err(SkillError::NotFound));
    assert_eq!(gone, Err(SkillError::ProjectNotFound));
}

fn mention(key: &str) -> MentionRef {
    MentionRef {
        provider: SKILLS.to_owned(),
        key: key.to_owned(),
    }
}

#[tokio::test]
async fn a_mention_gives_a_skill_s_instructions_and_offers_a_project_s_skill_that_is_off() {
    let harness = harness(true).await;
    write_skill(
        &harness.computer_skills(),
        "pdf-forms",
        "Fills PDF forms.",
        "",
    );
    write_skill(
        &harness.computer_skills(),
        "hidden",
        "Never in the list.",
        "user-invocable: false\n",
    );
    let (_folder, project) = project(&harness).await;
    write_skill(
        &project_skills(&project),
        "docx",
        "Writes Word documents.",
        "",
    );

    let suggestions = harness
        .runtime
        .suggest_mentions(SuggestMentions::new(
            MentionPlace::NewConversation {
                project_id: Some(project.id.clone()),
            },
            "",
        ))
        .await
        .unwrap();
    let group = suggestions
        .groups
        .iter()
        .find(|group| group.provider == SKILLS)
        .expect("the skills' group");
    let keys: Vec<(&str, &str, Option<&str>)> = group
        .entries
        .iter()
        .map(|entry| {
            (
                entry.key.as_str(),
                entry.token.as_str(),
                entry.warning.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        keys,
        [
            ("project:docx", "@skill_docx", Some("off")),
            ("computer:pdf-forms", "@skill_pdf_forms", None),
        ]
    );
    assert_eq!(skill_token("pdf-forms"), "@skill_pdf_forms");

    let request = one_request(
        &harness,
        StartConversation::new("Fill it. @skill_pdf_forms")
            .with_mentions(vec![mention("computer:pdf-forms")]),
    )
    .await;
    let said = format!("{:?}", request.messages);
    assert!(
        said.contains("The owner chose the skill pdf-forms for this message."),
        "{said}"
    );
    assert!(said.contains("THE pdf-forms INSTRUCTIONS."), "{said}");

    let forged = harness
        .runtime
        .start_conversation(
            StartConversation::new("Fill it.").with_mentions(vec![mention("computer:hidden")]),
        )
        .await;
    assert!(
        forged.is_err(),
        "a skill kept out of \"@\" is refused: {forged:?}"
    );
}

/// A secret planted in a skill's folder, which no request, part or stored
/// file may hold.
const PLANTED: &str = "PLANTED-SKILL-SECRET-6f1c";

/// The files of a folder that hold `needle`.
fn files_contain(directory: &Path, needle: &str) -> Vec<String> {
    fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            let found = path.is_file()
                && fs::read(&path)
                    .unwrap()
                    .windows(needle.len())
                    .any(|window| window == needle.as_bytes());
            found.then(|| path.display().to_string())
        })
        .collect()
}

#[tokio::test]
async fn a_secret_in_a_skill_s_folder_never_reaches_a_request_or_the_database() {
    let harness = harness(true).await;
    let pdf = write_skill(&harness.computer_skills(), "pdf", "Fills PDF forms.", "");
    fs::write(pdf.join(".env"), format!("TOKEN={PLANTED}\n")).unwrap();
    fs::write(
        pdf.join("credentials.json"),
        format!("{{\"token\":\"{PLANTED}\"}}"),
    )
    .unwrap();
    harness.model.push_reply(tool_calls(&[
        ("call_1", "read_skill", &read_skill("pdf")),
        (
            "call_2",
            "read_skill",
            &json!({ "name": "pdf", "path": ".env", "offset": null, "limit": null }).to_string(),
        ),
        (
            "call_3",
            "read_skill",
            &json!({ "name": "pdf", "path": "credentials.json", "offset": null, "limit": null })
                .to_string(),
        ),
    ]));
    harness.model.push_reply(reply("Done."));

    let started = harness
        .runtime
        .start_conversation(StartConversation::new("Fill the form."))
        .await
        .unwrap();
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    harness.store.flush().await.unwrap();

    for request in harness.model.reply_requests() {
        assert!(!format!("{request:?}").contains(PLANTED));
    }
    let calls = transcript.tool_calls(0);
    assert_eq!(calls.len(), 3);
    for refused in &calls[1..] {
        let result = refused.result.as_ref().unwrap();
        assert!(
            matches!(
                &result.outcome,
                ToolOutcome::Failed {
                    failure: mannie_types::tools::ToolFailure::SkillFileRefused { .. }
                }
            ),
            "{:?}",
            result.outcome
        );
    }
    assert_eq!(
        files_contain(harness.data_folder(), PLANTED),
        Vec::<String>::new()
    );
}
