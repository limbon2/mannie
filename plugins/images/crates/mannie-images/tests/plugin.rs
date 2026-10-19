//! The Images plugin through the plugin host and its dispatch (ADR-0030,
//! ADR-0033, ADR-0034; plugins/images/docs/README.md): its exported manifest
//! declares the file dialog, it starts off and the host refuses its commands
//! while it is, every command has a handler that refuses unknown fields, a
//! chosen or pasted image that it prepared reaches the model and leaves it
//! once its message is stored, a chosen file does not outlive the start that
//! kept it, turning it off drops its waiting images and keeps the stored
//! ones, and a host without a file dialog cannot start it.

#![cfg(test)]

use std::io::Cursor;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::{DynamicImage, ImageBuffer, ImageFormat as Format, Rgb};
use mannie_images::commands::{EXTENSIONS, MAX_FILE_BYTES, PickedImage, PreparedImage};
use mannie_images::{IMAGES, ImagesPlugin, PLUGIN_MANIFEST};
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::{CallFailure, HostOptions, HostedPlugin, PluginHost};
use mannie_plugin_harness::provider::{Content, Role};
use mannie_plugin_harness::runtime::{Harness, reply};
use mannie_plugin_harness::types::{
    ImageRef, ImageSize, PartContent, PluginState, ReadImage, StartConversation,
    StartedConversation,
};
use mannie_plugin_sdk::contracts::{ChatError, ImageFormat};
use mannie_plugin_sdk::error::SERVICE_UNAVAILABLE;
use mannie_plugin_sdk::host::ServiceProvider;
use mannie_plugin_sdk::ipc::HostRefusal;
use mannie_plugin_sdk::manifest::{Manifest, SecretsSection};
use mannie_plugin_sdk::plugin::Plugin;
use mannie_plugin_sdk::services::file_dialog::FileFilter;
use mannie_plugin_sdk::testing::{FakeFile, FakeFileDialog};
use serde_json::{Value, json};

/// A host whose file dialog is `dialog`, or that has none.
fn options(dialog: Option<&FakeFileDialog>) -> HostOptions {
    match dialog {
        Some(dialog) => {
            let dialog: Arc<dyn ServiceProvider> = Arc::new(dialog.clone());
            HostOptions::new().with_provider(dialog)
        }
        None => HostOptions::new(),
    }
}

/// Images on a host whose file dialog is `dialog`, or without one.
async fn setup_with(dialog: Option<&FakeFileDialog>) -> (Harness, PluginHost) {
    HostedPlugin::new(mannie_images::mannie_plugin(), options(dialog))
        .await
        .into_parts()
}

async fn setup() -> (Harness, PluginHost, FakeFileDialog) {
    let dialog = FakeFileDialog::new();
    let (harness, host) = setup_with(Some(&dialog)).await;
    (harness, host, dialog)
}

/// Calls a command of Images through the host's dispatch.
async fn invoke(host: &PluginHost, command: &str, input: Value) -> Result<Value, CallFailure> {
    host.invoke(IMAGES, command, input).await
}

/// The plugin's error with this JSON, as the dispatch answers it.
fn plugin_error(failure: Result<Value, CallFailure>) -> Value {
    match failure {
        Err(CallFailure::Plugin(error)) => error.error,
        other => panic!("not an error of the plugin: {other:?}"),
    }
}

fn plugin_off() -> Result<Value, CallFailure> {
    Err(CallFailure::Host(HostRefusal::plugin_off(IMAGES)))
}

async fn pick(host: &PluginHost) -> Vec<PickedImage> {
    let picked = invoke(host, "pick_images", json!({ "filterName": "Images" }))
        .await
        .unwrap();
    serde_json::from_value(picked).unwrap()
}

async fn prepare_picked(host: &PluginHost, picked_id: &str) -> Result<Value, CallFailure> {
    invoke(
        host,
        "prepare_picked_image",
        json!({ "pickedId": picked_id }),
    )
    .await
}

async fn prepare(host: &PluginHost, input: Value) -> PreparedImage {
    serde_json::from_value(invoke(host, "prepare_image", input).await.unwrap()).unwrap()
}

/// A PNG of `width` by `height` pixels.
fn png(width: u32, height: u32) -> Vec<u8> {
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
    png.into_inner()
}

/// A pasted PNG of `width` by `height` pixels, as the webview sends it.
fn pasted(width: u32, height: u32) -> Value {
    json!({
        "name": "Screenshot 2026-10-17.png",
        "data": BASE64.encode(png(width, height)),
    })
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
        .start_conversation(StartConversation::new(text).with_images(images))
        .await
}

