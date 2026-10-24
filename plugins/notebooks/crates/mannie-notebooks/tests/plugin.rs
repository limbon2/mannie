//! The plugin as the host and the core see it (plugins/notebooks/docs/
//! README.md, acceptance): its manifest, its interface and tables, the SDK's
//! rules through the real host, and with the file dialog and the models in
//! memory: notebooks and their names, sources and their refusals, a source
//! that is read, split, embedded and searched, a server that is down, a new
//! model, the provider's passages in a notebook's chat, and a removed source
//! whose text leaves every file.

#![cfg(test)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::StreamExt as _;
use mannie_notebooks::store::MODULE;
use mannie_notebooks::{NOTEBOOKS, PLUGIN_MANIFEST, mannie_plugin};
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::{CallFailure, HostOptions, HostedPlugin};
use mannie_plugin_harness::provider::{Content, ModelRequest, Role};
use mannie_plugin_harness::runtime::{HarnessOptions, reply, texts, tool_calls};
use mannie_plugin_harness::types::{
    AttachmentRef, MentionRef, PartContent, SendMessage, StartConversation, ToolCallStatus,
};
use mannie_plugin_sdk::context::HostProfile;
use mannie_plugin_sdk::host::ServiceProvider;
use mannie_plugin_sdk::ipc::ChannelMessage;
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::services::models::InputKind;
use mannie_plugin_sdk::testing::{
    FAKE_EMBEDDING_PROFILE, FAKE_RERANK_PROFILE, FakeFile, FakeFileDialog, FakeModels,
};
use serde_json::{Value, json};

/// A Markdown file of three sections.
const NOTES: &str = "# Optics\n\nLight bends when it passes from air into water, because its speed changes.\n\n## Lenses\n\nA convex lens brings parallel rays to a focus.\n\n# Baking\n\nBread needs flour, water, salt and yeast.\n";

/// A host of a validation build, whose waits before a failed server is tried
/// again are short, with the file dialog and the models in memory.
fn options(dialog: &FakeFileDialog, models: &FakeModels) -> HostOptions {
    let dialog: Arc<dyn ServiceProvider> = Arc::new(dialog.clone());
    let models: Arc<dyn ServiceProvider> = Arc::new(models.clone());
    HostOptions::new()
        .with_profile(HostProfile::Validation)
        .with_provider(dialog)
        .with_provider(models)
}

/// The plugin on, with its fakes.
async fn setup() -> (HostedPlugin, FakeFileDialog, FakeModels) {
    let dialog = FakeFileDialog::new();
    let models = FakeModels::new();
    let hosted = HostedPlugin::new(mannie_plugin(), options(&dialog, &models)).await;
    let view = hosted.turn_on().await;
    assert_eq!(
        view.state,
        mannie_plugin_harness::types::PluginState::On,
        "{view:?}"
    );
    (hosted, dialog, models)
}

async fn call(hosted: &HostedPlugin, command: &str, input: Value) -> Value {
    hosted
        .invoke(command, input)
        .await
        .unwrap_or_else(|failure| panic!("{command} failed: {failure:?}"))
}

/// The plugin's error as its JSON.
fn plugin_error(outcome: Result<Value, CallFailure>) -> Value {
    match outcome {
        Err(CallFailure::Plugin(error)) => error.error,
        other => panic!("not an error of the plugin: {other:?}"),
    }
}

async fn create(hosted: &HostedPlugin, name: &str) -> String {
    let notebook = call(hosted, "create_notebook", json!({ "name": name })).await;
    notebook["id"].as_str().unwrap().to_owned()
}

async fn choose_models(hosted: &HostedPlugin, rerank: bool) {
    call(
        hosted,
        "save_preferences",
        json!({
            "embeddingProfileId": FAKE_EMBEDDING_PROFILE,
            "rerankProfileId": if rerank { json!(FAKE_RERANK_PROFILE) } else { Value::Null },
            "maxPassages": 6
        }),
    )
    .await;
}

/// The bytes of each part that the tests upload, small so that a short file
/// takes several parts.
const TEST_PART_BYTES: usize = 16;

