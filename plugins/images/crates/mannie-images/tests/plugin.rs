//! The Images plugin through the plugin host (ADR-0030, ADR-0033;
//! docs/features/images.md): it starts off and its commands say so, a pasted
//! image that it prepared reaches the model and leaves it once its message is
//! stored, and turning it off drops its waiting images and keeps the stored
//! ones.

#![cfg(test)]

use std::io::Cursor;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::{DynamicImage, ImageBuffer, ImageFormat as Format, Rgb};
use mannie_images::commands::{
    ImagesError, MAX_FILE_BYTES, PrepareImage, PreparedImage, PreparedImageRef,
};
use mannie_images::{IMAGES, ImagesPlugin};
use mannie_plugins::{Grants, HostedPlugin, Plugin, PluginHost};
use mannie_provider::{Content, Role};
use mannie_runtime::testing::{Harness, HarnessOptions, reply};
use mannie_types::chat::{ChatError, StartConversation, StartedConversation};
use mannie_types::clock::SystemClock;
use mannie_types::images::{ImageFormat, ImageRef, ImageSize, ReadImage};
use mannie_types::plugins::PluginState;
use mannie_types::transcript::PartContent;

async fn setup() -> (Harness, ImagesPlugin, PluginHost) {
    let harness = Harness::with_options(HarnessOptions::default()).await;
    let plugin = ImagesPlugin::new();
    let hosted: Arc<dyn Plugin> = Arc::new(plugin.clone());
    let host = PluginHost::new(
        harness.runtime.clone(),
        harness.store.clone(),
        vec![HostedPlugin {
            plugin: hosted,
            grants: Grants::default(),
        }],
        Arc::new(SystemClock),
        env!("CARGO_PKG_VERSION").to_owned(),
    )
    .unwrap();
    (harness, plugin, host)
}

/// A pasted PNG of `width` by `height` pixels, as the webview sends it.
fn pasted(width: u32, height: u32) -> PrepareImage {
    let image = ImageBuffer::from_fn(width, height, |x, y| {
        Rgb([
            u8::try_from(x % 256).unwrap(),
            u8::try_from(y % 256).unwrap(),
            9,
        ])
    });
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(image)
        .write_to(&mut png, Format::Png)
        .unwrap();
    PrepareImage {
        name: "Screenshot 2026-10-17.png".to_owned(),
        data: BASE64.encode(png.into_inner()),
    }
}

fn reference(prepared: &PreparedImage) -> ImageRef {
    ImageRef {
        source: IMAGES.to_owned(),
        key: prepared.key.clone(),
    }
}

async fn start(
    harness: &Harness,
    text: &str,
    images: Vec<ImageRef>,
) -> Result<StartedConversation, ChatError> {
    harness
        .runtime
        .start_conversation(StartConversation {
            text: text.to_owned(),
            profile_id: None,
            project_id: None,
            mentions: Vec::new(),
            images,
        })
        .await
}

#[tokio::test]
async fn images_starts_off_and_its_commands_say_so() {
    let (_harness, plugin, host) = setup().await;

    host.start_chosen().await;

    assert_eq!(host.views()[0].state, PluginState::Off);
    assert_eq!(plugin.check_on(), Err(ImagesError::PluginOff));
    assert_eq!(
        plugin.prepare(pasted(4, 4)).await.unwrap_err(),
        ImagesError::PluginOff
    );
    assert_eq!(
        plugin
            .discard(&PreparedImageRef {
                key: "any".to_owned()
            })
            .unwrap_err(),
        ImagesError::PluginOff
    );
}