#[test]
fn the_embedded_manifest_declares_images_and_the_file_dialog() {
    let manifest = Manifest::parse(PLUGIN_MANIFEST).unwrap();
    let entry = mannie_images::mannie_plugin();

    assert_eq!(manifest.id.as_str(), IMAGES);
    assert_eq!(manifest.sdk, 1);
    assert_eq!(manifest.order, 20);
    assert_eq!(manifest.rust.entry, env!("CARGO_PKG_NAME"));
    assert_eq!(
        manifest.services,
        std::collections::BTreeMap::from([("file-dialog".to_owned(), 1)])
    );
    assert_eq!(manifest.secrets, SecretsSection::default());
    assert_eq!(entry.plugin.store_module(), None);
    assert_eq!(
        entry.plugin.interface().command_names(),
        [
            "pick_images",
            "prepare_picked_image",
            "prepare_image",
            "discard_prepared_image"
        ]
    );
    assert_eq!(
        entry.plugin.interface().constants()[0].name,
        "MAX_FILE_BYTES"
    );
}

#[tokio::test]
async fn images_keeps_the_rules_of_the_sdk() {
    let dialog = FakeFileDialog::new();

    let problems = conformance(mannie_images::mannie_plugin(), options(Some(&dialog))).await;

    assert_eq!(problems, Vec::<String>::new());
    assert_eq!(dialog.opened(), 0, "the check opens no dialog");
}

#[tokio::test]
async fn images_starts_off_and_the_host_refuses_its_commands() {
    let (_harness, host, dialog) = setup().await;

    host.start_chosen().await;

    assert_eq!(host.views()[0].state, PluginState::Off);
    for (command, input) in [
        ("pick_images", json!({ "filterName": "Images" })),
        ("prepare_picked_image", json!({ "pickedId": "any" })),
        ("prepare_image", pasted(4, 4)),
        ("discard_prepared_image", json!({ "key": "any" })),
    ] {
        assert_eq!(
            invoke(&host, command, input).await,
            plugin_off(),
            "{command}"
        );
    }
    assert_eq!(dialog.opened(), 0, "the dialog does not open for nothing");
}

#[tokio::test]
async fn every_command_has_a_handler_that_refuses_an_unknown_field() {
    let (_harness, host, dialog) = setup().await;
    host.set_enabled(IMAGES, true).await.unwrap();

    for command in ImagesPlugin::new().interface().command_names() {
        let answer = invoke(&host, command, json!({ "probe": 1 })).await;
        assert_eq!(
            answer,
            Err(CallFailure::Host(HostRefusal::InvalidInput { field: None })),
            "{command}"
        );
    }
    let blank_filter = invoke(&host, "pick_images", json!({ "filterName": " " })).await;
    assert_eq!(
        blank_filter,
        Err(CallFailure::Host(HostRefusal::InvalidInput {
            field: Some("filterName".to_owned())
        }))
    );
    assert_eq!(dialog.opened(), 0);
}

#[tokio::test]
async fn a_pasted_image_reaches_the_model_shrunk_and_leaves_once_its_message_is_stored() {
    let (harness, host, _dialog) = setup().await;
    host.set_enabled(IMAGES, true).await.unwrap();
    harness.model.push_reply(reply("A gradient."));

    let prepared = prepare(&host, pasted(2_500, 500)).await;
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
    let (harness, host, _dialog) = setup().await;
    host.set_enabled(IMAGES, true).await.unwrap();

    let prepared = prepare(&host, pasted(8, 8)).await;
    let discarded = invoke(
        &host,
        "discard_prepared_image",
        json!({ "key": prepared.key }),
    )
    .await;
    let removed = start(&harness, "Look", vec![reference(&prepared)]).await;

    assert_eq!(discarded, Ok(Value::Null));
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
    let (harness, host, _dialog) = setup().await;
    host.set_enabled(IMAGES, true).await.unwrap();
    harness.model.push_reply(reply("Seen."));
    let sent = prepare(&host, pasted(8, 8)).await;
    let waiting = prepare(&host, pasted(16, 16)).await;
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
    let (_harness, host, _dialog) = setup().await;
    host.set_enabled(IMAGES, true).await.unwrap();

    let too_long = invoke(
        &host,
        "prepare_image",
        json!({ "name": "big.png", "data": "A".repeat(MAX_FILE_BYTES.div_ceil(3) * 4 + 4) }),
    )
    .await;
    let not_base64 = invoke(
        &host,
        "prepare_image",
        json!({ "name": "odd.png", "data": "not base64!" }),
    )
    .await;

    assert_eq!(
        plugin_error(too_long),
        json!({ "code": "too_large", "maxBytes": 20_971_520 })
    );
    assert_eq!(plugin_error(not_base64), json!({ "code": "unreadable" }));
}

