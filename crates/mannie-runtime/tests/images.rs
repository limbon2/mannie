//! Images in the runtime (ADR-0033; plugins/images/docs/README.md): the sources of
//! plugins, the checks of a message's references and of what a source
//! returns, what the model reads of an image, and the bytes that the
//! transcript reads back, also after the plugin stops.

#![cfg(test)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::host::{ID_NOT_OWNED, ID_TAKEN};
use mannie_plugin_sdk::points::images::{ImageSource, ImageSources, SourcedImage};
use mannie_provider::{Content, ImageInput, ModelRequest, ProviderError, Role};
use mannie_runtime::testing::{
    Harness, HarnessOptions, SeenPart, add_plugin, failure, remove_plugin, reply,
};
use mannie_types::chat::{ChatError, SendMessage, StartConversation, StartedConversation};
use mannie_types::images::{
    ImageBytes, ImageData, ImageFormat, ImageRef, ImageSize, ImageView, ReadImage,
};
use mannie_types::run::RunFailure;
use mannie_types::transcript::PartContent;

/// A plugin's source that holds the images of `add`, refuses other keys with
/// `not_found`, and records the keys it was told to release.
#[derive(Debug)]
struct Shelf {
    id: &'static str,
    images: Mutex<HashMap<String, SourcedImage>>,
    released: Arc<Mutex<Vec<String>>>,
}

impl Shelf {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            images: Mutex::new(HashMap::new()),
            released: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn add(self, key: &str, image: SourcedImage) -> Self {
        self.images
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), image);
        self
    }
}

impl ImageSource for Shelf {
    fn id(&self) -> &str {
        self.id
    }

    fn resolve<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<SourcedImage, String>> {
        let image = self
            .images
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
            .ok_or_else(|| "not_found".to_owned());
        futures_util::future::ready(image).boxed()
    }

    fn release(&self, keys: &[String]) {
        self.released
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(keys);
    }
}

/// A PNG as a source gives it, whose bytes end with `marker`.
fn png(name: &str, marker: u8) -> SourcedImage {
    let name = name.to_owned();
    let format = ImageFormat::Png;
    let width = 800;
    let height = 600;
    let data = ImageBytes::from([b"\x89PNG\r\n\x1a\n".as_slice(), &[marker]].concat());
    let thumbnail_format = ImageFormat::Jpeg;
    let thumbnail = ImageBytes::from([b"\xff\xd8\xff".as_slice(), &[marker]].concat());
    SourcedImage::new(
        name,
        format,
        width,
        height,
        data,
        thumbnail_format,
        thumbnail,
    )
}

fn image(key: &str) -> ImageRef {
    ImageRef {
        source: "images".to_owned(),
        key: key.to_owned(),
    }
}

