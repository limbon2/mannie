use std::path::PathBuf;
use std::sync::Arc;

use mannie_host_kit::runtime::{InstructionContext, MachineContext, ToolEnvironment, texts};
use mannie_link::host::HostFacts;
use mannie_plugin_sdk::contracts::CeilingMode;

use super::{
    HOST_INSTRUCTIONS_HEADING, Host, WORKER, WORKER_ASK_GUIDANCE, WORKER_READ_ONLY_GUIDANCE,
    WORKER_TOOL_GUIDANCE, agents, instructions, worker_tool_guidance,
};

#[test]
fn a_worker_reads_its_host_and_never_the_owners_instructions() {
    let facts = HostFacts {
        system: "FreeBSD 14.4-RELEASE, amd64".to_owned(),
        hostname: "build-01".to_owned(),
        shells: vec!["sh".to_owned()],
        tools: vec!["git".to_owned(), "pkg".to_owned()],
    };
    let host = Arc::new(Host {
        name: "build-01".to_owned(),
        purpose: "Builds the backend".to_owned(),
        facts: facts.clone(),
    });
    let registry = instructions(&host);
    let folders = [PathBuf::from("/srv/app")];
    let context = InstructionContext {
        agent: &WORKER,
        owner_instructions: "Owner text.",
        checkpoint: None,
        tools: Some(ToolEnvironment {
            system: &facts.system,
            shell: Some("sh"),
            project: None,
            projects: &[],
            full_access: false,
            offered: &[],
        }),
        offers_tasks: false,
        machine: Some(MachineContext {
            mode: CeilingMode::Ask,
            allowed_folders: &folders,
            instructions: Some("Never restart nginx."),
        }),
    };

    let assembled = registry.assemble(&context);

    assert!(
        assembled.starts_with("You are a worker agent in Mannie, on the host build-01."),
        "{assembled}"
    );
    assert!(assembled.contains(WORKER_ASK_GUIDANCE), "{assembled}");
    assert!(
        assembled.contains("- Purpose: Builds the backend"),
        "{assembled}"
    );
    assert!(
        assembled.contains("- Programs on PATH: git, pkg"),
        "{assembled}"
    );
    let heading_at = assembled.find(HOST_INSTRUCTIONS_HEADING).unwrap();
    let rule_at = assembled.find("Never restart nginx.").unwrap();
    let workspace_at = assembled
        .find("- Workspace: /srv/app, the first allowed folder of this host")
        .unwrap();
    assert!(
        heading_at < rule_at && rule_at < workspace_at,
        "{assembled}"
    );
    assert!(!assembled.contains("Owner text."), "{assembled}");
    assert!(!assembled.contains(texts::TOOL_GUIDANCE), "{assembled}");
}

#[test]
fn a_read_only_task_is_told_that_changes_are_refused() {
    let guidance = worker_tool_guidance(CeilingMode::ReadOnly);

    assert!(guidance.starts_with(WORKER_TOOL_GUIDANCE));
    assert!(guidance.ends_with(WORKER_READ_ONLY_GUIDANCE));
}

#[test]
fn the_worker_registers_only_the_worker_agent() {
    let registry = agents();

    assert_eq!(registry.get(super::WORKER_AGENT), Some(&WORKER));
    assert!(
        registry
            .get(mannie_host_kit::runtime::CONVERSATION_AGENT)
            .is_none()
    );
    assert_eq!(WORKER.tools, mannie_host_kit::runtime::GENERAL.tools);
    assert_eq!(WORKER.max_tasks, 0);
    assert!(!WORKER.tools.contains(&"move_to_project"));
    // A task's commands in the background end with it (ADR-0041).
    for tool in ["start_command", "read_command_output", "stop_command"] {
        assert!(WORKER.tools.contains(&tool), "{tool}");
    }
}

#[test]
fn the_guidance_says_that_a_command_in_the_background_ends_with_the_task() {
    let guidance = worker_tool_guidance(CeilingMode::Autonomous);

    assert!(
        guidance.contains("start_command runs a command in the background until it ends, you stop it with stop_command, or your task ends"),
        "{guidance}"
    );
    assert!(guidance.contains("without & or nohup"), "{guidance}");
}
