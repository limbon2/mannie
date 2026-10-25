use std::collections::HashSet;
use std::sync::LazyLock;

use mannie_provider::{Content, ImageInput, Message, Role, ToolChoice, ToolDefinition};
use mannie_store::{KeptRun, ModelImage, ModelPart, ModelTurn, ProviderItem};
use mannie_types::ids::{ArchiveId, ConversationId, PartId, RunId, TaskId, TurnId};
use mannie_types::images::{ImageBytes, ImageFormat};
use mannie_types::provider::{ApiInterface, ReasoningEffort};
use mannie_types::run::RunState;
use mannie_types::tasks::{AwaitedTask, TaskDocument, TaskPartDocument, TaskRefusal, TaskState};
use mannie_types::tools::{
    ArchiveRef, NotRunReason, ToolCallDocument, ToolCallStatus, ToolImageRef, ToolOutcome,
    ToolResult,
};
use mannie_types::transcript::{PartKind, PartState};

use super::{RequestShape, SentImages, build_request};
use crate::agents::Requester;
use crate::texts;

fn part_id(number: u32) -> PartId {
    PartId::parse(&format!("0199e0f2-0000-7000-8000-{number:012}")).unwrap()
}

fn run_id(number: u32) -> RunId {
    RunId::parse(&format!("0199e0f2-0000-7000-b000-{number:012}")).unwrap()
}

fn part(number: u32, kind: Option<PartKind>, text: &str) -> ModelPart {
    ModelPart {
        id: part_id(number),
        run_id: (kind != Some(PartKind::UserText)).then(|| run_id(1)),
        kind,
        kind_name: kind.map_or("task_report", PartKind::as_str).to_owned(),
        state: PartState::Complete,
        text: text.to_owned(),
        provider_item: None,
        tool_call: None,
        task: None,
        attempt: u32::from(kind != Some(PartKind::UserText)),
        image: None,
        context: None,
    }
}

/// A part that a later model call of the reply wrote.
fn in_step(attempt: u32, part: ModelPart) -> ModelPart {
    ModelPart { attempt, ..part }
}

/// A tool call part with its document.
fn call(number: u32, arguments: &str, document: ToolCallDocument) -> ModelPart {
    ModelPart {
        tool_call: Some(document),
        ..part(number, Some(PartKind::ToolCall), arguments)
    }
}

/// The document of a call with a status.
fn document(call_id: &str, status: ToolCallStatus) -> ToolCallDocument {
    ToolCallDocument {
        status,
        ..ToolCallDocument::receiving(call_id.to_owned(), "read_file".to_owned())
    }
}

/// The document of a call that completed with `output`.
fn completed(call_id: &str, output: &str) -> ToolCallDocument {
    ToolCallDocument {
        result: Some(ToolResult {
            outcome: ToolOutcome::Unknown,
            output: output.to_owned(),
            archives: Vec::new(),
            images: Vec::new(),
        }),
        ..document(call_id, ToolCallStatus::Completed)
    }
}

fn turn(position: u32, run_state: Option<RunState>, parts: Vec<ModelPart>) -> ModelTurn {
    ModelTurn {
        turn_id: TurnId::parse(&format!("0199e0f2-0000-7000-9000-{position:012}")).unwrap(),
        position,
        run_id: Some(run_id(1)),
        run_state,
        kept_runs: Vec::new(),
        parts,
    }
}

static TOOLS: LazyLock<Vec<ToolDefinition>> = LazyLock::new(|| {
    vec![ToolDefinition {
        name: "read_file".to_owned(),
        description: "Reads a file.".to_owned(),
        parameters: serde_json::json!({ "type": "object" }),
        strict: true,
    }]
});

static NOTHING_LEFT_OUT: LazyLock<HashSet<PartId>> = LazyLock::new(HashSet::new);

static NO_IMAGES: LazyLock<SentImages> = LazyLock::new(SentImages::new);

static NO_FULL_CONTEXT: LazyLock<HashSet<PartId>> = LazyLock::new(HashSet::new);

