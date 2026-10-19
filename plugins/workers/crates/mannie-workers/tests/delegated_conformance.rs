//! The system tools through a delegated task (plugins/workers/docs/
//! delegation.md, acceptance; milestone M4's exit): a task that the master
//! delegates over the link uses every tool of the worker agent in the
//! allowed folder of an autonomous host, and the results land on the
//! worker's disk. A command that the host's rules do not cover asks the
//! owner through the master, and the master reads its archived output
//! through the link. CI runs it on Windows, Linux and FreeBSD.
//!
//! `read_tool_output` needs the ID of an archive from an earlier result,
//! which a scripted model cannot know, so the master's read of the archive
//! stands in for it here; the conformance suite of `mannie-tools` covers the
//! tool itself.

#![cfg(test)]

use std::fmt::Write as _;
use std::path::Path;

use mannie_plugin_harness::runtime::{WorkerOptions, reply, tool_calls};
use mannie_plugin_harness::types::RuleId;
use mannie_plugin_harness::types::TaskView;
use mannie_plugin_harness::types::{CommandRule, Decision, ToolCallStatus, ToolOutcome};
use mannie_plugin_sdk::contracts::CeilingMode;
use mannie_plugin_sdk::contracts::ReadToolOutput;
use mannie_plugin_sdk::contracts::TaskState;
use mannie_workers_testing::master::{LinkedMaster, paired};
use mannie_workers_testing::worker::{LinkedWorker, LinkedWorkerOptions};
use serde_json::{Value, json};

/// How many lines the long command prints: more than a tool result holds,
/// so its output is archived.
const LONG_OUTPUT_LINES: u32 = 20_000;

fn text(path: &Path) -> String {
    path.to_str().expect("a test path in UTF-8").to_owned()
}

fn arguments(activity: &str, mut fields: Value) -> String {
    fields["activity"] = json!(activity);
    fields.to_string()
}

/// The revision that `read_file` reports for `content`.
fn revision(content: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, content.as_bytes());
    let mut revision = "sha256:".to_owned();
    for byte in digest.as_ref() {
        // Writing to a String cannot fail.
        let _written = write!(revision, "{byte:02x}");
    }
    revision
}

/// A command whose output is longer than a tool result, on every system.
fn long_command() -> String {
    if cfg!(windows) {
        format!("1..{LONG_OUTPUT_LINES} | ForEach-Object {{ \"line $_\" }}")
    } else {
        format!("i=1; while [ $i -le {LONG_OUTPUT_LINES} ]; do echo \"line $i\"; i=$((i+1)); done")
    }
}

/// The calls of the worker agent, one in each step, in order.
fn script(work: &Path) -> Vec<(&'static str, String)> {
    let source = work.join("src");
    let notes = source.join("notes.txt");
    let copy = source.join("copy.txt");
    let moved = work.join("moved.txt");
    vec![
        ("system_info", arguments("Checking", json!({}))),
        (
            "create_directory",
            arguments("Creating", json!({ "path": text(&source) })),
        ),
        (
            "write_file",
            arguments(
                "Writing",
                json!({ "path": text(&notes), "content": "alpha\nbeta\n" }),
            ),
        ),
        (
            "read_file",
            arguments("Reading", json!({ "path": text(&notes) })),
        ),
        (
            "edit_file",
            arguments(
                "Editing",
                json!({
                    "path": text(&notes),
                    "expected_revision": revision("alpha\nbeta\n"),
                    "edits": [{ "old_text": "beta", "new_text": "gamma" }],
                }),
            ),
        ),
        (
            "file_info",
            arguments("Inspecting", json!({ "path": text(&notes) })),
        ),
        (
            "list_directory",
            arguments("Listing", json!({ "path": text(&source) })),
        ),
        (
            "find_files",
            arguments(
                "Finding",
                json!({ "pattern": "*.txt", "path": text(&source) }),
            ),
        ),
        (
            "search_text",
            arguments("Searching", json!({ "query": "gamma", "path": text(work) })),
        ),
        (
            "copy_path",
            arguments(
                "Copying",
                json!({ "source": text(&notes), "destination": text(&copy) }),
            ),
        ),
        (
            "move_path",
            arguments(
                "Moving",
                json!({ "source": text(&copy), "destination": text(&moved) }),
            ),
        ),
        (
            "delete_path",
            arguments("Deleting", json!({ "path": text(&moved) })),
        ),
        (
            "run_command",
            arguments(
                "Printing",
                json!({ "command": "echo delegated", "timeout_seconds": 60 }),
            ),
        ),
        (
            "run_command",
            arguments(
                "Counting",
                json!({ "command": long_command(), "timeout_seconds": 120 }),
            ),
        ),
    ]
}

