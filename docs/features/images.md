# Images

|              |                                                                                                                                                                                                                                                                                                                                                                           |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                  |
| Milestone    | M7                                                                                                                                                                                                                                                                                                                                                                        |
| Last updated | 2026-10-17                                                                                                                                                                                                                                                                                                                                                                |
| Related      | [ADR-0033](../adr/0033-let-plugins-attach-images-to-the-owners-messages.md), [ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md), [ADR-0004](../adr/0004-keep-system-access-in-the-rust-core.md), [Plugins](plugins.md), [Chat](chat/README.md), [Conversation view](chat/conversation-view.md), [Mentions](chat/mentions.md), [Providers](chat/providers.md) |

## Intent

The owner wants to send images in chat, as a plugin. An image gets into a message through a button that opens the system's file dialog, by pasting it and by dropping it on the message box. Mannie shrinks large images before sending them: "resize but pick up reasonable numbers, as far as i know chatgpt and claude do similar things". Every model receives the images, and one that cannot read them fails with the provider's error.

## Behavior

### Turning it on

- Images is a plugin on the Plugins page, with a switch. It starts off on a new installation and after the update from 0.2.0, like every plugin ([D28](../roadmap/milestones.md#5-decisions)).
- While Images is off, the message box has no button for images, and pasting or dropping a file adds nothing. Images that earlier messages hold still show in their conversations and still reach the model.

### Adding images to a message

- The message box gets a button, "Attach images", before the text. It opens the system's file dialog, which shows PNG, JPEG, GIF and WebP files and lets the owner choose several.
- Pasting an image into the message box, such as a screenshot, adds it. A paste that carries text pastes the text, as before, even when it also carries an image, as an office application's copy does.
- Dropping image files on the message box adds them, and the box shows that it takes the drop while files are dragged over it. Files dropped anywhere else in the window do nothing.
- Each image shows as a chip with its thumbnail and name, and a spinner while Mannie prepares it. Its button "Remove {name}" takes it out of the message.
- A message holds at most 10 images. An image beyond that, a file that is not an image, and an image that Mannie cannot prepare say what went wrong below the box, and the other images stay.
- A message can be sent once every image is prepared. It may have images and no text.
- Sending keeps the images in the box when it fails, together with the text. Images that wait in the box are kept while Mannie runs, up to 40 at a time across every box; quitting Mannie drops them.

### What Mannie does with an image

- Mannie reads the file, turns it upright as its camera recorded, and shrinks it so that its longer side is at most 2048 pixels. A PNG, GIF or WebP becomes a PNG, and a JPEG stays a JPEG at quality 85. An image that would still be larger than 5 MiB becomes a JPEG on a white background, and one that is still too large is refused.
- The image that the model gets holds no metadata: no place, camera or date.
- An animated GIF or WebP sends its first frame.
- A file larger than 20 MiB, an image wider or taller than 12,000 pixels, or one with more than 64 megapixels is refused before Mannie decodes it.

### What the model gets

- The model reads each image after the owner's text and mentions, in the same message. It never reads the image's name.
- Retrying a reply sends the same images again.
- Every model receives the images. A model that cannot read them fails with the provider's message, as any refused request does, and so does every later reply in that conversation until the owner chooses a model that reads images.
- When the conversation is summarized, each image becomes a line that names its size, and the summary's model does not see the image.

### The transcript

- The owner's message shows its images below its text and mentions, as thumbnails. A message without text shows only its images.
- Clicking a thumbnail, or pressing Enter on it, opens the image at full size in a dialog, which Escape closes.
- A new conversation whose first message has no text takes the name of its first image as its title.
- Deleting a conversation deletes its images.

### What it does not do

- It sends no image to a worker or a child agent, and the model cannot send images.
- It knows nothing of which models read images.
- It does not read other kinds of files, such as PDF documents.
- It cannot mention an image file of a project with "@": a mention gives the model text.

## Fit

- Adds the extension points of [ADR-0033](../adr/0033-let-plugins-attach-images-to-the-owners-messages.md): image content in provider requests, the part `image` with its table of bytes, image sources, `images` on `send_message` and `start_conversation`, `read_image`, and the UI's slot of image sources.
- Follows [Plugins](plugins.md): a switch, a start and stop that take effect at once, and the plugin's code in `plugins/images/`, its UI files named `images`, and its messages under `images_`.
- Uses the core's file dialog from the desktop crate, as adding a project does ([ADR-0004](../adr/0004-keep-system-access-in-the-rust-core.md)).
- Must not change: messages without images, the requests they make, pasting text into the message box, mentions, and what the model reads of a message's text.

## Open questions

None.

## Decisions

- 2026-10-17: the owner decided:
  - Images is a plugin, and its images come from a button with the file dialog, a paste and a drop.
  - Mannie shrinks images before sending them, to sizes like the ones that ChatGPT and Claude use.
  - Every model receives the images, and a model that cannot read them fails with the provider's error. There is no switch on provider profiles.
  - Images is M7, which ships together with M5 and M6 in v0.3.0.
  - Images starts off, like every plugin.
  - The model never reads the name of an image.
  - Approving the plan accepts ADR-0033 (D31).
- 2026-10-17: the defaults below were chosen when shaping, and wait for the owner's review:
  - A file is at most 20 MiB, an image at most 12,000 pixels on each side and 64 megapixels.
  - The longer side becomes at most 2048 pixels, which OpenAI's detailed reading also uses. The image is at most 5 MiB, which Anthropic's API also takes.
  - A thumbnail is at most 320 pixels on its longer side.
  - A message has at most 10 images.
  - The plugin keeps at most 40 waiting images, and 160 MiB of them, and drops the oldest first.

## Design

### The plugin

`plugins/images/crates/mannie-images` defines the plugin `images`:

- It has no store module and needs no grant. Its default is off.
- Its start contributes the image source `images`, and its stop drops every waiting image.
- Preparing an image: check the size, guess the format from the first bytes, read the dimensions from the header, decode with the `image` crate's limits, turn it upright by its EXIF orientation, convert it to 8 bits for each channel, shrink it with a Lanczos filter, and encode the image and its thumbnail. One image is decoded at a time, on a blocking thread.
- Waiting images live in memory, in the order they were added, with at most 40 entries and 160 MiB; adding one drops the oldest beyond that. A waiting image leaves when the owner removes its chip, when the message that holds it is stored, or when the plugin stops.

### Contracts

- The core's commands:
  - `send_message` and `start_conversation` gain `images`, a list of references with a `source` and a `key`, which is empty when left out.
  - `read_image` takes the conversation, the part and the size, `thumbnail` or `full`, and returns the format and the bytes as base64.
  - Errors: `message_empty` for a message without text or images, `too_many_images` with the limit, `image_unavailable` with the source, `image_refused` with the source and its reason, and `image_not_found` from `read_image`.
- The plugin's commands, which return `plugin_off` while Images is off:
  - `pick_images` opens the file dialog with the filter's name from the catalog and returns each chosen file with an ID and its name, and keeps its path in the core; `prepare_picked_image` prepares one of them.
  - `prepare_image` prepares the bytes of a pasted or dropped file, as base64, with its name.
  - `discard_prepared_image` drops a waiting image.
  - A prepared image has its key, name, format, width, height, size and thumbnail.
  - Errors: `too_large`, `too_wide` and `too_many_pixels` with their limits, `cannot_shrink` with the largest image that Mannie sends, `unsupported_format`, `unreadable`, `file_unreadable`, `not_found`, `dialog_failed`, `filter_name_invalid` and `store_unavailable`.
- The source refuses a key with `not_found`.
- The part `image` holds the name, the format, the width, the height and the size in bytes.

### Data

- The runtime module's migration 29 creates `runtime_images`: a row for each part `image`, with its conversation, the image's format and bytes, and the thumbnail's format and bytes. Deleting the part or the conversation deletes the row.
- An image is a row of `runtime_parts` with the kind `image` and no run, at the positions after the owner's text and mentions, written in the transaction that starts the turn. Its document holds the source, the name, the format, the dimensions, the size and the SHA-256 digest of the image.

### Requests

| API              | An image in the owner's message                                                                         |
| ---------------- | ------------------------------------------------------------------------------------------------------- |
| Responses        | `{"type": "input_image", "image_url": "data:image/png;base64,…"}` after the message's `input_text`      |
| Chat Completions | `{"type": "image_url", "image_url": {"url": "data:image/png;base64,…"}}` in the message's content array |

A Chat Completions message that holds only text keeps its content as a string. The context estimate counts an image as its width times its height divided by 750 tokens.

Each request sends every image of the conversation again, as base64. Ten images of 5 MiB make about 67 MB, and the request has 120 seconds to upload and get its first answer. Providers set their own limits on a request's size and its number of images, and their errors show as they are.

### The text the model sees

| Case                                 | What the model reads                                                 |
| ------------------------------------ | -------------------------------------------------------------------- |
| An image whose bytes are missing     | "[The owner attached an image here that Mannie can no longer read.]" |
| An image in a summary of the history | "Owner's image: {width}×{height} pixels."                            |

### Interface

- The core's slot `imageSources` takes the button, the files that a source takes, the files of a paste or a drop, and the words of its refusal codes. The message box keeps the images of its message, as chips above the mention chips.
- The part `image` has the core's renderer: a thumbnail that opens the image in a dialog.
- Messages: `images_plugin_name`, `images_plugin_description`, `images_attach`, `images_dialog_filter`, and the plugin's errors under `images_error_`; the core's words of the message box and the transcript under `chat_image`, and `chat_error_too_many_images`, `chat_error_image_unavailable`, `chat_error_image_refused` and `chat_error_image_not_found`.

### Loading, empty and error states

| View                          | Loading                                  | Empty          | Error                                                      |
| ----------------------------- | ---------------------------------------- | -------------- | ---------------------------------------------------------- |
| A chip in the message box     | A spinner on the chip                    | Not applicable | The chip leaves, and a line below the box says what failed |
| A thumbnail in the transcript | A placeholder of the image's proportions | Not applicable | "The image could not be loaded." with "Try again"          |
| The full image                | A spinner in the dialog                  | Not applicable | "The image could not be loaded." with "Try again"          |

### Threat notes

- A hostile image can try to exhaust memory or crash a decoder. The plugin refuses a file larger than 20 MiB before reading it, reads the dimensions from the header before decoding, decodes with the `image` crate's limits on dimensions and memory, decodes one image at a time, and uses decoders written in Rust (T-32).
- Photos carry metadata such as the place they were taken. Encoding the image again drops it, so the provider never receives it (T-33).
- The webview is untrusted. Script in it can send bytes it already holds as an image, which the plugin prepares like any paste, but paths never cross IPC, and the file dialog needs the owner (T-34). The core checks what a source returns, so a source that misbehaves cannot store anything but a PNG or a JPEG within the limits.
- Text in an image is content that the model reads, and grants no permission, as with any content ([ADR-0013](../adr/0013-journal-every-side-effect.md), decision 8).
- Deleting a conversation deletes its images through the store's `secure_delete`, and a test searches the database and its write-ahead log for their bytes.
- The plugin logs names and sizes only at `debug`, and never the bytes.

### Platform differences

- On Windows the file dialog is the system's, and on Linux the desktop portal's or GTK's, as for project folders.
- WebView2 and WebKitGTK deliver pasted and dropped files to the page once Tauri's own drop handling is off.

## Acceptance

- The plugin prepares PNG, JPEG, GIF and WebP files; turns images upright; drops metadata; shrinks large images and keeps small ones; falls back to JPEG for large PNGs; and refuses large files, huge dimensions, truncated files and files that are not images (unit tests in `plugins/images/crates/mannie-images/src/`). Waiting images leave as described, and the plugin's commands say when it is off (`plugins/images/crates/mannie-images/tests/plugin.rs`).
- The adapters send an image after the text, send a message with only an image, and keep requests without images as they were (unit tests in `crates/mannie-provider/src/`).
- A turn stores its images after its mentions, `turn_started` carries their views without bytes, `read_image` reads only the images of its conversation, and deleting a conversation leaves none of their bytes (store tests).
- The runtime tests (`crates/mannie-runtime/tests/images.rs`) show that an image reaches the model after the text and mentions; a message may have no text with an image; an 11th image, an unknown source and a refused key store nothing; the source releases its images once the turn is stored; a retry sends them again; they still reach the model after the plugin stops; and a new conversation takes its title from the image.
- The slot, the message box's images, the paste and drop rules and the plugin's words pass their Vitest tests.
- The end-to-end test (`tests/e2e/specs/images.test.ts`) turns Images on, attaches images with the button, a paste and a drop, sends them, finds them shrunk in the model's request and as thumbnails in the transcript, opens one at full size, and checks that pasting text still pastes text and that turning Images off keeps the stored images. Axe finds no WCAG 2.1 AA violation with the chips and the dialog open.