fn shape(interface: ApiInterface) -> RequestShape<'static> {
    RequestShape {
        instructions: "System.",
        model: "m",
        interface,
        binding: "p:1",
        max_output_tokens: 100,
        reasoning_effort: ReasoningEffort::Low,
        tools: &[],
        tool_choice: ToolChoice::Auto,
        note: None,
        left_out: &NOTHING_LEFT_OUT,
        requester: Requester::Owner,
        tool_images: &NO_IMAGES,
        full_context: &NO_FULL_CONTEXT,
    }
}

fn with_tools(interface: ApiInterface) -> RequestShape<'static> {
    RequestShape {
        tools: &TOOLS,
        ..shape(interface)
    }
}

fn tool_call(call_id: &str, arguments: &str) -> Content {
    Content::ToolCall {
        call_id: call_id.to_owned(),
        name: "read_file".to_owned(),
        arguments: arguments.to_owned(),
    }
}

fn tool_result(call_id: &str, output: &str) -> Message {
    Message {
        role: Role::Tool,
        content: vec![Content::ToolResult {
            call_id: call_id.to_owned(),
            output: output.to_owned(),
            images: Vec::new(),
        }],
    }
}

fn reasoning_with_item(binding: &str) -> ModelPart {
    ModelPart {
        provider_item: Some(ProviderItem {
            binding: binding.to_owned(),
            item: r#"{"type":"reasoning"}"#.to_owned(),
        }),
        ..part(2, Some(PartKind::Reasoning), "thinking")
    }
}

#[test]
fn turns_become_alternating_messages() {
    let turns = [
        turn(
            0,
            Some(RunState::Completed),
            vec![
                part(1, Some(PartKind::UserText), "Hi"),
                part(2, Some(PartKind::Text), "Hello!"),
            ],
        ),
        turn(
            1,
            Some(RunState::Running),
            vec![part(3, Some(PartKind::UserText), "How are you?")],
        ),
    ];

    let request = build_request(shape(ApiInterface::ChatCompletions), &turns);

    assert_eq!(request.instructions, "System.");
    assert_eq!(
        request.messages,
        [
            Message::text(Role::User, "Hi"),
            Message::text(Role::Assistant, "Hello!"),
            Message::text(Role::User, "How are you?"),
        ]
    );
}

/// A part that the owner's message mentions, which belongs to no run.
fn mention(number: u32, text: &str) -> ModelPart {
    ModelPart {
        run_id: None,
        attempt: 0,
        ..part(number, Some(PartKind::Mention), text)
    }
}

#[test]
fn mentions_follow_the_owner_s_text_in_the_same_message() {
    let turns = [turn(
        0,
        Some(RunState::Running),
        vec![
            part(1, Some(PartKind::UserText), "Compare @a.rs with @b.rs"),
            mention(2, "The owner mentioned the file a.rs."),
            mention(3, "The owner mentioned the file b.rs."),
        ],
    )];

    let request = build_request(shape(ApiInterface::Responses), &turns);

    assert_eq!(
        request.messages,
        [Message {
            role: Role::User,
            content: vec![
                Content::Text("Compare @a.rs with @b.rs".to_owned()),
                Content::Text("The owner mentioned the file a.rs.".to_owned()),
                Content::Text("The owner mentioned the file b.rs.".to_owned()),
            ],
        }]
    );
}

#[test]
fn a_note_that_joins_a_request_follows_the_owner_s_message_in_the_same_message() {
    let note = ModelPart {
        run_id: None,
        attempt: 0,
        ..part(
            3,
            Some(PartKind::ContinuationNote),
            "Background commands of this conversation:",
        )
    };
    let turns = [turn(
        0,
        Some(RunState::Running),
        vec![
            part(1, Some(PartKind::UserText), "Is the server up?"),
            mention(2, "The owner mentioned the file a.rs."),
            note,
            in_step(1, part(4, Some(PartKind::Text), "It is.")),
        ],
    )];

    let request = build_request(shape(ApiInterface::Responses), &turns);

    assert_eq!(
        request.messages,
        [
            Message {
                role: Role::User,
                content: vec![
                    Content::Text("Is the server up?".to_owned()),
                    Content::Text("The owner mentioned the file a.rs.".to_owned()),
                    Content::Text("Background commands of this conversation:".to_owned()),
                ],
            },
            Message::text(Role::Assistant, "It is."),
        ]
    );
}

