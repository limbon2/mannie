//! The plugin as the host and the core see it (plugins/git/docs/tools.md):
//! its manifest, the SDK's rules through the real host, and its tools
//! through the core's tool loop with the core's service `processes`, which
//! finds the installed git. The tools are offered where the conversation's
//! project is in a repository, read it without asking, give their cards'
//! data, and come with the section that tells the model how to use them.

use mannie_git::{GIT, PLUGIN_MANIFEST, mannie_plugin};
use mannie_git_testing::{TempFolder, TestRepository};
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::{HostOptions, HostedPlugin};
use mannie_plugin_harness::provider::ModelRequest;
use mannie_plugin_harness::runtime::{HarnessOptions, SeenPart, Transcript, reply, tool_calls};
use mannie_plugin_harness::types::{
    ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision, GrantScope,
    PartContent, PartId, ProgramEffectKey, StartConversation, ToolCallStatus, ToolCallView,
    ToolOutcome, UpdateToolsSettings,
};
use mannie_plugin_sdk::contracts::ProjectId;
use mannie_plugin_sdk::manifest::Manifest;
use serde_json::Value;

/// The tools that read, which every agent is offered.
const READING_TOOLS: [&str; 8] = [
    "git_status",
    "git_log",
    "git_show",
    "git_diff",
    "git_blame",
    "git_grep",
    "git_refs",
    "git_conflicts",
];

#[test]
fn the_manifest_runs_git_with_the_settings_that_keep_repository_programs_out() {
    let manifest = Manifest::parse(PLUGIN_MANIFEST).unwrap();

    assert_eq!(manifest.id.as_str(), GIT);
    assert!(manifest.declares("processes"));
    let git = &manifest.processes["git"];
    assert_eq!(git.fixed[0], "--no-pager");
    for setting in [
        "core.fsmonitor=false",
        "submodule.recurse=false",
        "merge.conflictStyle=zdiff3",
        "core.quotePath=false",
        "color.ui=false",
    ] {
        assert!(
            git.fixed.iter().any(|argument| argument == setting),
            "{setting}"
        );
    }
}

#[tokio::test]
async fn the_plugin_keeps_the_sdk_s_rules() {
    let problems = conformance(mannie_plugin(), HostOptions::new()).await;

    assert_eq!(problems, Vec::<String>::new());
}

/// The plugin, on, in a runtime whose conversations use tools.
async fn hosted() -> HostedPlugin {
    let core = HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    };
    let hosted = HostedPlugin::new(mannie_plugin(), HostOptions::new().with_core(core)).await;
    hosted.turn_on().await;
    hosted
}

/// A project whose folder is `folder`.
async fn project(hosted: &HostedPlugin, folder: &std::path::Path) -> ProjectId {
    hosted
        .core
        .runtime
        .add_project(
            folder
                .to_str()
                .expect("the test's folder is Unicode")
                .to_owned(),
        )
        .await
        .expect("the project is added")
        .id
}

/// The names of the tools that a request offered.
fn offered(request: &ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

#[tokio::test]
async fn the_tools_read_a_project_s_repository_without_asking() {
    let hosted = hosted().await;
    let core = &hosted.core;
    let repository = TestRepository::new();
    repository.write("a.txt", "one\n");
    repository.commit("Start");
    repository.write("a.txt", "two\n");
    let project = project(&hosted, repository.path()).await;
    core.model.push_reply(tool_calls(&[(
        "call_1",
        "git_status",
        r#"{"activity": "Reading the status", "repository": null}"#,
    )]));
    core.model.push_reply(reply("One file changed."));

    let started = core
        .runtime
        .start_conversation(
            StartConversation::new("OWNER: what changed?").with_project(Some(project)),
        )
        .await
        .unwrap();
    let transcript = core
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let [call]: [ToolCallView; 1] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.decision, Some(Decision::Workspace));
    assert_eq!(call.plugin.as_deref(), Some(GIT));
    let result = call.result.expect("the call ran");
    assert!(
        result.output.starts_with("On branch main at "),
        "{}",
        result.output
    );
    assert!(
        result
            .output
            .contains("Not staged (1):\n  modified: a.txt\n"),
        "{}",
        result.output
    );
    let ToolOutcome::Plugin {
        plugin,
        data,
        failure,
    } = result.outcome
    else {
        panic!("a plugin's outcome: {:?}", result.outcome);
    };
    assert_eq!(plugin, GIT);
    assert_eq!(failure, None);
    let data: Value = serde_json::from_str(&data.expect("the card's data")).unwrap();
    assert_eq!(data["branch"]["head"], "main");
    let request = &core.model.reply_requests()[0];
    for tool in READING_TOOLS {
        assert!(offered(request).iter().any(|name| name == tool), "{tool}");
    }
    assert!(
        request
            .instructions
            .contains("Git: the project is in a git repository.")
    );
}