#[tokio::test]
async fn a_pasted_image_reaches_the_model_shrunk_and_leaves_once_its_message_is_stored() {
    let (harness, plugin, host) = setup().await;
    host.set_enabled(IMAGES, true).await.unwrap();
    harness.model.push_reply(reply("A gradient."));

    let prepared = plugin.prepare(pasted(2_500, 500)).await.unwrap();
    let started = start(&harness, "What is this?", vec![reference(&prepared)])
        .await
        .unwrap();
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let again = start(&harness, "Again", vec![reference(&prepared)]).await;
    let request = harness.model.reply_requests().pop().unwrap();
    let owner = request
        .messages
        .iter()
        .find(|message| message.role == Role::User)
        .unwrap();

    assert_eq!(prepared.name, "Screenshot 2026-10-17.png");
    assert_eq!((prepared.width, prepared.height), (2_048, 410));
    assert_eq!(prepared.format, ImageFormat::Png);
    assert!(
        prepared.thumbnail.len() > 100,
        "{}",
        prepared.thumbnail.len()
    );
    let [Content::Text(text), Content::Image(image)] = owner.content.as_slice() else {
        panic!("{:?}", owner.content);
    };
    assert_eq!(text, "What is this?");
    assert_eq!((image.width, image.height), (2_048, 410));
    assert!(image.data.as_slice().starts_with(b"\x89PNG"));
    assert!(matches!(
        &transcript.parts(0)[1].content,
        PartContent::Image { image } if image.name == "Screenshot 2026-10-17.png"
    ));
    assert_eq!(
        again.unwrap_err(),
        ChatError::ImageRefused {
            image_source: IMAGES.to_owned(),
            reason: "not_found".to_owned()
        }
    );
}

#[tokio::test]
async fn a_removed_image_is_no_longer_sent() {
    let (harness, plugin, host) = setup().await;
    host.set_enabled(IMAGES, true).await.unwrap();

    let prepared = plugin.prepare(pasted(8, 8)).await.unwrap();
    plugin
        .discard(&PreparedImageRef {
            key: prepared.key.clone(),
        })
        .unwrap();
    let removed = start(&harness, "Look", vec![reference(&prepared)]).await;

    assert_eq!(
        removed.unwrap_err(),
        ChatError::ImageRefused {
            image_source: IMAGES.to_owned(),
            reason: "not_found".to_owned()
        }
    );
}

#[tokio::test]
async fn turning_images_off_drops_waiting_images_and_keeps_stored_ones() {
    let (harness, plugin, host) = setup().await;
    host.set_enabled(IMAGES, true).await.unwrap();
    harness.model.push_reply(reply("Seen."));
    let sent = plugin.prepare(pasted(8, 8)).await.unwrap();
    let waiting = plugin.prepare(pasted(16, 16)).await.unwrap();
    let started = start(&harness, "Keep", vec![reference(&sent)])
        .await
        .unwrap();
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let part = transcript.parts(0)[1].id.clone();

    let off = host.set_enabled(IMAGES, false).await.unwrap();
    let unavailable = start(&harness, "Later", vec![reference(&waiting)]).await;
    host.set_enabled(IMAGES, true).await.unwrap();
    let forgotten = start(&harness, "After", vec![reference(&waiting)]).await;
    let stored = harness
        .runtime
        .read_image(ReadImage {
            conversation_id: started.conversation_id.clone(),
            part_id: part,
            size: ImageSize::Full,
        })
        .await
        .unwrap();

    assert_eq!(off.state, PluginState::Off);
    assert_eq!(
        unavailable.unwrap_err(),
        ChatError::ImageUnavailable {
            image_source: IMAGES.to_owned()
        }
    );
    assert_eq!(
        forgotten.unwrap_err(),
        ChatError::ImageRefused {
            image_source: IMAGES.to_owned(),
            reason: "not_found".to_owned()
        }
    );
    assert_eq!(stored.format, ImageFormat::Png);
    assert!(BASE64.decode(stored.data).unwrap().starts_with(b"\x89PNG"));
}

#[tokio::test]
async fn pasted_bytes_that_are_too_long_or_not_base64_are_refused() {
    let (_harness, plugin, host) = setup().await;
    host.set_enabled(IMAGES, true).await.unwrap();

    let too_long = plugin
        .prepare(PrepareImage {
            name: "big.png".to_owned(),
            data: "A".repeat(MAX_FILE_BYTES.div_ceil(3) * 4 + 4),
        })
        .await;
    let not_base64 = plugin
        .prepare(PrepareImage {
            name: "odd.png".to_owned(),
            data: "not base64!".to_owned(),
        })
        .await;

    assert_eq!(
        too_long.unwrap_err(),
        ImagesError::TooLarge {
            max_bytes: 20_971_520
        }
    );
    assert_eq!(not_base64.unwrap_err(), ImagesError::Unreadable);
}