/// An image of the owner's message, whose bytes are `byte` repeated.
fn image(number: u32, byte: u8) -> ModelPart {
    ModelPart {
        run_id: None,
        attempt: 0,
        image: Some(ModelImage {
            format: ImageFormat::Png,
            width: 4,
            height: 2,
            digest: format!("digest-{byte}"),
            data: ImageBytes::from(vec![byte; 3]),
        }),
        ..part(number, Some(PartKind::Image), "")
    }
}

fn image_content(byte: u8) -> Content {
    Content::Image(ImageInput {
        format: ImageFormat::Png,
        width: 4,
        height: 2,
        data: ImageBytes::from(vec![byte; 3]),
    })
}

#[test]
fn images_follow_the_text_and_mentions_in_the_same_message() {
    let turns = [turn(
        0,
        Some(RunState::Running),
        vec![
            part(1, Some(PartKind::UserText), "What differs?"),
            mention(2, "The owner mentioned the file a.rs."),
            image(3, 7),
            image(4, 8),
        ],
    )];

    let request = build_request(shape(ApiInterface::ChatCompletions), &turns);

    assert_eq!(
        request.messages,
        [Message {
            role: Role::User,
            content: vec![
                Content::Text("What differs?".to_owned()),
                Content::Text("The owner mentioned the file a.rs.".to_owned()),
                image_content(7),
                image_content(8),
            ],
        }]
    );
}

#[test]
fn a_message_with_only_an_image_has_no_empty_text_and_a_lost_image_a_note() {
    let lost = ModelPart {
        image: None,
        ..image(3, 9)
    };
    let turns = [turn(
        0,
        Some(RunState::Running),
        vec![part(1, Some(PartKind::UserText), " "), image(2, 7), lost],
    )];

    let request = build_request(shape(ApiInterface::Responses), &turns);

    assert_eq!(
        request.messages,
        [Message {
            role: Role::User,
            content: vec![
                image_content(7),
                Content::Text(texts::IMAGE_MISSING.to_owned()),
            ],
        }]
    );
}

