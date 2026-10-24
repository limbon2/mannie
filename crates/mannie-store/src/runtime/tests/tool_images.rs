//! The images of tool results in the store (ADR-0044, decision 3): written
//! with the call that they belong to, read back in order by part and
//! position, refused for a part that is no open tool call, exported with
//! their conversation, and deleted with it.

use mannie_types::ids::{ConversationId, PartId};
use mannie_types::images::{ImageBytes, ImageFormat};
use mannie_types::tools::{ToolCallStatus, ToolImageRef, ToolOutcome, ToolResult};
use mannie_types::transcript::PartState;

use super::Fixture;
use crate::runtime::{Change, NewPart, NewToolImage, RuntimeStoreError};
use crate::{Durability, StoredToolImage};

fn tool_image(position: u32, marker: &[u8]) -> NewToolImage {
    let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
    data.extend_from_slice(marker);
    NewToolImage {
        position,
        format: ImageFormat::Png,
        width: 640,
        height: 480,
        data: ImageBytes::from(data),
    }
}

/// The batch that completes a call with images at these positions, as the
/// runtime writes it: the update that names them, the images, the end.
fn completing(call: &NewPart, images: Vec<NewToolImage>) -> Vec<Change> {
    let refs = images
        .iter()
        .map(|image| ToolImageRef {
            position: image.position,
            format: image.format,
            width: image.width,
            height: image.height,
            bytes: u32::try_from(image.data.len()).unwrap(),
        })
        .collect();
    let mut document = call.tool_call.clone().unwrap();
    document.status = ToolCallStatus::Completed;
    document.result = Some(ToolResult {
        outcome: ToolOutcome::Unknown,
        output: "A screenshot.".to_owned(),
        archives: Vec::new(),
        images: refs,
    });
    vec![
        Change::UpdatePart {
            part_id: call.id.clone(),
            tool_call: document,
            archives: Vec::new(),
        },
        Change::AddToolImages {
            part_id: call.id.clone(),
            images,
        },
        Change::FinishPart {
            part_id: call.id.clone(),
            state: PartState::Complete,
            tail: String::new(),
            provider_item: None,
        },
    ]
}

/// A conversation whose run made one tool call, which is still open.
async fn with_open_call(fixture: &Fixture) -> (ConversationId, NewPart) {
    let (conversation, turn, run, _) = fixture.start("Take a screenshot").await;
    let call = fixture.tool_part(&turn, &run, 1, "call_1");
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart { part: call.clone() },
            ],
        )
        .await;
    (conversation, call)
}