/// Drops a file as the UI does: an upload, its parts in order, and its end.
async fn drop_file(
    hosted: &HostedPlugin,
    notebook: &str,
    name: &str,
    bytes: &[u8],
) -> Result<Value, CallFailure> {
    let started = hosted
        .invoke(
            "start_upload",
            json!({ "notebookId": notebook, "name": name }),
        )
        .await?;
    let upload = started["uploadId"].clone();
    for (index, part) in bytes.chunks(TEST_PART_BYTES).enumerate() {
        hosted
            .invoke(
                "add_upload_part",
                json!({ "uploadId": upload, "index": index, "data": BASE64.encode(part) }),
            )
            .await?;
    }
    hosted
        .invoke("finish_upload", json!({ "uploadId": upload }))
        .await
}

/// The source with this ID in a list of the channel.
fn source_in<'a>(list: &'a Value, id: &str) -> Option<&'a Value> {
    list["notebooks"]
        .as_array()?
        .iter()
        .flat_map(|notebook| notebook["sources"].as_array().into_iter().flatten())
        .find(|source| source["id"] == id)
}

/// Follows the notebooks until a list holds a source with this ID that
/// `matches`, and returns the source.
async fn until_source(
    hosted: &HostedPlugin,
    id: &str,
    what: &str,
    matches: impl Fn(&Value) -> bool,
) -> Value {
    let mut lists = hosted
        .open_channel("follow_notebooks", Value::Null)
        .await
        .unwrap();
    let wait = async {
        while let Some(message) = lists.next().await {
            let ChannelMessage::Item { item } = message else {
                continue;
            };
            if let Some(source) = source_in(&item, id)
                && matches(source)
            {
                return source.clone();
            }
        }
        panic!("the channel ended before the source was {what}");
    };
    tokio::time::timeout(Duration::from_secs(30), wait)
        .await
        .unwrap_or_else(|_elapsed| panic!("the source never was {what}"))
}

/// Follows the notebooks until a list holds a source with this ID in a
/// state, and returns the source.
async fn until_state(hosted: &HostedPlugin, id: &str, state: &str) -> Value {
    until_source(hosted, id, state, |source| source["state"] == state).await
}

/// Whether a file of the folder holds the bytes.
fn files_contain(folder: &Path, needle: &[u8]) -> bool {
    std::fs::read_dir(folder).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.is_file()
            && std::fs::read(&path)
                .unwrap()
                .windows(needle.len())
                .any(|window| window == needle)
    })
}

