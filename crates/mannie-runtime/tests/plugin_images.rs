//! Images of tool results through a fake plugin (ADR-0044): an image reaches
//! the next request without its metadata and the owner reads it from the
//! call, an image past the limits is left out with a line that says why, and
//! only the newest results send their images while older ones become lines.

#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::ImageBytes;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, PluginTool, Tool, ToolAccess, ToolCall, ToolExecution, ToolImage, ToolRefusal,
    ToolResult, ToolSpec, Tools,
};
use mannie_provider::{Content, ImageInput, ModelRequest, Role};
use mannie_runtime::testing::{Harness, HarnessOptions, add_plugin, reply, tool_calls};
use mannie_types::chat::StartConversation;
use mannie_types::ids::{ConversationId, RunId};
use mannie_types::images::ImageFormat;
use mannie_types::tools::{ReadToolImage, ToolImageRef, ToolsError};
use serde_json::json;

/// The fake plugin's ID and tool.
const PLUGIN: &str = "shots";
const SNAP: &str = "shots_snap";

/// Text that no image that reaches the model or the owner may hold.
const PLANTED: &[u8] = b"planted-gps-51.5007N";

/// A PNG file of the size, with a text chunk that holds [`PLANTED`]. The
/// cleaner keeps checksums as they are, so any four bytes do.
fn png(width: u32, height: u32) -> Vec<u8> {
    let chunk = |kind: &[u8; 4], data: &[u8]| {
        let mut chunk = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
        chunk.extend_from_slice(kind);
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(&[0, 0, 0, 0]);
        chunk
    };
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
    file.extend(chunk(b"IHDR", &header));
    file.extend(chunk(b"tEXt", PLANTED));
    file.extend(chunk(b"IDAT", b"pixels"));
    file.extend(chunk(b"IEND", b""));
    file
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// A plugin's tool that reads only its own data and returns the next of the
/// results that a test queued.
#[derive(Debug, Default)]
struct Shots {
    results: Mutex<VecDeque<ToolResult>>,
}

impl Shots {
    fn queue(&self, output: &str, files: Vec<Vec<u8>>) {
        let images = files
            .into_iter()
            .map(|file| ToolImage::new(ImageBytes::from(file)))
            .collect();
        self.results
            .lock()
            .unwrap()
            .push_back(ToolResult::new(output).with_images(images));
    }
}

impl Tool for Shots {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            SNAP,
            "Takes screenshots.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        )
    }

    fn prepare(&self, _call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        async { Ok(Vec::new()) }.boxed()
    }

    fn execute(&self, _execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let result = self
            .results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| ToolResult::new("Nothing queued."));
        async move { result }.boxed()
    }
}

fn with_shots(shots: &Arc<Shots>) -> Contributions {
    let tool: Arc<dyn Tool> = Arc::<Shots>::clone(shots);
    Contributions::new().add::<Tools>(PluginTool::new(tool, AgentSelector::Conversations))
}