#[tokio::test]
async fn chosen_files_reach_the_model_through_the_file_dialog() {
    let (harness, host, dialog) = setup().await;
    host.set_enabled(IMAGES, true).await.unwrap();
    dialog.answer(vec![FakeFile::new("wide.png", png(3_000, 1_000))]);
    harness.model.push_reply(reply("A wide image."));

    let picked = pick(&host).await;
    let prepared: PreparedImage =
        serde_json::from_value(prepare_picked(&host, &picked[0].picked_id).await.unwrap()).unwrap();
    let again = prepare_picked(&host, &picked[0].picked_id).await;
    let started = start(&harness, "What is this?", vec![reference(&prepared)])
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let request = harness.model.reply_requests().pop().unwrap();
    let owner = request
        .messages
        .iter()
        .find(|message| message.role == Role::User)
        .unwrap();

    assert_eq!(
        dialog.filters(),
        [FileFilter::new("Images", EXTENSIONS.iter().copied())]
    );
    assert_eq!(picked[0].name, "wide.png");
    assert_eq!(
        dialog.reads(),
        [(
            "wide.png".to_owned(),
            u64::try_from(MAX_FILE_BYTES).unwrap()
        )]
    );
    assert_eq!(plugin_error(again), json!({ "code": "not_found" }));
    let [Content::Text(_text), Content::Image(image)] = owner.content.as_slice() else {
        panic!("{:?}", owner.content);
    };
    assert_eq!((image.width, image.height), (2_048, 683));
}

#[tokio::test]
async fn a_pick_that_ends_while_images_turns_off_keeps_no_chosen_file() {
    let (_harness, host, dialog) = setup().await;
    host.set_enabled(IMAGES, true).await.unwrap();
    dialog.answer(vec![FakeFile::new("a.png", png(8, 8))]);
    let gate = dialog.hold();

    let picking = tokio::spawn({
        let host = host.clone();
        async move { pick(&host).await }
    });
    while dialog.opened() == 0 {
        tokio::task::yield_now().await;
    }
    let turning_off = tokio::spawn({
        let host = host.clone();
        async move { host.set_enabled(IMAGES, false).await }
    });
    while host.views()[0].state != PluginState::Stopping {
        tokio::task::yield_now().await;
    }
    // Turning off waits for the pick, whose files belong to the start that
    // is stopping.
    gate.open();
    let picked = picking.await.unwrap();
    turning_off.await.unwrap().unwrap();
    host.set_enabled(IMAGES, true).await.unwrap();
    let after = prepare_picked(&host, &picked[0].picked_id).await;

    assert_eq!(picked[0].name, "a.png");
    assert_eq!(plugin_error(after), json!({ "code": "not_found" }));
    assert_eq!(dialog.reads(), Vec::new());
}

#[tokio::test]
async fn a_file_chosen_before_a_restart_is_not_read() {
    let (_harness, host, dialog) = setup().await;
    host.set_enabled(IMAGES, true).await.unwrap();
    dialog.answer(vec![FakeFile::new("a.png", png(8, 8))]);
    let picked = pick(&host).await;

    host.set_enabled(IMAGES, false).await.unwrap();
    host.set_enabled(IMAGES, true).await.unwrap();
    let after = prepare_picked(&host, &picked[0].picked_id).await;

    assert_eq!(plugin_error(after), json!({ "code": "not_found" }));
    assert_eq!(dialog.reads(), Vec::new());
}

#[tokio::test]
async fn images_fails_to_start_on_a_host_without_a_file_dialog() {
    let (_harness, host) = setup_with(None).await;

    let view = host.set_enabled(IMAGES, true).await.unwrap();
    let refused = invoke(&host, "pick_images", json!({ "filterName": "Images" })).await;

    assert_eq!(view.state, PluginState::Failed);
    assert_eq!(view.failure.as_deref(), Some(SERVICE_UNAVAILABLE));
    assert_eq!(refused, plugin_off());
}