/// A harness with the plugin `images`, whose source holds `one`, `two` and
/// `three`, and the keys it released.
async fn setup() -> (Harness, Arc<Mutex<Vec<String>>>) {
    let harness = Harness::with_options(HarnessOptions::default()).await;
    let shelf = Shelf::new("images")
        .add("one", png("one.png", 1))
        .add("two", png("C:\\Shots\\two.png", 2))
        .add("three", png("three.png", 3));
    let released = Arc::clone(&shelf.released);
    add_plugin(
        &harness.runtime,
        "images",
        Contributions::new().add::<ImageSources>(Arc::new(shelf)),
    )
    .unwrap();
    (harness, released)
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

fn released(keys: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    keys.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn image_views(parts: &[SeenPart]) -> Vec<(String, u32)> {
    parts
        .iter()
        .filter_map(|part| match &part.content {
            PartContent::Image { image } => Some((image.name.clone(), image.width)),
            _ => None,
        })
        .collect()
}

/// The first owner message of a request: its texts, and the last byte of
/// each image.
fn first_owner_message(request: &ModelRequest) -> Vec<String> {
    let message = request
        .messages
        .iter()
        .find(|message| message.role == Role::User)
        .expect("the request has an owner message");
    message
        .content
        .iter()
        .map(|content| match content {
            Content::Text(text) => text.clone(),
            Content::Image(ImageInput { data, .. }) => {
                format!("image {}", data.as_slice().last().unwrap())
            }
            other => format!("{other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn a_message_gives_the_model_its_images_after_its_text() {
    let (harness, released_keys) = setup().await;
    harness.model.push_reply(reply("Two pictures."));

    let started = start(
        &harness,
        "What differs?",
        vec![image("one"), image("two"), image("one")],
    )
    .await
    .unwrap();
    let released_at_start = released(&released_keys);
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let request = harness.model.reply_requests().pop().unwrap();

    assert_eq!(
        first_owner_message(&request),
        ["What differs?", "image 1", "image 2"]
    );
    assert_eq!(
        image_views(&transcript.parts(0)),
        [("one.png".to_owned(), 800), ("two.png".to_owned(), 800)]
    );
    assert_eq!(released_at_start, ["one", "two"]);
}

#[tokio::test]
async fn a_message_may_have_only_images_and_takes_its_title_from_the_first() {
    let (harness, _released) = setup().await;
    harness.model.push_reply(reply("A picture."));

    let started = start(&harness, "  ", vec![image("two")]).await.unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let empty = start(&harness, " ", Vec::new()).await;
    let conversations = harness.store.list_conversations().await.unwrap();
    let request = harness.model.reply_requests().pop().unwrap();

    assert_eq!(empty.unwrap_err(), ChatError::MessageEmpty);
    assert_eq!(conversations.len(), 1);
    assert_eq!(conversations[0].title, "two.png");
    assert_eq!(first_owner_message(&request), ["image 2"]);
}

#[tokio::test]
async fn refused_images_store_nothing_and_release_nothing() {
    let (harness, released_keys) = setup().await;
    let mut not_a_png = png("bad.gif", 9);
    not_a_png.data = ImageBytes::from(b"GIF89a".to_vec());
    let bad = Shelf::new("broken").add("bad", not_a_png);
    add_plugin(
        &harness.runtime,
        "broken",
        Contributions::new().add::<ImageSources>(Arc::new(bad)),
    )
    .unwrap();
    let eleven: Vec<ImageRef> = (0..11).map(|n| image(&n.to_string())).collect();

    let too_many = start(&harness, "Many", eleven).await;
    let unknown_source = start(
        &harness,
        "Nobody",
        vec![ImageRef {
            source: "nobody".to_owned(),
            key: "one".to_owned(),
        }],
    )
    .await;
    let unknown_key = start(&harness, "Gone", vec![image("one"), image("gone")]).await;
    let invalid = start(
        &harness,
        "Bad",
        vec![ImageRef {
            source: "broken".to_owned(),
            key: "bad".to_owned(),
        }],
    )
    .await;

    assert_eq!(too_many.unwrap_err(), ChatError::TooManyImages { max: 10 });
    assert_eq!(
        unknown_source.unwrap_err(),
        ChatError::ImageUnavailable {
            image_source: "nobody".to_owned()
        }
    );
    assert_eq!(
        unknown_key.unwrap_err(),
        ChatError::ImageRefused {
            image_source: "images".to_owned(),
            reason: "not_found".to_owned()
        }
    );
    assert_eq!(
        invalid.unwrap_err(),
        ChatError::ImageRefused {
            image_source: "broken".to_owned(),
            reason: "invalid_image".to_owned()
        }
    );
    assert_eq!(
        harness.store.list_conversations().await.unwrap(),
        Vec::new()
    );
    assert_eq!(harness.model.requests(), Vec::new());
    assert_eq!(released(&released_keys), Vec::<String>::new());
}

#[tokio::test]
async fn a_retry_sends_the_same_images_again() {
    let (harness, _released) = setup().await;
    harness
        .model
        .push_reply(failure(ProviderError::Permanent(RunFailure::BadRequest {
            status: 400,
        })));
    harness.model.push_reply(reply("Second try"));

    let started = start(&harness, "Look", vec![image("three")]).await.unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let retried = harness
        .runtime
        .retry_reply(&started.conversation_id)
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &retried.run_id)
        .await;
    let requests = harness.model.reply_requests();

    assert_eq!(requests.len(), 2);
    assert_eq!(
        first_owner_message(&requests[0]),
        first_owner_message(&requests[1])
    );
    assert_eq!(first_owner_message(&requests[1]), ["Look", "image 3"]);
}

#[tokio::test]
async fn stored_images_reach_the_model_and_read_back_after_the_plugin_stops() {
    let (harness, _released) = setup().await;
    harness.model.push_reply(reply("A picture."));
    harness.model.push_reply(reply("Still the picture."));
    let started = start(&harness, "Keep this", vec![image("one")])
        .await
        .unwrap();
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let part = transcript.parts(0)[1].id.clone();
    let other = start(&harness, "Elsewhere", Vec::new()).await.unwrap();

    assert!(remove_plugin(&harness.runtime, "images"));
    let refused = harness
        .runtime
        .send_message(
            SendMessage::new(started.conversation_id.clone(), "Another")
                .with_images(vec![image("two")]),
        )
        .await;
    let sent = harness
        .runtime
        .send_message(SendMessage::new(
            started.conversation_id.clone(),
            "And now?",
        ))
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &sent.run_id)
        .await;
    let read = |conversation_id, size| {
        harness.runtime.read_image(ReadImage {
            conversation_id,
            part_id: part.clone(),
            size,
        })
    };
    let thumbnail = read(started.conversation_id.clone(), ImageSize::Thumbnail).await;
    let full = read(started.conversation_id.clone(), ImageSize::Full).await;
    let elsewhere = read(other.conversation_id.clone(), ImageSize::Full).await;
    let request = harness.model.requests_for("Keep this").pop().unwrap();

    assert_eq!(
        refused.unwrap_err(),
        ChatError::ImageUnavailable {
            image_source: "images".to_owned()
        }
    );
    assert_eq!(first_owner_message(&request), ["Keep this", "image 1"]);
    assert_eq!(
        thumbnail.unwrap(),
        ImageData {
            format: ImageFormat::Jpeg,
            data: "/9j/AQ==".to_owned()
        }
    );
    assert_eq!(
        full.unwrap(),
        ImageData {
            format: ImageFormat::Png,
            data: "iVBORw0KGgoB".to_owned()
        }
    );
    assert_eq!(elsewhere.unwrap_err(), ChatError::ImageNotFound);
    assert_eq!(
        transcript.parts(0)[1].content,
        PartContent::Image {
            image: ImageView {
                name: "one.png".to_owned(),
                format: ImageFormat::Png,
                width: 800,
                height: 600,
                bytes: 9,
            }
        }
    );
}

#[tokio::test]
async fn a_plugin_s_sources_need_ids_of_their_own() {
    let harness = Harness::with_options(HarnessOptions::default()).await;
    let contributions = |ids: &[&'static str]| {
        ids.iter().fold(Contributions::new(), |contributions, id| {
            let source: Arc<dyn ImageSource> = Arc::new(Shelf::new(id));
            contributions.add::<ImageSources>(source)
        })
    };
    // The point and the code of a refusal.
    let refused = |plugin: &str, ids: &[&'static str]| {
        add_plugin(&harness.runtime, plugin, contributions(ids))
            .err()
            .map(|refusal| (refusal.point, refusal.code))
    };

    let foreign = refused("images", &["imagesx"]);
    let twice = refused("images", &["images", "images"]);
    let own = refused("images", &["images", "images.screens"]);

    assert_eq!(foreign, Some(("image-sources", ID_NOT_OWNED)));
    assert_eq!(twice, Some(("image-sources", ID_TAKEN)));
    assert_eq!(own, None);
}