async fn harness() -> Harness {
    Harness::with_options(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await
}

async fn start(harness: &Harness, text: &str) -> (ConversationId, RunId) {
    let started = harness
        .runtime
        .start_conversation(StartConversation::new(text))
        .await
        .unwrap();
    (started.conversation_id, started.run_id)
}

/// The output and the images of the result of `call_id` in a request.
fn result(request: &ModelRequest, call_id: &str) -> (String, Vec<ImageInput>) {
    request
        .messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .flat_map(|message| &message.content)
        .find_map(|content| match content {
            Content::ToolResult {
                call_id: id,
                output,
                images,
            } if id == call_id => Some((output.clone(), images.clone())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no result for {call_id}"))
}

#[tokio::test]
async fn an_image_of_a_result_reaches_the_next_request_and_the_owner_without_its_metadata() {
    let harness = harness().await;
    let shots = Arc::new(Shots::default());
    add_plugin(&harness.runtime, PLUGIN, with_shots(&shots)).unwrap();
    shots.queue("The page.", vec![png(32, 16)]);
    harness
        .model
        .push_reply(tool_calls(&[("call_1", SNAP, "{}")]));
    harness.model.push_reply(reply("It shows a form."));

    let (conversation, run) = start(&harness, "OWNER: show me the page").await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let requests = harness.model.requests();
    let (output, images) = result(&requests[1], "call_1");
    assert_eq!(output, "The page.");
    let [image] = images.as_slice() else {
        panic!("{images:?}");
    };
    assert_eq!(
        (image.format, image.width, image.height),
        (ImageFormat::Png, 32, 16)
    );
    assert!(!contains(image.data.as_slice(), PLANTED));
    assert!(contains(image.data.as_slice(), b"IDATpixels"));
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    let stored = call.result.expect("the call completed");
    assert_eq!(
        stored.images,
        vec![ToolImageRef {
            position: 0,
            format: ImageFormat::Png,
            width: 32,
            height: 16,
            bytes: u32::try_from(image.data.len()).unwrap(),
        }]
    );
    // The owner reads the image that the model saw, by the call's part.
    let part_id = transcript
        .parts(0)
        .into_iter()
        .find(|part| {
            matches!(
                part.content,
                mannie_types::transcript::PartContent::ToolCall { .. }
            )
        })
        .unwrap()
        .id;
    let read = |position: u32| {
        harness.runtime.read_tool_image(ReadToolImage {
            conversation_id: conversation.clone(),
            part_id: part_id.clone(),
            position,
        })
    };
    let data = read(0).await.unwrap();
    assert_eq!(data.format, ImageFormat::Png);
    assert_eq!(
        BASE64.decode(data.data).unwrap(),
        image.data.as_slice().to_vec()
    );
    assert_eq!(read(1).await, Err(ToolsError::ImageNotFound));
}

#[tokio::test]
async fn an_image_past_the_limits_is_left_out_with_a_line_that_says_why() {
    let harness = harness().await;
    let shots = Arc::new(Shots::default());
    add_plugin(&harness.runtime, PLUGIN, with_shots(&shots)).unwrap();
    let mut damaged = png(16, 16);
    damaged.truncate(30);
    shots.queue(
        "Shots.",
        vec![
            b"GIF89a\x01\0\x01\0".to_vec(),
            png(5000, 10),
            damaged,
            png(16, 16),
            png(16, 16),
            png(16, 16),
        ],
    );
    harness
        .model
        .push_reply(tool_calls(&[("call_1", SNAP, "{}")]));
    harness.model.push_reply(reply("One came through."));

    let (conversation, run) = start(&harness, "OWNER: take them").await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    let stored = call.result.expect("the call completed");
    let positions: Vec<u32> = stored.images.iter().map(|image| image.position).collect();
    assert_eq!(positions, vec![3]);
    let lines: Vec<&str> = stored.output.lines().collect();
    assert_eq!(lines.len(), 5, "{}", stored.output);
    assert_eq!(lines[0], "Shots.");
    assert!(lines[1].starts_with("[Image 1 "), "{}", stored.output);
    assert!(lines[2].contains("5000×10"), "{}", stored.output);
    assert!(lines[3].contains("damaged"), "{}", stored.output);
    assert!(lines[4].starts_with("[Images 5 to 6 "), "{}", stored.output);
    let (output, images) = result(&harness.model.requests()[1], "call_1");
    assert_eq!(output, stored.output);
    assert_eq!(images.len(), 1);
}

#[tokio::test]
async fn only_the_newest_results_send_their_images_and_older_ones_read_as_lines() {
    let harness = harness().await;
    let shots = Arc::new(Shots::default());
    add_plugin(&harness.runtime, PLUGIN, with_shots(&shots)).unwrap();
    for (index, call) in ["call_1", "call_2", "call_3", "call_4"]
        .into_iter()
        .enumerate()
    {
        let side = 8 * u32::try_from(index + 1).unwrap();
        shots.queue(&format!("Shots of {call}."), vec![png(side, side); 3]);
        harness.model.push_reply(tool_calls(&[(call, SNAP, "{}")]));
    }
    harness.model.push_reply(reply("Four pages."));

    let (conversation, run) = start(&harness, "OWNER: four pages").await;
    harness.wait_for_run(&conversation, &run).await;

    let requests = harness.model.requests();
    let last = requests.last().unwrap();
    let sent: Vec<usize> = ["call_1", "call_2", "call_3", "call_4"]
        .into_iter()
        .map(|call| result(last, call).1.len())
        .collect();
    // The three newest results, eight images in all: the oldest of them
    // sends two of its three.
    assert_eq!(sent, vec![0, 2, 3, 3]);
    let (oldest, _) = result(last, "call_1");
    assert_eq!(oldest.lines().count(), 4, "{oldest}");
    assert!(oldest.contains("[Image 1 of this result: a PNG image of 8×8 pixels"));
    let (cut, _) = result(last, "call_2");
    assert_eq!(cut.lines().count(), 2, "{cut}");
    assert!(cut.contains("[Image 3 of this result: a PNG image of 16×16 pixels"));
    // While it was the newest, the first result sent all its images.
    assert_eq!(result(&requests[1], "call_1").1.len(), 3);
}