/// Allows changes in the workspace, as the owner's setting does, and new
/// conversations in full access when `full_access` says so.
async fn allow_changes(hosted: &HostedPlugin, full_access: bool) {
    hosted
        .core
        .runtime
        .update_tools_settings(UpdateToolsSettings {
            allow_changes_in_workspace: true,
            command_environment: Vec::new(),
            full_access_for_new_conversations: full_access,
            wake_on_command_end: true,
        })
        .await
        .expect("the settings are saved");
}

/// A repository with one commit, a pre-commit hook and a change of `a.txt`.
fn hooked() -> TestRepository {
    let repository = TestRepository::new();
    repository.write("a.txt", "one\n");
    repository.commit("Start");
    repository.write_hook("pre-commit", "#!/bin/sh\nexit 0\n");
    repository.write("a.txt", "two\n");
    repository
}

/// A call of `git_commit` of every tracked change with `message`.
fn commit_all(message: &str) -> String {
    serde_json::json!({
        "activity": "Committing",
        "repository": null,
        "message": message,
        "amend": null,
        "all": true,
        "allow_empty": true,
        "fixup": null,
    })
    .to_string()
}

/// The tool call of the turn that waits for the owner, if one does.
fn waiting_call(transcript: &Transcript) -> Option<(PartId, ApprovalRequest)> {
    transcript.parts(0).into_iter().find_map(|part| match part {
        SeenPart {
            id,
            content: PartContent::ToolCall { call },
            ..
        } if call.status == ToolCallStatus::AwaitingApproval => {
            call.request.clone().map(|request| (id, request))
        }
        _other => None,
    })
}

#[tokio::test]
async fn a_commit_that_runs_a_hook_asks_and_a_grant_for_the_conversation_covers_the_next() {
    let hosted = hosted().await;
    allow_changes(&hosted, false).await;
    let core = &hosted.core;
    let repository = hooked();
    let project = project(&hosted, repository.path()).await;
    core.model
        .push_reply(tool_calls(&[("call_1", "git_commit", &commit_all("Two"))]));
    core.model.push_reply(tool_calls(&[(
        "call_2",
        "git_commit",
        &commit_all("Three"),
    )]));
    core.model.push_reply(reply("Committed twice."));

    let started = core
        .runtime
        .start_conversation(StartConversation::new("OWNER: commit").with_project(Some(project)))
        .await
        .unwrap();
    let conversation = &started.conversation_id;
    let transcript = core
        .wait_until(conversation, |transcript| {
            waiting_call(transcript).is_some()
        })
        .await;
    let (part_id, request) = waiting_call(&transcript).expect("the hooked commit asks");
    core.runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id,
            digest: request.digest.clone(),
            choice: ApprovalChoice::AllowForConversation,
        })
        .await
        .unwrap();
    let transcript = core.wait_for_run(conversation, &started.run_id).await;

    assert_eq!(request.reasons, vec![ApprovalReason::Program]);
    assert!(
        request.grants.iter().any(|grant| matches!(
            grant,
            GrantScope::Program { effects, .. }
                if effects == &[ProgramEffectKey::RunsFolderCode {
                    name: "pre-commit".to_owned(),
                    source: ".git/hooks/pre-commit".to_owned(),
                }]
        )),
        "{:?}",
        request.grants
    );
    let calls = transcript.tool_calls(0);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].decision, Some(Decision::AllowedForConversation));
    assert_eq!(
        calls[1].decision,
        Some(Decision::Grant),
        "the grant covers the same run"
    );
    assert_eq!(
        repository.git(&["log", "-1", "--format=%s"]).trim(),
        "Three"
    );
}

#[tokio::test]
async fn full_access_runs_a_hooked_commit_without_asking() {
    let hosted = hosted().await;
    allow_changes(&hosted, true).await;
    let core = &hosted.core;
    let repository = hooked();
    let project = project(&hosted, repository.path()).await;
    core.model
        .push_reply(tool_calls(&[("call_1", "git_commit", &commit_all("Two"))]));
    core.model.push_reply(reply("Committed."));

    let started = core
        .runtime
        .start_conversation(StartConversation::new("OWNER: commit").with_project(Some(project)))
        .await
        .unwrap();
    let transcript = core
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let [call]: [ToolCallView; 1] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed, "{call:?}");
    assert_eq!(call.request, None);
    assert_eq!(repository.git(&["log", "-1", "--format=%s"]).trim(), "Two");
}

#[tokio::test]
async fn the_tools_and_the_section_stay_out_where_no_repository_is() {
    let hosted = hosted().await;
    let core = &hosted.core;
    let plain = TempFolder::new();
    let project = project(&hosted, plain.path()).await;
    core.model.push_reply(reply("Done."));
    core.model.push_reply(reply("Done."));

    for project in [None, Some(project)] {
        let started = core
            .runtime
            .start_conversation(StartConversation::new("OWNER: hello").with_project(project))
            .await
            .unwrap();
        core.wait_for_run(&started.conversation_id, &started.run_id)
            .await;
    }

    for request in core.model.reply_requests() {
        assert!(
            !offered(&request)
                .iter()
                .any(|name| name.starts_with("git_")),
            "{:?}",
            offered(&request)
        );
        assert!(!request.instructions.contains("Git: the project"));
    }
}