#[test]
fn encrypted_reasoning_goes_back_only_to_its_binding_on_the_responses_api() {
    let same = [turn(
        0,
        Some(RunState::Completed),
        vec![
            part(1, Some(PartKind::UserText), "Q"),
            reasoning_with_item("p:1"),
            part(3, Some(PartKind::Text), "A"),
        ],
    )];
    let other = [turn(
        0,
        Some(RunState::Completed),
        vec![
            part(1, Some(PartKind::UserText), "Q"),
            reasoning_with_item("p:2"),
            part(3, Some(PartKind::Text), "A"),
        ],
    )];

    let responses = build_request(shape(ApiInterface::Responses), &same);
    let rebound = build_request(shape(ApiInterface::Responses), &other);
    let chat = build_request(shape(ApiInterface::ChatCompletions), &same);

    assert_eq!(
        responses.messages[1].content,
        [
            Content::ProviderItem(r#"{"type":"reasoning"}"#.to_owned()),
            Content::Text("A".to_owned())
        ]
    );
    assert_eq!(rebound.messages[1].content, [Content::Text("A".to_owned())]);
    assert_eq!(chat.messages[1].content, [Content::Text("A".to_owned())]);
}

#[test]
fn stopped_and_interrupted_replies_carry_markers() {
    let turns = [
        turn(
            0,
            Some(RunState::Stopped),
            vec![
                part(1, Some(PartKind::UserText), "Q1"),
                part(2, Some(PartKind::Text), "Partial"),
            ],
        ),
        turn(
            1,
            Some(RunState::Interrupted),
            vec![
                part(3, Some(PartKind::UserText), "Q2"),
                part(4, Some(PartKind::Text), "Half"),
            ],
        ),
        turn(
            2,
            Some(RunState::Failed),
            vec![part(5, Some(PartKind::UserText), "Q3")],
        ),
        turn(
            3,
            Some(RunState::Stopped),
            vec![part(6, Some(PartKind::UserText), "Q4")],
        ),
    ];

    let request = build_request(shape(ApiInterface::ChatCompletions), &turns);

    assert_eq!(
        request.messages[1].content,
        [
            Content::Text("Partial".to_owned()),
            Content::Text(texts::STOPPED_MARKER.to_owned())
        ]
    );
    assert_eq!(
        request.messages[3].content,
        [
            Content::Text("Half".to_owned()),
            Content::Text(texts::INTERRUPTED_MARKER.to_owned())
        ]
    );
    assert_eq!(
        request.messages[4],
        Message::text(Role::User, "Q3"),
        "a failed reply without text leaves nothing"
    );
    assert_eq!(
        request.messages[6],
        Message::text(Role::Assistant, texts::STOPPED_MARKER)
    );
}

#[test]
fn continuation_notes_and_unknown_parts_reach_the_model_in_place() {
    let turns = [turn(
        0,
        Some(RunState::Running),
        vec![
            part(1, Some(PartKind::UserText), "Q"),
            part(2, Some(PartKind::Text), "Before"),
            part(
                3,
                Some(PartKind::ContinuationNote),
                texts::CONTINUATION_NOTE,
            ),
            part(4, None, ""),
        ],
    )];

    let request = build_request(shape(ApiInterface::ChatCompletions), &turns);

    assert_eq!(
        request.messages[1],
        Message::text(Role::Assistant, "Before")
    );
    assert_eq!(
        request.messages[2],
        Message::text(Role::User, texts::CONTINUATION_NOTE)
    );
    assert_eq!(
        request.messages[3],
        Message::text(Role::Assistant, texts::unknown_part("task_report"))
    );
}

#[test]
fn tool_calls_are_followed_by_their_results_before_the_next_step() {
    let turns = [turn(
        0,
        Some(RunState::Completed),
        vec![
            part(1, Some(PartKind::UserText), "Read a.txt"),
            part(2, Some(PartKind::Text), "Reading it."),
            call(3, r#"{"path":"a.txt"}"#, completed("call_1", "File: a.txt")),
            call(4, r#"{"path":"b.txt"}"#, completed("call_2", "File: b.txt")),
            part(5, Some(PartKind::Text), "Both are short."),
        ],
    )];

    let request = build_request(with_tools(ApiInterface::ChatCompletions), &turns);

    assert_eq!(
        request.messages,
        [
            Message::text(Role::User, "Read a.txt"),
            Message {
                role: Role::Assistant,
                content: vec![
                    Content::Text("Reading it.".to_owned()),
                    tool_call("call_1", r#"{"path":"a.txt"}"#),
                    tool_call("call_2", r#"{"path":"b.txt"}"#),
                ],
            },
            tool_result("call_1", "File: a.txt"),
            tool_result("call_2", "File: b.txt"),
            Message::text(Role::Assistant, "Both are short."),
        ]
    );
    assert_eq!(request.tools.len(), 1);
}

#[test]
fn the_calls_of_each_step_get_an_assistant_message_of_their_own() {
    let turns = [turn(
        0,
        Some(RunState::Completed),
        vec![
            part(1, Some(PartKind::UserText), "Fix a.txt"),
            call(2, r#"{"path":"a.txt"}"#, completed("call_1", "File: a.txt")),
            in_step(
                2,
                call(
                    3,
                    r#"{"path":"a.txt"}"#,
                    completed("call_2", "Edited a.txt."),
                ),
            ),
            in_step(3, part(4, Some(PartKind::Text), "Fixed.")),
        ],
    )];

    let request = build_request(with_tools(ApiInterface::ChatCompletions), &turns);

    assert_eq!(
        request.messages,
        [
            Message::text(Role::User, "Fix a.txt"),
            Message {
                role: Role::Assistant,
                content: vec![tool_call("call_1", r#"{"path":"a.txt"}"#)],
            },
            tool_result("call_1", "File: a.txt"),
            Message {
                role: Role::Assistant,
                content: vec![tool_call("call_2", r#"{"path":"a.txt"}"#)],
            },
            tool_result("call_2", "Edited a.txt."),
            Message::text(Role::Assistant, "Fixed."),
        ]
    );
}

#[test]
fn calls_that_did_not_run_say_why() {
    let turns = [turn(
        0,
        Some(RunState::Running),
        vec![
            part(1, Some(PartKind::UserText), "Go"),
            call(2, "{}", document("denied", ToolCallStatus::Denied)),
            call(
                3,
                "not json",
                ToolCallDocument {
                    not_run: Some(NotRunReason::InvalidArguments {
                        detail: "expected value".to_owned(),
                    }),
                    ..document("invalid", ToolCallStatus::NotRun)
                },
            ),
            call(
                4,
                "{}",
                ToolCallDocument {
                    not_run: Some(NotRunReason::EarlierCallDenied),
                    ..document("skipped", ToolCallStatus::NotRun)
                },
            ),
            call(5, "{}", document("uncertain", ToolCallStatus::Uncertain)),
        ],
    )];

    let request = build_request(with_tools(ApiInterface::ChatCompletions), &turns);

    assert_eq!(
        request.messages[1].content[1],
        tool_call("invalid", "{}"),
        "arguments that are not JSON go back as an empty object"
    );
    assert_eq!(
        request.messages[2..],
        [
            tool_result("denied", texts::DENIED),
            tool_result("invalid", &texts::invalid_arguments("expected value")),
            tool_result("skipped", texts::SKIPPED_AFTER_DENIAL),
            tool_result("uncertain", texts::UNCERTAIN),
        ]
    );
}

#[test]
fn a_model_without_tools_reads_earlier_calls_as_text() {
    let turns = [turn(
        0,
        Some(RunState::Completed),
        vec![
            part(1, Some(PartKind::UserText), "Read a.txt"),
            call(2, r#"{"path":"a.txt"}"#, completed("call_1", "File: a.txt")),
            part(3, Some(PartKind::Text), "Done."),
        ],
    )];

    let request = build_request(shape(ApiInterface::ChatCompletions), &turns);

    assert_eq!(
        request.messages[1].content,
        [
            Content::Text(texts::tool_call_as_text(
                "read_file",
                r#"{"path":"a.txt"}"#,
                "File: a.txt"
            )),
            Content::Text("Done.".to_owned()),
        ]
    );
    assert_eq!(request.tools, Vec::new());
}

/// The document of a call that completed with a screenshot at position 0
/// and a chart at position 2.
fn with_images(call_id: &str) -> ToolCallDocument {
    let mut document = completed(call_id, "Two images.");
    if let Some(result) = &mut document.result {
        result.images = vec![
            ToolImageRef {
                position: 0,
                format: ImageFormat::Png,
                width: 1280,
                height: 720,
                bytes: 90_000,
            },
            ToolImageRef {
                position: 2,
                format: ImageFormat::Jpeg,
                width: 640,
                height: 480,
                bytes: 30_000,
            },
        ];
    }
    document
}

fn sent_image(byte: u8) -> ImageInput {
    ImageInput {
        format: ImageFormat::Png,
        width: 1280,
        height: 720,
        data: ImageBytes::from(vec![byte; 4]),
    }
}

#[test]
fn a_result_carries_its_sent_images_and_names_the_others_in_a_line() {
    let turns = [turn(
        0,
        Some(RunState::Running),
        vec![
            part(1, Some(PartKind::UserText), "Show me"),
            call(2, "{}", with_images("call_1")),
        ],
    )];
    let sent = SentImages::from([(part_id(2), vec![(0, sent_image(7))])]);

    let request = build_request(
        RequestShape {
            tool_images: &sent,
            ..with_tools(ApiInterface::Responses)
        },
        &turns,
    );

    assert_eq!(
        request.messages[2],
        Message {
            role: Role::Tool,
            content: vec![Content::ToolResult {
                call_id: "call_1".to_owned(),
                output: format!(
                    "Two images.\n{}",
                    texts::tool_image_not_sent(3, ImageFormat::Jpeg, 640, 480)
                ),
                images: vec![sent_image(7)],
            }],
        }
    );
}

#[test]
fn a_model_without_tools_reads_every_image_of_a_result_as_a_line() {
    let turns = [turn(
        0,
        Some(RunState::Completed),
        vec![
            part(1, Some(PartKind::UserText), "Show me"),
            call(2, "{}", with_images("call_1")),
        ],
    )];
    let sent = SentImages::from([(part_id(2), vec![(0, sent_image(7))])]);

    let request = build_request(
        RequestShape {
            tool_images: &sent,
            ..shape(ApiInterface::ChatCompletions)
        },
        &turns,
    );

    let expected = format!(
        "Two images.\n{}\n{}",
        texts::tool_image_not_sent(1, ImageFormat::Png, 1280, 720),
        texts::tool_image_not_sent(3, ImageFormat::Jpeg, 640, 480)
    );
    assert_eq!(
        request.messages[1].content,
        [Content::Text(texts::tool_call_as_text(
            "read_file",
            "{}",
            &expected
        ))]
    );
}

#[test]
fn results_left_out_name_their_archives() {
    let archive = ArchiveId::parse("0199e0f2-0000-7000-a000-000000000001").unwrap();
    let mut output = completed("call_1", "a long output");
    if let Some(result) = &mut output.result {
        result.archives.push(ArchiveRef {
            id: archive.clone(),
            name: "stdout".to_owned(),
            bytes: 100_000,
            lines: 2_000,
        });
    }
    let turns = [turn(
        0,
        Some(RunState::Running),
        vec![
            part(1, Some(PartKind::UserText), "Build"),
            call(2, "{}", output),
        ],
    )];
    let left_out = HashSet::from([part_id(2)]);

    let request = build_request(
        RequestShape {
            left_out: &left_out,
            ..with_tools(ApiInterface::ChatCompletions)
        },
        &turns,
    );

    assert_eq!(
        request.messages[2],
        tool_result("call_1", &texts::result_left_out(&[archive.to_string()]))
    );
}

#[test]
fn a_kept_run_ends_with_its_marker_before_the_next_run() {
    let mut kept_call = call(2, "{}", completed("call_1", "File: a.txt"));
    kept_call.run_id = Some(run_id(7));
    let turns = [ModelTurn {
        kept_runs: vec![KeptRun {
            run_id: run_id(7),
            state: RunState::Stopped,
        }],
        ..turn(
            0,
            Some(RunState::Completed),
            vec![
                part(1, Some(PartKind::UserText), "Read a.txt"),
                kept_call,
                part(3, Some(PartKind::Text), "It says hello."),
            ],
        )
    }];

    let request = build_request(with_tools(ApiInterface::ChatCompletions), &turns);

    assert_eq!(
        request.messages,
        [
            Message::text(Role::User, "Read a.txt"),
            Message {
                role: Role::Assistant,
                content: vec![tool_call("call_1", "{}")],
            },
            tool_result("call_1", "File: a.txt"),
            Message::text(Role::Assistant, texts::STOPPED_MARKER),
            Message::text(Role::Assistant, "It says hello."),
        ]
    );
}

#[test]
fn a_note_ends_the_request() {
    let turns = [turn(
        0,
        Some(RunState::Running),
        vec![part(1, Some(PartKind::UserText), "Go")],
    )];

    let request = build_request(
        RequestShape {
            note: Some(texts::TOOL_BUDGET_USED),
            tool_choice: ToolChoice::None,
            ..with_tools(ApiInterface::ChatCompletions)
        },
        &turns,
    );

    assert_eq!(
        request.messages.last(),
        Some(&Message::text(Role::User, texts::TOOL_BUDGET_USED))
    );
    assert_eq!(request.tool_choice, ToolChoice::None);
}

/// A task part: a call of a task tool with its task section.
fn task_part(number: u32, document: ToolCallDocument, section: TaskPartDocument) -> ModelPart {
    ModelPart {
        tool_call: Some(document),
        task: Some(section),
        ..part(number, Some(PartKind::Task), "{}")
    }
}

/// The task that a start assigned, as its part records it.
fn assigned(number: u32, background: bool, state: TaskState) -> TaskDocument {
    TaskDocument {
        task_id: TaskId::parse(&format!("0199e0f2-0000-7000-c000-{number:012}")).unwrap(),
        number,
        conversation_id: ConversationId::parse(&format!("0199e0f2-0000-7000-d000-{number:012}"))
            .unwrap(),
        agent: "explore".to_owned(),
        title: "Look".to_owned(),
        background,
        follow_up: false,
        state,
        instruction: String::new(),
        activity: None,
        tool_calls: 0,
        approval: None,
        report: None,
        failure: None,
        started_at: None,
        ended_at: None,
        worker: None,
        executor: None,
        detached: false,
    }
}

fn start_call(call_id: &str, status: ToolCallStatus) -> ToolCallDocument {
    ToolCallDocument {
        status,
        ..ToolCallDocument::receiving(call_id.to_owned(), "start_task".to_owned())
    }
}

#[test]
fn task_parts_read_as_calls_with_their_results() {
    let reported = TaskPartDocument {
        task: Some(assigned(1, false, TaskState::Reported)),
        output: Some(texts::task_reported(1, texts::THE_CHILD, "Found it.")),
        ..TaskPartDocument::default()
    };
    // A start in the background stays open while the task runs.
    let mut background = task_part(
        3,
        start_call("start_2", ToolCallStatus::Running),
        TaskPartDocument {
            task: Some(assigned(2, true, TaskState::Running)),
            output: Some(texts::task_started_in_background(2, "explore")),
            ..TaskPartDocument::default()
        },
    );
    background.state = PartState::Streaming;
    let refused = TaskPartDocument {
        refusal: Some(TaskRefusal::StillRunning { number: 2 }),
        ..TaskPartDocument::default()
    };
    // A part that a crash ended before the model got a result.
    let interrupted = TaskPartDocument {
        task: Some(assigned(4, false, TaskState::Interrupted)),
        ..TaskPartDocument::default()
    };
    let turns = [turn(
        0,
        Some(RunState::Running),
        vec![
            part(1, Some(PartKind::UserText), "Split it"),
            task_part(
                2,
                start_call("start_1", ToolCallStatus::Completed),
                reported,
            ),
            background,
            task_part(
                4,
                start_call("follow_1", ToolCallStatus::Completed),
                refused,
            ),
            task_part(
                5,
                start_call("start_4", ToolCallStatus::Canceled),
                interrupted,
            ),
        ],
    )];

    let request = build_request(with_tools(ApiInterface::ChatCompletions), &turns);

    let start = |call_id: &str| Content::ToolCall {
        call_id: call_id.to_owned(),
        name: "start_task".to_owned(),
        arguments: "{}".to_owned(),
    };
    assert_eq!(
        request.messages[1].content,
        [
            start("start_1"),
            start("start_2"),
            start("follow_1"),
            start("start_4"),
        ]
    );
    assert_eq!(
        request.messages[2..],
        [
            tool_result(
                "start_1",
                &texts::task_reported(1, texts::THE_CHILD, "Found it.")
            ),
            tool_result("start_2", &texts::task_started_in_background(2, "explore")),
            tool_result("follow_1", &texts::task_still_running(2)),
            tool_result("start_4", &texts::task_interrupted(4, texts::THE_CHILD)),
        ]
    );
}

#[test]
fn reports_left_out_say_how_to_read_them_again_and_a_child_reads_its_own_denial() {
    let waited = TaskPartDocument {
        awaited: vec![
            AwaitedTask {
                number: 1,
                title: "One".to_owned(),
                state: TaskState::Reported,
            },
            AwaitedTask {
                number: 3,
                title: "Three".to_owned(),
                state: TaskState::Failed,
            },
        ],
        output: Some("Task 1 reported:\n\nlong".to_owned()),
        ..TaskPartDocument::default()
    };
    let wait_call = ToolCallDocument {
        status: ToolCallStatus::Completed,
        ..ToolCallDocument::receiving("wait_1".to_owned(), "wait_for_tasks".to_owned())
    };
    let turns = [turn(
        0,
        Some(RunState::Running),
        vec![
            part(1, Some(PartKind::UserText), "Wait"),
            task_part(2, wait_call, waited),
            call(3, "{}", document("denied", ToolCallStatus::Denied)),
        ],
    )];
    let left_out = HashSet::from([part_id(2)]);

    let request = build_request(
        RequestShape {
            left_out: &left_out,
            requester: Requester::Agent,
            ..with_tools(ApiInterface::ChatCompletions)
        },
        &turns,
    );

    assert_eq!(
        request.messages[2..],
        [
            tool_result("wait_1", &texts::report_left_out(&[1, 3])),
            tool_result("denied", texts::CHILD_DENIED),
        ]
    );
}
