use mannie_provider::{Content, Message, Role};
use mannie_store::{ModelPart, ModelTurn, ProviderItem};
use mannie_types::ids::{PartId, TurnId};
use mannie_types::provider::{ApiInterface, ReasoningEffort};
use mannie_types::run::RunState;
use mannie_types::transcript::{PartKind, PartState};

use super::{RequestShape, build_request};
use crate::texts;

fn part(number: u32, kind: Option<PartKind>, text: &str) -> ModelPart {
    ModelPart {
        id: PartId::parse(&format!("0199e0f2-0000-7000-8000-{number:012}")).unwrap(),
        kind,
        kind_name: kind.map_or("tool_call", PartKind::as_str).to_owned(),
        state: PartState::Complete,
        text: text.to_owned(),
        provider_item: None,
    }
}

fn turn(position: u32, run_state: Option<RunState>, parts: Vec<ModelPart>) -> ModelTurn {
    ModelTurn {
        turn_id: TurnId::parse(&format!("0199e0f2-0000-7000-9000-{position:012}")).unwrap(),
        position,
        run_state,
        parts,
    }
}

fn shape(interface: ApiInterface) -> RequestShape<'static> {
    RequestShape {
        instructions: "System.",
        model: "m",
        interface,
        binding: "p:1",
        max_output_tokens: 100,
        reasoning_effort: ReasoningEffort::Low,
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
        Message::text(Role::Assistant, texts::unknown_part("tool_call"))
    );
}
