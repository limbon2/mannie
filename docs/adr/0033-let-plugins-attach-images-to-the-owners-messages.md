# ADR-0033: Let plugins attach images to the owner's messages

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D31)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Date     | 2026-10-17                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Related  | [ADR-0002](0002-grow-through-extension-points.md), [ADR-0004](0004-keep-system-access-in-the-rust-core.md), [ADR-0007](0007-generate-ipc-bindings-from-rust.md), [ADR-0011](0011-own-the-provider-adapters.md), [ADR-0020](0020-complete-the-ui-shell-slots.md), [ADR-0030](0030-split-mannie-into-a-core-and-plugins.md), [ADR-0032](0032-let-features-and-plugins-add-kinds-of-mentions.md), [Extension points](../architecture/extension-points.md), [Security](../architecture/security.md), [Images](../../plugins/images/docs/README.md) |

Later changes: [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md) refines decisions 2 and 10: the image source moves into the plugin SDK, and the file dialog is a host service that returns handles.

## Context

- On 2026-10-17 the owner asked to send images in chat, built as a plugin. Images get into a message through a button that opens the system's file dialog, by pasting and by dropping files on the message box. Mannie shrinks them before sending, "as far as i know chatgpt and claude do similar things".
- Nothing in Mannie carries an image today. The content of a provider message is text and tool artifacts, `send_message` and `start_conversation` take text and mentions, the transcript has no part for an image, and the store holds no binary data. The message box has no slot that a plugin can fill ([ADR-0020](0020-complete-the-ui-shell-slots.md)), and mentions carry text only ([ADR-0032](0032-let-features-and-plugins-add-kinds-of-mentions.md)).
- The core never depends on a plugin, and conversation parts that a plugin added stay readable while it is off ([ADR-0030](0030-split-mannie-into-a-core-and-plugins.md), decisions 10 and 12). A new extension point needs an ADR ([ADR-0002](0002-grow-through-extension-points.md)).
- The webview is untrusted, and file access stays in Rust ([ADR-0004](0004-keep-system-access-in-the-rust-core.md)). Its Content Security Policy allows images from `data:` and `blob:` URLs only, and the asset protocol is off.
- The IPC bindings are generated from Rust ([ADR-0007](0007-generate-ipc-bindings-from-rust.md)). tauri-specta binds strings, and a `Vec<u8>` travels as a JSON array of numbers, about 3.5 times its size.
- Both APIs Mannie speaks take images inline as `data:` URLs (verified 2026-10-17 in OpenAI's images and vision guide): the Responses API as an `input_image` part of a user message, and Chat Completions as an `image_url` part of a user message whose content is an array. Servers that implement Chat Completions, such as vLLM, Ollama and LM Studio, accept the same form for models that read images, and refuse it for models that do not.
- Photos from phones carry metadata such as the place they were taken. A model provider receives what Mannie sends ([security](../architecture/security.md), §11).
- The `image` crate 0.25.10 (verified 2026-10-17) decodes PNG, JPEG, GIF and WebP in Rust, applies limits on dimensions and memory, reads the EXIF orientation and encodes PNG and JPEG. It is MIT OR Apache-2.0, and its dependencies are allowed by the [license policy](0017-license-and-dependency-policy.md).

## Decision

1. The owner's message can hold images. `send_message` and `start_conversation` gain `images`, a list of references with a source and a key, which is empty when left out. A message has at most 10 images. Its text may be blank when it has at least one image, and a new conversation whose first message has no text takes its title from the name of its first image.
2. Image sources are an extension point of `mannie-runtime`. A source has an ID and two functions: resolving a key into an image, and releasing the keys of a message once it is stored. Sources come only from plugins' contributions, with the ID rule of mention providers: the plugin's ID, or the plugin's ID and a dot ([ADR-0030](0030-split-mannie-into-a-core-and-plugins.md), decision 5, grows). A reference to a source that is not there fails with `image_unavailable`, and a refused key fails with `image_refused` and the source's code. Nothing is stored, and the message box keeps the message.
3. The core checks what a source returns: an image in PNG or JPEG by its signature, of at most 2048 pixels on each side as the source states them and at most 5 MiB, and a thumbnail in PNG or JPEG of at most 512 KiB, which the source keeps within 320 pixels. The core decodes neither. It cleans the name and computes the SHA-256 digest of the image.
4. Each image is stored as an owner part of the kind `image`, after the owner's text and mentions and in the same transaction. Its document holds what the owner sees: the source, the name, the format, the size, the digest and the dimensions. The bytes of the image and of its thumbnail live in a table of their own, `runtime_images`, which is deleted with the part and the conversation. The views that the UI receives carry no bytes; the core command `read_image` returns the thumbnail or the full image of one part, as base64.
5. The provider adapters send images. `Content` gains an image with its format, dimensions and bytes:
   - The Responses API gets an `input_image` part with a `data:` URL.
   - Chat Completions gets an `image_url` part. A message whose content is all text keeps the string form, so requests without images do not change.
   - The context estimate counts an image as its width times its height divided by 750 tokens, never as the text of its URL.
6. The model reads each image after the owner's text and mentions, in the same user message, and never its name. Retrying a reply sends the stored image again. Compaction replaces an image with a line that names its dimensions, and the checkpoint's hash includes its digest.
7. The UI gets a slot of image sources ([ADR-0020](0020-complete-the-ui-shell-slots.md) grows). An entry gives a button for the message box, says which files it takes, takes files from a paste or a drop, and words its refusal codes. The message box keeps the images of the message being written, shows each as a chip with its thumbnail, sends their references and keeps them when sending fails.
   - A paste takes files only when it carries no text, so pasting text works as before and never becomes an image.
   - A drop is taken on the message box when it carries files. The main window turns off Tauri's own handling of dropped files (`dragDropEnabled: false`), so the page receives them, and the shell refuses files dropped anywhere else.
8. Stored images stay readable and keep reaching the model while the plugin that added them is off ([ADR-0030](0030-split-mannie-into-a-core-and-plugins.md), decision 12). The part's renderer is the core's.
9. Mannie does not know which models read images. Every model receives them, and a model that cannot read them fails with the provider's error.
10. The first source is the plugin `images` ([images](../../plugins/images/docs/README.md)), in `plugins/images/`. It decodes with the `image` crate, applies the EXIF orientation, scales the long side to at most 2048 pixels and encodes again, which drops the metadata. Paths stay in Rust: the desktop crate opens the file dialog and hands the plugin the chosen paths, and pasted or dropped files reach it as base64 strings.

## Consequences

- Positive: the owner shows the model a screenshot or a photo. A later feature or plugin adds a source, such as a screenshot of a worker's screen, without editing chat. Images reach every API the same way, the metadata of photos never leaves the machine, and requests stay small.
- Negative: the store holds binary data for the first time, and a conversation with many images grows the database. Each request sends every image of the conversation again, as base64. The message box grows a second kind of attachment besides mentions, and a slot to keep stable.
- Risks:
  - A hostile file exhausts memory or crashes a decoder. The plugin checks the size and the dimensions in the header before decoding, applies the decoder's limits, decodes one image at a time and uses decoders written in Rust. Tests feed it truncated, oversized and mislabeled files.
  - A source returns something other than what it says. The core checks the signatures, sizes and dimensions again.
  - A conversation with many images exceeds a provider's request limit or Mannie's timeout. The provider's error shows, and the [feature document](../../plugins/images/docs/README.md) records the limits.
  - Script in the webview attaches files on its own. It can only send bytes that it already holds; paths never cross IPC, and the file dialog needs the owner.

## Alternatives considered

| Option                                                        | Why not chosen                                                                                                  |
| ------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Images as a kind of mention                                   | A mention is a token in the text that the "@" list chose, and its content is text                               |
| The bytes in the part's view                                  | Every snapshot and event of the conversation would carry megabytes to the webview                               |
| Files beside the database                                     | Deleting a conversation would have to delete files too, outside the store's transaction and its `secure_delete` |
| Resizing in the webview, with a canvas                        | The core could not trust the result, and the metadata check would move to untrusted code                        |
| Raw binary IPC bodies                                         | Generated bindings cannot call such commands ([ADR-0007](0007-generate-ipc-bindings-from-rust.md))              |
| Tauri's own drop events, with paths                           | Paths would reach the webview, and the drop's place on the page would need a second channel                     |
| A part kind that the plugin owns                              | Stored images would stop reaching the model and showing in the transcript while the plugin is off               |
| A switch on each provider profile for models that read images | The owner chose to send images to every model and let the provider's error show                                 |

## Validation

M7:

- Tests of the adapters' requests with an image, with an image alone, and without one, whose bodies stay as they were.
- Tests of the store and the runtime with a fake source: the image reaches the model after the text and mentions, a retry sends it again, the source releases it only once the turn is stored, a refused key and an 11th image store nothing, stored images still reach the model after the plugin stops, and deleting the conversation leaves none of its bytes in the database or its write-ahead log.
- Tests of the plugin's processing with decoded and hostile files, and the end-to-end tests of the [images](../../plugins/images/docs/README.md) feature with the button, a paste and a drop, with axe.
