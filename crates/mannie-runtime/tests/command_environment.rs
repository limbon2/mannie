//! A secret in Mannie's own environment never reaches a command
//! (docs/features/system-tools/commands.md, environment; AGENTS.md: secrets
//! never appear in child process environments).
//!
//! The parent starts this test binary again, running the ignored
//! `child_runs_env`, with a planted secret in its environment. The child runs
//! a conversation whose model runs a command that prints every variable it
//! receives, and prints the command's output. The parent checks that the
//! output lists the allowed variables and not the planted one.

#![cfg(test)]
#![cfg(any(windows, target_os = "linux", target_os = "freebsd"))]

use std::process::Command;

use mannie_runtime::testing::{Harness, HarnessOptions, Transcript, reply, tool_calls};
use mannie_types::chat::StartConversation;
use mannie_types::tools::{ApprovalChoice, DecideApproval, ToolCallStatus};
use mannie_types::transcript::PartContent;
use serde_json::json;

const PLANTED_NAME: &str = "MANNIE_TEST_PLANTED_SECRET";
const PLANTED_VALUE: &str = "sk-planted-6c1f0e2b9d";
const OUTPUT_START: &str = "=== command output ===";
const OUTPUT_END: &str = "=== end of output ===";

/// The call that waits for the owner, if one does.
fn awaiting(transcript: &Transcript) -> Option<(mannie_types::ids::PartId, String)> {
    transcript
        .parts(0)
        .into_iter()
        .find_map(|part| match part.content {
            PartContent::ToolCall { call } if call.status == ToolCallStatus::AwaitingApproval => {
                call.request.map(|request| (part.id, request.digest))
            }
            _ => None,
        })
}

#[tokio::test]
#[ignore = "runs only as the child process of a_planted_secret_never_reaches_a_command"]
async fn child_runs_env() {
    if std::env::var_os(PLANTED_NAME).is_none() {
        return;
    }
    let harness = Harness::with_options(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await;
    let workspace = tempfile::tempdir().unwrap();
    let project = harness
        .runtime
        .add_project(workspace.path().to_str().unwrap().to_owned())
        .await
        .unwrap();
    let command = if cfg!(windows) {
        "Get-ChildItem env: | ForEach-Object { $_.Name + '=' + $_.Value }"
    } else {
        "env"
    };
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        "run_command",
        &json!({ "command": command }).to_string(),
    )]));
    harness.model.push_reply(reply("Done."));
    let started = harness
        .runtime
        .start_conversation(StartConversation {
            text: "Show the environment".to_owned(),
            profile_id: None,
            project_id: Some(project.id),
        })
        .await
        .unwrap();
    let transcript = harness
        .wait_until(&started.conversation_id, |transcript| {
            awaiting(transcript).is_some()
        })
        .await;
    let (part_id, digest) = awaiting(&transcript).unwrap();
    harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: started.conversation_id.clone(),
            part_id,
            digest,
            choice: ApprovalChoice::AllowOnce,
        })
        .await
        .unwrap();
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let output = transcript.tool_calls(0)[0]
        .result
        .as_ref()
        .map(|result| result.output.clone())
        .unwrap_or_default();
    println!("\n{OUTPUT_START}\n{output}\n{OUTPUT_END}");
}

#[test]
fn a_planted_secret_never_reaches_a_command() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "child_runs_env",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(PLANTED_NAME, PLANTED_VALUE)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some((command_output, _)) = stdout
        .split_once(OUTPUT_START)
        .and_then(|(_, rest)| rest.split_once(OUTPUT_END))
    else {
        panic!("the child printed no command output: {stdout}");
    };

    assert!(
        command_output.to_uppercase().contains("PATH="),
        "the command ran without its allowed variables: {command_output}"
    );
    assert!(!command_output.contains(PLANTED_VALUE), "{command_output}");
    assert!(!command_output.contains(PLANTED_NAME), "{command_output}");
}