/// The texts of the last owner message of a request.
fn last_owner_message(request: &ModelRequest) -> Vec<String> {
    request
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::User)
        .map(|message| {
            message
                .content
                .iter()
                .filter_map(|content| match content {
                    Content::Text(text) => Some(text.clone()),
                    _other => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_manifest_declares_the_services_and_the_parser_and_the_module_names_the_tables() {
    let manifest = Manifest::parse(PLUGIN_MANIFEST).unwrap();

    assert_eq!(manifest.id.as_str(), NOTEBOOKS);
    for service in ["store", "file-dialog", "models"] {
        assert!(manifest.declares(service), "{service}");
    }
    assert_eq!(manifest.crates.exclusive, ["pdf-extract", "lopdf"]);
    assert_eq!(MODULE.name, manifest.id.prefix());
    for migration in MODULE.migrations {
        assert!(migration.sql().contains("notebooks_sources"));
    }
}

#[tokio::test]
async fn the_plugin_keeps_the_rules_of_the_sdk() {
    let dialog = FakeFileDialog::new();
    let models = FakeModels::new();

    let problems = conformance(mannie_plugin(), options(&dialog, &models)).await;

    assert_eq!(problems, Vec::<String>::new());
}

#[tokio::test]
async fn notebooks_have_names_of_their_own_and_go_with_their_sources() {
    let (hosted, _dialog, models) = setup().await;
    choose_models(&hosted, false).await;
    let physics = create(&hosted, "Physics").await;

    let taken = plugin_error(
        hosted
            .invoke("create_notebook", json!({ "name": " physics " }))
            .await,
    );
    let renamed = call(
        &hosted,
        "rename_notebook",
        json!({ "notebookId": physics, "name": "Physics notes" }),
    )
    .await;
    let source = drop_file(&hosted, &physics, "notes.md", NOTES.as_bytes())
        .await
        .unwrap();
    until_state(&hosted, source["id"].as_str().unwrap(), "ready").await;
    call(&hosted, "delete_notebook", json!({ "notebookId": physics })).await;
    let gone = plugin_error(
        hosted
            .invoke("delete_notebook", json!({ "notebookId": physics }))
            .await,
    );
    let list = {
        let mut lists = hosted
            .open_channel("follow_notebooks", Value::Null)
            .await
            .unwrap();
        match lists.next().await {
            Some(ChannelMessage::Item { item }) => item,
            other => panic!("{other:?}"),
        }
    };

    assert_eq!(taken, json!({ "code": "name_taken" }));
    assert_eq!(renamed["name"], "Physics notes");
    assert_eq!(gone, json!({ "code": "notebook_not_found" }));
    assert_eq!(list, json!({ "notebooks": [] }));
    assert_ne!(models.embedded().len(), 0);
}

#[tokio::test]
async fn a_dropped_file_is_read_split_embedded_with_its_place_and_becomes_ready() {
    let (hosted, _dialog, models) = setup().await;
    choose_models(&hosted, false).await;
    let notebook = create(&hosted, "Physics").await;

    let added = drop_file(&hosted, &notebook, "C:\\Docs\\notes.md", NOTES.as_bytes())
        .await
        .unwrap();
    let ready = until_state(&hosted, added["id"].as_str().unwrap(), "ready").await;
    let text = call(
        &hosted,
        "read_source",
        json!({ "sourceId": added["id"], "firstSection": 0 }),
    )
    .await;

    assert_eq!(added["name"], "notes.md");
    assert_eq!(added["kind"], "markdown");
    assert_eq!(added["state"], "reading");
    assert_eq!(
        (ready["passages"].clone(), ready["embedded"].clone()),
        (json!(3), json!(3))
    );
    assert_eq!(text["total"], 3);
    assert_eq!(
        text["sections"][1]["place"],
        json!({ "kind": "headings", "path": ["Optics", "Lenses"] })
    );
    let documents: Vec<String> = models
        .embedded()
        .into_iter()
        .filter(|(kind, _texts)| *kind == InputKind::Document)
        .flat_map(|(_kind, texts)| texts)
        .collect();
    assert!(
        documents.contains(
            &"notes.md, Optics > Lenses\n\nA convex lens brings parallel rays to a focus."
                .to_owned()
        ),
        "{documents:?}"
    );
}

#[tokio::test]
async fn an_upload_takes_its_parts_in_order_and_ends_when_it_is_cancelled() {
    let (hosted, _dialog, _models) = setup().await;
    let notebook = create(&hosted, "Physics").await;
    let start = |name: &'static str| {
        let hosted = &hosted;
        let notebook = notebook.clone();
        async move {
            call(
                hosted,
                "start_upload",
                json!({ "notebookId": notebook, "name": name }),
            )
            .await["uploadId"]
                .clone()
        }
    };
    let part = |upload: &Value, index: u32, data: &str| json!({ "uploadId": upload, "index": index, "data": data });

    let skipping = start("a.md").await;
    call(&hosted, "add_upload_part", part(&skipping, 0, "IyBB")).await;
    let skipped = plugin_error(
        hosted
            .invoke("add_upload_part", part(&skipping, 2, "IyBB"))
            .await,
    );
    let after_skip = plugin_error(
        hosted
            .invoke("finish_upload", json!({ "uploadId": skipping }))
            .await,
    );
    let cancelled = start("b.md").await;
    call(&hosted, "cancel_upload", json!({ "uploadId": cancelled })).await;
    let after_cancel = plugin_error(
        hosted
            .invoke("finish_upload", json!({ "uploadId": cancelled }))
            .await,
    );
    let garbled = start("c.md").await;
    let undecodable = plugin_error(
        hosted
            .invoke("add_upload_part", part(&garbled, 0, "not base64!"))
            .await,
    );
    let after_garble = plugin_error(
        hosted
            .invoke("finish_upload", json!({ "uploadId": garbled }))
            .await,
    );

    assert_eq!(
        skipped,
        json!({ "code": "upload_part_out_of_order", "expected": 1 })
    );
    assert_eq!(after_skip, json!({ "code": "upload_not_found" }));
    assert_eq!(after_cancel, json!({ "code": "upload_not_found" }));
    assert_eq!(
        undecodable,
        json!({ "code": "refused", "reason": "unreadable" })
    );
    assert_eq!(after_garble, json!({ "code": "upload_not_found" }));
}

#[tokio::test]
async fn files_are_refused_by_their_type_and_as_duplicates() {
    let (hosted, dialog, _models) = setup().await;
    let notebook = create(&hosted, "Physics").await;
    drop_file(&hosted, &notebook, "notes.md", NOTES.as_bytes())
        .await
        .unwrap();

    let duplicate = plugin_error(drop_file(&hosted, &notebook, "copy.md", NOTES.as_bytes()).await);
    let picture = plugin_error(drop_file(&hosted, &notebook, "photo.png", b"\x89PNG").await);
    let nowhere = plugin_error(drop_file(&hosted, "missing", "a.md", b"# A").await);
    dialog.answer(vec![
        FakeFile::new("other.txt", b"Something else entirely".to_vec()),
        FakeFile::new("notes again.md", NOTES.as_bytes().to_vec()),
        FakeFile::unreadable("gone.pdf"),
    ]);
    let picked = call(
        &hosted,
        "pick_sources",
        json!({ "notebookId": notebook, "filterName": "Documents" }),
    )
    .await;

    assert_eq!(
        duplicate,
        json!({ "code": "refused", "reason": "duplicate" })
    );
    assert_eq!(
        picture,
        json!({ "code": "refused", "reason": "unsupported_type" })
    );
    assert_eq!(nowhere, json!({ "code": "notebook_not_found" }));
    assert_eq!(picked["added"].as_array().unwrap().len(), 1);
    assert_eq!(picked["added"][0]["name"], "other.txt");
    assert_eq!(
        picked["refused"],
        json!([
            { "name": "notes again.md", "reason": "duplicate" },
            { "name": "gone.pdf", "reason": "unreadable" }
        ])
    );
    assert_eq!(
        dialog.filters()[0].extensions,
        ["pdf", "md", "markdown", "txt"]
    );
}

#[tokio::test]
async fn a_source_waits_for_a_model_and_a_failed_read_says_why() {
    let (hosted, _dialog, _models) = setup().await;
    let notebook = create(&hosted, "Physics").await;

    let waiting = drop_file(&hosted, &notebook, "notes.md", NOTES.as_bytes())
        .await
        .unwrap();
    until_state(&hosted, waiting["id"].as_str().unwrap(), "waiting").await;
    let not_text = drop_file(
        &hosted,
        &notebook,
        "broken.txt",
        &[0xff, 0xfe, 0x00, 0xd8, 0x41],
    )
    .await
    .unwrap();
    let failed = until_state(&hosted, not_text["id"].as_str().unwrap(), "failed").await;
    let retry = plugin_error(
        hosted
            .invoke("retry_source", json!({ "sourceId": not_text["id"] }))
            .await,
    );
    choose_models(&hosted, false).await;
    let ready = until_state(&hosted, waiting["id"].as_str().unwrap(), "ready").await;

    assert_eq!(failed["failure"], "not_utf8");
    assert_eq!(retry, json!({ "code": "nothing_to_retry" }));
    assert_eq!(ready["embedded"], 3);
}

#[tokio::test]
async fn a_server_that_is_down_pauses_the_source_which_fails_and_can_be_tried_again() {
    let (hosted, _dialog, models) = setup().await;
    choose_models(&hosted, false).await;
    let notebook = create(&hosted, "Physics").await;
    models.set_unreachable(true);

    let source = drop_file(&hosted, &notebook, "notes.md", NOTES.as_bytes())
        .await
        .unwrap();
    let id = source["id"].as_str().unwrap();
    let paused = until_source(&hosted, id, "paused", |source| {
        source["state"] == "indexing" && !source["failure"].is_null()
    })
    .await;
    let failed = until_state(&hosted, id, "failed").await;
    models.set_unreachable(false);
    call(&hosted, "retry_source", json!({ "sourceId": id })).await;
    let ready = until_state(&hosted, id, "ready").await;

    assert_eq!(paused["failure"], "server_unreachable");
    assert_eq!(failed["failure"], "server_unreachable");
    assert_eq!(failed["passages"], 3);
    assert_eq!(ready["embedded"], 3);
    assert_eq!(ready["failure"], Value::Null);
}

#[tokio::test]
async fn a_new_model_indexes_every_source_again() {
    let (hosted, _dialog, models) = setup().await;
    choose_models(&hosted, false).await;
    let notebook = create(&hosted, "Physics").await;
    let source = drop_file(&hosted, &notebook, "notes.md", NOTES.as_bytes())
        .await
        .unwrap();
    let id = source["id"].as_str().unwrap();
    until_state(&hosted, id, "ready").await;
    let embedded_before = models.embedded().len();

    models.set_vector_key(FAKE_EMBEDDING_PROFILE, "fake-vectors-2");
    tokio::time::timeout(Duration::from_secs(30), async {
        while models.embedded().len() == embedded_before {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the source is embedded again");
    let ready = until_state(&hosted, id, "ready").await;

    assert_eq!(ready["embedded"], 3);
}

#[tokio::test]
async fn a_notebook_s_chat_searches_with_each_message_and_the_one_before_it() {
    let (hosted, _dialog, models) = setup().await;
    choose_models(&hosted, true).await;
    let notebook = create(&hosted, "Physics").await;
    let source = drop_file(&hosted, &notebook, "notes.md", NOTES.as_bytes())
        .await
        .unwrap();
    let source_id = source["id"].as_str().unwrap().to_owned();
    until_state(&hosted, &source_id, "ready").await;
    let core = &hosted.core;
    core.model.push_reply(reply("Its speed changes."));
    core.model
        .push_reply(reply("A convex lens brings them to a focus."));

    let started = core
        .runtime
        .start_conversation(StartConversation::new("light water speed").with_home(Some(
            AttachmentRef {
                provider: NOTEBOOKS.to_owned(),
                key: notebook.clone(),
            },
        )))
        .await
        .unwrap();
    core.wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let follow_up = core
        .runtime
        .send_message(SendMessage::new(
            started.conversation_id.clone(),
            "and lenses?",
        ))
        .await
        .unwrap();
    let transcript = core
        .wait_for_run(&started.conversation_id, &follow_up.run_id)
        .await;
    let requests = core.model.reply_requests();
    let first = last_owner_message(&requests[0]);

    assert_eq!(first[0], "light water speed");
    assert!(
        first[1].starts_with(&format!(
            "{}\n\n[1] notes.md, Optics · cite: [notes.md › Optics](cite:notebooks/{source_id}:0)\n\
             Light bends when it passes from air into water",
            texts::CONTEXT_HEADING
        )),
        "{}",
        first[1]
    );
    assert!(
        requests[0].instructions.contains(&format!(
            "- Notebook \"Physics\" (notebook {notebook})\n  - \"notes.md\" (source {source_id}, Markdown)"
        )),
        "{}",
        requests[0].instructions
    );
    // The follow-up is searched alone and after the message before it, and
    // the reranker reads both; no passage is dropped for its score.
    let (kind, queries) = models.embedded().pop().unwrap();
    assert_eq!(kind, InputKind::Query);
    assert_eq!(queries, ["and lenses?", "light water speed\nand lenses?"]);
    let reranked = models.reranked();
    assert_eq!(reranked.len(), 2);
    assert_eq!(reranked[1].0, "light water speed\nand lenses?");
    let gathered: Vec<usize> = (0..2)
        .flat_map(|position| transcript.parts(position))
        .filter_map(|part| match part.content {
            PartContent::Context { context } => Some(context.items.len()),
            _other => None,
        })
        .collect();
    assert_eq!(gathered, [3, 3]);
}

/// The plugin on, with its fakes, in a runtime whose conversations use
/// tools.
async fn setup_with_tools() -> (HostedPlugin, FakeModels) {
    let dialog = FakeFileDialog::new();
    let models = FakeModels::new();
    let core = HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    };
    let hosted =
        HostedPlugin::new(mannie_plugin(), options(&dialog, &models).with_core(core)).await;
    hosted.turn_on().await;
    (hosted, models)
}

/// The names of the tools that a request offered.
fn offered(request: &ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

/// A notebook as an attachment of a conversation.
fn notebook_ref(notebook: &str) -> AttachmentRef {
    AttachmentRef {
        provider: NOTEBOOKS.to_owned(),
        key: notebook.to_owned(),
    }
}

/// The arguments of a read of a passage.
fn read_passage(source: &str, passage: u32) -> String {
    json!({
        "source": source,
        "passage": passage,
        "page": null,
        "page_end": null,
        "section": null,
        "offset": null,
        "limit": null,
    })
    .to_string()
}

const NOTEBOOK_TOOLS: [&str; 3] = [
    "list_notebook_sources",
    "search_notebooks",
    "read_notebook_source",
];

#[tokio::test]
async fn the_tools_list_search_and_read_only_the_attached_notebooks() {
    let (hosted, _models) = setup_with_tools().await;
    choose_models(&hosted, true).await;
    let physics = create(&hosted, "Physics").await;
    let secrets = create(&hosted, "Secrets").await;
    let notes = drop_file(&hosted, &physics, "notes.md", NOTES.as_bytes())
        .await
        .unwrap();
    let notes_id = notes["id"].as_str().unwrap().to_owned();
    let plan = call(
        &hosted,
        "add_text_source",
        json!({ "notebookId": secrets, "title": "Plan", "text": "The lens plan is secret." }),
    )
    .await;
    let plan_id = plan["id"].as_str().unwrap().to_owned();
    until_state(&hosted, &notes_id, "ready").await;
    until_state(&hosted, &plan_id, "ready").await;
    let core = &hosted.core;
    core.model.push_reply(tool_calls(&[
        (
            "call_1",
            "search_notebooks",
            r#"{"query":"convex lens","notebooks":null,"sources":null,"limit":1}"#,
        ),
        (
            "call_2",
            "read_notebook_source",
            &read_passage(&notes_id, 1),
        ),
        ("call_3", "list_notebook_sources", r#"{"notebook":null}"#),
        ("call_4", "read_notebook_source", &read_passage(&plan_id, 0)),
        (
            "call_5",
            "search_notebooks",
            &json!({ "query": "lens", "notebooks": [secrets], "sources": null, "limit": null })
                .to_string(),
        ),
    ]));
    core.model.push_reply(reply("A convex lens focuses them."));

    let started = core
        .runtime
        .start_conversation(
            StartConversation::new("OWNER: what do lenses do?")
                .with_home(Some(notebook_ref(&physics))),
        )
        .await
        .unwrap();
    let transcript = core
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let calls = transcript.tool_calls(0);
    let output = |index: usize| -> String {
        calls[index]
            .result
            .as_ref()
            .map(|result| result.output.clone())
            .unwrap_or_default()
    };
    assert!(
        output(0).contains(&format!(
            "[1] notes.md, Optics > Lenses · source {notes_id}, passage 1 · cite: [notes.md › Lenses](cite:notebooks/{notes_id}:1)\nA convex lens brings parallel rays to a focus."
        )),
        "{}",
        output(0)
    );
    assert!(!output(0).contains("[2]"), "{}", output(0));
    assert!(
        output(1).contains(&format!(
            "--- section 1, Optics > Lenses · cite: [notes.md › Lenses](cite:notebooks/{notes_id}:1) ---\nA convex lens"
        )),
        "{}",
        output(1)
    );
    assert!(
        output(1).ends_with("The source ends here."),
        "{}",
        output(1)
    );
    assert!(
        output(2).starts_with(&format!(
            "Notebook \"Physics\" (notebook {physics}), 1 sources:\n- \"notes.md\" (source {notes_id}, Markdown, 3 passages)"
        )),
        "{}",
        output(2)
    );
    // A source or a notebook that is not attached is refused before the call
    // runs, and nothing of it reaches the model.
    for refused in [3, 4] {
        assert_eq!(calls[refused].status, ToolCallStatus::NotRun, "{refused}");
    }
    let replies = core.model.reply_requests();
    let read_back = format!("{:?}", replies[1].messages);
    assert!(!read_back.contains("secret"), "{read_back}");
    assert!(
        NOTEBOOK_TOOLS
            .iter()
            .all(|tool| offered(&replies[0]).iter().any(|name| name == tool))
    );
}

#[tokio::test]
async fn a_mention_attaches_a_notebook_whose_tools_later_messages_get() {
    let (hosted, _models) = setup_with_tools().await;
    choose_models(&hosted, true).await;
    let physics = create(&hosted, "Physics").await;
    let source = drop_file(&hosted, &physics, "notes.md", NOTES.as_bytes())
        .await
        .unwrap();
    until_state(&hosted, source["id"].as_str().unwrap(), "ready").await;
    let core = &hosted.core;
    for answer in ["Hello.", "It bends.", "Lenses focus."] {
        core.model.push_reply(reply(answer));
    }

    let started = core
        .runtime
        .start_conversation(StartConversation::new("OWNER: hello"))
        .await
        .unwrap();
    core.wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let conversation = started.conversation_id;
    let named =
        core.runtime
            .send_message(
                SendMessage::new(conversation.clone(), "@Physics why does light bend?")
                    .with_mentions(vec![MentionRef {
                        provider: NOTEBOOKS.to_owned(),
                        key: physics.clone(),
                    }]),
            )
            .await
            .unwrap();
    core.wait_for_run(&conversation, &named.run_id).await;
    let later = core
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "and lenses?"))
        .await
        .unwrap();
    core.wait_for_run(&conversation, &later.run_id).await;
    let requests = core.model.reply_requests();
    let summary = core
        .store
        .conversation_summary(&conversation)
        .await
        .unwrap()
        .unwrap();

    assert!(
        NOTEBOOK_TOOLS
            .iter()
            .all(|tool| !offered(&requests[0]).iter().any(|name| name == tool))
    );
    let mentioned = last_owner_message(&requests[1]);
    assert_eq!(mentioned[0], "@Physics why does light bend?");
    assert_eq!(
        mentioned[1],
        "[The owner attached \"Physics\" to this conversation with this message.]"
    );
    assert!(
        mentioned[2].starts_with(texts::CONTEXT_HEADING),
        "{mentioned:?}"
    );
    // A later message gathers nothing, and the model searches with the tools.
    assert_eq!(last_owner_message(&requests[2]), ["and lenses?"]);
    assert!(
        NOTEBOOK_TOOLS
            .iter()
            .all(|tool| offered(&requests[2]).iter().any(|name| name == tool))
    );
    assert_eq!(summary.attachments.len(), 1);
    assert_eq!(summary.attachments[0].label, "Physics");
    assert_eq!(summary.home, None);
}

#[tokio::test]
async fn removing_a_source_leaves_none_of_its_text_in_the_store_s_files() {
    let (hosted, _dialog, _models) = setup().await;
    choose_models(&hosted, false).await;
    let notebook = create(&hosted, "Secrets").await;
    let planted = "planted-notebook-text-9f3e1c planted-notebook-text-9f3e1c";
    let source = call(
        &hosted,
        "add_text_source",
        json!({ "notebookId": notebook, "title": "Plan", "text": planted }),
    )
    .await;
    let id = source["id"].as_str().unwrap();
    until_state(&hosted, id, "ready").await;
    let folder = hosted.core.database().parent().unwrap().to_path_buf();
    assert!(files_contain(&folder, b"planted-notebook-text-9f3e1c"));

    call(&hosted, "remove_source", json!({ "sourceId": id })).await;

    assert!(
        !files_contain(&folder, b"planted-notebook-text-9f3e1c"),
        "the text is gone from every file of the store"
    );
}

#[tokio::test]
async fn a_passage_s_place_counts_as_the_webview_counts() {
    let (hosted, _dialog, _models) = setup().await;
    choose_models(&hosted, false).await;
    let notebook = create(&hosted, "Languages").await;
    let source = call(
        &hosted,
        "add_text_source",
        json!({ "notebookId": notebook, "title": "Grüße", "text": "Grüße aus Köln. Hallo." }),
    )
    .await;
    let id = source["id"].as_str().unwrap();
    until_state(&hosted, id, "ready").await;

    let place = call(
        &hosted,
        "locate_passage",
        json!({ "key": format!("{id}:0") }),
    )
    .await;
    let missing = plugin_error(
        hosted
            .invoke("locate_passage", json!({ "key": "nobody:0" }))
            .await,
    );

    assert_eq!(
        place,
        json!({ "sourceId": id, "section": 0, "start": 0, "end": 22 })
    );
    assert_eq!(missing, json!({ "code": "passage_not_found" }));
}

#[tokio::test]
async fn turning_the_plugin_off_and_on_keeps_the_notebooks_and_reads_the_preferences() {
    let (hosted, _dialog, _models) = setup().await;
    choose_models(&hosted, true).await;
    create(&hosted, "Physics").await;

    hosted.turn_off().await;
    let off = hosted.invoke("read_preferences", Value::Null).await;
    hosted.turn_on().await;
    let preferences = call(&hosted, "read_preferences", Value::Null).await;
    let profiles = call(&hosted, "list_search_profiles", Value::Null).await;
    let mut lists = hosted
        .open_channel("follow_notebooks", Value::Null)
        .await
        .unwrap();
    let Some(ChannelMessage::Item { item }) = lists.next().await else {
        panic!("the channel sends the notebooks");
    };

    assert!(matches!(off, Err(CallFailure::Host(_))), "{off:?}");
    assert_eq!(preferences["rerankProfileId"], FAKE_RERANK_PROFILE);
    assert_eq!(profiles["embeddings"][0]["id"], FAKE_EMBEDDING_PROFILE);
    assert_eq!(item["notebooks"][0]["name"], "Physics");
}
