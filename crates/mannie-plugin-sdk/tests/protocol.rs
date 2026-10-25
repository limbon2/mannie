//! The protocol rules of the SDK (ADR-0034, decision 3): the SDK's values
//! cross JSON unchanged, so that a later adapter could carry them across a
//! process or WebAssembly boundary, and the shapes that break the rules are
//! listed with the reason that SDK 1 keeps them.

#![cfg(test)]

use std::collections::BTreeSet;

use mannie_plugin_sdk::context::{HostInfo, HostProfile};
use mannie_plugin_sdk::contracts::{
    AccessMode, ArchiveId, CommandError, ContextLocation, ImageFormat, MentionEntry,
    MentionSummary, ProfileSnapshot, SubscriptionEnd, TaskExecutorRef, TaskState, ToolHint,
    ToolServerEndpoint, WorkItem,
};
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::ipc::{ChannelMessage, HostRefusal, InputRefusal, PluginFailure};
use mannie_plugin_sdk::points::agents::Requester;
use mannie_plugin_sdk::points::context::{ContextItem, ContextRequest};
use mannie_plugin_sdk::points::executors::{
    ExecutorRefusal, ExecutorSection, FollowUpPlan, PlanError, SubmitError, TaskPlan,
    TaskSubmission,
};
use mannie_plugin_sdk::points::mentions::{ProviderSuggestions, ReadVerdict, RenderedMention};
use mannie_plugin_sdk::points::sections::SectionContext;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, ExternalToolAccess, PathAccess, RemoteCommandAccess, ToolAccess, ToolArchive,
    ToolCall, ToolExecution, ToolImage, ToolOffering, ToolRefusal, ToolResult, ToolSpec,
};
use mannie_plugin_sdk::services::background_commands::{
    BackgroundError, BackgroundScope, BackgroundTicket, RemoteEnding, RemoteOutput, StartedCommand,
};
use mannie_plugin_sdk::services::credentials::LendError;
use mannie_plugin_sdk::services::executor_host::{ExecutorTask, HostFailure, OutboxItem};
use mannie_plugin_sdk::services::file_dialog::{DialogError, FileFilter};
use mannie_plugin_sdk::services::models::{InputKind, ModelsError, SearchModel, SearchModelKind};
use mannie_plugin_sdk::services::secrets::SecretsError;
use serde_json::json;

/// A module of the SDK whose shapes break the protocol rules, such as a trait
/// that borrows its arguments or returns a future that borrows them.
#[derive(Debug)]
struct KnownException {
    /// The module's path inside the SDK, such as `points::images`.
    module: &'static str,
    /// Why SDK 1 keeps the shape, and what reshapes it.
    reason: &'static str,
}

/// SDK 1 moves today's extension points in their current shapes (decision 3).
/// Each change that moves one into the SDK adds its module here, and the SDK
/// change that comes before third-party plugins reshapes them and empties
/// this list.
const KNOWN_EXCEPTIONS: &[KnownException] = &[
    KnownException {
        module: "context",
        reason: "the start context hands the plugin the host's clock, whose `now` answers at \
                 once as the core's clock does, and the APIs of the services that the host bound, \
                 and the default's context hands it the queries of its module, so neither context \
                 crosses JSON; a boundary builds them on the plugin's side from the `HostInfo`, \
                 which does, with a clock of the plugin's process and proxies of the services",
    },
    KnownException {
        module: "plugin",
        reason: "the plugin's definition is the in-process registration of SDK 1: an entry \
                 embeds the manifest's text as `&'static str` and holds the plugin as a trait \
                 object, `Plugin::store_module` returns a `services::store` module, and \
                 `Started` hands the host the plugin's contributions, whose points are listed \
                 here, with its instance as a trait object that the host keeps; a boundary that \
                 loads plugins from elsewhere replaces the entry and carries the instance's \
                 calls, whose results, a `WorkItem` and a `StartFailure`, cross JSON",
    },
    KnownException {
        module: "points::agents",
        reason: "an agent spec keeps the shape of the runtime's agent definitions: its name, its \
                 instructions and its tool names are `&'static str`, which a plugin defines as \
                 constants, because the runtime keeps the names of the agents that runs can \
                 start for the life of the process, so a spec does not cross JSON",
    },
    KnownException {
        module: "points::executors",
        reason: "the task executor keeps its shape from ADR-0031 (decision 4): `kind` and \
                 `agent` lend `&'static str`, `plan` takes a `PlanCall` that borrows the call's \
                 arguments and profile, `kind`, `agent`, `tool`, `section`, `outlives_reply`, \
                 `plan`, `plan_follow_up` and `document` answer at once instead of returning \
                 futures, and `plan_follow_up`, `document` and every method that returns a \
                 future apart from `submit` borrow some of their arguments, such as the target, \
                 the profile, the task or the submission",
    },
    KnownException {
        module: "points::images",
        reason: "the image source keeps its shape from ADR-0033 (decision 2): `id` lends its \
                 text, `resolve` returns a future that borrows the source and the key, `release` \
                 borrows the keys, and `SourcedImage` carries the image's bytes as \
                 `contracts::ImageBytes`, which does not serialize, so the re-export is part \
                 of this exception",
    },
    KnownException {
        module: "points::mentions",
        reason: "the mention provider keeps its shape from ADR-0032 (decision 2): `id` lends its \
                 text, `suggest` and `resolve` return futures that borrow the provider, the \
                 context and the query or key, and `resolve` hands the core a `MentionContent` \
                 that renders on each call; the context does not serialize and its `can_read` \
                 calls the core's `ReadCheck`, a callback into host code (ADR-0034, decision 3)",
    },
    KnownException {
        module: "points::sections",
        reason: "a section's ID is a `&'static str`, as the IDs of the core's sections are, and \
                 `SectionBody::Source` holds the plugin's `SectionSource`, which the core calls \
                 for each call of a run, so a section does not cross JSON; the `SectionContext` \
                 that a source renders from does",
    },
    KnownException {
        module: "services::credentials",
        reason: "a lent credential carries the credential as `contracts::Secret`, which never \
                 serializes, so that a credential cannot reach a log, the database or an IPC \
                 response by accident (security architecture, rules for handling secrets); a \
                 boundary that carries a lend needs a transport of its own for the secret",
    },
    KnownException {
        module: "services::file_dialog",
        reason: "a file handle names a file in the table of one plugin's binding and reads \
                 once, so it neither clones nor crosses JSON, and `ChosenFile` holds one; a \
                 boundary keeps the table on the host's side and carries the handles' numbers, \
                 and a filter and the errors cross JSON",
    },
    KnownException {
        module: "services::local_servers",
        reason: "a server's spec carries its secrets as `contracts::Secret`, which never \
                 serializes, for the reason of `services::credentials`, and a running server \
                 is a trait object whose output and errors the plugin takes once; a boundary \
                 carries the spec's secrets in a transport of their own and proxies the \
                 server, while its output, its end and its errors cross JSON",
    },
    KnownException {
        module: "services::secrets",
        reason: "a plugin's secret crosses the service as `contracts::Secret`, which never \
                 serializes, for the reason of `services::credentials`; a boundary that \
                 carries the service needs a transport of its own for the secret",
    },
    KnownException {
        module: "services::store",
        reason: "a module's name, its migrations and its statements are `&'static str`, which \
                 keeps a plugin's SQL as text in its source, where review sees it, and the store \
                 prepares each listed statement when the module registers; a constraint's code \
                 is a `&'static str` that the plugin defines, so a module, a statement and a \
                 batch do not cross JSON",
    },
];