#[tokio::test]
async fn the_images_of_a_result_are_kept_with_their_call_and_read_back_by_position() {
    let fixture = Fixture::new();
    let (conversation, call) = with_open_call(&fixture).await;
    let first = tool_image(0, b"first");
    let third = tool_image(2, b"third");
    let (other, _) = with_open_call(&fixture).await;

    fixture
        .apply(
            &conversation,
            completing(&call, vec![first.clone(), third.clone()]),
        )
        .await;

    let read = |conversation: ConversationId, part: PartId, position: u32| {
        let store = fixture.store.clone();
        async move {
            store
                .tool_image(&conversation, &part, position)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        read(conversation.clone(), call.id.clone(), 2).await,
        Some((ImageFormat::Png, third.data.clone()))
    );
    assert_eq!(read(conversation.clone(), call.id.clone(), 1).await, None);
    assert_eq!(read(other.clone(), call.id.clone(), 0).await, None);
    let stored = fixture
        .store
        .tool_images(&conversation, &[call.id.clone(), fixture.part_id()])
        .await
        .unwrap();
    let positions: Vec<(PartId, Vec<u32>)> = stored
        .iter()
        .map(|(part, images)| {
            (
                part.clone(),
                images.iter().map(|image| image.position).collect(),
            )
        })
        .collect();
    assert_eq!(positions, vec![(call.id.clone(), vec![0, 2])]);
    assert_eq!(
        stored[0].1[0],
        StoredToolImage {
            position: 0,
            format: ImageFormat::Png,
            width: 640,
            height: 480,
            data: first.data,
        }
    );
    // The images belong to the conversation of their part, and nowhere else.
    assert_eq!(
        fixture
            .store
            .tool_images(&other, std::slice::from_ref(&call.id))
            .await
            .unwrap(),
        Vec::new()
    );
}

#[tokio::test]
async fn only_an_open_tool_call_of_the_conversation_takes_images() {
    let fixture = Fixture::new();
    let (conversation, turn, run, _) = fixture.start("Take a screenshot").await;
    let call = fixture.tool_part(&turn, &run, 1, "call_1");
    let text = fixture.text_part(&turn, &run, 1);
    fixture
        .apply(
            &conversation,
            vec![
                Change::StartRun {
                    run_id: run.clone(),
                },
                Change::StartPart { part: call.clone() },
                Change::StartPart { part: text.clone() },
            ],
        )
        .await;
    fixture
        .apply(&conversation, completing(&call, vec![tool_image(0, b"a")]))
        .await;
    let (other, _) = with_open_call(&fixture).await;

    let add = |conversation: &ConversationId, part: &PartId| {
        let store = fixture.store.clone();
        let conversation = conversation.clone();
        let part = part.clone();
        async move {
            store
                .apply(
                    &conversation,
                    vec![Change::AddToolImages {
                        part_id: part,
                        images: vec![tool_image(1, b"b")],
                    }],
                    Durability::Full,
                )
                .await
        }
    };
    let on_finished = add(&conversation, &call.id).await;
    let on_text = add(&conversation, &text.id).await;
    let elsewhere = add(&other, &text.id).await;

    assert!(
        matches!(on_finished, Err(RuntimeStoreError::PartNotStreaming { .. })),
        "{on_finished:?}"
    );
    assert!(
        matches!(on_text, Err(RuntimeStoreError::NotAToolCall { .. })),
        "{on_text:?}"
    );
    assert!(elsewhere.is_err(), "{elsewhere:?}");
}

#[tokio::test]
async fn the_export_holds_the_images_of_its_calls_and_warns_of_a_missing_one() {
    let fixture = Fixture::new();
    let (conversation, call) = with_open_call(&fixture).await;
    let mut batch = completing(&call, vec![tool_image(0, b"kept"), tool_image(1, b"gone")]);
    // The update names both images, and only the first is stored.
    if let Change::AddToolImages { images, .. } = &mut batch[1] {
        images.truncate(1);
    }
    fixture.apply(&conversation, batch).await;

    let record = fixture
        .store
        .conversation_record(&conversation)
        .await
        .unwrap()
        .unwrap();

    let [image] = record.tool_images.as_slice() else {
        panic!("{:?}", record.tool_images);
    };
    assert_eq!(image.part_id, call.id);
    assert_eq!(image.position, 0);
    assert!(image.data.ends_with(b"kept"));
    let missing: Vec<&str> = record
        .warnings
        .iter()
        .map(|warning| warning.reference.as_str())
        .collect();
    assert_eq!(missing, vec![format!("{}#1", call.id).as_str()]);
}

#[tokio::test]
async fn a_deleted_conversation_leaves_no_tool_image_bytes_behind() {
    let fixture = Fixture::new();
    let (conversation, call) = with_open_call(&fixture).await;
    let planted = b"planted-tool-image-bytes-9e41d2";
    fixture
        .apply(
            &conversation,
            completing(&call, vec![tool_image(0, planted)]),
        )
        .await;
    assert!(fixture.files_contain_bytes(planted));

    assert!(
        fixture
            .store
            .delete_conversation(&conversation)
            .await
            .unwrap()
    );

    assert!(
        !fixture.files_contain_bytes(planted),
        "the image is gone from every file"
    );
}
