//! A test master for the smoke tests of the worker's service
//! (scripts/ci/worker-service-*): it pairs with the installed worker at an
//! address with the code that `mannie-worker pair` printed, delegates one
//! task that runs a command, allows the command when the worker asks,
//! checks that the command ran and that the report came back, and removes
//! the worker again. The worker's model is a fixture server on this host,
//! so the worker must run on this host too.
//!
//! ```sh
//! cargo run -p mannie-link --features testing --example smoke-master -- 127.0.0.1:7445 <code>
//! ```

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the test master reports its result on the console"
)]

use std::process::ExitCode;

use mannie_provider::testing::{FixtureResponse, FixtureServer};
use mannie_runtime::testing::{reply, tool_calls};
use mannie_types::tasks::TaskState;
use mannie_workers::testing::{LinkedMaster, LinkedMasterOptions, delegation_to};
use mannie_workers::view::WorkerStatus;
use serde_json::{Value, json};

/// The report that the worker's model writes.
const REPORT: &str = "Report: the command ran.";

/// One event of a chat completions stream.
fn chunk(delta: &Value, finish_reason: Option<&str>) -> String {
    let event = json!({
        "id": "chatcmpl-smoke",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": "test-model",
        "choices": [{ "index": 0, "delta": delta, "finish_reason": finish_reason }],
    });
    format!("data: {event}\n\n")
}

/// A reply of the worker's model that calls one tool.
fn tool_call(name: &str, arguments: &Value) -> FixtureResponse {
    let call = json!({
        "role": "assistant",
        "tool_calls": [{
            "index": 0,
            "id": "call_smoke",
            "type": "function",
            "function": { "name": name, "arguments": arguments.to_string() },
        }],
    });
    let body = chunk(&call, None) + &chunk(&json!({}), Some("tool_calls")) + "data: [DONE]\n\n";
    FixtureResponse::event_stream(body)
}

/// A reply of the worker's model in text.
fn text(content: &str) -> FixtureResponse {
    let body = chunk(&json!({ "role": "assistant", "content": content }), None)
        + &chunk(&json!({}), Some("stop"))
        + "data: [DONE]\n\n";
    FixtureResponse::event_stream(body)
}

async fn smoke(address: &str, code: &str) -> Result<String, String> {
    let provider = FixtureServer::start()
        .await
        .map_err(|error| format!("the fixture server did not start: {error}"))?;
    provider.respond(tool_call(
        "run_command",
        &json!({ "activity": "Printing", "command": "echo mannie-smoke", "timeout_seconds": 60 }),
    ));
    provider.respond(text("The command printed its word."));
    provider.respond(text(REPORT));
    let master = LinkedMaster::with_options(LinkedMasterOptions {
        profile_url: provider.base_url().to_string(),
        ..LinkedMasterOptions::default()
    })
    .await;
    let worker = master
        .pair(address, code)
        .await
        .map_err(|error| format!("pairing failed: {error:?}"))?;
    master.wait_for_status(WorkerStatus::Connected).await;
    master.model.push_reply(tool_calls(&[(
        "call_1",
        "delegate_task",
        &delegation_to(&worker.name, "Print a word with a command, then report."),
    )]));
    master
        .model
        .push_reply(reply("The worker ran the command."));
    let conversation = master
        .start_conversation("Run a command on the worker")
        .await;
    let waiting = master
        .tasks_when(&conversation, |tasks| {
            tasks
                .first()
                .is_some_and(|task| task.approval.is_some() || !task.state.is_active())
        })
        .await;
    if waiting[0].approval.is_some() {
        master
            .allow_once(&waiting[0])
            .await
            .map_err(|error| format!("allowing the command failed: {error:?}"))?;
    }
    let ended = master
        .tasks_when(&conversation, |tasks| {
            tasks.first().is_some_and(|task| !task.state.is_active())
        })
        .await;
    let task = &ended[0];
    if task.state != TaskState::Reported || task.report.as_deref() != Some(REPORT) {
        return Err(format!(
            "the task ended as {:?} with the report {:?}",
            task.state, task.report
        ));
    }
    // The command's result goes back to the worker's model in its next call;
    // only a command that ran has an exit code.
    let ran = provider
        .requests()
        .iter()
        .skip(1)
        .any(|request| String::from_utf8_lossy(&request.body).contains("Exit code: 0"));
    if !ran {
        return Err("the worker's model never got the command's result".to_owned());
    }
    master
        .hub
        .remove(&worker.id)
        .await
        .map_err(|error| format!("removing the worker failed: {error:?}"))?;
    Ok(worker.name)
}

#[tokio::main]
async fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [address, code] = arguments.as_slice() else {
        eprintln!("usage: smoke-master <address> <code>");
        return ExitCode::FAILURE;
    };
    match smoke(address, code).await {
        Ok(name) => {
            println!(
                "smoke-master: {name} paired, ran a delegated command, reported, and was removed"
            );
            ExitCode::SUCCESS
        }
        Err(problem) => {
            eprintln!("smoke-master: {problem}");
            ExitCode::FAILURE
        }
    }
}