#[test]
fn every_known_exception_names_its_module_once_and_gives_its_reason() {
    let mut modules = BTreeSet::new();
    for exception in KNOWN_EXCEPTIONS {
        assert!(!exception.module.trim().is_empty(), "{exception:?}");
        assert!(!exception.reason.trim().is_empty(), "{exception:?}");
        assert!(
            modules.insert(exception.module),
            "listed twice: {exception:?}"
        );
    }
}

#[test]
fn a_plugin_id_crosses_json_as_its_text() {
    let id = PluginId::parse("log-viewer").unwrap();

    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, r#""log-viewer""#);

    let decoded: PluginId = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, id);
}

#[test]
fn an_image_format_crosses_json_as_its_name() {
    for (format, json) in [
        (ImageFormat::Png, r#""png""#),
        (ImageFormat::Jpeg, r#""jpeg""#),
    ] {
        assert_eq!(serde_json::to_string(&format).unwrap(), json);
        assert_eq!(serde_json::from_str::<ImageFormat>(json).unwrap(), format);
    }
    let error = serde_json::from_str::<ImageFormat>(r#""gif""#).unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn a_read_verdict_crosses_json_as_its_name() {
    for (verdict, json) in [
        (ReadVerdict::Allowed, r#""allowed""#),
        (ReadVerdict::Ask, r#""ask""#),
        (ReadVerdict::Refused, r#""refused""#),
    ] {
        assert_eq!(serde_json::to_string(&verdict).unwrap(), json);
        assert_eq!(serde_json::from_str::<ReadVerdict>(json).unwrap(), verdict);
    }
    let error = serde_json::from_str::<ReadVerdict>(r#""maybe""#).unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn a_requester_crosses_json_as_its_name() {
    for (requester, json) in [
        (Requester::Owner, r#""owner""#),
        (Requester::Agent, r#""agent""#),
    ] {
        assert_eq!(serde_json::to_string(&requester).unwrap(), json);
        assert_eq!(serde_json::from_str::<Requester>(json).unwrap(), requester);
    }
    let error = serde_json::from_str::<Requester>(r#""machine""#).unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn a_section_context_crosses_json_unchanged() {
    let json = json!({
        "agent": "explore",
        "requester": "agent",
        "usesTools": true,
        "offersTasks": false,
        "fullAccess": true
    });

    let context = serde_json::from_value::<SectionContext>(json.clone()).unwrap();

    assert_eq!(context.agent, "explore");
    assert_eq!(context.requester, Requester::Agent);
    assert!(context.uses_tools);
    assert!(!context.offers_tasks);
    assert!(context.full_access);
    assert_eq!(serde_json::to_value(&context).unwrap(), json);
    let without_requester = json!({
        "agent": "explore",
        "usesTools": true,
        "offersTasks": false,
        "fullAccess": true
    });
    let error = serde_json::from_value::<SectionContext>(without_requester).unwrap_err();
    assert!(error.to_string().contains("requester"), "{error}");
    let mut with_tools = json;
    with_tools["offeredTools"] = json!(["read_file", "git_status"]);
    let offered = serde_json::from_value::<SectionContext>(with_tools.clone()).unwrap();
    assert!(offered.offers_tool("git_status"));
    assert!(!offered.offers_tool("git_push"));
    assert_eq!(serde_json::to_value(&offered).unwrap(), with_tools);
}

#[test]
fn a_provider_s_suggestions_and_rendered_mention_cross_json_unchanged() {
    let entry = MentionEntry {
        key: "docs/".to_owned(),
        label: "docs/".to_owned(),
        token: "@docs/".to_owned(),
        variant: "folder".to_owned(),
        warning: None,
        expands: true,
    };
    let suggestions = ProviderSuggestions::new(vec![entry], true);
    let rendered = RenderedMention::new(
        "The owner mentioned the folder docs.".to_owned(),
        "docs/".to_owned(),
        "folder".to_owned(),
        MentionSummary {
            code: "entries".to_owned(),
            values: [("shown".to_owned(), 4)].into(),
        },
    );

    let suggestions_json = serde_json::to_value(&suggestions).unwrap();
    let rendered_json = serde_json::to_value(&rendered).unwrap();

    assert_eq!(
        suggestions_json,
        json!({
            "entries": [{
                "key": "docs/",
                "label": "docs/",
                "token": "@docs/",
                "variant": "folder",
                "warning": null,
                "expands": true
            }],
            "incomplete": true
        })
    );
    assert_eq!(
        rendered_json,
        json!({
            "text": "The owner mentioned the folder docs.",
            "label": "docs/",
            "variant": "folder",
            "summary": { "code": "entries", "values": { "shown": 4 } }
        })
    );
    assert_eq!(
        serde_json::from_value::<ProviderSuggestions>(suggestions_json).unwrap(),
        suggestions
    );
    assert_eq!(
        serde_json::from_value::<RenderedMention>(rendered_json).unwrap(),
        rendered
    );
}

#[test]
fn a_context_request_and_its_items_cross_json_unchanged() {
    let request_json = json!({
        "conversationId": "0199e0f2-0000-7000-8000-000000000001",
        "keys": ["n1", "n2"],
        "message": "Why does light bend?",
        "maxBytes": 16384,
        "maxItems": 20
    });
    let item = ContextItem::new(
        "s1:4".to_owned(),
        "Optics.pdf".to_owned(),
        Some(ContextLocation::Headings {
            path: vec!["Waves".to_owned()],
        }),
        "Light bends.".to_owned(),
    );

    let request = serde_json::from_value::<ContextRequest>(request_json.clone()).unwrap();
    let item_json = serde_json::to_value(&item).unwrap();

    assert_eq!(request.keys, ["n1", "n2"]);
    assert_eq!((request.max_bytes, request.max_items), (16_384, 20));
    assert_eq!(serde_json::to_value(&request).unwrap(), request_json);
    assert_eq!(
        item_json,
        json!({
            "key": "s1:4",
            "title": "Optics.pdf",
            "location": { "kind": "headings", "path": ["Waves"] },
            "text": "Light bends."
        })
    );
    assert_eq!(
        serde_json::from_value::<ContextItem>(item_json).unwrap(),
        item
    );
    let error = serde_json::from_value::<ContextItem>(json!({
        "key": "s1:4",
        "title": "Optics.pdf",
        "location": { "kind": "chapter", "number": 2 },
        "text": "Light bends."
    }))
    .unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn json_with_an_id_that_breaks_the_rules_does_not_decode() {
    for (json, rule) in [
        (r#""Workers""#, "lower-case words"),
        (r#""runtime""#, "reserved"),
        (r#""tauri-plugin-x""#, "tauri-plugin-"),
        (r#""runtime-x""#, "runtime_*"),
    ] {
        let error = serde_json::from_str::<PluginId>(json).unwrap_err();
        assert!(error.to_string().contains(rule), "{json}: {error}");
    }
    for json in ["17", "null", r#"{"id":"workers"}"#, r#"["workers"]"#] {
        let error = serde_json::from_str::<PluginId>(json).unwrap_err();
        assert!(error.is_data(), "{json}: {error}");
    }
}

#[test]
fn a_tool_spec_crosses_json_with_its_schema() {
    let spec = ToolSpec::new(
        "notes_find",
        "Finds the owner's notes.",
        json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"],
            "additionalProperties": false
        }),
    );

    let json = serde_json::to_value(&spec).unwrap();

    assert_eq!(
        json,
        json!({
            "name": "notes_find",
            "description": "Finds the owner's notes.",
            "parameters": {
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"],
                "additionalProperties": false
            }
        })
    );
    assert_eq!(serde_json::from_value::<ToolSpec>(json).unwrap(), spec);
    let error =
        serde_json::from_value::<ToolSpec>(json!({ "name": "notes_find", "parameters": {} }))
            .unwrap_err();
    assert!(error.to_string().contains("description"), "{error}");
}

#[test]
fn an_agent_selector_crosses_json_as_its_name_or_its_agents() {
    for (selector, json) in [
        (AgentSelector::Conversations, json!("conversations")),
        (AgentSelector::ChildAgents, json!("child_agents")),
        (AgentSelector::Every, json!("every")),
        (
            AgentSelector::Agents(vec!["explore".to_owned(), "reviewer".to_owned()]),
            json!({ "agents": ["explore", "reviewer"] }),
        ),
    ] {
        assert_eq!(serde_json::to_value(&selector).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<AgentSelector>(json).unwrap(),
            selector
        );
    }
    for json in [json!("owners"), json!({ "agents": "explore" }), json!(null)] {
        let error = serde_json::from_value::<AgentSelector>(json.clone()).unwrap_err();
        assert!(error.is_data(), "{json}: {error}");
    }
}

#[test]
fn a_tool_call_and_its_execution_cross_json_with_their_conversation() {
    let json = json!({
        "arguments": "{\"query\":\"groceries\"}",
        "conversationId": "0199e0f2-0000-7000-8000-000000000001"
    });

    let call = serde_json::from_value::<ToolCall>(json.clone()).unwrap();
    let execution = serde_json::from_value::<ToolExecution>(json.clone()).unwrap();

    assert_eq!(call.arguments, r#"{"query":"groceries"}"#);
    assert_eq!(
        call.conversation_id.as_str(),
        "0199e0f2-0000-7000-8000-000000000001"
    );
    assert_eq!(serde_json::to_value(&call).unwrap(), json);
    assert_eq!(execution.arguments, call.arguments);
    assert_eq!(execution.conversation_id, call.conversation_id);
    // An execution without the fields of ADR-0036 authorized nothing,
    // reserved no archive and has the largest result limit.
    assert_eq!(execution.authorized, Vec::new());
    assert_eq!(execution.archive_ids, Vec::new());
    assert_eq!(execution.result_limit, 64 * 1024);
    let mut with_limit = json;
    with_limit["resultLimit"] = json!(64 * 1024);
    assert_eq!(serde_json::to_value(&execution).unwrap(), with_limit);
    let error = serde_json::from_value::<ToolCall>(json!({
        "arguments": "{}",
        "conversationId": "conversation one"
    }))
    .unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn a_tool_s_refusal_and_result_cross_json_unchanged() {
    let refusal = ToolRefusal::new("the query is empty");
    let result = ToolResult::new("2 notes match.");

    let refusal_json = serde_json::to_value(&refusal).unwrap();
    let result_json = serde_json::to_value(&result).unwrap();

    assert_eq!(refusal_json, json!({ "detail": "the query is empty" }));
    assert_eq!(result_json, json!({ "output": "2 notes match." }));
    assert_eq!(
        serde_json::from_value::<ToolRefusal>(refusal_json).unwrap(),
        refusal
    );
    assert_eq!(
        serde_json::from_value::<ToolResult>(result_json).unwrap(),
        result
    );
}

#[test]
fn a_remote_command_crosses_json_with_its_kind_and_nothing_else_decodes_to_an_access() {
    let access = ToolAccess::RemoteCommand(RemoteCommandAccess::new(
        "0199e0f2-0000-7000-8000-0000000000aa".to_owned(),
        "build-box".to_owned(),
        "deploy@build-box.lan:22".to_owned(),
        "SHA256:vh3fjoyg7pBnxLhORIqZjyJQqYYN/gzBTuj+mPdeJlo".to_owned(),
        "uname -a".to_owned(),
    ));
    let json = json!({
        "kind": "remote_command",
        "target": "0199e0f2-0000-7000-8000-0000000000aa",
        "label": "build-box",
        "endpoint": "deploy@build-box.lan:22",
        "identity": "SHA256:vh3fjoyg7pBnxLhORIqZjyJQqYYN/gzBTuj+mPdeJlo",
        "command": "uname -a"
    });

    assert_eq!(serde_json::to_value(&access).unwrap(), json);
    assert_eq!(serde_json::from_value::<ToolAccess>(json).unwrap(), access);
    for json in [
        json!("remote"),
        json!({ "remote": { "origin": "https://example.com" } }),
        json!({ "kind": "path", "path": "/etc/hosts" }),
        json!({ "kind": "remote_command", "target": "a" }),
        json!(null),
    ] {
        let error = serde_json::from_value::<ToolAccess>(json.clone()).unwrap_err();
        assert!(error.is_data(), "{json}: {error}");
    }
}

#[test]
fn a_result_carries_its_images_as_base64_and_one_without_leaves_them_out() {
    let result = ToolResult::new("A screenshot of the page.")
        .with_images(vec![ToolImage::new(b"\x89PNG\r\n\x1a\nimage".to_vec())]);
    let json = json!({
        "output": "A screenshot of the page.",
        "images": [{ "data": "iVBORw0KGgppbWFnZQ==" }]
    });

    assert_eq!(serde_json::to_value(&result).unwrap(), json);
    assert_eq!(serde_json::from_value::<ToolResult>(json).unwrap(), result);
    assert_eq!(
        serde_json::to_value(ToolResult::new("Done.")).unwrap(),
        json!({ "output": "Done." })
    );
    let error = serde_json::from_value::<ToolResult>(json!({
        "output": "Done.",
        "images": [{ "data": "not base64!" }]
    }))
    .unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn a_tool_servers_call_crosses_json_with_its_endpoint_and_leaves_out_what_it_lacks() {
    let plain = ToolAccess::ExternalTool(ExternalToolAccess::new(
        "0199e0f2-0000-7000-8000-0000000000cc".to_owned(),
        "Issues".to_owned(),
        ToolServerEndpoint::Url {
            url: "https://mcp.example.com/mcp".to_owned(),
        },
        "create_issue".to_owned(),
        r#"{"title":"Crash"}"#.to_owned(),
    ));
    let described = ToolAccess::ExternalTool(
        ExternalToolAccess::new(
            "0199e0f2-0000-7000-8000-0000000000dd".to_owned(),
            "Everything".to_owned(),
            ToolServerEndpoint::Program {
                program: "npx".to_owned(),
                arguments: vec![
                    "-y".to_owned(),
                    "@modelcontextprotocol/server-everything".to_owned(),
                ],
            },
            "echo".to_owned(),
            "{}".to_owned(),
        )
        .with_title("Echo")
        .with_hints(vec![ToolHint::ReadOnly, ToolHint::OpenWorld]),
    );
    let plain_json = json!({
        "kind": "external_tool",
        "target": "0199e0f2-0000-7000-8000-0000000000cc",
        "label": "Issues",
        "endpoint": { "kind": "url", "url": "https://mcp.example.com/mcp" },
        "tool": "create_issue",
        "arguments": "{\"title\":\"Crash\"}"
    });
    let described_json = json!({
        "kind": "external_tool",
        "target": "0199e0f2-0000-7000-8000-0000000000dd",
        "label": "Everything",
        "endpoint": {
            "kind": "program",
            "program": "npx",
            "arguments": ["-y", "@modelcontextprotocol/server-everything"]
        },
        "tool": "echo",
        "title": "Echo",
        "arguments": "{}",
        "hints": ["read_only", "open_world"]
    });

    assert_eq!(serde_json::to_value(&plain).unwrap(), plain_json);
    assert_eq!(serde_json::to_value(&described).unwrap(), described_json);
    assert_eq!(
        serde_json::from_value::<ToolAccess>(plain_json).unwrap(),
        plain
    );
    assert_eq!(
        serde_json::from_value::<ToolAccess>(described_json).unwrap(),
        described
    );
    for json in [
        json!({ "kind": "external_tool", "target": "a", "label": "b", "tool": "c", "arguments": "{}" }),
        json!({
            "kind": "external_tool",
            "target": "a",
            "label": "b",
            "endpoint": { "kind": "socket", "path": "/run/server.sock" },
            "tool": "c",
            "arguments": "{}"
        }),
        json!({
            "kind": "external_tool",
            "target": "a",
            "label": "b",
            "endpoint": { "kind": "url", "url": "https://mcp.example.com/mcp" },
            "tool": "c",
            "arguments": "{}",
            "hints": ["harmless"]
        }),
    ] {
        let error = serde_json::from_value::<ToolAccess>(json.clone()).unwrap_err();
        assert!(error.is_data(), "{json}: {error}");
    }
}

#[test]
fn a_result_carries_its_archives_and_an_execution_its_authorized_accesses() {
    let archive_id = ArchiveId::parse("0199e0f2-0000-7000-8000-0000000000cc").unwrap();
    let result = ToolResult::new("Exit code: 0").with_archives(vec![ToolArchive::new(
        archive_id.clone(),
        "stdout",
        "hello\n".to_owned(),
    )]);

    let json = serde_json::to_value(&result).unwrap();

    assert_eq!(
        json,
        json!({
            "output": "Exit code: 0",
            "archives": [{
                "id": "0199e0f2-0000-7000-8000-0000000000cc",
                "name": "stdout",
                "content": "hello\n"
            }]
        })
    );
    assert_eq!(serde_json::from_value::<ToolResult>(json).unwrap(), result);
    let execution = serde_json::from_value::<ToolExecution>(json!({
        "arguments": "{}",
        "conversationId": "0199e0f2-0000-7000-8000-000000000001",
        "authorized": [{
            "kind": "remote_command",
            "target": "t",
            "label": "build-box",
            "endpoint": "deploy@build-box.lan:22",
            "identity": "SHA256:x",
            "command": "uptime"
        }],
        "resultLimit": 4096,
        "archiveIds": ["0199e0f2-0000-7000-8000-0000000000cc"]
    }))
    .unwrap();
    assert_eq!(execution.authorized.len(), 1);
    assert_eq!(execution.result_limit, 4096);
    assert_eq!(execution.archive_ids, vec![archive_id]);
}

#[test]
fn a_path_crosses_json_with_its_mode_and_its_state() {
    let plain = ToolAccess::Path(PathAccess::new("/work/repository", AccessMode::Read));
    let pinned = ToolAccess::Path(
        PathAccess::new("/work/repository", AccessMode::Write).with_state("HEAD 1a2b"),
    );
    let plain_json = json!({ "kind": "path", "path": "/work/repository", "mode": "read" });
    let pinned_json = json!({
        "kind": "path",
        "path": "/work/repository",
        "mode": "write",
        "state": "HEAD 1a2b"
    });

    assert_eq!(serde_json::to_value(&plain).unwrap(), plain_json);
    assert_eq!(serde_json::to_value(&pinned).unwrap(), pinned_json);
    assert_eq!(
        serde_json::from_value::<ToolAccess>(plain_json).unwrap(),
        plain
    );
    assert_eq!(
        serde_json::from_value::<ToolAccess>(pinned_json).unwrap(),
        pinned
    );
    for json in [
        json!({ "kind": "path", "path": "/work", "mode": "rename" }),
        json!({ "kind": "path", "mode": "read" }),
    ] {
        let error = serde_json::from_value::<ToolAccess>(json.clone()).unwrap_err();
        assert!(error.is_data(), "{json}: {error}");
    }
}

#[test]
fn a_call_and_its_execution_carry_the_workspace_and_omit_none() {
    let json = json!({
        "arguments": "{}",
        "conversationId": "0199e0f2-0000-7000-8000-000000000001",
        "workspace": "/work/repository"
    });

    let call = serde_json::from_value::<ToolCall>(json.clone()).unwrap();
    let execution = serde_json::from_value::<ToolExecution>(json.clone()).unwrap();

    assert_eq!(
        call.workspace.as_deref(),
        Some(std::path::Path::new("/work/repository"))
    );
    assert_eq!(execution.workspace, call.workspace);
    assert_eq!(serde_json::to_value(&call).unwrap(), json);
    let general = serde_json::from_value::<ToolCall>(json!({
        "arguments": "{}",
        "conversationId": "0199e0f2-0000-7000-8000-000000000001"
    }))
    .unwrap();
    assert_eq!(general.workspace, None);
    assert!(
        serde_json::to_value(&general)
            .unwrap()
            .get("workspace")
            .is_none()
    );
}

#[test]
fn a_refusal_carries_its_code_and_a_result_its_card_data_and_failure() {
    let refusal =
        ToolRefusal::new("This folder is not a repository.").with_code("not_a_repository");
    let result = ToolResult::new("2 files conflict.")
        .with_data(json!({ "conflicted": ["a.txt", "b.txt"] }))
        .with_failure("merge_conflicts");

    let refusal_json = serde_json::to_value(&refusal).unwrap();
    let result_json = serde_json::to_value(&result).unwrap();

    assert_eq!(
        refusal_json,
        json!({ "detail": "This folder is not a repository.", "code": "not_a_repository" })
    );
    assert_eq!(
        result_json,
        json!({
            "output": "2 files conflict.",
            "data": { "conflicted": ["a.txt", "b.txt"] },
            "failure": "merge_conflicts"
        })
    );
    assert_eq!(
        serde_json::from_value::<ToolRefusal>(refusal_json).unwrap(),
        refusal
    );
    assert_eq!(
        serde_json::from_value::<ToolResult>(result_json).unwrap(),
        result
    );
}

#[test]
fn an_offering_crosses_json_with_its_marker() {
    for (offering, json) in [
        (ToolOffering::Always, json!("always")),
        (ToolOffering::InWorkspace, json!("in_workspace")),
        (
            ToolOffering::WhereWorkspaceHolds(".git".to_owned()),
            json!({ "where_workspace_holds": ".git" }),
        ),
    ] {
        assert_eq!(serde_json::to_value(&offering).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<ToolOffering>(json).unwrap(),
            offering
        );
    }
}

/// A worker as a task's target.
fn worker_target() -> TaskExecutorRef {
    TaskExecutorRef {
        kind: "workers".to_owned(),
        target_id: "0199e0f2-0000-7000-8000-0000000000aa".to_owned(),
        target_name: "build-01".to_owned(),
        noun: "the worker".to_owned(),
    }
}

fn worker_target_json() -> serde_json::Value {
    json!({
        "kind": "workers",
        "targetId": "0199e0f2-0000-7000-8000-0000000000aa",
        "targetName": "build-01",
        "noun": "the worker"
    })
}

#[test]
fn a_task_plan_and_a_follow_up_plan_cross_json_with_their_target_and_options() {
    let title = "Build it".to_owned();
    let instruction = "Build the backend.".to_owned();
    let plan = TaskPlan::new(
        worker_target(),
        title,
        instruction,
        false,
        json!({ "readOnly": true }),
    );
    let follow_up = FollowUpPlan::new(worker_target(), json!({ "readOnly": false }));

    let plan_json = serde_json::to_value(&plan).unwrap();
    let follow_up_json = serde_json::to_value(&follow_up).unwrap();

    assert_eq!(
        plan_json,
        json!({
            "target": worker_target_json(),
            "title": "Build it",
            "instruction": "Build the backend.",
            "background": false,
            "options": { "readOnly": true }
        })
    );
    assert_eq!(
        follow_up_json,
        json!({ "target": worker_target_json(), "options": { "readOnly": false } })
    );
    assert_eq!(serde_json::from_value::<TaskPlan>(plan_json).unwrap(), plan);
    assert_eq!(
        serde_json::from_value::<FollowUpPlan>(follow_up_json).unwrap(),
        follow_up
    );
    let error = serde_json::from_value::<TaskPlan>(json!({
        "target": worker_target_json(),
        "title": "Build it",
        "instruction": "Build the backend.",
        "options": {}
    }))
    .unwrap_err();
    assert!(error.to_string().contains("background"), "{error}");
}

#[test]
fn an_executors_refusal_crosses_json_alone_and_inside_its_errors() {
    let refusal = ExecutorRefusal::new(
        "worker_offline",
        Some("build-01".to_owned()),
        "Not run: build-01 is offline.",
    );
    let refusal_json = json!({
        "code": "worker_offline",
        "target": "build-01",
        "text": "Not run: build-01 is offline."
    });

    assert_eq!(serde_json::to_value(&refusal).unwrap(), refusal_json);
    assert_eq!(
        serde_json::from_value::<ExecutorRefusal>(refusal_json.clone()).unwrap(),
        refusal
    );
    for (error, json) in [
        (
            PlanError::InvalidArguments {
                detail: "the title is empty".to_owned(),
            },
            json!({ "invalid_arguments": { "detail": "the title is empty" } }),
        ),
        (
            PlanError::Refused(refusal.clone()),
            json!({ "refused": refusal_json }),
        ),
    ] {
        assert_eq!(serde_json::to_value(&error).unwrap(), json);
        assert_eq!(serde_json::from_value::<PlanError>(json).unwrap(), error);
    }
    for (error, json) in [
        (
            SubmitError::Refused(refusal),
            json!({ "refused": refusal_json }),
        ),
        (SubmitError::Stopped, json!("stopped")),
    ] {
        assert_eq!(serde_json::to_value(&error).unwrap(), json);
        assert_eq!(serde_json::from_value::<SubmitError>(json).unwrap(), error);
    }
    let without_target = ExecutorRefusal::new("plugin_off", None, "Not run.");
    assert_eq!(
        serde_json::to_value(&without_target).unwrap(),
        json!({ "code": "plugin_off", "target": null, "text": "Not run." })
    );
    for json in [json!("refused"), json!({ "stopped": {} }), json!("unknown")] {
        let error = serde_json::from_value::<SubmitError>(json.clone()).unwrap_err();
        assert!(error.is_data(), "{json}: {error}");
    }
}

#[test]
fn a_task_submission_crosses_json_with_its_target_and_profile() {
    let json = json!({
        "taskId": "0199e0f2-0000-7000-8000-000000000001",
        "conversationId": "0199e0f2-0000-7000-8000-000000000002",
        "parentId": "0199e0f2-0000-7000-8000-000000000003",
        "number": 2,
        "followUp": true,
        "title": "Build it",
        "instruction": "Build the backend.",
        "target": worker_target_json(),
        "options": { "readOnly": false },
        "profile": {
            "id": "0199e0f2-0000-7000-8000-0000000000bb",
            "configRevision": 3,
            "interface": "chat_completions",
            "baseUrl": "https://models.example.com/v1",
            "model": "model-1",
            "contextWindow": 128_000,
            "outputLimit": 8_000,
            "reasoningEffort": "server_default",
            "server": null,
            "useTools": true,
            "hasCredential": true
        }
    });

    let submission = serde_json::from_value::<TaskSubmission>(json.clone()).unwrap();

    assert_eq!(
        submission.task_id.as_str(),
        "0199e0f2-0000-7000-8000-000000000001"
    );
    assert_eq!(
        submission.parent_id.as_str(),
        "0199e0f2-0000-7000-8000-000000000003"
    );
    assert_eq!(submission.number, 2);
    assert!(submission.follow_up);
    assert_eq!(submission.title, "Build it");
    assert_eq!(submission.target, worker_target());
    assert_eq!(
        submission.profile,
        serde_json::from_value::<ProfileSnapshot>(json["profile"].clone()).unwrap()
    );
    assert_eq!(serde_json::to_value(&submission).unwrap(), json);
    let mut without_profile = json;
    without_profile
        .as_object_mut()
        .unwrap()
        .remove("profile")
        .unwrap();
    let error = serde_json::from_value::<TaskSubmission>(without_profile).unwrap_err();
    assert!(error.to_string().contains("profile"), "{error}");
}

#[test]
fn an_executor_section_crosses_json_unchanged() {
    let section = ExecutorSection::new(310, "Workers you can give tasks to: build-01.");

    let json = serde_json::to_value(&section).unwrap();

    assert_eq!(
        json,
        json!({ "position": 310, "text": "Workers you can give tasks to: build-01." })
    );
    assert_eq!(
        serde_json::from_value::<ExecutorSection>(json).unwrap(),
        section
    );
    let error =
        serde_json::from_value::<ExecutorSection>(json!({ "position": -1, "text": "Workers." }))
            .unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn a_host_profile_crosses_json_as_its_name() {
    for (profile, json) in [
        (HostProfile::Release, r#""release""#),
        (HostProfile::Development, r#""development""#),
        (HostProfile::Validation, r#""validation""#),
    ] {
        assert_eq!(serde_json::to_string(&profile).unwrap(), json);
        assert_eq!(serde_json::from_str::<HostProfile>(json).unwrap(), profile);
    }
    let error = serde_json::from_str::<HostProfile>(r#""debug""#).unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn the_hosts_facts_cross_json_unchanged() {
    let json = json!({ "version": "0.3.0", "sdkMajor": 1, "profile": "validation" });

    let host = serde_json::from_value::<HostInfo>(json.clone()).unwrap();

    assert_eq!(host, HostInfo::new("0.3.0", HostProfile::Validation));
    assert_eq!(serde_json::to_value(&host).unwrap(), json);
    let without_profile = json!({ "version": "0.3.0", "sdkMajor": 1 });
    let error = serde_json::from_value::<HostInfo>(without_profile).unwrap_err();
    assert!(error.to_string().contains("profile"), "{error}");
}

#[test]
fn a_work_item_crosses_json_with_its_code_and_count() {
    let json = json!({ "code": "active_tasks", "count": 2 });

    let item = serde_json::from_value::<WorkItem>(json.clone()).unwrap();

    assert_eq!(item, WorkItem::new("active_tasks", 2));
    assert_eq!(serde_json::to_value(&item).unwrap(), json);
}

#[test]
fn a_start_failure_crosses_json_as_its_code() {
    let failure = StartFailure::new("storage_failed");

    let json = serde_json::to_string(&failure).unwrap();

    assert_eq!(json, r#""storage_failed""#);
    assert_eq!(
        serde_json::from_str::<StartFailure>(&json).unwrap(),
        failure
    );
    let error = serde_json::from_str::<StartFailure>("17").unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn an_executor_task_crosses_json_with_its_target_and_document() {
    let json = json!({
        "id": "0199e0f2-0000-7000-8000-000000000001",
        "conversationId": "0199e0f2-0000-7000-8000-000000000002",
        "state": "running",
        "target": "0199e0f2-0000-7000-8000-0000000000aa",
        "document": "{\"version\":2}"
    });

    let task = serde_json::from_value::<ExecutorTask>(json.clone()).unwrap();

    assert_eq!(task.id.as_str(), "0199e0f2-0000-7000-8000-000000000001");
    assert_eq!(
        task.conversation_id.as_str(),
        "0199e0f2-0000-7000-8000-000000000002"
    );
    assert_eq!(task.state, TaskState::Running);
    assert_eq!(task.target, "0199e0f2-0000-7000-8000-0000000000aa");
    assert_eq!(task.document.as_deref(), Some(r#"{"version":2}"#));
    assert_eq!(serde_json::to_value(&task).unwrap(), json);
    let without_document = ExecutorTask::new(
        task.id.clone(),
        task.conversation_id.clone(),
        task.state,
        task.target,
        None,
    );
    let mut json_without_document = json;
    json_without_document["document"] = json!(null);
    assert_eq!(
        serde_json::to_value(&without_document).unwrap(),
        json_without_document
    );
    let error = serde_json::from_value::<ExecutorTask>(json!({
        "id": "0199e0f2-0000-7000-8000-000000000001",
        "conversationId": "0199e0f2-0000-7000-8000-000000000002",
        "state": "paused",
        "target": "build-01",
        "document": null
    }))
    .unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn an_outbox_item_crosses_json_with_its_target_and_conversation() {
    let json = json!({
        "target": "0199e0f2-0000-7000-8000-0000000000aa",
        "conversationId": "0199e0f2-0000-7000-8000-000000000002"
    });

    let item = serde_json::from_value::<OutboxItem>(json.clone()).unwrap();

    assert_eq!(
        item,
        OutboxItem::new(
            "0199e0f2-0000-7000-8000-0000000000aa".to_owned(),
            item.conversation_id.clone()
        )
    );
    assert_eq!(serde_json::to_value(&item).unwrap(), json);
    let error = serde_json::from_value::<OutboxItem>(json!({ "target": "build-01" })).unwrap_err();
    assert!(error.to_string().contains("conversationId"), "{error}");
}

#[test]
fn a_host_failure_crosses_json_as_its_code() {
    let failure = HostFailure::new("storage_failed");

    let json = serde_json::to_value(&failure).unwrap();

    assert_eq!(json, json!({ "code": "storage_failed" }));
    assert_eq!(
        serde_json::from_value::<HostFailure>(json).unwrap(),
        failure
    );
    let error = serde_json::from_value::<HostFailure>(json!("storage_failed")).unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn a_lend_error_crosses_json_as_its_name() {
    for (error, json) in [
        (LendError::ProfileMissing, r#""profile_missing""#),
        (
            LendError::CredentialUnavailable,
            r#""credential_unavailable""#,
        ),
        (LendError::StorageFailed, r#""storage_failed""#),
        (LendError::NotPermitted, r#""not_permitted""#),
    ] {
        assert_eq!(serde_json::to_string(&error).unwrap(), json);
        assert_eq!(serde_json::from_str::<LendError>(json).unwrap(), error);
    }
    let error = serde_json::from_str::<LendError>(r#""expired""#).unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn a_secrets_error_crosses_json_as_its_name() {
    for (error, json) in [
        (SecretsError::Unavailable, r#""unavailable""#),
        (SecretsError::NotPermitted, r#""not_permitted""#),
    ] {
        assert_eq!(serde_json::to_string(&error).unwrap(), json);
        assert_eq!(serde_json::from_str::<SecretsError>(json).unwrap(), error);
    }
    let error = serde_json::from_str::<SecretsError>(r#""locked""#).unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn a_search_model_and_the_kinds_of_a_call_cross_json_unchanged() {
    let json = json!({
        "id": "0199e0f2-0000-7000-8000-000000000001",
        "name": "GPU box",
        "model": "an-embedding-model",
        "kind": "embeddings",
        "vectorKey": "3f2a",
    });

    let model = serde_json::from_value::<SearchModel>(json.clone()).unwrap();

    assert_eq!(
        model,
        SearchModel::new(
            "0199e0f2-0000-7000-8000-000000000001",
            "GPU box",
            "an-embedding-model",
            SearchModelKind::Embeddings,
            "3f2a"
        )
    );
    assert_eq!(serde_json::to_value(&model).unwrap(), json);
    assert_eq!(
        serde_json::to_value(SearchModelKind::Rerank).unwrap(),
        json!("rerank")
    );
    assert_eq!(
        serde_json::to_value(InputKind::Query).unwrap(),
        json!("query")
    );
    assert_eq!(
        serde_json::to_value(InputKind::Document).unwrap(),
        json!("document")
    );
}

#[test]
fn a_models_error_crosses_json_as_its_code_and_fields() {
    for (error, json) in [
        (
            ModelsError::ProfileNotFound,
            json!({ "code": "profile_not_found" }),
        ),
        (ModelsError::Unreachable, json!({ "code": "unreachable" })),
        (
            ModelsError::Refused {
                failure: "model_not_found".to_owned(),
            },
            json!({ "code": "refused", "failure": "model_not_found" }),
        ),
        (
            ModelsError::InputTooLong,
            json!({ "code": "input_too_long" }),
        ),
    ] {
        assert_eq!(serde_json::to_value(&error).unwrap(), json);
        assert_eq!(serde_json::from_value::<ModelsError>(json).unwrap(), error);
    }
    let error = serde_json::from_value::<ModelsError>(json!({ "code": "teleported" })).unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn a_file_filter_crosses_json_with_its_label_and_extensions() {
    let json = json!({ "label": "Images", "extensions": ["png", "jpg"] });

    let filter = serde_json::from_value::<FileFilter>(json.clone()).unwrap();

    assert_eq!(filter, FileFilter::new("Images", ["png", "jpg"]));
    assert_eq!(serde_json::to_value(&filter).unwrap(), json);
}

#[test]
fn a_dialog_error_crosses_json_as_its_code_and_fields() {
    for (error, json) in [
        (DialogError::Failed, json!({ "code": "failed" })),
        (
            DialogError::InvalidFilter,
            json!({ "code": "invalid_filter" }),
        ),
        (DialogError::UnknownFile, json!({ "code": "unknown_file" })),
        (
            DialogError::TooLarge {
                max_bytes: 20_971_520,
            },
            json!({ "code": "too_large", "maxBytes": 20_971_520 }),
        ),
        (DialogError::Unreadable, json!({ "code": "unreadable" })),
    ] {
        assert_eq!(serde_json::to_value(&error).unwrap(), json);
        assert_eq!(serde_json::from_value::<DialogError>(json).unwrap(), error);
    }
    let error = serde_json::from_value::<DialogError>(json!({ "code": "locked" })).unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn a_host_refusal_crosses_json_with_its_code_and_arguments() {
    for (refusal, json) in [
        (
            HostRefusal::plugin_off("images"),
            json!({ "code": "plugin_off", "plugin": "images" }),
        ),
        (
            HostRefusal::InvalidInput {
                field: Some("filterName".to_owned()),
            },
            json!({ "code": "invalid_input", "field": "filterName" }),
        ),
        (
            HostRefusal::InputTooLarge { max_bytes: 65_536 },
            json!({ "code": "input_too_large", "maxBytes": 65_536 }),
        ),
        (HostRefusal::NotAllowed, json!({ "code": "not_allowed" })),
    ] {
        assert_eq!(serde_json::to_value(&refusal).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<HostRefusal>(json).unwrap(),
            refusal
        );
    }
    let error = serde_json::from_value::<HostRefusal>(json!({ "code": "unplugged" })).unwrap_err();
    assert!(error.is_data(), "{error}");
}

#[test]
fn a_channel_message_crosses_json_as_an_item_or_the_host_s_end() {
    let item = ChannelMessage::Item {
        item: json!([1, 2]),
    };
    let end = ChannelMessage::<serde_json::Value>::Ended {
        reason: SubscriptionEnd::PluginOff,
    };

    assert_eq!(
        serde_json::to_value(&item).unwrap(),
        json!({ "kind": "item", "item": [1, 2] })
    );
    assert_eq!(
        serde_json::to_value(&end).unwrap(),
        json!({ "kind": "ended", "reason": "plugin_off" })
    );
    assert_eq!(
        serde_json::from_value::<ChannelMessage<serde_json::Value>>(
            json!({ "kind": "ended", "reason": "shutdown" })
        )
        .unwrap(),
        ChannelMessage::Ended {
            reason: SubscriptionEnd::Shutdown
        }
    );
}

#[test]
fn a_plugin_failure_crosses_json_with_its_origin_beside_the_code() {
    let host = CommandError {
        error: PluginFailure::<serde_json::Value>::Host(HostRefusal::ShuttingDown),
        description: "Mannie is quitting".to_owned(),
    };
    let plugin = CommandError {
        error: PluginFailure::<serde_json::Value>::Plugin(
            json!({ "code": "too_large", "maxBytes": 10 }),
        ),
        description: "the file is larger than 10 bytes".to_owned(),
    };

    assert_eq!(
        serde_json::to_value(&host).unwrap(),
        json!({ "origin": "host", "code": "shutting_down", "description": "Mannie is quitting" })
    );
    assert_eq!(
        serde_json::to_value(&plugin).unwrap(),
        json!({
            "origin": "plugin",
            "code": "too_large",
            "maxBytes": 10,
            "description": "the file is larger than 10 bytes",
        })
    );
    assert_eq!(
        serde_json::from_value::<PluginFailure<serde_json::Value>>(
            json!({ "origin": "host", "code": "plugin_off", "plugin": "workers" })
        )
        .unwrap(),
        PluginFailure::Host(HostRefusal::plugin_off("workers"))
    );
}

#[test]
fn an_input_refusal_crosses_json_with_its_field() {
    let json = json!({ "field": "name" });

    let refusal = serde_json::from_value::<InputRefusal>(json.clone()).unwrap();

    assert_eq!(refusal, InputRefusal::new("name"));
    assert_eq!(serde_json::to_value(&refusal).unwrap(), json);
}

#[test]
fn a_remote_command_in_the_background_crosses_json_with_its_mode() {
    let access = ToolAccess::RemoteCommand(
        RemoteCommandAccess::new(
            "t".to_owned(),
            "build-box".to_owned(),
            "deploy@build-box.lan:22".to_owned(),
            "SHA256:x".to_owned(),
            "journalctl -f".to_owned(),
        )
        .in_background(),
    );
    let json = json!({
        "kind": "remote_command",
        "target": "t",
        "label": "build-box",
        "endpoint": "deploy@build-box.lan:22",
        "identity": "SHA256:x",
        "command": "journalctl -f",
        "background": true
    });

    assert_eq!(serde_json::to_value(&access).unwrap(), json);
    assert_eq!(serde_json::from_value::<ToolAccess>(json).unwrap(), access);
}

#[test]
fn an_execution_carries_its_ticket_and_omits_none() {
    let json = json!({
        "arguments": "{}",
        "conversationId": "0199e0f2-0000-7000-8000-000000000001",
        "resultLimit": 4096,
        "background": "0199e0f2-0000-7000-8000-0000000000dd"
    });

    let execution = serde_json::from_value::<ToolExecution>(json.clone()).unwrap();
    let mut plain = execution.clone();
    plain.background = None;
    let without = serde_json::to_value(&plain).unwrap();

    assert_eq!(
        execution.background,
        Some(BackgroundTicket::new(
            "0199e0f2-0000-7000-8000-0000000000dd".to_owned()
        ))
    );
    assert_eq!(serde_json::to_value(&execution).unwrap(), json);
    assert!(without.get("background").is_none(), "{without}");
}

#[test]
fn the_output_of_a_command_in_the_background_crosses_json_with_its_kind() {
    let cases = [
        (
            RemoteOutput::lines(vec!["started".to_owned()]),
            json!({ "kind": "lines", "lines": ["started"] }),
        ),
        (
            RemoteOutput::ended(RemoteEnding::Exited { status: 3 }),
            json!({ "kind": "ended", "ending": { "kind": "exited", "status": 3 } }),
        ),
        (
            RemoteOutput::ended(RemoteEnding::Signaled {
                signal: "TERM".to_owned(),
            }),
            json!({ "kind": "ended", "ending": { "kind": "signaled", "signal": "TERM" } }),
        ),
        (
            RemoteOutput::ended(RemoteEnding::Closed),
            json!({ "kind": "ended", "ending": { "kind": "closed" } }),
        ),
        (
            RemoteOutput::ended(RemoteEnding::SessionClosed),
            json!({ "kind": "ended", "ending": { "kind": "session_closed" } }),
        ),
    ];
    for (output, json) in cases {
        assert_eq!(serde_json::to_value(&output).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<RemoteOutput>(json).unwrap(),
            output
        );
    }
}

#[test]
fn a_start_in_the_background_and_its_errors_cross_json() {
    let cases = [
        (
            BackgroundError::TooMany {
                scope: BackgroundScope::Conversation,
                limit: 8,
            },
            json!({ "code": "too_many", "scope": "conversation", "limit": 8 }),
        ),
        (
            BackgroundError::TicketInvalid,
            json!({ "code": "ticket_invalid" }),
        ),
        (
            BackgroundError::ShuttingDown,
            json!({ "code": "shutting_down" }),
        ),
        (
            BackgroundError::StorageFailed,
            json!({ "code": "storage_failed" }),
        ),
    ];
    for (error, json) in cases {
        assert_eq!(serde_json::to_value(&error).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<BackgroundError>(json).unwrap(),
            error
        );
    }
    let started = StartedCommand::new(2);
    assert_eq!(
        serde_json::to_value(started).unwrap(),
        json!({ "number": 2 })
    );
    assert_eq!(
        serde_json::from_value::<StartedCommand>(json!({ "number": 2 })).unwrap(),
        started
    );
}