/// A worker whose ceiling is autonomous, with a host rule for `echo`, and
/// a master paired with it.
async fn autonomous_worker_and_master() -> (LinkedWorker, LinkedMaster) {
    paired(LinkedWorkerOptions {
        worker: WorkerOptions {
            mode: CeilingMode::Autonomous,
            rules: vec![CommandRule {
                id: RuleId::parse("0199e0f2-0000-7000-8000-000000000071").unwrap(),
                words: vec!["echo".to_owned()],
                folder: None,
                created_at: mannie_plugin_sdk::contracts::UtcMillis::EPOCH,
            }],
            ..WorkerOptions::default()
        },
        ..LinkedWorkerOptions::default()
    })
    .await
}

/// Scripts the master's delegation and the worker agent's `calls`.
fn script_models(worker: &LinkedWorker, master: &LinkedMaster, calls: &[(&str, String)]) {
    let delegate = json!({
        "activity": "Delegating",
        "worker": "build-01",
        "title": "Exercise the tools",
        "instruction": "Use every tool once, then report.",
        "background": false,
        "read_only": false,
    })
    .to_string();
    master
        .model
        .push_reply(tool_calls(&[("call_1", "delegate_task", &delegate)]));
    master.model.push_reply(reply("Every tool worked."));
    for (number, (tool, arguments)) in calls.iter().enumerate() {
        let id = format!("call_{number}");
        worker
            .harness
            .model
            .push_reply(tool_calls(&[(id.as_str(), tool, arguments.as_str())]));
    }
    worker.harness.model.push_reply(reply("Done."));
    worker
        .harness
        .model
        .push_reply(reply("Report: every tool worked."));
}

/// Runs the delegating conversation and allows the call that asks once,
/// then returns the task when it ended.
async fn delegate_and_allow(master: &LinkedMaster) -> TaskView {
    let conversation = master
        .start_conversation("Exercise the tools on the server")
        .await;
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| task.approval.is_some())
        })
        .await;
    master.allow_once(&tasks[0]).await.unwrap();
    let tasks = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| !task.state.is_active())
        })
        .await;
    tasks[0].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delegated_task_uses_every_tool_of_the_worker_agent() {
    let (worker, master) = autonomous_worker_and_master().await;
    let work = worker.harness.allowed.clone();
    let calls = script(&work);
    script_models(&worker, &master, &calls);

    let task = delegate_and_allow(&master).await;
    let made = worker
        .harness
        .snapshot(&task.conversation_id)
        .await
        .tool_calls(0);

    assert_eq!(task.state, TaskState::Reported, "{task:?}");
    assert_eq!(made.len(), calls.len());
    for (call, (tool, _arguments)) in made.iter().zip(&calls) {
        assert_eq!(call.tool, *tool);
        assert_eq!(call.status, ToolCallStatus::Completed, "{call:?}");
        let outcome = call.result.as_ref().map(|result| &result.outcome);
        assert!(
            !matches!(outcome, Some(ToolOutcome::Failed { .. })),
            "{call:?}"
        );
    }
    assert_eq!(
        made.iter().map(|call| call.decision).collect::<Vec<_>>()[12..],
        [Some(Decision::HostRule), Some(Decision::AllowedOnce)]
    );
    assert_eq!(
        std::fs::read_to_string(work.join("src").join("notes.txt")).unwrap(),
        "alpha\ngamma\n"
    );
    assert!(!work.join("src").join("copy.txt").exists());
    assert!(!work.join("moved.txt").exists());
    let echoed = made[12].result.as_ref().unwrap();
    assert!(echoed.output.contains("delegated"), "{}", echoed.output);
    let archive = made[13]
        .result
        .as_ref()
        .and_then(|result| result.archives.first())
        .expect("the long output is archived");
    let page = master
        .runtime
        .tool_output(ReadToolOutput {
            conversation_id: task.conversation_id.clone(),
            archive_id: archive.id.clone(),
            first_line: 100,
            limit: 3,
        })
        .await
        .unwrap();
    assert_eq!(page.lines.len(), 3, "{page:?}");
    assert!(page.lines[0].starts_with("line "), "{page:?}");
    assert!(page.total_lines >= LONG_OUTPUT_LINES, "{page:?}");
}
