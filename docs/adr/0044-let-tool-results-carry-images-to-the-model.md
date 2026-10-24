# ADR-0044: Let tool results carry images to the model

|          |                                                                                                                                                                                                                                                                                                                                                                                                                              |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D42)                                                                                                                                                                                                                                                                                                                                                                                                               |
| Date     | 2026-10-24                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Related  | [ADR-0011](0011-own-the-provider-adapters.md), [ADR-0033](0033-let-plugins-attach-images-to-the-owners-messages.md), [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), [ADR-0040](0040-export-complete-conversation-records.md), [ADR-0043](0043-let-plugins-offer-the-tools-of-tool-servers.md), [Tools](../architecture/tools.md), [Security](../architecture/security.md), [Images](../../plugins/images/docs/README.md) |

## Context

- The owner wants the images that MCP tools return, such as screenshots and charts, to reach the model ([decision D42](../roadmap/milestones.md#5-decisions)).
- A tool's result is text today: the SDK's `ToolResult`, the core's `Done`, the stored result and the provider's `Content::ToolResult` each carry one string. Images reach the model only from the owner's messages (ADR-0033). The table `runtime_images` holds one image for each part, keyed by the part, with a thumbnail that the plugin Images makes.
- The `image` crate belongs to the plugin Images alone, and a plugin cannot depend on another, so neither the core nor the plugin MCP can decode, shrink or re-encode an image with it.
- Facts verified on 2026-10-24:
  - The Responses API takes `function_call_output.output` as a string or as an array of `input_text`, `input_image` and `input_file` items. Chat Completions takes only text in a tool message.
  - The context estimate counts an image by its pixels, about 750 to a token (`crates/mannie-runtime/src/budget.rs`), so an image of 8192 by 8192 pixels would count as about 89,000 tokens.
  - Every request sends again what the context holds, so a long run would send every earlier image again.
  - EXIF and XMP metadata can carry the place where a photo was taken (T-33), and a file system server can return the owner's photos.

## Decision

1. A tool's result may carry images: PNG or JPEG, at most 4 for each result, each at most 5 MiB, at most 4096 pixels on a side and 16.8 million pixels in all. The core reads the format and the size from the file's header without decoding its pixels. An image beyond these limits is left out, and the result says what was left out and why.
2. The core removes metadata by the files' structure, without decoding. A JPEG keeps its frame, tables, scans, color profile and Adobe segment, and loses its comments and every other application segment, such as EXIF, XMP and IPTC. A photo that its EXIF turns gets a new EXIF segment that holds only the orientation, so that it still shows as it was taken. A PNG keeps its critical chunks and the ancillary chunks that change how its pixels look or move, and loses every other chunk, such as text, `eXIf`, time and provenance. Bytes after the end of the image go too.
3. The images are stored in a table of their own, `runtime_tool_images`, by the tool call's part and their position in it, with their format, width and height. A new change, `AddToolImages`, writes them in the transaction that completes the call. They are deleted with their part and their conversation. They get no thumbnail and are never shrunk.
4. The stored result names its images. A document written before them has none, which reads as no images, and a reader of an earlier version ignores the field, so the link's documents of a worker's tools, which return no images, read on both sides.
5. A request sends the images of the 3 newest results that hold images, at most 8 images in all. An older result, a result that the context leaves out, a summary of compaction and a model without tools read one line for each image instead, with its format and its size.
6. The Responses adapter sends a result with images as an array of its text and an `input_image` for each image. The Chat Completions adapter sends the tool messages as text and then one user message with the images of that step's results, each introduced by the call it came from. If a server refuses images in tool output, the provider's error fails the request, as for the owner's images (D31).
7. The context estimate counts each image as the provider reads it, scaled down to 2048 pixels on its longer side. The call's details in the transcript show its images through a command that reads them by part and position, and the conversation export writes them as files beside the call (refines ADR-0040 d1).

Earlier decisions that this ADR changes:

| ADR  | Decision | Change                                                                                                                                            |
| ---- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0011 | d1       | Refined: the adapters send the images of tool results, in the placement that each interface takes                                                 |
| 0033 | d4, d5   | Refined: a tool's images are stored apart from the owner's, without thumbnails, and their metadata is removed by structure instead of re-encoding |
| 0034 | d6       | Refined: a plugin tool's result may carry images                                                                                                  |
| 0040 | d1       | Refined: a conversation's record holds the images of its tool calls                                                                               |

## Consequences

- Positive: a screenshot, a chart or a photo that a tool returns reaches the model, and the owner sees what the model saw. The core's own tools can return images later through the same path.
- Negative: images cost tokens and room in the database. A large image goes as it is, since nothing shrinks it. A model that cannot read images fails with its provider's error when a tool returns one, so the plugin MCP gives each server a switch for its images.
- Risks: an image can carry text that tries to instruct the model, which is content like any other and grants nothing. A file that hides metadata in a segment that the core keeps reaches the provider with it; the tests cover the segments that cameras and editors write.

## Alternatives considered

| Option                                             | Why not chosen                                                                                         |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Shrinking and re-encoding with the `image` crate   | It belongs to the plugin Images, and moving it into the core would change ADR-0033                     |
| Storing tool images in `runtime_images`            | Its key allows one image for each part, and it requires a thumbnail                                    |
| Sending every image of the conversation each time  | A long run would send the same images again and again                                                  |
| Images in a user message for the Responses API too | The Responses API takes them in the result itself, which keeps them beside the call that produced them |

## Validation

M12:

- Unit tests of the header reader and the removal of metadata with a corpus of PNG and JPEG files, among them files with EXIF, XMP and text chunks.
- Store tests: the images of a call are written with it, read back in order, and deleted with their conversation, with no bytes of a planted image left in the database or its write-ahead log.
- Provider tests of both request bodies, and fixtures recorded from the Responses API and one Chat Completions server.
- Runtime tests: a result's images reach the request, older ones become lines, and the estimate counts them.
